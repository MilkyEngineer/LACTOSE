//! Network contracts (architecture section 4, `net-api`): what the simulation side
//! (`sim`, `predict`, `app`, the game crates) and a transport agree on, so that the
//! milestone 1 netcode spike can be written against them and milestone 4's `lactose-net`
//! implements them.
//!
//! Foundation tier, outside the determinism boundary; boundary crates may use the modules
//! the `README.md` names as theirs ([`staged`], [`staging`], [`relevance`], [`quant`],
//! [`lag`]) for types and for operations that are deterministic by construction
//! (architecture section 8.2). The `README.md` is the contract: it states what the
//! signatures cannot, and what a caller may rely on.
//!
//! # Status
//!
//! A facade: every public signature is final and every body that decides anything is
//! `todo!()`. The implementation lane replaces the bodies behind this surface and removes
//! the crate-level `allow` with the last `todo!()`.
//!
//! # Where to start
//!
//! - [`StagedFrame`] is what a client receives for one server tick, [`StagedInbox`] where
//!   `app` puts it between runs, and the client worlds' `StagedState` hooks apply it at
//!   the input sync point; [`RemoteSlot`] is the component a staged spawn leaves behind.
//! - [`StagingFrame`] is what the server's replication gather produces per frame and
//!   [`GatherState`] the resource that holds it; [`register_systems`] places the gather in
//!   `Pass::Gather`.
//! - [`RegionRelevance`] is the message a region's relevance sends, built from a
//!   `RegionExport`, and what the client builds its `RegionBundle` from.
//! - [`QuantisedPosition`] and [`QuantisedRotation`] are the wire values, with the
//!   conversions through `lactose_math`.
//! - [`handshake::Handshake`] is what each side sends first, [`transport::Transport`] the
//!   datagram trait a loopback or UDP implementation is a page of, [`packet`] the framing
//!   shapes, [`interest::ClientInterest`] the per-client state the packer keeps, and
//!   [`interpolation::InterpolationBuffer`] the lock-free resource the presentation
//!   schedule samples. [`clock`], [`lag`] and [`message`] are named places.
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
//! // Between runs on a client: a frame from the network thread into the world's inbox.
//! let frame = std::sync::Arc::new(StagedFrame::new(simulation.tick()));
//! let inbox = simulation.world_mut().service_mut::<StagedInbox>().expect("setup inserted it");
//! inbox.push(frame);
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

pub mod clock;
pub mod handshake;
pub mod interest;
pub mod interpolation;
pub mod lag;
pub mod message;
pub mod packet;
pub mod quant;
pub mod relevance;
pub mod staged;
pub mod staging;
pub mod transport;

use lactose_core::Result;
use lactose_ecs::World;
use lactose_sim::{Pass, Registration, TypeTable};
pub use quant::{QuantisedPosition, QuantisedRotation};
pub use relevance::{OverlayPayload, RegionChange, RegionRelevance};
pub use staged::{
    InterpolationSlot, RemoteSlot, StagedFrame, StagedInbox, StagedRow, StagedRows, StagedSpawn,
    Tombstone,
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
/// frames the server's gather produces. Both are inserted in every kind, since a world
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
/// `World::insert_service`; the buffer is shared by its writer and readers across
/// threads; a transport is owned by one thread; the wire-shaped values are plain data of
/// the stated sizes; `RemoteSlot` is a component; and the constants agree with the
/// sections that fix them.
const _: () = {
    use bytemuck::Pod;
    use lactose_reflect::{Component, Persistence, Reflect};
    use lactose_sim::TICK_RATE_HZ;

    const fn send_sync<T: Send + Sync>() {}
    const fn send<T: Send + ?Sized>() {}
    const fn plain<T: Pod + Send + Sync + 'static>() {}
    const fn component<T: Component>() {}

    send_sync::<StagedFrame>();
    send_sync::<StagedInbox>();
    send_sync::<StagedRows<'_>>();
    send_sync::<StagingFrame>();
    send_sync::<GatherState>();
    send_sync::<GatheredRows<'_>>();
    send_sync::<RegionRelevance>();
    send_sync::<RegionChange>();
    send_sync::<interest::ClientInterest>();
    send_sync::<packet::ConnectionHandles>();
    send_sync::<packet::PacketRing>();
    send_sync::<interpolation::InterpolationBuffer>();
    send::<interpolation::InterpolationWriter>();
    send_sync::<interpolation::InterpolationReader>();
    send_sync::<interpolation::RemoteSample>();
    send_sync::<clock::ClockEstimate>();
    send_sync::<lag::LagCompensationHistory>();
    send_sync::<transport::Datagram>();
    send_sync::<transport::TransportEvent>();
    send::<dyn transport::Transport>();

    plain::<handshake::Handshake>();
    plain::<handshake::ProtocolHash>();
    plain::<packet::EntityHandle>();
    plain::<packet::PacketSequence>();
    plain::<QuantisedPosition>();
    plain::<QuantisedRotation>();
    component::<RemoteSlot>();

    assert!(size_of::<handshake::Handshake>() == handshake::Handshake::ENCODED_LEN);
    assert!(size_of::<packet::EntityHandle>() == 4);
    assert!(size_of::<QuantisedPosition>() == 12);
    assert!(size_of::<QuantisedRotation>() == 8);
    assert!(size_of::<RemoteSlot>() == 8);
    assert!(matches!(RemoteSlot::SCHEMA.info().persistence(), Persistence::Derived));
    assert!(
        RemoteSlot::SCHEMA.info().tracking().extracted
            && !RemoteSlot::SCHEMA.info().tracking().replicated
    );
    assert!(
        quant::POSITION_UNITS_PER_METRE > 0
            && (quant::POSITION_UNITS_PER_METRE as u32).is_power_of_two()
    );
    assert!(transport::MAX_DATAGRAM_BYTES <= u16::MAX as usize);
    assert!(lag::LAG_COMPENSATION_MAX_TICKS * 1000 / TICK_RATE_HZ <= 250);
    assert!(clock::MAX_RATE_ADJUST_THOUSANDTHS == 30);
    assert!(packet::AckField::WINDOW == 32);
    assert!(packet::EntityHandle::NONE.is_none());
};
