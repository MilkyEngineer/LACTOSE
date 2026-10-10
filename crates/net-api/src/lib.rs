//! Network contracts inside the determinism boundary (architecture section 4, `net-api`):
//! what the simulation side (`sim`, `predict`, `app`) and `lactose-net` agree on about
//! what enters and leaves a world. The host side of networking (quantisation, the
//! handshake, the transport, packets and handles, per-client interest, the interpolation
//! buffer, clock sync, control messages) is `lactose-net-host`, outside the boundary.
//!
//! Foundation tier, inside the determinism boundary (architecture section 8.1). Every row
//! is keyed by `StableKey`; nothing here reads a clock or computes a float. The
//! `README.md` is the contract: it states what the signatures cannot, and what a caller
//! may rely on.
//!
//! # Status
//!
//! A facade: every public signature is final and every body that decides anything is
//! `todo!()`. The implementation lane replaces the bodies behind this surface and removes
//! the crate-level `allow` with the last `todo!()`.
//!
//! # Where to start
//!
//! - [`StagedFrame`] is what a client received for one server tick, [`StagedInbox`] where
//!   `app` puts it between runs, and the client worlds' `StagedState` hooks apply it at
//!   the input sync point; [`RemoteSlot`] is the component a staged spawn leaves behind.
//! - [`StagingFrame`] is what the server's replication gather produces per frame and
//!   [`GatherState`] the resource that holds it; [`register_systems`] places the gather in
//!   `Pass::Gather`.
//! - [`RegionRelevance`] is the message a region's relevance sends, built from a
//!   `RegionExport`, and what the client builds its `RegionBundle` from.
//! - [`lag`] is a named place.
//!
//! ```no_run
//! use lactose_ecs::WorldKind;
//! use lactose_net_api::{GatherState, StagedFrame, StagedInbox};
//! use lactose_sim::{SimulationBuilder, TypeTable};
//!
//! # fn main() -> lactose_core::Result<()> {
//! // What `app` does for every world kind.
//! let mut types = TypeTable::new();
//! lactose_net_api::register_types(&mut types)?;
//! let mut simulation = SimulationBuilder::new(WorldKind::Client, types)
//!     .setup(lactose_net_api::setup)
//!     .systems(lactose_net_api::register_systems)
//!     .build()?;
//!
//! // Between runs on a client: a frame from the network thread into the world's inbox,
//! // and its region changes into the step's `TickInput` only if the inbox accepted it.
//! // The next step advances the world to `tick() + 1` and its hook pops the frames due
//! // at that tick, so that is the tick `app` asks `due` for.
//! let next = lactose_core::Tick(simulation.tick().0 + 1);
//! let frame = std::sync::Arc::new(StagedFrame::new(next));
//! let inbox = simulation.world_mut().service_mut::<StagedInbox>().expect("setup inserted it");
//! let accepted = inbox.push(frame);
//! assert!(accepted);
//! for due in inbox.due(next) {
//!     // fold `due.time_scale` and `due.regions` into the step's `TickInput`
//!     # let _ = due;
//! }
//!
//! // Between runs on the server: the gathered frame out, to the packer.
//! let gathered = simulation.world_mut().service_mut::<GatherState>().expect("setup").take();
//! # let _ = gathered;
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]
#![allow(
    dead_code,
    unused_variables,
    reason = "facade: every body is `todo!()` until the implementation lane replaces it, and \
              the fields and parameters it will use are declared now"
)]

pub mod lag;
pub mod relevance;
pub mod staged;
pub mod staging;

use lactose_core::Result;
use lactose_ecs::World;
use lactose_sim::{Pass, Registration, TypeTable};
pub use relevance::{OverlayPayload, RegionChange, RegionRelevance};
pub use staged::{
    DueFrames, InterpolationSlot, RemoteSlot, StagedFrame, StagedInbox, StagedRow, StagedRows,
    StagedSpawn, Tombstone,
};
pub use staging::{GatherState, GatheredRow, GatheredRows, StagingFrame};

/// Registers this crate's one component, [`RemoteSlot`], so a client world can hold it;
/// harmless in every other kind. `app` calls it on the one `TypeTable` beside every other
/// crate's `register_types`.
pub fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<RemoteSlot>()
}

/// Inserts the two service resources this crate's consumers reach between runs: a
/// [`StagedInbox`] for the frames a client world applies, and a [`GatherState`] for the
/// frames the server's gather produces. Both are inserted in every kind, since a service
/// resource that no system names costs nothing; `app` adds it with
/// `SimulationBuilder::setup`.
pub fn setup(world: &mut World) -> Result<()> {
    world.insert_service::<StagedInbox>(Box::new(StagedInbox::new()));
    world.insert_service::<GatherState>(Box::new(GatherState::new()));
    Ok(())
}

/// Places the replication gather (`net::gather`, `worlds = [server]`) in `Pass::Gather`;
/// `StageBuilder::add` skips it in every other kind. `app` adds it with
/// `SimulationBuilder::systems`.
pub fn register_systems(registration: &mut Registration) {
    registration.pass(Pass::Gather).add(&staging::GATHER);
}

/// What the contract promises at the type level, pinned so the implementation cannot lose
/// it without a compile error: the frames travel between the network thread, `app` and
/// the worlds and are shared by `Arc`; the two service resources satisfy
/// `World::insert_service`; `RemoteSlot` is a derived, extracted component of eight bytes
/// and its slot is plain data; and the lag cap agrees with section 9.5.
const _: () = {
    use lactose_reflect::bytemuck::{CheckedBitPattern, NoUninit};
    use lactose_reflect::{Component, Persistence, Reflect};
    use lactose_sim::TICK_RATE_HZ;

    const fn send_sync<T: Send + Sync>() {}
    const fn plain<T: NoUninit + CheckedBitPattern + Copy + Send + Sync + 'static>() {}
    const fn component<T: Component>() {}

    send_sync::<StagedFrame>();
    send_sync::<StagedInbox>();
    send_sync::<StagedRows<'_>>();
    send_sync::<DueFrames<'_>>();
    send_sync::<StagingFrame>();
    send_sync::<GatherState>();
    send_sync::<GatheredRows<'_>>();
    send_sync::<RegionRelevance>();
    send_sync::<RegionChange>();
    send_sync::<lag::LagCompensationHistory>();

    plain::<InterpolationSlot>();
    component::<RemoteSlot>();

    assert!(size_of::<InterpolationSlot>() == 8);
    assert!(size_of::<RemoteSlot>() == 8);
    assert!(matches!(RemoteSlot::SCHEMA.info().persistence(), Persistence::Derived));
    assert!(RemoteSlot::SCHEMA.info().tracking().extracted);
    assert!(!RemoteSlot::SCHEMA.info().tracking().replicated);
    assert!(lag::LAG_COMPENSATION_MAX_TICKS * 1000 / TICK_RATE_HZ <= 250);
};
