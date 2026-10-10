//! Packet-level framing (architecture section 9.3), as types only: the short
//! per-connection entity handle and the table that issues it, the packet sequence and the
//! acknowledgement field, the tick header, and the ring of packet-to-entity records that
//! lets an acknowledgement advance exactly the entities a packet carried. The byte
//! encoding is `lactose-net`'s, milestone 4; the shapes are fixed here.
//!
//! Host-only: built and consumed by the packer and the network thread.

use std::collections::{BTreeMap, VecDeque, btree_map};

use bytemuck::{Pod, Zeroable};
use lactose_core::{Result, StableKey, Tick};

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

#[derive(Clone, Copy, Debug)]
struct HandleSlot {
    generation: u16,
    key: Option<StableKey>,
}

/// The handles one connection has assigned, kept identically by the server (which assigns
/// them) and the client (which learns them from spawns and tombstones). At most
/// [`EntityHandle::CAPACITY`] at once; the relevance cap keeps it far below.
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
    /// `ResourceExhausted` when every slot is assigned.
    pub fn assign(&mut self, key: StableKey) -> Result<EntityHandle> {
        todo!()
    }

    /// Releases `handle` and returns its key, or `None` if the handle is stale or
    /// unassigned. The slot's generation advances (wrapping from `u16::MAX` to one), so
    /// the released handle never names the slot's next entity.
    pub fn release(&mut self, handle: EntityHandle) -> Option<StableKey> {
        todo!()
    }

    /// The key `handle` names, or `None` if the handle is stale or unassigned.
    #[must_use]
    pub fn key_of(&self, handle: EntityHandle) -> Option<StableKey> {
        todo!()
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
/// `latest - (n + 1)` was received.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
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
}

/// What every packet begins with, in both directions.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct TickHeader {
    /// This packet's sequence.
    pub sequence: PacketSequence,
    /// The sender's tick when it was built: on the server, the tick its rows are no newer
    /// than; on the client, the newest input tick it carries.
    pub tick: Tick,
    /// What the sender has received from the peer.
    pub ack: AckField,
}

/// What one sent packet carried, so an acknowledgement advances exactly those entities'
/// acked ticks and a loss re-queues exactly them.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PacketRecord {
    /// The packet.
    pub sequence: PacketSequence,
    /// The server tick the packet's rows were no newer than.
    pub tick: Tick,
    /// Each entity the packet carried state for, with the tick of that state.
    pub entities: Vec<(EntityHandle, Tick)>,
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
        todo!()
    }

    /// Takes the record of `sequence`, if the ring holds it.
    pub fn acknowledge(&mut self, sequence: PacketSequence) -> Option<PacketRecord> {
        todo!()
    }

    /// Takes every record whose sequence is before `oldest_live`, in sequence order: the
    /// packets an acknowledgement field can no longer cover, treated as lost.
    pub fn expire(&mut self, oldest_live: PacketSequence) -> Vec<PacketRecord> {
        todo!()
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
