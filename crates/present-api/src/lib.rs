//! The presentation interface (architecture sections 5.3 and 13): what gameplay, the
//! simulation side and the render side agree on about what leaves a world to be shown.
//! Gameplay reaches presentation through this crate without naming a renderer, an audio
//! device or a window: it authors the components, emits the request events, and fills the
//! view-models and the debug-draw buffer. `app` places extract and publishes its
//! snapshots; the renderer implements the sink; `lactose-presentation` reads the snapshot.
//!
//! Foundation tier, outside the determinism boundary, a crate boundary crates may depend
//! on (`[boundary_internal]`) and a lint-file carrier (architecture section 8.1): the
//! crate declares components and a `#[system]`, so it carries the boundary's lint file and
//! is held to section 8.2. Every key is a `StableKey`, nothing here reads a clock, and the
//! one conversion to 32-bit floats is `lactose_math`'s. The `README.md` is the contract:
//! it states what the signatures cannot, and what a caller may rely on.
//!
//! # Status
//!
//! A facade: every public signature is final and every body that decides anything is
//! `todo!()`. The implementation lane replaces the bodies behind this surface and removes
//! the crate-level `allow` with the last `todo!()`.
//!
//! # Where to start
//!
//! - [`MeshInstance`], [`Light`], [`Decal`] and [`CameraRig`] are the components a game
//!   or a cooked region puts on an entity; [`AudioRequest`], [`VfxRequest`] and
//!   [`DebugShape`] are the events a system emits through `PresentEvents`.
//! - [`register_systems`] places extract in `Pass::Extract`; [`ExtractState`] is the
//!   service resource `app` installs an [`InstanceSink`] and a [`SnapshotWriter`] in, and
//!   publishes through, once per frame.
//! - [`snapshot_channel`] makes the triple buffer; [`SnapshotReader::acquire`] gives the
//!   render side a [`PresentFrame`]: two [`PresentSnapshot`]s and the frame's events.
//! - [`ViewModels`] and [`DebugDraw`] are the render side's own: the values a UI shows and
//!   the segments a frame draws over the scene.
//!
//! ```no_run
//! use lactose_ecs::WorldKind;
//! use lactose_present_api::{ExtractState, snapshot_channel};
//! use lactose_sim::{SimulationBuilder, TypeTable};
//!
//! # fn main() -> lactose_core::Result<()> {
//! // What `app` does for every world kind.
//! let mut types = TypeTable::new();
//! lactose_physics_api::register_types(&mut types)?;
//! lactose_present_api::register_types(&mut types)?;
//! let mut simulation = SimulationBuilder::new(WorldKind::Server, types)
//!     .setup(lactose_present_api::setup)
//!     .systems(lactose_present_api::register_systems)
//!     .build()?;
//!
//! // Once, for a world that is shown: the snapshot channel, its writing half into the
//! // world, its reading half to the presentation schedule. The renderer's sink goes in
//! // the same way, with `set_sink`.
//! let (writer, mut reader) = snapshot_channel();
//! let extract = simulation.world_mut().service_mut::<ExtractState>().expect("setup inserted it");
//! extract.set_snapshot(Some(writer));
//!
//! // Every frame, after `Simulation::run_passes` has run extract: the frame's events
//! // and the snapshot the pass wrote, published together.
//! let events = simulation.world_mut().take_present_events();
//! simulation.world_mut().service_mut::<ExtractState>().expect("setup").publish(events);
//!
//! // On the render side, every frame.
//! if let Some(frame) = reader.acquire() {
//!     for emission in &frame.events().emitted {
//!         // play, spawn or draw it
//!         # let _ = emission;
//!     }
//! }
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]
#![allow(
    dead_code,
    unused_variables,
    reason = "facade: every deciding body is `todo!()` until the implementation lane replaces \
              it, and the fields and parameters it will use are declared now"
)]

mod components;
mod debug;
mod events;
mod extract;
mod snapshot;
mod view;

pub use components::{CameraRig, Decal, Light, LightKind, MaterialAsset, MeshAsset, MeshInstance};
pub use debug::{DebugDraw, DebugLine};
pub use events::{AudioRequest, DebugShape, DebugShapeKind, EffectAsset, SoundAsset, VfxRequest};
pub use extract::{ExtractPass, ExtractState, InstanceRow, InstanceSink, RenderTransform};
use lactose_core::Result;
use lactose_ecs::World;
use lactose_sim::{Pass, Registration, TypeTable};
pub use snapshot::{
    PresentFrame, PresentSnapshot, SnapshotReader, SnapshotRows, SnapshotWriter, TransformPair,
    snapshot_channel,
};
pub use view::{ViewModel, ViewModelIter, ViewModels};

