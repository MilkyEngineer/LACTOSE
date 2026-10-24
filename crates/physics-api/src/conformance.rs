//! The conformance suite a backend's test crate runs (architecture sections 10 and 20):
//! the snapshot-and-restore run that `lactose-physics-rapier` must pass before this crate
//! is frozen, as a function so that every backend runs the same scene under every layout,
//! and the [`validity`] run that holds a backend to the rules of `README.md`, "Validity".
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
use lactose_math::{Quat, Scalar, Vec3};
use lactose_sim::{
    CurrentTick, Registration, Simulation, SimulationBuilder, Stage, TickInput, TypeTable,
};

use crate::backend::{BodyDesc, PhysicsBackend, StepInput, StepOutput};
use crate::command::{PhysicsCommand, PhysicsCommands, PhysicsExt};
use crate::components::{
    BodyHandle, BodyType, Collider, Joint, JointHandle, JointKind, RigidBody, Shape, Transform,
    Velocity, locked_axes,
};
use crate::service::{PhysicsConfig, register_types, restore, setup, snapshot};
use crate::systems::register_systems;
use crate::validity::Invalid;

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

// ---- The validity run ----

/// The bytes of `backend`'s world.
fn bytes(backend: &dyn PhysicsBackend) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    backend.snapshot(&mut out)?;
    Ok(out)
}

fn mismatch(what: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidData, what.into())
}

/// What a creating call must have done with a value the rules refuse: returned the
/// `Invalid` as an `InvalidData` error whose message is the `Invalid`'s.
fn refused_as(what: &str, result: Result<()>, expected: &Invalid) -> Result<()> {
    match result {
        Ok(()) => Err(mismatch(format!("{what}: accepted, but the rules refuse it: {expected}"))),
        Err(error) if error.kind() != ErrorKind::InvalidData => {
            Err(mismatch(format!("{what}: refused with {:?}, not InvalidData", error.kind())))
        }
        Err(error) if error.message() != expected.to_string() => Err(mismatch(format!(
            "{what}: refused with the message {:?}, not the rules' {:?}",
            error.message(),
            expected.to_string()
        ))),
        Err(_) => Ok(()),
    }
}

/// The description of a dynamic body with `collider` at `transform`, at rest.
fn described<'a>(transform: &'a Transform, collider: &'a Collider) -> BodyDesc<'a> {
    BodyDesc {
        body: Some(RigidBody::dynamic()),
        collider: Some(collider),
        transform,
        velocity: None,
        frozen: false,
    }
}

fn vec(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(scalar(x), scalar(y), scalar(z))
}

/// One edit of a valid value that breaks a rule.
type Edit<T> = fn(&mut T);

/// A rotation about nothing whose squared length is one plus `off`.
fn off_unit(off: f64) -> Quat {
    Quat::new(Scalar::ZERO, Scalar::ZERO, Scalar::ZERO, scalar((1.0 + off).sqrt()))
}

