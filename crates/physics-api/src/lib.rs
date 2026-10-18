//! The physics interface (architecture sections 4 and 16): the components that describe a
//! body, its collider and its joints and the two the write-back writes; the command channel
//! a kernel writes through a `ChannelWriter<PhysicsCommands>` parameter; contact and trigger
//! events; queries over the stepped state; the trait a backend implements; and the snapshot
//! and restore the prediction world rolls back with. `lactose-physics-rapier` is the backend
//! behind it.
//!
//! Foundation tier, inside the determinism boundary. `README.md` is the contract: it
//! states what the signatures cannot, and what a caller and a backend may rely on.
//!
//! # Status
//!
//! Implemented against the naive ECS and tested against a test backend (`tests/common`);
//! the surface is frozen once `lactose-physics-rapier` passes [`conformance`].
//!
//! # Where to start
//!
//! - A row is a body when it has a [`Transform`] and a [`RigidBody`] or a [`Collider`];
//!   a [`Joint`] is a row of its own. The write-back writes [`Transform`], [`Velocity`]
//!   and [`Teleported`] at the sync point that begins `Stage::Post`.
//! - A kernel moves a body through the [`PhysicsExt`] methods on a
//!   `ChannelWriter<PhysicsCommands>` parameter in `Stage::Input` or `Stage::Simulation`,
//!   or by writing the body's rows; it reads contacts through `Events<ContactEvent>` in
//!   `Stage::Post` and queries through `Res<Physics>` anywhere.
//! - A backend implements [`PhysicsBackend`]; `app` hands it to [`setup`] with the solver
//!   pool built outside the boundary, and adds [`register_systems`] and
//!   [`register_types`].
//! - `lactose-predict` takes [`snapshot`] beside `World::snapshot`, gives it back to
//!   [`restore`] after `World::restore`, and names what it corrected with [`corrected`].
//! - [`conformance`] is the run a backend passes before this surface is frozen.
//!
//! ```no_run
//! use lactose_ecs::WorldKind;
//! use lactose_physics_api::{PhysicsBackend, PhysicsConfig};
//! use lactose_sim::{SimulationBuilder, TypeTable};
//!
//! # fn backend_with_pool() -> Box<dyn PhysicsBackend> { unimplemented!() }
//! # fn main() -> lactose_core::Result<()> {
//! // In `app`: the backend crate builds its solver pool outside the boundary and wraps it.
//! let backend: Box<dyn PhysicsBackend> = backend_with_pool();
//! let config = PhysicsConfig { reserved_workers: 2, ..PhysicsConfig::default() };
//! let mut types = TypeTable::new();
//! lactose_physics_api::register_types(&mut types)?;
//! let simulation = SimulationBuilder::new(WorldKind::Server, types)
//!     .setup(move |world| lactose_physics_api::setup(world, backend, config))
//!     .systems(lactose_physics_api::register_systems)
//!     .build()?;
//! # let _ = simulation;
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]

mod backend;
mod command;
mod components;
pub mod conformance;
mod event;
mod query;
mod service;
mod sync;
mod systems;

pub use backend::{BodyDesc, BodyState, PhysicsBackend, StepInput, StepOutput};
pub use command::{PhysicsCommand, PhysicsCommands, PhysicsExt};
pub use components::{
    BodyHandle, BodyType, Collider, CollisionGroups, Joint, JointHandle, JointKind, RigidBody,
    Shape, ShapeKind, Teleported, Transform, Velocity, locked_axes,
};
pub use event::{ContactEvent, ContactPhase, TriggerEvent};
pub use query::{QueryFilter, Ray, RayHit, ShapeHit};
pub use service::{
    Physics, PhysicsConfig, PhysicsSnapshot, corrected, register_types, restore, setup, snapshot,
};
pub use systems::{register_systems, sets};

/// What the contract promises at the type level, pinned so the implementation cannot lose
/// it without a compile error: kernels on every worker read `Physics` at once; a snapshot,
/// a command and the step's lists cross threads; the events and the channel are what the
/// ECS requires of them, and the channel is not cleared at the input sync point; and no
/// layout depends on the scalar's width (architecture section 16): each component is its
/// scalars, then a tail that is a multiple of eight bytes, so the sizes below hold for a
/// four-byte scalar as for an eight-byte one.
const _: () = {
    use lactose_ecs::{CommandChannel, Event};
    use lactose_math::Scalar;
    use lactose_reflect::Component;

    const fn send_sync<T: Send + Sync>() {}
    const fn event<E: Event>() {}
    const fn channel<C: CommandChannel>() {}
    const fn component<T: Component>() {}

    send_sync::<Physics>();
    send_sync::<PhysicsSnapshot>();
    send_sync::<PhysicsCommand>();
    send_sync::<StepOutput>();
    send_sync::<Box<dyn PhysicsBackend>>();
    send_sync::<conformance::ConformanceReport>();

    event::<ContactEvent>();
    event::<TriggerEvent>();
    channel::<PhysicsCommands>();
    assert!(!<PhysicsCommands as CommandChannel>::CLEAR_AT_INPUT);

    component::<Transform>();
    component::<Velocity>();
    component::<Teleported>();
    component::<RigidBody>();
    component::<Collider>();
    component::<Joint>();
    component::<BodyHandle>();
    component::<JointHandle>();

    const S: usize = Scalar::BYTES;
    assert!(size_of::<Transform>() == 7 * S);
    assert!(size_of::<Velocity>() == 6 * S);
    assert!(size_of::<Teleported>() == 4);
    assert!(size_of::<RigidBody>() == 4 * S + 8);
    assert!(size_of::<Shape>() == 4 * S + 8);
    assert!(size_of::<CollisionGroups>() == 8);
    assert!(size_of::<Collider>() == 15 * S + 24);
    assert!(size_of::<Joint>() == 11 * S + 40);
    assert!(size_of::<BodyHandle>() == 8);
    assert!(size_of::<JointHandle>() == 8);
};
