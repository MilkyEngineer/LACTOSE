//! The five systems of this crate and the sets a game orders itself by (architecture
//! sections 5.2, 6.1 and 17; `lactose-sim`'s `Stage::Simulation` and `Stage::Post`). In
//! `Stage::Simulation`: the advance, which runs each character's state machine and phase
//! from its parameters and writes its root motion and events; and the evaluation, which
//! writes the root-local sim pose. In `Stage::Post`, after the physics write-back and in
//! the server world alone, which alone has bone rows: a limp ragdoll's bone rows report
//! where the step left them, the owner's pose is fitted to them, and an animated
//! character's bone rows are swept to its pose. The module is
//! private: [`register_systems`] places all five, and a consumer orders itself against
//! them through [`sets`] alone.

use lactose_core::Result;
use lactose_ecs::{Commands, Ctx, EventWriter, Events, Mut, StableKeyOf, system};
use lactose_physics_api::Transform;
use lactose_sim::{Registration, Stage, TypeTable};

use crate::components::{AnimParams, AnimRig, AnimState, BoneLink, Bones, RootMotion, SimPose};
use crate::event::{AnimEvent, BoneMoved};

/// The system sets this crate declares: three in `Stage::Simulation`, three in
/// `Stage::Post`, each chain ordered as listed. A game system that touches a component of
/// this crate is in one of them or in a set declared `.after(..)` one of them, and the
/// schedule refuses the build otherwise (`README.md`, "The tick").
pub mod sets {
    use lactose_ecs::SystemSet;

    /// Where the game writes [`AnimParams`](crate::AnimParams), in `Stage::Simulation`
    /// before the advance: a game system placed `in_set = lactose_anim::sets::DRIVE` is
    /// ordered before every system of this crate. This crate places nothing in it.
    pub static DRIVE: SystemSet = SystemSet::new("anim::drive");

    /// The advance (`anim::advance`): the state machine, the phase, the root motion and the
    /// events. A reader of [`AnimState`](crate::AnimState), [`RootMotion`](crate::RootMotion)
    /// or `Events<AnimEvent>` in `Stage::Simulation` is in a set declared
    /// `.after(&[&ADVANCE])` or `.after(&[&EVALUATE])`.
    pub static ADVANCE: SystemSet = SystemSet::new("anim::advance").after(&[&DRIVE]);

    /// The evaluation (`anim::evaluate`): the root-local [`SimPose`](crate::SimPose). A
    /// reader of it in `Stage::Simulation` is in a set declared `.after(&[&EVALUATE])`.
    pub static EVALUATE: SystemSet = SystemSet::new("anim::evaluate").after(&[&ADVANCE]);

    /// A limp ragdoll's bone rows reporting their stepped transforms
    /// (`anim::report_bones`), in `Stage::Post` after the physics write-back. This set and
    /// the two after it hold systems in the server world alone and are empty in a
    /// prediction world, which has no bone rows.
    pub static REPORT: SystemSet = SystemSet::new("anim::report");

    /// The ragdoll fit (`anim::fit_ragdoll`): a limp character's `SimPose` rewritten from
    /// its bone rows.
    pub static FIT: SystemSet = SystemSet::new("anim::fit").after(&[&REPORT]);

    /// The bone sweep (`anim::drive_bones`): an animated character's bone rows written to
    /// its pose through commands, which apply at the stage's closing sync point and so
    /// conflict with nothing. A `Stage::Post` reader of [`SimPose`](crate::SimPose) is in
    /// a set declared `.after(&[&BONES])` (or `.after(&[&FIT])`, the fit being the writer
    /// it must follow); a `Stage::Post` reader of `Transform` needs no edge against this
    /// crate.
    pub static BONES: SystemSet = SystemSet::new("anim::bones").after(&[&FIT]);
}

/// Registers this crate's components with `types`: [`AnimRig`], [`AnimState`],
/// [`AnimParams`], [`RootMotion`], [`SimPose`], [`Bones`] and [`BoneLink`]. `app` calls it
/// beside every other crate's `register_types`; the `Transform`, `RigidBody`, `Collider`
/// and `Joint` the bone rows carry are `lactose_physics_api::register_types`'s.
pub fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<AnimRig>()?;
    types.register::<AnimState>()?;
    types.register::<AnimParams>()?;
    types.register::<RootMotion>()?;
    types.register::<SimPose>()?;
    types.register::<Bones>()?;
    types.register::<BoneLink>()?;
    Ok(())
}

/// Adds this crate's five systems and six sets: the advance and the evaluation to
/// `Stage::Simulation`, the report, the fit and the sweep to `Stage::Post`. What `app`
/// passes to `SimulationBuilder::systems`. The advance and the evaluation declare
/// `worlds = [server, prediction]` and the three `Stage::Post` systems `worlds = [server]`,
/// so a prediction registration adds the `Stage::Post` sets with nothing in them and a
/// `Client` registration adds the sets alone.
pub fn register_systems(registration: &mut Registration) {
    registration
        .stage(Stage::Simulation)
        .sets(&[&sets::DRIVE, &sets::ADVANCE, &sets::EVALUATE])
        .add(&ADVANCE)
        .add(&EVALUATE);
    registration
        .stage(Stage::Post)
        .sets(&[&sets::REPORT, &sets::FIT, &sets::BONES])
        .add(&REPORT_BONES)
        .add(&FIT_RAGDOLL)
        .add(&DRIVE_BONES);
}

