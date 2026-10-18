//! Packet-level framing (architecture section 9.3), as types only: the short
//! per-connection entity handle and the table that issues it, the packet sequence and the
//! acknowledgement field, the tick header, and the ring of packet-to-entity records that
//! lets an acknowledgement advance exactly the entities a packet carried. The byte
//! encoding is `lactose-net`'s, milestone 4; the shapes are fixed here.
//!
//! Host-only: built and consumed by the packer and the network thread.

use std::collections::{BTreeMap, VecDeque, btree_map};

use bytemuck::{Pod, Zeroable};
use lactose_core::{Error, ErrorKind, Result, StableKey, Tick};
use lactose_net_api::Tombstone;

/// An entity as one connection names it on the wire: a sixteen-bit slot index and the
/// generation the slot was assigned at, so a slot reused after release is told apart
/// from its previous entity. Four bytes of plain data; the all-zero value is
/// [`NONE`](Self::NONE), since a generation is never zero.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default, Pod, Zeroable)]
#[repr(C)]
pub struct EntityHandle {
    /// The slot.
    pub index: u16,
    /// The generation, from one.
    pub generation: u16,
}

impl EntityHandle {
    /// No entity.
    pub const NONE: Self = Self { index: 0, generation: 0 };

    /// How many entities one connection can name at once.
    pub const CAPACITY: usize = 1 << 16;

    /// Whether this is [`NONE`](Self::NONE).
    #[must_use]
    pub const fn is_none(self) -> bool {
        self.generation == 0
    }
}

/// One slot of a [`ConnectionHandles`]. Generation zero marks a slot this table has never
/// assigned or recorded, whose next generation is whatever arrives (the mirror) or one
/// (the server).
#[derive(Clone, Copy, Debug)]
struct HandleSlot {
    generation: u16,
    key: Option<StableKey>,
}

impl HandleSlot {
    const UNUSED: Self = Self { generation: 0, key: None };
}

/// The generation after `generation`, skipping zero.
const fn next_generation(generation: u16) -> u16 {
    if generation == u16::MAX { 1 } else { generation + 1 }
}

/// The handles one connection has assigned, kept identically by the server, which
/// [`assign`](Self::assign)s, and the client, which mirrors with [`insert`](Self::insert)
/// from the handle every spawn on the wire carries and [`release`](Self::release) from
/// tombstones, and never assigns (the two ends release at different times and packets
/// reorder, so an assigning mirror would drift). At most [`EntityHandle::CAPACITY`] at
/// once; the relevance cap keeps it far below.
#[derive(Clone, Debug, Default)]
pub struct ConnectionHandles {
    slots: Vec<HandleSlot>,
    free: Vec<u16>,
    by_key: BTreeMap<StableKey, EntityHandle>,
}

impl ConnectionHandles {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The handle of `key`, assigning one if it has none: the most recently released
    /// slot, else the next never-used slot, with the slot's current generation. Fails with
    /// `ResourceExhausted` when every slot is assigned. The server's side.
    pub fn assign(&mut self, key: StableKey) -> Result<EntityHandle> {
        if let Some(&handle) = self.by_key.get(&key) {
            return Ok(handle);
        }
        // A slot on the free list may have been filled by `insert` since its release; a
        // table that assigns never inserts, so this skip is for robustness only.
        let index = loop {
            match self.free.pop() {
                Some(index) if self.slots[usize::from(index)].key.is_none() => break index,
                Some(_) => {}
                None if self.slots.len() < EntityHandle::CAPACITY => {
                    self.slots.push(HandleSlot::UNUSED);
                    break u16::try_from(self.slots.len() - 1).expect("below the capacity");
                }
                None => {
                    return Err(Error::new(
                        ErrorKind::ResourceExhausted,
                        "every entity handle of the connection is assigned",
                    ));
                }
            }
        };
        let slot = &mut self.slots[usize::from(index)];
        slot.generation = slot.generation.max(1);
        slot.key = Some(key);
        let handle = EntityHandle { index, generation: slot.generation };
        self.by_key.insert(key, handle);
        Ok(handle)
    }

