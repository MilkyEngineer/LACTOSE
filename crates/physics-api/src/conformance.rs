//! The conformance suite a backend's test crate runs (architecture sections 10 and 20):
//! the snapshot-and-restore run that `lactose-physics-rapier` must pass before this crate
//! is frozen, as a function so that every backend runs the same scene under every layout.
//!
//! ```no_run
//! use lactose_ecs::{Executor, LayoutConfig};
//! use lactose_physics_api::conformance::snapshot_restore;
//! use lactose_physics_api::{PhysicsBackend, PhysicsConfig};
//!
//! # fn backend(_threads: u32) -> Box<dyn PhysicsBackend> { unimplemented!() }
//! # fn main() -> lactose_core::Result<()> {
//! let executor = Executor::builder().build()?;
//! // The backend's test builds its pool outside the boundary, here with one thread and
//! // with several, under the production layout and under SOUR's, and requires every
//! // report equal.
//! let layout = LayoutConfig::default();
//! let one = snapshot_restore(&executor, backend(1), PhysicsConfig::default(), layout)?;
//! let many = snapshot_restore(&executor, backend(4), PhysicsConfig::default(), layout)?;
//! assert_eq!(one, many);
//! # Ok(()) }
//! ```

use lactose_core::{EntityRef, Error, ErrorKind, NetId, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{
    ClaimedBy, Commands, Executor, LayoutConfig, Mut, OwnedBy, RegionConfig, RegionId, RegionMode,
    Res, StableKeyOf, SystemSet, World, WorldKind, system,
};
use lactose_math::{Scalar, Vec3};
use lactose_sim::{
    CurrentTick, Registration, Simulation, SimulationBuilder, Stage, TickInput, TypeTable,
};

use crate::backend::PhysicsBackend;
use crate::command::PhysicsExt;
use crate::components::{
    BodyType, Collider, Joint, JointKind, RigidBody, Shape, Transform, Velocity,
};
use crate::service::{PhysicsConfig, register_types, restore, setup, snapshot};
use crate::systems::register_systems;

/// How many ticks the run simulates.
pub const TICKS: u32 = 10;

/// The tick after which the snapshot is taken.
pub const SNAPSHOT_TICK: u32 = 3;

/// How many dynamic bodies the scene drops.
pub const BODIES: u32 = 64;

/// What [`snapshot_restore`] records. Two reports from backends that differ only in their
/// pool's thread count, from two layouts, or from two architectures are equal.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ConformanceReport {
    /// `Simulation::hash` after each of the [`TICKS`] ticks of the first run.
    pub first: Vec<(Tick, u64)>,
    /// `Simulation::hash` after each tick from [`SNAPSHOT_TICK`] + 1 to [`TICKS`] of the
    /// run resumed from the snapshot. Equal, tick for tick, to the tail of `first`.
    pub resumed: Vec<(Tick, u64)>,
    /// The physics snapshot's size at [`SNAPSHOT_TICK`], what milestone 1 measures against
    /// the prediction-world budget.
    pub snapshot_bytes: usize,
    /// [`PhysicsSnapshot::hash`](crate::PhysicsSnapshot::hash) of a snapshot taken after
    /// tick [`TICKS`] of the first run and of the resumed run. Reported, not required
    /// equal, until the verification item on the bytes' order stability decides.
    pub physics_hash: (u64, u64),
}