/// The advance: one character's state machine for one tick (`README.md`, "The state
/// machine", states the order). With the rig's machine from the region's assets: enters the
/// initial state if the state is unset or out of range; ends a blend whose outgoing state
/// is out of range or whose clip is missing; with the state's clip, tries the transitions
/// in order against the parameters and the state as they are, taking the first whose
/// conditions all hold; then advances the phase (and the outgoing phase of a blend) by the
/// state's rate, writes the root motion the clip moved by between the two phases, emits an
/// `AnimEvent` for every authored event the advance passed, and clears the triggers it
/// read. Writes `AnimParams` only when a trigger was set. With the machine missing from the
/// region's table, a hold: marks `AnimState` held and leaves the rest of it and `AnimParams`
/// as they are, writes `RootMotion::ZERO` and emits nothing; with the state's clip missing,
/// the same after the state has been set, except that a hold on the tick that entered the
/// initial state does not mark it held, so the entry is reported on that tick.
#[system(name = "anim::advance", worlds = [server, prediction], in_set = sets::ADVANCE)]
pub(crate) fn advance(
    ctx: Ctx,
    events: EventWriter<AnimEvent>,
    rig: &AnimRig,
    params: Mut<AnimParams>,
    state: Mut<AnimState>,
    motion: Mut<RootMotion>,
) {
    todo!()
}

/// The evaluation: [`SimPose::evaluate`] with the rig's skeleton, the state's clip at its
/// phase and the blend's outgoing clip at its phase, written whole into the row. With the
/// skeleton, the clip or (during a blend) the outgoing clip missing, or the state unset,
/// or the character's bones limp (the fit in `Stage::Post` writes the pose then), writes
/// [`SimPose::EMPTY`]. The per-character work the milestone 1 budget of 0.5 ms for 200
/// characters bounds (`README.md`, "Proof for milestone 1").
#[system(name = "anim::evaluate", worlds = [server, prediction], in_set = sets::EVALUATE)]
pub(crate) fn evaluate(
    ctx: Ctx,
    rig: &AnimRig,
    state: &AnimState,
    bones: Option<&Bones>,
    pose: Mut<SimPose>,
) {
    todo!()
}

/// A bone row reporting its stepped `Transform` to its owner, when its link says the owner
/// is limp; an animated character's bone rows emit nothing. Server world only, as every
/// bone system is: bone rows exist there alone.
#[system(name = "anim::report_bones", worlds = [server], in_set = sets::REPORT)]
pub(crate) fn report_bones(moved: EventWriter<BoneMoved>, link: &BoneLink, transform: &Transform) {
    todo!()
}

/// A limp character's pose rebuilt from its bone rows, in skeleton order: a bone a
/// `BoneMoved` reported is taken into root-local space against the character's own
/// stepped `Transform`; a bone no row reported (one without a body) is its parent-local
/// sample from the state's clip at its phase (blended as `anim::evaluate` blends) composed
/// onto its parent's fitted pose, so hands and feet hang off the fallen limbs and sockets
/// with them. Written whole into `SimPose`. Does nothing for a character whose bones are
/// not limp; with the skeleton or the clip missing, writes `SimPose::EMPTY`.
#[system(name = "anim::fit_ragdoll", worlds = [server], in_set = sets::FIT)]
#[expect(clippy::too_many_arguments, reason = "a kernel's parameters are its row columns")]
pub(crate) fn fit_ragdoll(
    ctx: Ctx,
    moved: Events<BoneMoved>,
    key: StableKeyOf,
    rig: &AnimRig,
    state: &AnimState,
    bones: &Bones,
    root: &Transform,
    pose: Mut<SimPose>,
) {
    todo!()
}

/// An animated character's bone rows swept to its pose: for every bone with a row,
/// `Commands::set` of the bone's `Transform` at `SimPose::world` of the character's stepped
/// root, which the next tick's physics sync sweeps the kinematic body to during its step.
/// Does nothing for a limp character, whose bodies move themselves. The write of
/// `Transform` is deliberately not declared: the commands apply at the stage's closing sync
/// point, after every `Stage::Post` system has run, so no reader in the stage can see them
/// and a declaration would only order every `Transform` reader of the stage against this
/// set for nothing (and falsify physics-api's "every other `Stage::Post` system reads the
/// stepped rows after the write-back with no edge").
#[system(name = "anim::drive_bones", worlds = [server], in_set = sets::BONES)]
pub(crate) fn drive_bones(cmd: Commands, bones: &Bones, pose: &SimPose, root: &Transform) {
    todo!()
}
