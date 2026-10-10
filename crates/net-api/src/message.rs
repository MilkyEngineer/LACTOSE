//! Client-to-server control messages that are neither inputs nor replication (architecture
//! sections 9.3 and 14): the readiness acknowledgement the server waits for at a region
//! boundary, and the resync request a client makes instead of diverging.
//!
//! Host-only: `lactose-net` encodes and sends them; nothing in the simulation names them.

use lactose_core::Tick;
use lactose_ecs::RegionId;
use lactose_sim::AssetFailureEvent;

/// The client has loaded `region` (its bundle entered its replicated world at `tick`) and
/// its asset table is pinned, so the server may let the player cross into it
/// (architecture section 14: "the server holds a player at a region boundary until the
/// client acknowledges readiness"). Recorded in
/// [`ClientInterest::ready`](crate::interest::ClientInterest::ready).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ClientReady {
    /// The region.
    pub region: RegionId,
    /// The client's tick at which the region entered.
    pub tick: Tick,
}

/// Why a client asks for a resync (architecture sections 9.3, 11 and 14).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ResyncReason {
    /// A client-only asset failure, tick-stamped by the client's input sync point: the
    /// client cannot build the region the server considers relevant.
    AssetFailure(AssetFailureEvent),
    /// The client fell further behind than the catch-up cap recovers (section 11).
    BehindCatchUp,
    /// The client's replicated world disagrees with the server (a hash the server sent
    /// for a region did not match), so it asks for the region again.
    Divergence {
        /// The region.
        region: RegionId,
    },
}

/// A request for a full relevance-set snapshot (a [`StagedFrame`](crate::staged::StagedFrame)
/// with `full` set), the one path late join and resync share (architecture section 9.3).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ResyncRequest {
    /// The client's tick when it asked.
    pub tick: Tick,
    /// Why.
    pub reason: ResyncReason,
}