    /// Records that the peer assigned `handle` to `key`: the client's mirror of the
    /// server's table. The slot takes `handle`'s generation. Recording the same (handle,
    /// key) again is a no-op; it fails with `InvalidData` if the slot holds another live
    /// key, if the slot's generation is newer than `handle`'s (a spawn reordered after the
    /// tombstone that released it), or if `key` has a different handle already. "Newer"
    /// is the wrapping comparison, since generations wrap from `u16::MAX` to one: the
    /// slot's generation is newer than `handle`'s when `slot.wrapping_sub(handle)` is
    /// between one and 32767. The receiver rules of the README make the reorder case
    /// unreachable (a packet is staged only if its tick is after the newest frame staged),
    /// so the error is a defect, not a path.
    pub fn insert(&mut self, handle: EntityHandle, key: StableKey) -> Result<()> {
        if handle.is_none() {
            return Err(Error::new(ErrorKind::InvalidData, "a spawn names the empty handle"));
        }
        if let Some(&held) = self.by_key.get(&key) {
            return if held == handle {
                Ok(())
            } else {
                Err(Error::new(ErrorKind::InvalidData, "the key already has another handle"))
            };
        }
        let index = usize::from(handle.index);
        if self.slots.len() <= index {
            self.slots.resize(index + 1, HandleSlot::UNUSED);
        }
        let slot = &mut self.slots[index];
        if slot.key.is_some() {
            return Err(Error::new(ErrorKind::InvalidData, "the slot holds another live key"));
        }
        let ahead = slot.generation.wrapping_sub(handle.generation);
        if slot.generation != 0 && (1..=i16::MAX.unsigned_abs()).contains(&ahead) {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "the slot's generation is newer than the spawn's handle",
            ));
        }
        slot.generation = handle.generation;
        slot.key = Some(key);
        self.by_key.insert(key, handle);
        Ok(())
    }

    /// Forgets every assignment and every generation: what the client does when it stages
    /// a full frame, whose spawns carry fresh handles, and only then. From the resync
    /// request or the join until that frame the receiver stages nothing and drops every
    /// packet whose tick is not after the full frame's, so no handle from before the
    /// resync is looked up after the clear. On a resync the server clears the client's
    /// table the same way and re-enters every relevant entity.
    pub fn clear(&mut self) {
        self.slots.clear();
        self.free.clear();
        self.by_key.clear();
    }

    /// Releases `handle` and returns its key, or `None` if the handle is stale or
    /// unassigned. The slot's generation advances (wrapping from `u16::MAX` to one), so
    /// the released handle never names the slot's next entity.
    pub fn release(&mut self, handle: EntityHandle) -> Option<StableKey> {
        let key = self.key_of(handle)?;
        let slot = &mut self.slots[usize::from(handle.index)];
        slot.key = None;
        slot.generation = next_generation(slot.generation);
        self.by_key.remove(&key);
        self.free.push(handle.index);
        Some(key)
    }

    /// The key `handle` names, or `None` if the handle is stale or unassigned.
    #[must_use]
    pub fn key_of(&self, handle: EntityHandle) -> Option<StableKey> {
        let slot = self.slots.get(usize::from(handle.index))?;
        if handle.is_none() || slot.generation != handle.generation { None } else { slot.key }
    }

    /// The handle of `key`, if assigned.
    #[must_use]
    pub fn handle_of(&self, key: StableKey) -> Option<EntityHandle> {
        self.by_key.get(&key).copied()
    }

    /// Every assigned (key, handle), in ascending key order.
    #[must_use]
    pub fn iter(&self) -> AssignedHandles<'_> {
        AssignedHandles { entries: self.by_key.iter() }
    }

    /// How many handles are assigned.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    /// Whether none is assigned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}

/// Iterates assigned handles in ascending key order.
pub struct AssignedHandles<'a> {
    entries: btree_map::Iter<'a, StableKey, EntityHandle>,
}

impl Iterator for AssignedHandles<'_> {
    type Item = (StableKey, EntityHandle);

    fn next(&mut self) -> Option<Self::Item> {
        self.entries.next().map(|(&key, &handle)| (key, handle))
    }
}

/// A packet's sequence number on one connection in one direction, wrapping.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Pod, Zeroable)]
#[repr(transparent)]
pub struct PacketSequence(pub u16);

impl PacketSequence {
    /// The sequence after this one.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }

    /// Whether this sequence is after `other`: the wrapping difference is positive and
    /// below half the range, so two sequences in play must be less than 32768 apart.
    #[must_use]
    pub const fn after(self, other: Self) -> bool {
        let distance = self.0.wrapping_sub(other.0);
        distance != 0 && distance < 1 << 15
    }

    /// How many sequences after `other` this one is, wrapping; zero if not after.
    #[must_use]
    pub const fn distance_after(self, other: Self) -> u16 {
        if self.after(other) { self.0.wrapping_sub(other.0) } else { 0 }
    }
}

/// Which packets the sender of a header has received: the newest sequence, and one bit
/// for each of the [`WINDOW`](Self::WINDOW) sequences before it, bit `n` meaning
/// `latest - (n + 1)` was received. A field always acknowledges its `latest`, so a peer
/// that has received nothing sends none ([`TickHeader::ack`] is `None`). Since sequences
/// wrap, the bits must come from the sequences received within the window behind
/// `latest` alone, never from a set that remembers an earlier lap;
/// [`received`](Self::received) builds the field that way.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AckField {
    /// The newest sequence received.
    pub latest: PacketSequence,
    /// The sequences before it, as bits.
    pub bits: u32,
}

