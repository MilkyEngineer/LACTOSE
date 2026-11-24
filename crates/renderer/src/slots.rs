//! The render slot table (architecture section 12.4, D49): which record of the persistent
//! instance buffer each instance holds, keyed by the kind of its world and its
//! `StableKey`. The table is the allocator; the GPU buffer is the storage.
//!
//! It is a side table of the renderer's, never a column: no world holds a slot, no system
//! is declared for it, and an `EntityId` never reaches it. It changes only inside
//! [`Renderer::prepare`](crate::Renderer::prepare), from what the writers handed over
//! (`README.md`, "Slots").

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use lactose_asset_api::AssetRef;
use lactose_core::StableKey;
use lactose_ecs::WorldKind;
use lactose_present_api::MeshAsset;

use crate::key::KeyBits;

/// A render slot: the index of one instance's record in the persistent instance buffer.
///
/// A slot means something only while an instance holds it: a freed slot is given to the
/// next new instance. A slot is never sent to a simulation and never stored in a world.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Slot(u32);

impl Slot {
    /// The slot of the record at `index`. The table's plumbing, and a test's way to name
    /// a record it read back: the table hands every slot it allocates to its caller.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// The index of the slot's record in the instance buffer.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// The instance that holds a slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SlotHolder {
    /// The kind of the world the instance is in.
    pub kind: WorldKind,
    /// The instance's key in that world.
    pub key: StableKey,
    /// The mesh the instance's newest row named. The triangle pass of milestone 2 draws
    /// one built-in triangle for every instance and does not read it.
    pub mesh: AssetRef<MeshAsset>,
}

/// The slot table: every instance the renderer holds, and the slot of each.
///
/// Read through [`Renderer::slots`](crate::Renderer::slots). Every method is a read; the
/// table changes inside [`Renderer::prepare`](crate::Renderer::prepare) and
/// [`Renderer::torn`](crate::Renderer::torn) alone, so what it returns is the state after
/// the last of those calls, and it is the same on every run that handed the renderer the
/// same passes (`README.md`, "Determinism").
pub struct SlotTable {
    /// The most slots the table gives out: the records of the instance buffer.
    capacity: u32,
    /// One for every slot index the table has ever given, by index: slots are given lowest
    /// first, so these are the indices below the length.
    records: Vec<SlotRecord>,
    /// The slot of each instance held.
    index: HashMap<(WorldKind, KeyBits), u32>,
    /// The freed indices below `records.len()`, lowest first.
    free: BinaryHeap<Reverse<u32>>,
    /// One more than the highest held slot.
    extent: u32,
}

/// What the table keeps for one slot index.
pub(crate) struct SlotRecord {
    /// The instance that holds it.
    pub(crate) holder: Option<SlotHolder>,
    /// The epoch of its kind ([`crate::frame`]'s base rule) in which its instance took it.
    pub(crate) born: u64,
    /// The epoch of its kind in which its instance's base was taken, if one was.
    pub(crate) base: Option<u64>,
}

impl SlotTable {
    /// The most instances the table holds: `RendererDesc::instances`.
    #[must_use]
    pub const fn capacity(&self) -> u32 {
        self.capacity
    }

    /// How many instances hold a slot.
    #[must_use]
    pub fn live(&self) -> u32 {
        // At most `capacity` instances hold a slot.
        self.index.len() as u32
    }

    /// One more than the highest slot an instance holds, or zero when none does: the
    /// instance count of the triangle pass's draw, which skips the freed slots below it.
    #[must_use]
    pub fn extent(&self) -> u32 {
        self.extent
    }

    /// The slot of the instance `key` in a world of `kind`, or `None` if the renderer
    /// holds no such instance.
    #[must_use]
    pub fn get(&self, kind: WorldKind, key: StableKey) -> Option<Slot> {
        self.index.get(&(kind, KeyBits::of(key))).copied().map(Slot)
    }

    /// The instance that holds `slot`, or `None` for a free slot and for one past the
    /// capacity.
    #[must_use]
    pub fn holder(&self, slot: Slot) -> Option<SlotHolder> {
        self.records.get(slot.0 as usize).and_then(|record| record.holder)
    }

    /// Every held slot with its holder, in ascending slot order.
    #[must_use]
    pub const fn iter(&self) -> SlotIter<'_> {
        SlotIter { table: self, next: 0 }
    }

    /// An empty table of `capacity` slots.
    pub(crate) fn new(capacity: u32) -> Self {
        Self {
            capacity,
            records: Vec::new(),
            index: HashMap::new(),
            free: BinaryHeap::new(),
            extent: 0,
        }
    }

    /// How many slot indices the table has ever given: every index below this.
    pub(crate) fn given(&self) -> u32 {
        // At most `capacity` records.
        self.records.len() as u32
    }

    /// The slot of `key` in `kind`, from its number.
    pub(crate) fn lookup(&self, kind: WorldKind, key: KeyBits) -> Option<u32> {
        self.index.get(&(kind, key)).copied()
    }

    /// The record of a slot index the table has given.
    pub(crate) fn record_mut(&mut self, slot: u32) -> &mut SlotRecord {
        &mut self.records[slot as usize]
    }

    /// The records of every index the table has given, by index.
    pub(crate) fn records(&self) -> &[SlotRecord] {
        &self.records
    }

    /// Gives the lowest free slot to `holder`, born in `epoch`, or `None` when every slot
    /// is held.
    pub(crate) fn allocate(&mut self, holder: SlotHolder, epoch: u64) -> Option<u32> {
        let slot = match self.free.pop() {
            Some(Reverse(slot)) => slot,
            None if self.given() < self.capacity => {
                self.records.push(SlotRecord { holder: None, born: 0, base: None });
                self.given() - 1
            }
            None => return None,
        };
        let previous = self.index.insert((holder.kind, KeyBits::of(holder.key)), slot);
        assert!(previous.is_none(), "an instance holds two slots");
        self.records[slot as usize] = SlotRecord { holder: Some(holder), born: epoch, base: None };
        self.extent = self.extent.max(slot + 1);
        Some(slot)
    }

    /// Frees a held slot.
    pub(crate) fn release(&mut self, slot: u32) {
        let record = &mut self.records[slot as usize];
        let holder = record.holder.take().expect("a freed slot was held");
        record.base = None;
        self.index.remove(&(holder.kind, KeyBits::of(holder.key)));
        self.free.push(Reverse(slot));
        while self.extent > 0 && self.records[self.extent as usize - 1].holder.is_none() {
            self.extent -= 1;
        }
    }
}

/// The held slots of a [`SlotTable`], in ascending slot order.
#[derive(Clone)]
pub struct SlotIter<'a> {
    /// The table walked.
    table: &'a SlotTable,
    /// The next slot to look at.
    next: u32,
}

impl Iterator for SlotIter<'_> {
    type Item = (Slot, SlotHolder);

    fn next(&mut self) -> Option<Self::Item> {
        while self.next < self.table.extent {
            let slot = self.next;
            self.next += 1;
            if let Some(holder) = self.table.records[slot as usize].holder {
                return Some((Slot(slot), holder));
            }
        }
        None
    }
}
