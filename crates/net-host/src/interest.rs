//! Per-client interest (architecture section 9.3): the relevance set, a priority
//! accumulator per relevant entity, a per-entity acked change tick, the connection's
//! handles and its packet ring, as the types the gather's frame and the packer share. The
//! packer itself (filling each packet by priority up to the client's cap, co-located
//! clients sharing per-region dirty lists) is `lactose-net`'s, milestone 4.
//!
//! Host-only: owned by the server's packing thread.

use std::collections::{BTreeMap, BTreeSet};

use lactose_core::{PlayerId, Result, StableKey, Tick};
use lactose_ecs::RegionId;
use lactose_net_api::{Tombstone, TombstoneCause};

use crate::packet::{
    AckField, ConnectionHandles, EntityHandle, PacketRecord, PacketRing, PacketSequence,
};
use crate::transport::{ConnectionId, MAX_DATAGRAM_BYTES};

/// The deployment's relevance rule: region membership within a radius of the player's
/// region, measured in regions as `RegionCell::distance` is, plus an always-relevant set.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RelevancePolicy {
    /// A region within this many regions of the player's is relevant.
    pub radius: u32,
    /// Entities relevant to every client wherever they are (a world boss, a quest
    /// object).
    pub always: BTreeSet<StableKey>,
}

/// A relevant entity's send priority, accumulated per frame from distance and staleness
/// and reset when the entity is sent, so an entity starved by the cap rises until it goes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct Priority(pub u32);

impl Priority {
    /// Nothing accumulated.
    pub const ZERO: Self = Self(0);

    /// Adds `weight`, saturating.
    pub fn accumulate(&mut self, weight: u32) {
        self.0 = self.0.saturating_add(weight);
    }

    /// Back to zero: the entity was sent.
    pub fn reset(&mut self) {
        self.0 = 0;
    }
}

/// One relevant entity, as the packer tracks it for one client.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EntityInterest {
    /// The entity's handle on this connection.
    pub handle: EntityHandle,
    /// Its send priority.
    pub priority: Priority,
    /// The newest tick of its state the client has acknowledged, or `None` until the
    /// first acknowledgement: the entity is then sent whole, every replicated component
    /// from the packer's mirror, until one is acknowledged, and by the rows changed since
    /// `acked` afterwards. An entity the receiving player has claimed is sent whole in
    /// every packet that carries it (architecture section 9.4), whatever `acked` says.
    pub acked: Option<Tick>,
    /// The packet its newest state went out in and is not yet acknowledged, if any; a
    /// loss of that packet clears it so the entity is sent again.
    pub in_flight: Option<PacketSequence>,
    /// The tombstone owed, once the entity was destroyed or left the client's set; the
    /// entity is forgotten (or re-enters) once it is acknowledged.
    pub tombstone: Option<Tombstone>,
    /// Whether the receiving player has claimed the entity, as
    /// [`ClientInterest::set_claimed`] last said.
    pub claimed: bool,
    /// Whether the entity became relevant again while its tombstone was owed: once the
    /// tombstone is acknowledged it re-enters under a new handle instead of being
    /// forgotten.
    pub reentering: bool,
}

impl EntityInterest {
    fn entered(handle: EntityHandle, claimed: bool) -> Self {
        Self {
            handle,
            priority: Priority::ZERO,
            acked: None,
            in_flight: None,
            tombstone: None,
            claimed,
            reentering: false,
        }
    }

    /// Whether a packet that carries the entity's state carries it whole, every
    /// replicated component from the mirror: when nothing of it is acknowledged, and in
    /// every packet while the receiving player has it claimed. A whole send of an entity
    /// whose state is not acknowledged travels as a spawn, which the receiver stages as an
    /// upsert if the entity is live.
    #[must_use]
    pub const fn sends_whole(&self) -> bool {
        self.claimed || self.acked.is_none()
    }
}

/// The per-client bandwidth cap (architecture section 9.3).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SendBudget {
    /// The most payload bytes one packet carries, at most
    /// [`MAX_DATAGRAM_BYTES`].
    pub bytes_per_packet: u32,
    /// The send rate, twenty to thirty.
    pub packets_per_second: u16,
}

impl Default for SendBudget {
    fn default() -> Self {
        Self { bytes_per_packet: MAX_DATAGRAM_BYTES as u32 - 100, packets_per_second: 30 }
    }
}