impl AckField {
    /// How many sequences before `latest` the field covers.
    pub const WINDOW: u16 = 32;

    /// Whether the field acknowledges `sequence`.
    #[must_use]
    pub const fn covers(&self, sequence: PacketSequence) -> bool {
        if sequence.0 == self.latest.0 {
            return true;
        }
        let behind = self.latest.distance_after(sequence);
        behind != 0 && behind <= Self::WINDOW && self.bits & (1 << (behind - 1)) != 0
    }

    /// The field of a peer whose field was `field` once `sequence` arrives. A sequence
    /// after `latest` becomes `latest`, the bits shifting by the distance with the old
    /// `latest` set among them; one within the window behind sets its bit; any other
    /// (a duplicate of `latest`, or older than the window) leaves the field as it was.
    #[must_use]
    pub const fn received(field: Option<Self>, sequence: PacketSequence) -> Self {
        let Some(field) = field else { return Self { latest: sequence, bits: 0 } };
        if sequence.after(field.latest) {
            let ahead = sequence.0.wrapping_sub(field.latest.0);
            let bits = match ahead {
                1..Self::WINDOW => field.bits << ahead | 1 << (ahead - 1),
                Self::WINDOW => 1 << (ahead - 1),
                _ => 0,
            };
            return Self { latest: sequence, bits };
        }
        let behind = field.latest.distance_after(sequence);
        if behind != 0 && behind <= Self::WINDOW {
            return Self { latest: field.latest, bits: field.bits | (1 << (behind - 1)) };
        }
        field
    }
}

/// What every packet begins with, in both directions.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct TickHeader {
    /// This packet's sequence.
    pub sequence: PacketSequence,
    /// The sender's tick when it was built: on the server, the tick its rows are no newer
    /// than; on the client, the newest input tick it carries.
    pub tick: Tick,
    /// What the sender has received from the peer, or `None` before it has received
    /// anything.
    pub ack: Option<AckField>,
}

/// What one sent packet carried, so an acknowledgement advances exactly those entities'
/// acked ticks and a loss re-queues exactly them.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PacketRecord {
    /// The packet.
    pub sequence: PacketSequence,
    /// The server tick the packet's rows were no newer than.
    pub tick: Tick,
    /// Each entity the packet carried state for, with the tick of that state.
    pub entities: Vec<(EntityHandle, Tick)>,
    /// Each tombstone the packet carried: an acknowledgement releases the handle only if
    /// it is the tombstone still owed.
    pub tombstones: Vec<(EntityHandle, Tombstone)>,
}

/// The ring of the last sent packets' records for one connection (architecture section
/// 9.3), in sequence order.
#[derive(Clone, Debug)]
pub struct PacketRing {
    records: VecDeque<PacketRecord>,
    capacity: usize,
}

impl PacketRing {
    /// The capacity `ClientInterest::new` uses: about two seconds at thirty packets per
    /// second.
    pub const DEFAULT_CAPACITY: usize = 64;

    /// An empty ring holding at most `capacity` records.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self { records: VecDeque::with_capacity(capacity), capacity }
    }

    /// Records a sent packet. When the ring is full the oldest record is evicted and
    /// returned, to be treated as lost.
    pub fn record(&mut self, record: PacketRecord) -> Option<PacketRecord> {
        if self.capacity == 0 {
            return Some(record);
        }
        let evicted =
            if self.records.len() == self.capacity { self.records.pop_front() } else { None };
        self.records.push_back(record);
        evicted
    }

    /// Takes the record of `sequence`, if the ring holds it.
    pub fn acknowledge(&mut self, sequence: PacketSequence) -> Option<PacketRecord> {
        let at = self.records.iter().position(|record| record.sequence == sequence)?;
        self.records.remove(at)
    }

    /// Takes every record whose sequence is before `oldest_live`, in sequence order: the
    /// packets an acknowledgement field can no longer cover, treated as lost.
    pub fn expire(&mut self, oldest_live: PacketSequence) -> Vec<PacketRecord> {
        let mut expired = Vec::new();
        while let Some(record) =
            self.records.pop_front_if(|record| oldest_live.after(record.sequence))
        {
            expired.push(record);
        }
        expired
    }

    /// Takes every entry of `handle`'s state out of the held records, tombstones kept:
    /// an acknowledgement of a packet sent before then no longer counts for the entity.
    pub(crate) fn forget_states(&mut self, handle: EntityHandle) {
        for record in &mut self.records {
            record.entities.retain(|&(held, _)| held != handle);
        }
    }

    /// Drops every record: the connection's handles start over.
    pub(crate) fn clear(&mut self) {
        self.records.clear();
    }

    /// How many records the ring holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the ring holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The most records the ring holds.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}
