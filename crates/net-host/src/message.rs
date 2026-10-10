//! Control messages that are neither inputs nor replication (architecture sections 9.3
//! and 14): the join acceptance, the readiness acknowledgement the server waits for at a
//! region boundary, the overlay request after a digest mismatch, and the resync request a
//! client makes instead of diverging. Control messages, with
//! [`RegionRelevance`](lactose_net_api::RegionRelevance) and the time scale, travel on a
//! reliable ordered channel `lactose-net` builds over the transport's datagrams at
//! milestone 4; an overlay exceeds one datagram, so that channel fragments.
//!
//! Host-only: `lactose-net` encodes and sends them; nothing in the simulation names them.

use lactose_core::{PlayerId, Tick};
use lactose_ecs::RegionId;
use lactose_sim::AssetFailureEvent;

/// The server accepted the join: the player's identity for the session and the server
/// tick the client's worlds start at. The first message after the handshake, before the
/// full frame that is the late join: the client builds its replicated world at `tick`
/// (`SimulationBuilder::tick`), or `set_tick`s one it has, so that the full frame for
/// `tick` is due at its first sync point. From this message until the full frame for
/// `tick` is staged the receiver stages nothing and drops every packet whose tick is not
/// after `tick` (the full frame travels on the reliable channel and deltas sent after it
/// overtake it), and it clears its handle mirror and slots when it stages that frame.
/// Further fields (the player's region, the server's send rate) are milestone 4's.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct JoinAccepted {
    /// The player, as the server assigned it; what the prediction world's hook and the
    /// input packets name.
    pub player: PlayerId,
    /// The server tick the client's worlds start at.
    pub tick: Tick,
}

/// The client has loaded `region` (its bundle entered its replicated world at `tick`) and
/// its asset table is pinned, so the server may let the player cross into it
/// (architecture section 14: "the server holds a player at a region boundary until the
/// client acknowledges readiness"). Recorded in
/// [`ClientInterest::ready`](crate::interest::ClientInterest::ready), the network's copy;
/// the hold itself moves simulation state and enters the server tick as a journaled input
/// at milestone 4 (a `lactose-sim` addition, `SessionEvent::Ready { region }` or a
/// `TickInput` field).
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

/// A request for a full relevance-set snapshot (a
/// [`StagedFrame`](lactose_net_api::StagedFrame) with `full` set), the one path late join
/// and resync share (architecture section 9.3). From the request until the full frame is
/// staged the receiver stages nothing and drops every packet whose tick is not after the
/// full frame's; the server, on receiving it, clears the client's `ConnectionHandles` and
/// re-enters every relevant entity, so the full frame's spawns carry fresh handles.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ResyncRequest {
    /// The client's tick when it asked.
    pub tick: Tick,
    /// Why.
    pub reason: ResyncReason,
}

/// The client's cached overlay does not match the digest a
/// [`RegionRelevance`](lactose_net_api::RegionRelevance) carried, so it asks for the
/// overlay in full; the server answers with the same message, `Full`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct OverlayRequest {
    /// The region.
    pub region: RegionId,
}