/// Everything the server keeps per connected client to replicate to it.
#[derive(Clone, Debug)]
pub struct ClientInterest {
    /// The player.
    pub player: PlayerId,
    /// The connection.
    pub connection: ConnectionId,
    /// The cap.
    pub budget: SendBudget,
    /// The regions relevant to the client now; a region entering is sent as a
    /// [`RegionRelevance`](lactose_net_api::RegionRelevance), one leaving as
    /// `RegionChange::Irrelevant`.
    pub regions: BTreeSet<RegionId>,
    /// The regions the client has acknowledged as ready (architecture section 14); the
    /// server holds the player at the boundary of a relevant region not in here.
    pub ready: BTreeSet<RegionId>,
    /// The relevance set, by origin key, with each entity's state.
    pub entities: BTreeMap<StableKey, EntityInterest>,
    /// The connection's handle table, the server's copy.
    pub handles: ConnectionHandles,
    /// The packets in flight.
    pub packets: PacketRing,
}

impl ClientInterest {
    /// A client with nothing relevant yet.
    #[must_use]
    pub fn new(player: PlayerId, connection: ConnectionId, budget: SendBudget) -> Self {
        Self {
            player,
            connection,
            budget,
            regions: BTreeSet::new(),
            ready: BTreeSet::new(),
            entities: BTreeMap::new(),
            handles: ConnectionHandles::new(),
            packets: PacketRing::new(PacketRing::DEFAULT_CAPACITY),
        }
    }

    /// Makes `key` relevant: assigns a handle and an [`EntityInterest`] with nothing
    /// acknowledged, so the entity is sent whole. An entity already relevant keeps its
    /// state. An entity whose tombstone is still owed is marked
    /// [`reentering`](EntityInterest::reentering) and keeps its handle until the
    /// tombstone is acknowledged, when it re-enters under a new one: the client releases
    /// the old handle when it stages the tombstone, so the old handle must never name the
    /// entity again. Returns the entity's current handle. Fails as
    /// `ConnectionHandles::assign` does.
    pub fn enter(&mut self, key: StableKey) -> Result<EntityHandle> {
        if let Some(entity) = self.entities.get_mut(&key) {
            if entity.tombstone.is_some() {
                entity.reentering = true;
            }
            return Ok(entity.handle);
        }
        let handle = self.handles.assign(key)?;
        self.entities.insert(key, EntityInterest::entered(handle, false));
        Ok(handle)
    }

    /// Owes the client `tombstone` for `key`: the entity was destroyed, or left the
    /// client's set alive in `tombstone.region`; the handle is released once the
    /// tombstone is acknowledged. Clears `in_flight`, so the tombstone goes out with the
    /// next packet. A key that is not relevant is ignored. One whose tombstone is already
    /// owed stops [`reentering`](EntityInterest::reentering); `tombstone` replaces an owed
    /// [`Left`](TombstoneCause::Left) one, so the tombstone owed names where the entity is
    /// now (a client whose hook kept the entity for the first despawns it at the second,
    /// which it stages under the handle the first released), and an owed
    /// [`Destroyed`](TombstoneCause::Destroyed) one is kept.
    pub fn leave(&mut self, key: StableKey, tombstone: Tombstone) {
        let Some(entity) = self.entities.get_mut(&key) else { return };
        entity.reentering = false;
        let replaces = entity.tombstone.is_none_or(|owed| owed.cause == TombstoneCause::Left);
        if replaces {
            entity.tombstone = Some(tombstone);
            entity.in_flight = None;
        }
    }

    /// Records whether the receiving player has claimed `key`. A change is owed to the
    /// client as a spawn of the live entity (its interpolation slot set or removed), so it
    /// voids what was acknowledged, as [`resend_whole`](Self::resend_whole) does. A key
    /// that is not relevant is ignored.
    pub fn set_claimed(&mut self, key: StableKey, claimed: bool) {
        let Some(entity) = self.entities.get_mut(&key) else { return };
        if entity.claimed != claimed {
            entity.claimed = claimed;
            self.resend_whole(key);
        }
    }

    /// Owes the client a whole send of `key`, as if nothing of it were acknowledged: what
    /// the packer does when the entity moves between two regions the client has, so an
    /// entity the client could not spawn arrives whole beside its move. Every record in
    /// the ring stops counting for the entity, so an acknowledgement of a packet sent
    /// before this call never marks it acknowledged again. A key that is not relevant is
    /// ignored.
    pub fn resend_whole(&mut self, key: StableKey) {
        let Some(entity) = self.entities.get_mut(&key) else { return };
        entity.acked = None;
        entity.in_flight = None;
        self.packets.forget_states(entity.handle);
    }