/// Holds `backend` to the validity rules (`README.md`, "Validity"; D135): what the
/// predicates refuse, the backend refuses or ignores with the predicates' message and
/// without touching its world, and what they accept, it accepts. The backend is driven
/// directly, with no world, so a backend's test crate calls it once per pool size.
///
/// The run: two dynamic unit spheres are created and the world's bytes are taken. Then,
/// for a table that breaks each rule of each predicate at least once, every refused
/// [`Transform`], [`Velocity`], [`RigidBody`] and [`Collider`] goes through `create_body`
/// in the description of a third body (`InvalidData` with the `Invalid`'s message) and
/// through its `set_*` on the first sphere (ignored); every refused [`Joint`] goes through
/// `create_joint` between the two spheres (`InvalidData`), and a valid one between the
/// first sphere and itself (`InvalidData`); after every call the bytes equal those taken.
/// Then each refused [`PhysicsCommand`] is the one command of a step under no gravity,
/// after which the first sphere, if reported, is where it was created, at rest and not
/// teleported. Last, the edges the rules accept (a rotation off unit by half the
/// tolerance, a negated identity, a zero mass, a negative gravity scale, every axis
/// locked, a capsule of zero half height, zero coefficients, a restitution above one, a
/// fixed or spherical joint with a zero axis, a revolute joint about a scaled axis, equal
/// limits) go through `create_body` and `create_joint` and are accepted.
///
/// Fails with `InvalidData` naming the first value the backend treated otherwise.
pub fn validity(mut backend: Box<dyn PhysicsBackend>) -> Result<()> {
    let nan = vec(f64::NAN, 0.0, 0.0);
    let ball = Collider::new(Shape::sphere(Scalar::HALF));
    let edited_body = |edit: Edit<RigidBody>| {
        let mut body = RigidBody::dynamic();
        edit(&mut body);
        body
    };
    let edited_collider = |edit: Edit<Collider>| {
        let mut collider = ball;
        edit(&mut collider);
        collider
    };
    let (first, second, third, joint_key) = (sphere(0), sphere(1), sphere(2), sphere(3));
    let origin = Transform::at(vec(0.0, 5.0, 0.0));
    backend.create_body(first, &described(&origin, &ball))?;
    backend.create_body(second, &described(&Transform::at(vec(3.0, 5.0, 0.0)), &ball))?;
    let baseline = bytes(&*backend)?;
    let unchanged = |backend: &dyn PhysicsBackend, what: &str| -> Result<()> {
        if bytes(backend)? == baseline {
            Ok(())
        } else {
            Err(mismatch(format!("{what}: the refused call changed the backend's bytes")))
        }
    };
    let rules_accept = |what: &str| mismatch(format!("{what}: the rules accept it"));

    let transforms = [
        ("a NaN translation", Transform::at(nan)),
        ("an infinite translation", Transform::at(vec(0.0, f64::INFINITY, 0.0))),
        (
            "a NaN rotation",
            Transform::new(
                Vec3::ZERO,
                Quat::new(scalar(f64::NAN), Scalar::ZERO, Scalar::ZERO, Scalar::ONE),
            ),
        ),
        ("a rotation off unit by twice the tolerance", Transform::new(Vec3::ZERO, off_unit(2e-6))),
        (
            "a zero rotation",
            Transform::new(
                Vec3::ZERO,
                Quat::new(Scalar::ZERO, Scalar::ZERO, Scalar::ZERO, Scalar::ZERO),
            ),
        ),
    ];
    for (what, transform) in &transforms {
        let expected = transform.validate().err().ok_or_else(|| rules_accept(what))?;
        let create = format!("create_body with {what}");
        let result = backend.create_body(third, &described(transform, &ball)).map(|_| ());
        refused_as(&create, result, &expected)?;
        unchanged(&*backend, &create)?;
        backend.set_transform(first, transform);
        unchanged(&*backend, &format!("set_transform with {what}"))?;
    }

    let velocities = [
        ("a NaN linear velocity", Velocity { linear: nan, angular: Vec3::ZERO }),
        (
            "an infinite angular velocity",
            Velocity { linear: Vec3::ZERO, angular: vec(0.0, 0.0, f64::NEG_INFINITY) },
        ),
    ];
    for (what, velocity) in &velocities {
        let expected = velocity.validate().err().ok_or_else(|| rules_accept(what))?;
        let create = format!("create_body with {what}");
        let desc = BodyDesc { velocity: Some(velocity), ..described(&origin, &ball) };
        refused_as(&create, backend.create_body(third, &desc).map(|_| ()), &expected)?;
        unchanged(&*backend, &create)?;
        backend.set_velocity(first, velocity);
        unchanged(&*backend, &format!("set_velocity with {what}"))?;
    }

    let bodies: [(&str, Edit<RigidBody>); 6] = [
        ("a NaN mass", |b| b.mass = Scalar::from_f64(f64::NAN)),
        ("a negative mass", |b| b.mass = -Scalar::ONE),
        ("a negative linear damping", |b| b.linear_damping = -Scalar::HALF),
        ("a negative angular damping", |b| b.angular_damping = -Scalar::HALF),
        ("a NaN gravity scale", |b| b.gravity_scale = Scalar::from_f64(f64::NAN)),
        ("an unknown locked-axes bit", |b| b.locked_axes = 1 << 6),
    ];
    for (what, edit) in bodies {
        let body = edited_body(edit);
        let expected = body.validate().err().ok_or_else(|| rules_accept(what))?;
        let create = format!("create_body with {what}");
        let desc = BodyDesc { body: Some(body), ..described(&origin, &ball) };
        refused_as(&create, backend.create_body(third, &desc).map(|_| ()), &expected)?;
        unchanged(&*backend, &create)?;
        backend.set_body(first, Some(&body));
        unchanged(&*backend, &format!("set_body with {what}"))?;
    }

    let colliders: [(&str, Edit<Collider>); 11] = [
        ("a zero sphere radius", |c| c.shape = Shape::sphere(Scalar::ZERO)),
        ("a negative capsule radius", |c| c.shape = Shape::capsule(Scalar::HALF, -Scalar::ONE)),
        ("a negative capsule half height", |c| {
            c.shape = Shape::capsule(-Scalar::HALF, Scalar::ONE)
        }),
        ("a zero cuboid extent", |c| {
            c.shape = Shape::cuboid(Vec3::new(Scalar::ONE, Scalar::ZERO, Scalar::ONE));
        }),
        ("a NaN in a shape field the kind does not read", |c| {
            c.shape.half_extents.x = Scalar::from_f64(f64::NAN);
        }),
        ("a NaN offset", |c| {
            c.offset = Vec3::new(Scalar::from_f64(f64::NAN), Scalar::ZERO, Scalar::ZERO)
        }),
        ("a non-unit collider rotation", |c| {
            c.rotation = Quat::new(Scalar::ZERO, Scalar::ZERO, Scalar::ZERO, Scalar::TWO);
        }),
        ("a negative friction", |c| c.friction = -Scalar::HALF),
        ("a negative restitution", |c| c.restitution = -Scalar::HALF),
        ("a negative density", |c| c.density = -Scalar::ONE),
        ("a negative impact threshold", |c| c.impact_threshold = -Scalar::ONE),
    ];
    for (what, edit) in colliders {
        let collider = edited_collider(edit);
        let expected = collider.validate().err().ok_or_else(|| rules_accept(what))?;
        let create = format!("create_body with {what}");
        let desc = BodyDesc { collider: Some(&collider), ..described(&origin, &ball) };
        refused_as(&create, backend.create_body(third, &desc).map(|_| ()), &expected)?;
        unchanged(&*backend, &create)?;
        backend.set_collider(first, Some(&collider));
        unchanged(&*backend, &format!("set_collider with {what}"))?;
    }

    let joint = |kind: JointKind| Joint::new(kind, first, second);
    let joints = [
        ("a NaN anchor", joint(JointKind::Spherical).with_anchors(nan, Vec3::ZERO)),
        ("a zero revolute axis", joint(JointKind::Revolute).with_axis(Vec3::ZERO)),
        ("a zero prismatic axis", joint(JointKind::Prismatic).with_axis(Vec3::ZERO)),
        (
            "reversed limits on a fixed joint",
            joint(JointKind::Fixed).with_limits(Scalar::HALF, -Scalar::HALF),
        ),
        (
            "reversed limits on a spherical joint",
            joint(JointKind::Spherical).with_limits(Scalar::HALF, -Scalar::HALF),
        ),
        (
            "reversed limits on a revolute joint",
            joint(JointKind::Revolute).with_limits(Scalar::HALF, -Scalar::HALF),
        ),
        ("a NaN limit on an unlimited joint", {
            let mut unlimited = joint(JointKind::Fixed);
            unlimited.limit_max = Scalar::from_f64(f64::NAN);
            unlimited
        }),
    ];
    for (what, value) in &joints {
        let expected = value.validate().err().ok_or_else(|| rules_accept(what))?;
        let create = format!("create_joint with {what}");
        let result = backend.create_joint(joint_key, value, first, second).map(|_| ());
        refused_as(&create, result, &expected)?;
        unchanged(&*backend, &create)?;
    }
    let same = "create_joint between a body and itself";
    match backend.create_joint(joint_key, &joint(JointKind::Fixed), first, first) {
        Ok(_) => return Err(mismatch(format!("{same}: accepted"))),
        Err(error) if error.kind() != ErrorKind::InvalidData => {
            return Err(mismatch(format!(
                "{same}: refused with {:?}, not InvalidData",
                error.kind()
            )));
        }
        Err(_) => unchanged(&*backend, same)?,
    }

    let commands = [
        ("a NaN impulse", PhysicsCommand::ApplyImpulse { body: first, impulse: nan }),
        (
            "a NaN point",
            PhysicsCommand::ApplyImpulseAt { body: first, impulse: Vec3::X, point: nan },
        ),
        ("a NaN torque impulse", PhysicsCommand::ApplyTorqueImpulse { body: first, torque: nan }),
        ("a NaN force", PhysicsCommand::ApplyForce { body: first, force: nan }),
        ("a NaN torque", PhysicsCommand::ApplyTorque { body: first, torque: nan }),
        (
            "a teleport to a NaN",
            PhysicsCommand::Teleport { body: first, transform: Transform::at(nan) },
        ),
        (
            "a teleport with a non-unit rotation",
            PhysicsCommand::Teleport {
                body: first,
                transform: Transform::new(Vec3::ZERO, off_unit(2e-6)),
            },
        ),
    ];
    let mut output = StepOutput::default();
    for (tick, (what, command)) in (1..).zip(commands) {
        if command.validate().is_ok() {
            return Err(rules_accept(what));
        }
        let input = StepInput {
            tick: Tick(tick),
            dt: scalar(1.0 / 60.0),
            gravity: Vec3::ZERO,
            commands: std::slice::from_ref(&command),
        };
        output.clear();
        backend.step(&input, &mut output);
        if let Some(state) = output.bodies.iter().find(|state| state.key == first)
            && (state.transform != origin || state.velocity != Velocity::ZERO || state.teleported)
        {
            return Err(mismatch(format!("a step with {what} moved the body: {state:?}")));
        }
    }

    let minus_identity = Quat::new(Scalar::ZERO, Scalar::ZERO, Scalar::ZERO, -Scalar::ONE);
    let accepted: [(&str, Transform, RigidBody, Collider); 7] = [
        (
            "a rotation off unit by half the tolerance",
            Transform::new(vec(0.0, 9.0, 0.0), off_unit(0.5e-6)),
            RigidBody::dynamic(),
            ball,
        ),
        (
            "a negated identity rotation",
            Transform::new(vec(0.0, 11.0, 0.0), minus_identity),
            RigidBody::dynamic(),
            ball,
        ),
        (
            "a zero mass and a negative gravity scale",
            Transform::at(vec(0.0, 13.0, 0.0)),
            edited_body(|b| {
                b.mass = Scalar::ZERO;
                b.gravity_scale = -Scalar::ONE;
            }),
            ball,
        ),
        (
            "every axis locked",
            Transform::at(vec(0.0, 15.0, 0.0)),
            edited_body(|b| b.locked_axes = locked_axes::TRANSLATION | locked_axes::ROTATION),
            ball,
        ),
        (
            "a capsule of zero half height",
            Transform::at(vec(0.0, 17.0, 0.0)),
            RigidBody::kinematic(),
            Collider::new(Shape::capsule(Scalar::ZERO, Scalar::HALF)),
        ),
        (
            "zero coefficients",
            Transform::at(vec(0.0, 19.0, 0.0)),
            RigidBody::fixed(),
            edited_collider(|c| {
                c.friction = Scalar::ZERO;
                c.restitution = Scalar::ZERO;
                c.density = Scalar::ZERO;
                c.impact_threshold = Scalar::ZERO;
            }),
        ),
        (
            "a restitution above one",
            Transform::at(vec(0.0, 21.0, 0.0)),
            RigidBody::dynamic(),
            edited_collider(|c| c.restitution = Scalar::from_f64(7.0)),
        ),
    ];
    for (index, (what, transform, body, collider)) in accepted.iter().enumerate() {
        let desc = BodyDesc { body: Some(*body), ..described(transform, collider) };
        if let Err(invalid) = desc.validate() {
            return Err(mismatch(format!("{what}: the rules refuse it: {invalid}")));
        }
        let key = sphere(10 + u32::try_from(index).map_err(|_| mismatch("too many edges"))?);
        backend
            .create_body(key, &desc)
            .map_err(|error| mismatch(format!("create_body with {what}: refused: {error}")))?;
    }
    let accepted_joints = [
        ("a fixed joint with a zero axis", joint(JointKind::Fixed).with_axis(Vec3::ZERO)),
        ("a spherical joint with a zero axis", joint(JointKind::Spherical).with_axis(Vec3::ZERO)),
        (
            "a revolute joint about a scaled axis",
            joint(JointKind::Revolute).with_axis(vec(0.0, 0.0, 3.0)),
        ),
        (
            "a prismatic joint with equal limits",
            joint(JointKind::Prismatic).with_limits(Scalar::HALF, Scalar::HALF),
        ),
    ];
    for (what, value) in &accepted_joints {
        if let Err(invalid) = value.validate() {
            return Err(mismatch(format!("{what}: the rules refuse it: {invalid}")));
        }
        backend
            .create_joint(joint_key, value, first, second)
            .map_err(|error| mismatch(format!("create_joint with {what}: refused: {error}")))?;
        backend.remove_joint(joint_key);
    }
    Ok(())
}