/// Runs ten ticks, snapshots at three, restores and re-runs, and requires the hash equal
/// at every tick after the restore (architecture section 10), with `backend` behind this
/// crate's three systems in a `Prediction` world under `Directed` region control and the
/// storage `layout` (SOUR's `harness::layout_mode`, or the production default).
///
/// The scene, written so that the restore has pending work of every kind to carry: one
/// region created directly, a fixed cuboid floor (a `Collider` row), [`BODIES`] dynamic
/// spheres of unit radius spawned directly with `Net` keys and a `ClaimedBy` for player
/// one in a stack that topples, with an impulse each recorded through the channel on the
/// first tick; two bones as kinematic bodies and a limited spherical `Joint` row between
/// them, driven by `Transform` writes from a kernel; and a proxy, a `RigidBody` row
/// without a claim. An exclusive `Stage::Input` system after `sim`'s `sets::INPUT` calls
/// `World::set_input_tick(player one, tick)` every tick, and the `Stage::Post` spawn at
/// tick [`SNAPSHOT_TICK`] comes from a kernel on a claimed body, so the spawn is predicted
/// rather than dropped; the same `Stage::Post` system despawns a body and writes a
/// `Transform` through `cmd.set`, so the snapshot point holds pending log entries, pending
/// keys and a pending command. After the restore a kernel writes a `Velocity` through
/// `Mut`, records a command, switches a bone to dynamic through `Mut<RigidBody>`, and the
/// region is frozen for one tick and thawed; every write, command, freeze and thaw
/// described as after the restore happens at the same ticks in the first run. Every step
/// takes `TickInput::default()` (the mode changes go through `world_mut`, tests only).
/// After tick [`SNAPSHOT_TICK`] a `World::snapshot` and a [`snapshot`](crate::snapshot) are
/// taken; after tick [`TICKS`] both are restored and the run continues to [`TICKS`] again.
/// Driven with `Executor::block_on`, so it is called from a test's main thread.
///
/// Fails with `InvalidData` naming the first tick whose hash differs, with what building
/// the simulation fails with, and as a step fails.
pub fn snapshot_restore(
    executor: &Executor,
    backend: Box<dyn PhysicsBackend>,
    config: PhysicsConfig,
    layout: LayoutConfig,
) -> Result<ConformanceReport> {
    let mut types = TypeTable::new();
    register_types(&mut types)?;
    let mut simulation = SimulationBuilder::new(WorldKind::Prediction, types)
        .layout(layout)
        .setup(move |world| setup(world, backend, config))
        .systems(register_systems)
        .systems(register_scene)
        .build()?;
    spawn_scene(simulation.world_mut())?;

    let mut first = Vec::new();
    let mut saved = None;
    for tick in 1..=TICKS {
        simulation = step(executor, simulation, tick)?;
        first.push((simulation.tick(), simulation.hash()?));
        if tick == SNAPSHOT_TICK {
            saved = Some((simulation.world().snapshot(), snapshot(simulation.world())?));
        }
    }
    let first_physics = snapshot(simulation.world())?;
    let (world_snapshot, physics_snapshot) = saved.expect("the run passes the snapshot tick");
    simulation.world_mut().restore(&world_snapshot)?;
    restore(simulation.world_mut(), &physics_snapshot)?;

    let mut resumed = Vec::new();
    for tick in SNAPSHOT_TICK + 1..=TICKS {
        simulation = step(executor, simulation, tick)?;
        resumed.push((simulation.tick(), simulation.hash()?));
    }
    let resumed_physics = snapshot(simulation.world())?;
    for (&(tick, first_hash), &(_, resumed_hash)) in
        first[SNAPSHOT_TICK as usize..].iter().zip(&resumed)
    {
        if first_hash != resumed_hash {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!(
                    "the world hash after tick {} of the resumed run differs from the first \
                     run's: {resumed_hash:#018x} against {first_hash:#018x}",
                    tick.0
                ),
            ));
        }
    }
    Ok(ConformanceReport {
        first,
        resumed,
        snapshot_bytes: physics_snapshot.size_in_bytes(),
        physics_hash: (first_physics.hash(), resumed_physics.hash()),
    })
}

// ---- The scene ----

const REGION: RegionId = RegionId(1);
const PLAYER: PlayerId = PlayerId(1);
/// The tick before whose step the region is frozen; it is thawed before the next one.
const FREEZE_TICK: u32 = 7;
/// The tick whose kernel writes a velocity and records a command.
const VELOCITY_TICK: u32 = 5;
/// The tick whose kernel switches a bone to dynamic.
const LIMP_TICK: u32 = 6;

const fn key(n: u64) -> StableKey {
    StableKey::Net(NetId::cooked(n))
}

const FLOOR: StableKey = key(1);
const BONE_A: StableKey = key(2);
const BONE_B: StableKey = key(3);
const JOINT: StableKey = key(4);
const PROXY: StableKey = key(5);

/// The `index`th sphere of the stack.
const fn sphere(index: u32) -> StableKey {
    key(100 + index as u64)
}

fn sphere_index(key: StableKey) -> Option<u32> {
    (0..BODIES).find(|&index| sphere(index) == key)
}

fn scalar(value: f64) -> Scalar {
    Scalar::from_f64(value)
}

