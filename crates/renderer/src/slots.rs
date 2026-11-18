//! The render slot table (architecture section 12.4, D49): which record of the persistent
//! instance buffer each instance holds, keyed by the kind of its world and its
//! `StableKey`. The table is the allocator; the GPU buffer is the storage.
//!
//! It is a side table of the renderer's, never a column: no world holds a slot, no system
//! is declared for it, and an `EntityId` never reaches it. It changes only inside
//! [`Renderer::prepare`](crate::Renderer::prepare), from what the writers handed over
//! (`README.md`, "Slots").

use lactose_asset_api::AssetRef;
use lactose_core::StableKey;
use lactose_ecs::WorldKind;
use lactose_present_api::MeshAsset;

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
        todo!()
    }

    /// One more than the highest slot an instance holds, or zero when none does: the
    /// instance count of the triangle pass's draw, which skips the freed slots below it.
    #[must_use]
    pub fn extent(&self) -> u32 {
        todo!()
    }

    /// The slot of the instance `key` in a world of `kind`, or `None` if the renderer
    /// holds no such instance.
    #[must_use]
    pub fn get(&self, kind: WorldKind, key: StableKey) -> Option<Slot> {
        todo!()
    }

    /// The instance that holds `slot`, or `None` for a free slot and for one past the
    /// capacity.
    #[must_use]
    pub fn holder(&self, slot: Slot) -> Option<SlotHolder> {
        todo!()
    }

    /// Every held slot with its holder, in ascending slot order.
    #[must_use]
    pub const fn iter(&self) -> SlotIter<'_> {
        SlotIter { table: self, next: 0 }
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
        todo!()
    }
}
