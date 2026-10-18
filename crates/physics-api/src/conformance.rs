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
    ChannelWriter, ClaimedBy, Commands, Executor, LayoutConfig, Mut, OwnedBy, RegionConfig,
    RegionId, RegionMode, Res, StableKeyOf, SystemSet, World, WorldKind, system,
};
use lactose_math::{Scalar, Vec3};
use lactose_sim::{
    CurrentTick, Registration, Simulation, SimulationBuilder, Stage, TickInput, TypeTable,
};

use crate::backend::PhysicsBackend;
use crate::command::{PhysicsCommands, PhysicsExt};
use crate::components::{
    BodyHandle, BodyType, Collider, Joint, JointHandle, JointKind, RigidBody, Shape, Transform,
    Velocity,
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
/// `Transform` through `cmd.set`, so the snapshot point holds pending log entries and
/// pending keys, and the simulation kernel records an impulse at the tick after it, which
/// the re-simulated tick records again. After the restore a kernel writes a `Velocity`
/// through `Mut`, records a command, switches a bone to dynamic through `Mut<RigidBody>`,
/// and the region is frozen for one tick and thawed; every write, command, freeze and thaw
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

/// The obligation [`PhysicsBackend::snapshot`] states for the bytes of the empty world,
/// which [`setup`] keeps and the sync restores whenever it has removed every body (D124):
/// that they restore to a world that steps as a newly built one does. In a `Server` world
/// under `Directed` region control and the storage `layout`, a backend from `new_backend`
/// creates and steps a floor, a leaning stack of spheres and two jointed boxes for a few
/// ticks; every row is despawned, so the sync removes every body and restores those bytes;
/// the scene is spawned again and run. A second backend from `new_backend`, in a world built
/// at the tick the scene was spawned again, runs the same scene. Returns the world hashes
/// after each tick of the scene, the two worlds' being equal, and requires the handle rows
/// equal at the end, so a backend that keeps state outside its bytes (a handle counter, an
/// arena's free list, a warm start) fails here rather than in a reload.
///
/// Fails with `InvalidData` naming the first tick whose hash differs, or the first key
/// whose handle row does, with what building the simulation fails with, and as a step
/// fails.
pub fn empty_restore(
    executor: &Executor,
    new_backend: &dyn Fn() -> Box<dyn PhysicsBackend>,
    config: PhysicsConfig,
    layout: LayoutConfig,
) -> Result<Vec<(Tick, u64)>> {
    let build = |tick: Tick| -> Result<Simulation> {
        let mut types = TypeTable::new();
        register_types(&mut types)?;
        let backend = new_backend();
        let mut simulation = SimulationBuilder::new(WorldKind::Server, types)
            .layout(layout)
            .tick(tick)
            .setup(move |world| setup(world, backend, config))
            .systems(register_systems)
            .build()?;
        let region = RegionConfig { id: REGION, seed: 7, mode: RegionMode::Simulated };
        simulation.world_mut().create_region(region)?;
        Ok(simulation)
    };
    let run = |mut simulation: Simulation, ticks: u32, hashes: &mut Vec<(Tick, u64)>| {
        for _ in 0..ticks {
            simulation = executor.block_on(simulation.step(executor, TickInput::default()))?;
            hashes.push((simulation.tick(), simulation.hash()?));
        }
        Ok::<_, Error>(simulation)
    };

    let mut used = build(Tick(0))?;
    let keys = spawn_reload_scene(used.world_mut())?;
    used = run(used, WARM_TICKS, &mut Vec::new())?;
    for &key in &keys {
        used.world_mut().despawn(key)?;
    }
    used = run(used, 1, &mut Vec::new())?;
    spawn_reload_scene(used.world_mut())?;
    let mut fresh = build(used.tick())?;
    spawn_reload_scene(fresh.world_mut())?;

    let (mut ours, mut theirs) = (Vec::new(), Vec::new());
    let used = run(used, TICKS, &mut ours)?;
    let fresh = run(fresh, TICKS, &mut theirs)?;
    if let Some((&(tick, hash), &(_, fresh_hash))) =
        ours.iter().zip(&theirs).find(|(ours, theirs)| ours != theirs)
    {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "the world hash after tick {} of the emptied backend differs from a new \
                 backend's: {hash:#018x} against {fresh_hash:#018x}",
                tick.0
            ),
        ));
    }
    let handles = |world: &World, key: StableKey| {
        (world.get::<BodyHandle>(key).copied(), world.get::<JointHandle>(key).copied())
    };
    if let Some(&key) =
        keys.iter().find(|&&key| handles(used.world(), key) != handles(fresh.world(), key))
    {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("the emptied backend's handle row for {key:?} differs from a new backend's"),
        ));
    }
    Ok(ours)
}