fn spawn_scene(world: &mut World) -> Result<()> {
    world.create_region(RegionConfig { id: REGION, seed: 7, mode: RegionMode::Simulated })?;
    let floor = Shape::cuboid(Vec3::new(Scalar::from_i32(50), Scalar::ONE, Scalar::from_i32(50)));
    world.spawn(FLOOR, REGION, (Transform::IDENTITY, Collider::new(floor)))?;
    let claim = ClaimedBy { player: PLAYER };
    for index in 0..BODIES {
        // A stack leaning a little more with every sphere, so it topples.
        let lean = scalar(0.05) * Scalar::from_u32(index);
        let height = scalar(2.0) + scalar(2.0) * Scalar::from_u32(index);
        world.spawn(
            sphere(index),
            REGION,
            (
                Transform::at(Vec3::new(lean, height, Scalar::ZERO)),
                Velocity::ZERO,
                RigidBody::dynamic(),
                Collider::new(Shape::sphere(Scalar::ONE)),
                claim,
            ),
        )?;
    }
    let bone = Collider::new(Shape::capsule(Scalar::HALF, scalar(0.1)));
    for (key, x) in [(BONE_A, 10), (BONE_B, 11)] {
        let at = Transform::at(Vec3::new(Scalar::from_i32(x), Scalar::TWO, Scalar::ZERO));
        world.spawn(key, REGION, (at, RigidBody::kinematic(), bone, claim))?;
    }
    let joint = Joint::new(JointKind::Spherical, BONE_A, BONE_B)
        .with_anchors(Vec3::X * Scalar::HALF, -Vec3::X * Scalar::HALF)
        .with_limits(-Scalar::ONE, Scalar::ONE);
    world.spawn(JOINT, REGION, (joint, OwnedBy { owner: EntityRef::new(BONE_A) }))?;
    let proxy = Transform::at(Vec3::new(Scalar::from_i32(-10), Scalar::TWO, Scalar::ZERO));
    world.spawn(
        PROXY,
        REGION,
        (proxy, RigidBody::dynamic(), Collider::new(Shape::cuboid(Vec3::ONE))),
    )?;
    Ok(())
}

/// One step of the scene: the mode changes through `world_mut`, then the step.
fn step(executor: &Executor, mut simulation: Simulation, tick: u32) -> Result<Simulation> {
    if tick == FREEZE_TICK {
        simulation.world_mut().set_region_mode(REGION, RegionMode::Frozen)?;
    } else if tick == FREEZE_TICK + 1 {
        simulation.world_mut().set_region_mode(REGION, RegionMode::Simulated)?;
    }
    Ok(executor.block_on(simulation.step(executor, TickInput::default()))?)
}

fn register_scene(registration: &mut Registration) {
    registration.stage(Stage::Input).sets(&[&INPUT_TICK_SET]).add(&INPUT_TICK);
    registration.stage(Stage::Simulation).add(&DRIVE);
    registration.stage(Stage::Post).add(&REACT);
}

static INPUT_TICK_SET: SystemSet =
    SystemSet::new("physics_api::conformance::input_tick_set").after(&[&lactose_sim::sets::INPUT]);

/// Player one has an input tick every tick, so a spawn from a claimed body is predicted.
#[system(
    name = "physics_api::conformance::input_tick",
    worlds = [server, prediction],
    exclusive,
    in_set = INPUT_TICK_SET
)]
fn input_tick(world: &mut World) {
    let tick = world.tick();
    world.set_input_tick(PLAYER, tick);
}

/// The simulation stage's kernel: the first tick's impulses, the bones driven by
/// `Transform` writes, and the post-restore velocity write, command and limp bone.
#[system(name = "physics_api::conformance::drive", worlds = [server, prediction])]
fn drive(
    cmd: Commands,
    key: StableKeyOf,
    now: Res<CurrentTick>,
    pos: Mut<Transform>,
    body: Mut<RigidBody>,
    vel: Option<Mut<Velocity>>,
) {
    let (tick, key) = (now.get().0, *key);
    if tick == 1
        && let Some(index) = sphere_index(key)
    {
        let push =
            Vec3::new(Scalar::from_u32(index % 5), Scalar::ZERO, Scalar::from_u32(index % 3));
        cmd.physics().apply_impulse(key, push);
    }
    if key == BONE_A || key == BONE_B {
        pos.translation.y = Scalar::TWO + scalar(0.1) * Scalar::from_u32(tick % 7);
    }
    if tick == VELOCITY_TICK && key == sphere(4) {
        if let Some(vel) = vel.as_mut() {
            vel.linear.x = Scalar::from_i32(3);
        }
        cmd.physics().apply_torque_impulse(sphere(5), Vec3::Y);
    }
    if tick == LIMP_TICK && key == BONE_A {
        body.body_type = BodyType::Dynamic;
    }
}

/// The post stage's kernel at the snapshot tick, on a claimed body: a predicted spawn, a
/// despawn, a `Transform` written through `cmd.set` and a command, all left for the next
/// tick's sync and step when the snapshot is taken.
#[system(name = "physics_api::conformance::react", worlds = [server, prediction], writes = [Transform])]
fn react(cmd: Commands, key: StableKeyOf, now: Res<CurrentTick>, body: &RigidBody) {
    if now.get().0 != SNAPSHOT_TICK || *key != sphere(0) {
        return;
    }
    let at = Transform::at(Vec3::new(Scalar::from_i32(20), Scalar::TWO, Scalar::ZERO));
    cmd.spawn((at, Velocity::ZERO, *body, Collider::new(Shape::sphere(Scalar::HALF))));
    cmd.despawn(sphere(1));
    cmd.set(sphere(2), Transform::at(Vec3::new(Scalar::from_i32(-20), Scalar::TWO, Scalar::ZERO)));
    cmd.physics().apply_impulse(sphere(3), Vec3::X);
}