    /// Records a packet the packer sent: each entity and tombstone it carried has its
    /// `in_flight` set to the packet and its priority reset, and the record enters the
    /// ring. When the ring is full its oldest record is evicted and treated as lost, as
    /// an expired one is.
    pub fn sent(&mut self, record: PacketRecord) {
        let handles = record.entities.iter().map(|&(handle, _)| handle);
        for handle in handles.chain(record.tombstones.iter().map(|&(handle, _)| handle)) {
            if let Some(entity) =
                self.handles.key_of(handle).and_then(|key| self.entities.get_mut(&key))
            {
                entity.in_flight = Some(record.sequence);
                entity.priority.reset();
            }
        }
        if let Some(evicted) = self.packets.record(record) {
            self.lost(&evicted);
        }
    }

    /// Applies the peer's acknowledgement field: every packet it covers is taken from the
    /// ring and each entity it carried advances its `acked` to the state's tick if newer
    /// and clears `in_flight` if it names that packet; records older than the field's
    /// window are expired as lost, clearing `in_flight` so their entities are sent again;
    /// an entity whose tombstone is acknowledged is forgotten and its handle released, or
    /// re-entered under a new handle if it is [`reentering`](EntityInterest::reentering).
    pub fn acknowledge(&mut self, ack: AckField) {
        let mut sequence = ack.latest;
        for _ in 0..=AckField::WINDOW {
            if ack.covers(sequence)
                && let Some(record) = self.packets.acknowledge(sequence)
            {
                self.delivered(&record);
            }
            sequence = PacketSequence(sequence.0.wrapping_sub(1));
        }
        let oldest_live = PacketSequence(ack.latest.0.wrapping_sub(AckField::WINDOW));
        for record in self.packets.expire(oldest_live) {
            self.lost(&record);
        }
    }

    /// The server's side of a resync: the handles and the packet ring start over, every
    /// entity whose tombstone is owed is forgotten unless it is
    /// [`reentering`](EntityInterest::reentering), and every other entity re-enters under
    /// a fresh handle, assigned in ascending key order, with its claim kept and `acked`
    /// at `tick`, the tick of the full frame, which carries each of them whole on the
    /// reliable channel. The full frame names exactly the entities left in `entities`.
    pub fn resync(&mut self, tick: Tick) {
        self.handles.clear();
        self.packets.clear();
        self.entities.retain(|_, entity| entity.tombstone.is_none() || entity.reentering);
        for (&key, entity) in &mut self.entities {
            let handle =
                self.handles.assign(key).expect("no more entities than handles held before");
            *entity = EntityInterest {
                acked: Some(tick),
                ..EntityInterest::entered(handle, entity.claimed)
            };
        }
    }

    fn delivered(&mut self, record: &PacketRecord) {
        for &(handle, tick) in &record.entities {
            let Some(entity) =
                self.handles.key_of(handle).and_then(|key| self.entities.get_mut(&key))
            else {
                continue;
            };
            if entity.acked.is_none_or(|acked| tick_after(tick, acked)) {
                entity.acked = Some(tick);
            }
            if entity.in_flight == Some(record.sequence) {
                entity.in_flight = None;
            }
        }
        for &(handle, tombstone) in &record.tombstones {
            let Some(key) = self.handles.key_of(handle) else { continue };
            let Some(entity) = self.entities.get_mut(&key) else { continue };
            if entity.tombstone != Some(tombstone) {
                continue;
            }
            self.handles.release(handle);
            if entity.reentering {
                let handle = self.handles.assign(key).expect("a handle was just released");
                *entity = EntityInterest::entered(handle, entity.claimed);
            } else {
                self.entities.remove(&key);
            }
        }
    }

    fn lost(&mut self, record: &PacketRecord) {
        let handles = record.entities.iter().map(|&(handle, _)| handle);
        for handle in handles.chain(record.tombstones.iter().map(|&(handle, _)| handle)) {
            if let Some(entity) =
                self.handles.key_of(handle).and_then(|key| self.entities.get_mut(&key))
                && entity.in_flight == Some(record.sequence)
            {
                entity.in_flight = None;
            }
        }
    }
}

/// Whether `tick` is after `than` under the wrapping comparison the frames use.
const fn tick_after(tick: Tick, than: Tick) -> bool {
    let distance = tick.0.wrapping_sub(than.0);
    distance != 0 && distance < 1 << 31
}
