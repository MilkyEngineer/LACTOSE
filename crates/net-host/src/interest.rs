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

use crate::packet::{AckField, ConnectionHandles, EntityHandle, PacketRing, PacketSequence};
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
    /// The tick the entity was destroyed at, once a tombstone is owed; cleared, and the
    /// entity forgotten, once the tombstone is acknowledged.
    pub tombstone: Option<Tick>,
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
    /// state. Fails as `ConnectionHandles::assign` does.
    pub fn enter(&mut self, key: StableKey) -> Result<EntityHandle> {
        todo!()
    }

    /// Owes the client a tombstone for `key`, destroyed at `tick`; the handle is released
    /// once the tombstone is acknowledged. A key that is not relevant is ignored.
    pub fn leave(&mut self, key: StableKey, tick: Tick) {
        todo!()
    }

    /// Applies the peer's acknowledgement field: every packet it covers is taken from the
    /// ring and each entity it carried advances its `acked` to the state's tick if newer
    /// and clears `in_flight` if it names that packet; records older than the field's
    /// window are expired as lost, clearing `in_flight` so their entities are sent again;
    /// an entity whose tombstone is acknowledged is forgotten and its handle released.
    pub fn acknowledge(&mut self, ack: AckField) {
        todo!()
    }
}
