//! Compile-only consumer sketches against the facade: what a game crate writes (a
//! character's state machine driven by a sketch input, a footstep from a phase crossing,
//! root motion applied by a movement sketch, a hit test and a ragdoll from the sim
//! skeleton), what presentation reads, and `app`'s registration, type-checked and never run
//! (every deciding body of the crate is `todo!()`). The one test builds the schedule of
//! every world kind, with the prediction world's build check, so the sets, the transient
//! writers and the replication policies are shown to pass the real schedule. They pin the
//! surface the README promises: a change that breaks one is a signature change to list.

#![forbid(unsafe_code)]
#![allow(dead_code, unused_variables, missing_docs, clippy::too_many_arguments)]

use lactose_anim::{
    AnimEvent, AnimEventKey, AnimParams, AnimRig, AnimState, BoneIndex, BoneKey, BoneLink,
    BoneRole, Bones, Clip, Phase, RootMotion, SimPose, Skeleton, StateMachine, sets,
};
use lactose_asset_api::AssetRef;
use lactose_core::{Result, StableKey};
use lactose_ecs::{
    Commands, Ctx, Events, Mut, PresentEvent, PresentEvents, PresentKind, Res, StableKeyOf,
    SystemSet, WorldKind, system,
};
use lactose_math::Scalar;
use lactose_physics_api::{RigidBody, Transform, Velocity};
use lactose_reflect::{component, value};
use lactose_sim::{Registration, Simulation, SimulationBuilder, Stage, TickDelta, TypeTable};

// ---- 1. A game crate: the character and what drives its state machine ----

/// What the game decided the character wants this tick, from its input and its physics
/// queries: the sketch stands in for `PlayerInput` and the game's movement rules.
#[component(key = "sketch::intent", persistent, replicated)]
pub struct Intent {
    #[field(id = 1)]
    pub speed: Scalar,
    #[field(id = 2)]
    pub jump: u32,
    #[field(id = 3)]
    pub airborne: u32,
}

#[component(key = "sketch::health", persistent, replicated)]
pub struct Health(#[field(id = 1)] pub u32);

/// A presentation event, as `lactose-present-api` will declare one.
#[value]
#[derive(PartialEq, Eq, Debug)]
pub struct AudioRequest(#[field(id = 1)] pub u32);

impl AudioRequest {
    pub const FOOTSTEP: Self = Self(1);
}

impl PresentEvent for AudioRequest {
    const KIND: PresentKind = PresentKind::from_name("sketch::audio");
}

/// The parameter slots, flags and triggers the game's state machine asset is authored
/// against; the game declares them as constants beside the asset.
pub const SPEED: u8 = 0;
pub const AIRBORNE_FLAG: u8 = 0;
pub const JUMP_TRIGGER: u8 = 0;
/// An authored clip event the game reacts to.
pub const SWING: AnimEventKey = AnimEventKey::from_name("sword.swing");
/// A socket the game reads.
pub const GRIP: BoneKey = BoneKey::from_name("socket.grip.r");

/// After the advance and the evaluation: readers of the state, the root motion and the
/// events in `Stage::Simulation`.
pub static MOVEMENT: SystemSet = SystemSet::new("sketch::movement").after(&[&sets::EVALUATE]);
/// After the bone sweep: readers of the stepped pose in `Stage::Post`.
pub static HITS: SystemSet = SystemSet::new("sketch::hits").after(&[&sets::BONES]);

/// Spawns a character: the five components of this crate beside the physics ones and the
/// game's. `AnimState::UNSET` enters the machine's initial state at the first advance.
pub fn spawn_character(
    cmd: &mut Commands<'_>,
    machine: AssetRef<StateMachine>,
    skeleton: AssetRef<Skeleton>,
) -> StableKey {
    let key = cmd.spawn((
        Transform::IDENTITY,
        Velocity::ZERO,
        AnimRig::new(machine, skeleton),
        AnimState::UNSET,
        AnimParams::ZERO,
        RootMotion::ZERO,
        SimPose::EMPTY,
        Intent { speed: Scalar::ZERO, jump: 0, airborne: 0 },
    ));
    cmd.set(key, Health(100));
    key
}

/// The game's parameter writer, in `sets::DRIVE`: values, a level flag and a one-tick
/// trigger, through `set_if_neq` so an unchanged tick stamps nothing.
#[system(name = "sketch::drive_anim", worlds = [server, prediction], in_set = sets::DRIVE)]
fn drive_anim(intent: &Intent, params: Mut<AnimParams>) {
    let mut next = *params;
    next.values[usize::from(SPEED)] = intent.speed;
    next.set_flag(AIRBORNE_FLAG, intent.airborne != 0);
    if intent.jump != 0 {
        next.trigger(JUMP_TRIGGER);
    }
    params.set_if_neq(next);
}

/// Section 6.1's `footsteps`, with the phase as this crate counts it, and an authored
/// event read from the character's own events.
#[system(name = "sketch::footsteps", worlds = [server, prediction], in_set = MOVEMENT)]
fn footsteps(
    present: PresentEvents,
    events: Events<AnimEvent>,
    key: StableKeyOf,
    anim: &AnimState,
) {
    if anim.phase_crossed(Phase::HALF) {
        present.emit(*key, AudioRequest::FOOTSTEP);
    }
    for event in events.for_target(*key) {
        if event.event == SWING {
            let _ = (event.state, event.phase);
        }
    }
}

/// The movement sketch: the clip's root motion applied by the character's own system, as
/// a kinematic sweep target through `Transform` and as the matching `Velocity`.
#[system(name = "sketch::apply_root_motion", worlds = [server, prediction], in_set = MOVEMENT)]
fn apply_root_motion(
    dt: Res<TickDelta>,
    motion: &RootMotion,
    pos: Mut<Transform>,
    vel: Mut<Velocity>,
) {
    let next = motion.apply(&pos);
    vel.linear = (next.translation - pos.translation) / dt.0;
    pos.set_if_neq(next);
}

/// A hit test in `Stage::Post`, after the bone sweep: the right hand and the grip socket
/// placed by the stepped root, from which the game casts its queries.
#[system(name = "sketch::sword_reach", worlds = [server, prediction], in_set = HITS)]
fn sword_reach(ctx: Ctx, rig: &AnimRig, pose: &SimPose, root: &Transform) {
    let Some(assets) = ctx.assets() else { return };
    let Ok(skeleton) = assets.get(rig.skeleton) else { return };
    let hand: Option<Transform> =
        skeleton.bone_of(BoneRole::HandRight).and_then(|bone| pose.world(root, bone));
    let grip: Option<Transform> =
        skeleton.socket(GRIP).and_then(|socket| pose.socket(skeleton, root, socket));
}

/// The ragdoll sketch: bone rows spawned from the pose when the character gets them, and
/// made limp when it dies. The helpers record commands on other rows, so the kernel
/// declares what they write.
#[system(
    name = "sketch::ragdoll",
    worlds = [server, prediction],
    in_set = HITS,
    writes = [Bones, BoneLink, RigidBody]
)]
fn ragdoll(
    ctx: Ctx,
    cmd: Commands,
    key: StableKeyOf,
    rig: &AnimRig,
    hp: &Health,
    bones: Option<&Bones>,
    pose: &SimPose,
    root: &Transform,
) {
    let Some(assets) = ctx.assets() else { return };
    let Ok(skeleton) = assets.get(rig.skeleton) else { return };
    match bones {
        None => Bones::spawn(&mut cmd, *key, skeleton, root, pose),
        Some(bones) if hp.0 == 0 && !bones.is_limp() => {
            bones.set_limp(&mut cmd, *key, skeleton, true);
        }
        Some(_) => {}
    }
}

