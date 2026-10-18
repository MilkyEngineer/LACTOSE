//! Animation inside the determinism boundary (architecture sections 4 and 17): state
//! machines, phase, events, root motion, and the deterministic sim skeleton that covers
//! sockets, hands, feet, head and ragdoll bodies. Full pose evaluation and skinning are
//! presentation and renderer work and are not here.
//!
//! Foundation tier, inside the determinism boundary. `README.md` is the contract: it
//! states what the signatures cannot, and what a game crate, `predict`, `net`,
//! presentation and the asset pipeline may rely on.
//!
//! # Status
//!
//! Facade: every public signature is final and every deciding body is `todo!()`. The
//! implementation lane replaces the bodies behind this surface and removes the crate-level
//! `allow` below with the last of them.
//!
//! # Where to start
//!
//! - A character is a row with [`AnimRig`] (which state machine and skeleton),
//!   [`AnimState`] (the state, its phase, a blend), [`AnimParams`] (what the game tells the
//!   machine), and [`RootMotion`] and [`SimPose`] (what this crate writes every tick).
//! - The game writes [`AnimParams`] in [`sets::DRIVE`]; `anim::advance` runs the machine
//!   and writes [`AnimState`], [`RootMotion`] and [`AnimEvent`]s; `anim::evaluate` writes
//!   the root-local [`SimPose`]; the game reads them after [`sets::EVALUATE`]
//!   ([`AnimState::phase_crossed`] is section 6.1's `footsteps` test) and applies the root
//!   motion itself.
//! - In `Stage::Post`, after the physics write-back, the bone rows of a ragdoll
//!   ([`Bones`], [`BoneLink`]) are swept to the pose or, limp, drive it; a reader of the
//!   stepped pose is in a set after [`sets::BONES`].
//! - The authored data are three simulation asset kinds, [`Clip`], [`StateMachine`] and
//!   [`Skeleton`], read from the region's exact-version table; their loaders are pure
//!   decoders.
//! - `app` adds [`register_types`] and [`register_systems`] beside every other crate's.
//!
//! ```no_run
//! use lactose_ecs::WorldKind;
//! use lactose_sim::{SimulationBuilder, TypeTable};
//!
//! # fn main() -> lactose_core::Result<()> {
//! let mut types = TypeTable::new();
//! lactose_physics_api::register_types(&mut types)?;
//! lactose_anim::register_types(&mut types)?;
//! let simulation = SimulationBuilder::new(WorldKind::Server, types)
//!     .systems(lactose_anim::register_systems)
//!     // ... every other crate's `setup` and `register_systems` ...
//!     .build()?;
//! # let _ = simulation;
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]
#![allow(
    dead_code,
    unused_variables,
    reason = "facade: every deciding body is `todo!()`; the implementation removes this with the last of them"
)]

mod assets;
mod components;
mod event;
mod keys;
mod phase;
mod systems;

pub use assets::{
    Bone, BoneBody, BoneJoint, BoneRole, Clip, ClipEvent, ClipLoader, Condition, MAX_SIM_BONES,
    PARAM_SLOTS, RootSample, Skeleton, SkeletonLoader, Socket, State, StateMachine,
    StateMachineLoader, Transition,
};
pub use components::{
    AnimParams, AnimRig, AnimState, Blend, BoneIndex, BoneLink, BonePose, Bones, Outgoing,
    ParamIndex, RootMotion, SimPose, StateIndex,
};
pub use event::{AnimEvent, BoneMoved};
pub use keys::{AnimEventKey, BoneKey, StateKey};
pub use phase::Phase;
pub use systems::{register_systems, register_types, sets};

/// What the contract promises at the type level, pinned so the implementation cannot lose
/// it without a compile error: every component is a reflected component; the two events
/// are what the ECS requires of them; the three asset kinds are simulation assets with
/// loaders, shared between threads; and no layout depends on the scalar's width
/// (architecture section 16): each component is its scalars, then a tail that is a multiple
/// of eight bytes, so the sizes below hold for a four-byte scalar as for an eight-byte one,
/// and a `SimPose` of eight-byte scalars stays under the ECS's 1 KiB component warning.
const _: () = {
    use lactose_asset_api::{Loader, SimulationAsset};
    use lactose_ecs::Event;
    use lactose_math::Scalar;
    use lactose_reflect::Component;

    const fn send_sync<T: Send + Sync>() {}
    const fn event<E: Event>() {}
    const fn component<T: Component>() {}
    const fn simulation_asset<T: SimulationAsset>() {}
    const fn loader<L: Loader>() {}

    component::<AnimRig>();
    component::<AnimState>();
    component::<AnimParams>();
    component::<RootMotion>();
    component::<SimPose>();
    component::<Bones>();
    component::<BoneLink>();

    event::<AnimEvent>();
    event::<BoneMoved>();

    simulation_asset::<Clip>();
    simulation_asset::<StateMachine>();
    simulation_asset::<Skeleton>();
    loader::<ClipLoader>();
    loader::<StateMachineLoader>();
    loader::<SkeletonLoader>();
    send_sync::<Clip>();
    send_sync::<StateMachine>();
    send_sync::<Skeleton>();

    const S: usize = Scalar::BYTES;
    assert!(size_of::<Phase>() == 4);
    assert!(size_of::<BonePose>() == 7 * S);
    assert!(size_of::<AnimRig>() == 16);
    assert!(size_of::<AnimState>() == 24);
    assert!(size_of::<AnimParams>() == PARAM_SLOTS * S + 8);
    assert!(size_of::<RootMotion>() == 4 * S);
    assert!(size_of::<SimPose>() == 7 * MAX_SIM_BONES * S + 8);
    assert!(7 * MAX_SIM_BONES * 8 + 8 <= 1024);
    assert!(size_of::<Bones>() == 16 * MAX_SIM_BONES + 8);
    assert!(size_of::<BoneLink>() == 24);
};
