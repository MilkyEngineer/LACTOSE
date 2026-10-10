//! Player input as the simulation sees it (architecture section 4, `input`): the quantised
//! per-tick sample, its byte image on the wire and in the journal, the applier that writes
//! it at the input sync point, and the redundancy window. The host side (raw samples, the
//! quantiser, the latest-sample slot, sources and mapping) is `lactose-input-host`.
//!
//! Foundation tier, inside the determinism boundary (architecture section 8.1): every
//! value a kernel reads from a sample is an integer or is built from one through
//! `lactose_math` here, under the boundary lints and the boundary build ID, and no float
//! arrives from the host side. The `README.md` is the contract: it states what the
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
//! - [`PlayerInput`] is the component a kernel takes as `&PlayerInput`: movement, absolute
//!   yaw and pitch, held, pressed and released [`Actions`], all integers;
//!   [`PlayerInput::aim`], [`PlayerInput::world_movement`] and [`Yaw::rotation`] derive
//!   directions through `lactose_math`.
//! - [`PlayerInput::to_sample_bytes`] and [`PlayerInput::from_sample_bytes`] are the image
//!   a [`lactose_sim::PlayerSample`] carries; [`PlayerInput::to_player_sample`] builds the
//!   frame entry.
//! - [`PlayerInputApplier`] is the `InputApplier` `app` installs, [`register_types`] and
//!   [`register_systems`] the pair it calls, and [`player_entity`] the one definition of a
//!   player's entity.
//! - [`SampleHistory`] is the redundancy window, [`PlayerInput::substitute`] the
//!   substitution rule and [`PlayerInput::after_substitute`] the repair of the edges a
//!   substitution swallowed (architecture section 9.2).
//!
//! ```no_run
//! use lactose_core::{PlayerId, Tick};
//! use lactose_ecs::WorldKind;
//! use lactose_input::{Action, Actions, Axis2, PlayerInput, PlayerInputApplier};
//! use lactose_sim::{SimulationBuilder, TickInput, TypeTable};
//!
//! # fn main() -> lactose_core::Result<()> {
//! // What `app` does for every world kind.
//! let mut types = TypeTable::new();
//! lactose_input::register_types(&mut types)?;
//! let simulation = SimulationBuilder::new(WorldKind::Server, types)
//!     .systems(lactose_input::register_systems)
//!     .inputs(Box::new(PlayerInputApplier::new()))
//!     .build()?;
//!
//! // What a test or `lactose-predict` puts in a frame: a sample, as bytes, per player.
//! let input = PlayerInput {
//!     movement: Axis2 { x: 0, y: lactose_input::AXIS_SCALE },
//!     held: Actions::EMPTY.with(Action::Sprint),
//!     ..PlayerInput::IDLE
//! };
//! let mut frame = TickInput::default();
//! frame.inputs.insert(PlayerId(1), input.to_player_sample(Tick(7)));
//! # let _ = (simulation, frame);
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]
#![allow(
    dead_code,
    unused_variables,
    reason = "facade: every body is `todo!()` until the implementation lane replaces it, and \
              the fields and parameters it will use are declared now"
)]

// The sample's byte image is its memory image, which is little-endian on every supported
// target (x86-64 and aarch64); on any other the image would differ from what the wire and
// the journal hold, so the crate refuses to build there rather than encode differently.
#[cfg(not(target_endian = "little"))]
compile_error!(
    "lactose-input defines a sample's byte image as its little-endian memory image; a \
     big-endian target needs an explicit encoder before it is supported"
);

pub mod apply;
pub mod history;
pub mod sample;

pub use apply::{PlayerInputApplier, player_entity, register_systems, register_types, sets};
pub use history::{REDUNDANT_SAMPLES, SampleHistory, SampleIter};
pub use sample::{
    AXIS_SCALE, Action, Actions, Axis2, Pitch, PlayerInput, SAMPLE_SIZE, UNITS_PER_TURN, Yaw,
};

/// What the contract promises at the type level, pinned so the implementation cannot lose
/// it without a compile error: the sample is a twenty-byte padding-free transient
/// component whose zero value is idle, its value types are plain data, the applier is what
/// `SimulationBuilder::inputs` takes, and the history travels between threads.
const _: () = {
    use lactose_reflect::bytemuck::{CheckedBitPattern, NoUninit};
    use lactose_reflect::{Component, Persistence, Reflect};
    use lactose_sim::InputApplier;

    const fn send_sync<T: Send + Sync>() {}
    const fn plain<T: NoUninit + CheckedBitPattern + Copy + Send + Sync + 'static>() {}
    const fn component<T: Component>() {}
    const fn applier<T: InputApplier>() {}

    plain::<PlayerInput>();
    component::<PlayerInput>();
    plain::<Axis2>();
    plain::<Yaw>();
    plain::<Pitch>();
    plain::<Actions>();
    applier::<PlayerInputApplier>();
    send_sync::<SampleHistory>();
    send_sync::<SampleIter<'_>>();

    assert!(SAMPLE_SIZE == 20, "the sample image is twenty bytes");
    assert!(size_of::<PlayerInput>() == SAMPLE_SIZE);
    assert!(align_of::<PlayerInput>() == 4);
    assert!(size_of::<Axis2>() == 4 && size_of::<Yaw>() == 2 && size_of::<Pitch>() == 2);
    assert!(size_of::<Actions>() == 4);
    assert!(PlayerInput::SCHEMA.info().layout().size() == SAMPLE_SIZE);
    assert!(
        matches!(PlayerInput::SCHEMA.info().persistence(), Persistence::Transient),
        "the sample is zeroed at every input sync point"
    );
    assert!(Pitch::LIMIT as u32 * 4 == UNITS_PER_TURN, "the pitch limit is a quarter turn");
    assert!(Action::NAMED < Actions::BITS);
    assert!(Action::Crouch.bit() + 1 == Action::NAMED, "every named bit is below NAMED");
    assert!(SampleHistory::CAPACITY == REDUNDANT_SAMPLES);
};