pub fn register_types_game(types: &mut TypeTable) -> Result<()> {
    types.register::<Intent>()?;
    types.register::<Health>()?;
    lactose_physics_api::register_types(types)?;
    lactose_anim::register_types(types)
}

pub fn register_systems_game(registration: &mut Registration) {
    registration
        .stage(Stage::Simulation)
        .sets(&[&MOVEMENT])
        .add(&DRIVE_ANIM)
        .add(&FOOTSTEPS)
        .add(&APPLY_ROOT_MOTION);
    registration.stage(Stage::Post).sets(&[&HITS]).add(&SWORD_REACH).add(&RAGDOLL);
}

// ---- 2. Presentation: what extract copies and the presentation schedule reads ----

/// The snapshot carries `AnimRig`, `AnimState` and `SimPose` beside the root `Transform`
/// (all four extracted); presentation evaluates the render pose from the state, its phase
/// and its blend, and places the sim bones and sockets by the interpolated root.
fn presentation_reads(
    rig: &AnimRig,
    state: &AnimState,
    pose: &SimPose,
    root: &Transform,
    skeleton: &Skeleton,
    clip: &Clip,
) {
    let (state_index, phase, blend) = (state.state(), state.phase(), state.blend());
    let fraction: Scalar = phase.fraction();
    for bone in 0..pose.len() {
        let bone = BoneIndex::try_from(bone).expect("below MAX_SIM_BONES");
        let placed: Option<Transform> = pose.world(root, bone);
    }
    let grip = skeleton.socket(GRIP).and_then(|socket| pose.socket(skeleton, root, socket));
    let _ = (state_index, fraction, blend, grip, clip.looping);
}

// ---- 3. app: the registration functions, built for every world kind ----

fn app_build(kind: WorldKind) -> Result<Simulation> {
    let mut types = TypeTable::new();
    register_types_game(&mut types)?;
    let mut builder = SimulationBuilder::new(kind, types)
        .systems(lactose_anim::register_systems)
        .systems(register_systems_game);
    if kind == WorldKind::Prediction {
        builder = builder.check(lactose_predict::check_prediction_reads);
    }
    builder.build()
}

/// The sketches build beside this crate's systems in every world kind, the prediction
/// world under its build check: the sets order every conflict, every transient reader has
/// a declared writer before it, and every component a `[server, prediction]` system names
/// is replicated, transient or derived.
#[test]
fn the_sketches_build_in_every_world_kind() {
    for kind in [WorldKind::Server, WorldKind::Prediction, WorldKind::Client] {
        if let Err(error) = app_build(kind) {
            panic!("{kind:?}: {error}");
        }
    }
}