/// Registers this crate's four components, [`MeshInstance`], [`Light`], [`Decal`] and
/// [`CameraRig`], so a world of any kind can hold them. `app` calls it on the one
/// `TypeTable` beside every other crate's `register_types`; `lactose-physics-api`'s
/// registers the `Transform` and `Teleported` extract reads.
pub fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<MeshInstance>()?;
    types.register::<Light>()?;
    types.register::<Decal>()?;
    types.register::<CameraRig>()
}

/// Inserts the [`ExtractState`] service resource, with no output installed, in every kind
/// of world: a pass with no output does nothing, so a world that is never shown pays
/// nothing for it. `app` adds it with `SimulationBuilder::setup`.
pub fn setup(world: &mut World) -> Result<()> {
    world.insert_service::<ExtractState>(Box::new(ExtractState::new()));
    Ok(())
}

/// Places extract (`present::extract`, `worlds = [server, client, prediction]`) in
/// `Pass::Extract`, alone. `app` adds it with `SimulationBuilder::systems`, and the build
/// of a prediction world passes `lactose_predict::check_prediction_reads` because the
/// system's key is in `lactose_predict::EXCLUSIVE_SYSTEMS`.
pub fn register_systems(registration: &mut Registration) {
    registration.pass(Pass::Extract).add(&extract::EXTRACT);
}

/// What the contract promises at the type level, pinned so the implementation cannot lose
/// it without a compile error: the state is a service resource and so is shared between a
/// pass's jobs; a sink is a trait object that satisfies the same bounds; the channel's
/// halves move to the threads that use them and the frame is handed to the presentation
/// schedule's jobs; every component is persistent (so hashed and saved), replicated and
/// extracted, and of the size `README.md` states; every request is a presentation event;
/// and the two components extract reads beside its own are extracted.
const _: () = {
    use lactose_ecs::PresentEvent;
    use lactose_math::Scalar;
    use lactose_physics_api::{Teleported, Transform};
    use lactose_reflect::{Component, Persistence, Reflect};

    const fn send_sync<T: ?Sized + Send + Sync>() {}
    // What `World::insert_service` takes. Naming `dyn InstanceSink` is also what holds the
    // trait to being usable as a trait object.
    const fn service<T: ?Sized + Send + Sync + 'static>() {}
    const fn copy<T: Copy + Send + Sync>() {}
    const fn present_event<P: PresentEvent>() {}
    const fn authored<T: Component>() -> bool {
        let info = T::SCHEMA.info();
        matches!(info.persistence(), Persistence::Persistent)
            && info.tracking().replicated
            && info.tracking().extracted
    }

    service::<ExtractState>();
    service::<dyn InstanceSink>();
    send_sync::<Box<dyn InstanceSink>>();
    send_sync::<SnapshotWriter>();
    send_sync::<SnapshotReader>();
    send_sync::<PresentSnapshot>();
    send_sync::<PresentFrame<'static>>();
    send_sync::<SnapshotRows<'static, MeshInstance>>();
    send_sync::<ViewModels>();
    send_sync::<ViewModel<'static>>();
    send_sync::<ViewModelIter<'static>>();
    send_sync::<DebugDraw>();

    copy::<InstanceRow>();
    copy::<RenderTransform>();
    copy::<ExtractPass>();
    copy::<TransformPair>();
    copy::<DebugLine>();
    copy::<PresentFrame<'static>>();
    copy::<ViewModel<'static>>();

    present_event::<AudioRequest>();
    present_event::<VfxRequest>();
    present_event::<DebugShape>();

    assert!(authored::<MeshInstance>());
    assert!(authored::<Light>());
    assert!(authored::<Decal>());
    assert!(authored::<CameraRig>());
    assert!(Transform::SCHEMA.info().tracking().extracted);
    assert!(Teleported::SCHEMA.info().tracking().extracted);

    assert!(Scalar::BYTES == 8);
    assert!(size_of::<MeshInstance>() == 8);
    assert!(size_of::<Light>() == 64);
    assert!(size_of::<Decal>() == 32);
    assert!(size_of::<CameraRig>() == 32);
    assert!(size_of::<AudioRequest>() == 32);
    assert!(size_of::<VfxRequest>() == 64);
    assert!(size_of::<DebugShape>() == 56);
};