/// How many ticks [`empty_restore`]'s first backend steps the scene before it is emptied.
const WARM_TICKS: u32 = 20;

/// [`empty_restore`]'s scene in `REGION`: a fixed floor, a stack of spheres resting on it
/// and on each other, leaning so that it topples, and two boxes under a fixed joint. The
/// keys, joint last.
fn spawn_reload_scene(world: &mut World) -> Result<Vec<StableKey>> {
    let floor = Shape::cuboid(Vec3::new(Scalar::from_i32(50), Scalar::ONE, Scalar::from_i32(50)));
    let floor_at = Transform::at(Vec3::new(Scalar::ZERO, -Scalar::ONE, Scalar::ZERO));
    world.spawn(FLOOR, REGION, (floor_at, Collider::new(floor)))?;
    let mut keys = vec![FLOOR];
    for index in 0..8 {
        let lean = scalar(0.1) * Scalar::from_u32(index);
        let height = Scalar::HALF + Scalar::from_u32(index);
        let at = Transform::at(Vec3::new(lean, height, Scalar::ZERO));
        let ball = Collider::new(Shape::sphere(Scalar::HALF));
        world.spawn(sphere(index), REGION, (at, Velocity::ZERO, RigidBody::dynamic(), ball))?;
        keys.push(sphere(index));
    }
    for (key, x) in [(BONE_A, 4), (BONE_B, 5)] {
        let at = Transform::at(Vec3::new(Scalar::from_i32(x), Scalar::HALF, Scalar::ZERO));
        let cube = Collider::new(Shape::cuboid(Vec3::ONE * scalar(0.4)));
        world.spawn(key, REGION, (at, Velocity::ZERO, RigidBody::dynamic(), cube))?;
        keys.push(key);
    }
    let joint = Joint::new(JointKind::Fixed, BONE_A, BONE_B)
        .with_anchors(Vec3::X * Scalar::HALF, -Vec3::X * Scalar::HALF);
    world.spawn(JOINT, REGION, (joint,))?;
    keys.push(JOINT);
    Ok(keys)
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
/// `Transform` writes, the impulse of the tick after the snapshot, recorded again when that
/// tick is re-simulated, and the post-restore velocity write, command and limp bone.
#[system(name = "physics_api::conformance::drive", worlds = [server, prediction])]
fn drive(
    physics: ChannelWriter<PhysicsCommands>,
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
        physics.apply_impulse(key, push);
    }
    if tick == SNAPSHOT_TICK + 1 && key == sphere(0) {
        physics.apply_impulse(sphere(3), Vec3::X);
    }
    if key == BONE_A || key == BONE_B {
        pos.translation.y = Scalar::TWO + scalar(0.1) * Scalar::from_u32(tick % 7);
    }
    if tick == VELOCITY_TICK && key == sphere(4) {
        if let Some(vel) = vel.as_mut() {
            vel.linear.x = Scalar::from_i32(3);
        }
        physics.apply_torque_impulse(sphere(5), Vec3::Y);
    }
    if tick == LIMP_TICK && key == BONE_A {
        body.body_type = BodyType::Dynamic;
    }
}

/// The post stage's kernel at the snapshot tick, on a claimed body: a predicted spawn, a
/// despawn and a `Transform` written through `cmd.set`, all left for the next tick's sync
/// when the snapshot is taken. It records no command: a writer of the physics channel
/// after the step is a schedule build error (D126).
#[system(name = "physics_api::conformance::react", worlds = [server, prediction], writes = [Transform])]
fn react(cmd: Commands, key: StableKeyOf, now: Res<CurrentTick>, body: &RigidBody) {
    if now.get().0 != SNAPSHOT_TICK || *key != sphere(0) {
        return;
    }
    let at = Transform::at(Vec3::new(Scalar::from_i32(20), Scalar::TWO, Scalar::ZERO));
    cmd.spawn((at, Velocity::ZERO, *body, Collider::new(Shape::sphere(Scalar::HALF))));
    cmd.despawn(sphere(1));
    cmd.set(sphere(2), Transform::at(Vec3::new(Scalar::from_i32(-20), Scalar::TWO, Scalar::ZERO)));
}
