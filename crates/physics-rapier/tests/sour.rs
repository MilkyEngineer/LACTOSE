//! The Rapier backend under SOUR (architecture section 8.3): `physics-api`'s SOUR scenes,
//! with this backend in place of the test one, each built from the runner's session through
//! the glue `lactose-sim`'s README shows (the protocol module by `#[path]`, the executor from
//! `lactose-task`) and `physics-api`'s test harness by path, recording `(tick, hash)` after
//! every step. The solver pool has two threads in every mode: the pool's size is not one of
//! SOUR's axes, and `determinism.rs` and `conformance.rs` vary it. The prediction scene also
//! rolls back and records the re-simulated ticks again, which the runner requires equal to
//! the first run's. Beside them, `physics-api`'s conformance run with the hash of the
//! backend's bytes, and a limited hinge driven against both its limits. Outside the runner
//! the session is production and nothing is recorded.

#![forbid(unsafe_code)]

#[path = "../../physics-api/tests/common/mod.rs"]
mod common;

#[path = "../../../tools/xtask/src/harness/protocol.rs"]
#[expect(
    clippy::wrong_self_convention,
    reason = "the runner's module as written; it is included, not owned, here"
)]
mod protocol;

use std::num::NonZeroU32;

use common::*;
use lactose_core::{Error, ErrorKind, Result, StableKey};
use lactose_ecs::{
    ClaimedBy, Executor, LayoutConfig, OwnedBy, RegionConfig, RegionMode, World, WorldKind,
};
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::{
    Collider, Joint, JointKind, PhysicsBackend, PhysicsCommand, PhysicsConfig, RigidBody, Shape,
    Transform, Velocity, conformance, corrected, restore, snapshot,
};
use lactose_sim::harness::{ChunkCapacity, layout_mode, layout_mode_of, run_recorded};
use lactose_sim::{Simulation, TickInput};
use lactose_task::ChaosConfig;
use protocol::{Capacity, Layout, Recorder, RunConfig, Session};

/// What a test builds from its session, and reports back.
struct SessionSetup {
    session: Session,
    executor: Executor,
    layout: LayoutConfig,
}

impl SessionSetup {
    /// Reads the session and builds the executor and layout it asks for.
    fn from_env() -> Result<Self> {
        let session = Session::from_env().map_err(protocol_error)?;
        let config = *session.config();
        let layout = match config.layout {
            Layout::Production => layout_mode(ChunkCapacity::Production, 0)?,
            Layout::Perturbed { capacity: Capacity::Rows(n), seed } => {
                layout_mode(ChunkCapacity::Rows(n), seed)?
            }
            Layout::Perturbed { capacity: Capacity::Bytes(n), seed } => {
                layout_mode(ChunkCapacity::Bytes(n), seed)?
            }
        };
        let mut executor = Executor::builder();
        if let Some(workers) = config.workers {
            executor = executor.frame_workers(workers.get());
        }
        if let Some(seed) = config.chaos_seed {
            executor = executor.chaos(ChaosConfig {
                seed,
                shuffle_steals: true,
                inject_yields: true,
                force_split: true,
                record: true,
            });
        }
        Ok(Self { session, executor: executor.build()?, layout })
    }

    /// Opens the recorder with what the executor and `layout`, the layout the test's world
    /// reports, were built with.
    #[expect(
        clippy::disallowed_methods,
        reason = "SOUR's glue is test code outside the step: it reports what the executor \
                  was built with"
    )]
    fn recorder(&self, test: &str, layout: &LayoutConfig) -> Result<Recorder> {
        let config = self.session.config();
        let applied = RunConfig {
            workers: config.workers.map(|_| {
                NonZeroU32::new(self.executor.frame_worker_count()).expect("at least one worker")
            }),
            // The world's layout as the runner names it; one no SOUR mode builds fails here.
            layout: match layout_mode_of(layout)? {
                (ChunkCapacity::Production, _) => Layout::Production,
                (ChunkCapacity::Rows(n), seed) => {
                    Layout::Perturbed { capacity: Capacity::Rows(n), seed }
                }
                (ChunkCapacity::Bytes(n), seed) => {
                    Layout::Perturbed { capacity: Capacity::Bytes(n), seed }
                }
            },
            chaos_seed: self.executor.chaos_config().map(|chaos| chaos.seed),
        };
        self.session.recorder(test, &applied).map_err(protocol_error)
    }

    /// Writes the execution log a chaos run records.
    #[expect(
        clippy::disallowed_methods,
        reason = "SOUR's glue is test code outside the step: it writes the chaos log"
    )]
    fn finish(&self, test: &str) -> Result<()> {
        if let Some(log) = self.executor.execution_log() {
            self.session.write_execution_log(test, log.as_bytes()).map_err(protocol_error)?;
        }
        Ok(())
    }
}

fn protocol_error(error: protocol::ProtocolError) -> Error {
    Error::new(ErrorKind::InvalidData, error.to_string())
}

const TICKS: u32 = 150;
/// The tick after which the prediction scene is snapshotted and later restored.
const ROLLBACK_TICK: u32 = 30;
const SPHERES: u64 = 40;
const FLOOR: u64 = 1;
const BONE_A: u64 = 2;
const BONE_B: u64 = 3;
const JOINT: u64 = 4;
const PROXY: u64 = 5;
const DRIFTER: u64 = 11;
const BOOKKEEPING: u64 = 12;

fn sphere_key(index: u64) -> StableKey {
    key(100 + index)
}

/// The backend every scene here steps with: Rapier on a pool of two threads.
#[expect(
    clippy::disallowed_methods,
    reason = "the test builds the solver pool that lactose-app builds in a shipped binary"
)]
fn backend() -> Box<dyn PhysicsBackend> {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap();
    lactose_physics_rapier::backend(pool)
}

/// `physics-api`'s scene: a floor reporting contacts, a falling stack, two bones joined and
/// driven, a second region frozen and thawed, and game code that pushes, teleports, writes
/// velocities, spawns, despawns and moves through the run; stepped by Rapier.
fn scene(kind: WorldKind, layout: LayoutConfig) -> Harness {
    let mut options = Options::new(kind);
    options.layout = layout;
    let mut h = options.build_with(backend(), CallLog::default());
    let second = RegionConfig { id: OTHER_REGION, seed: 9, mode: RegionMode::Simulated };
    h.world_mut().create_region(second).unwrap();
    let claim = ClaimedBy { player: PLAYER };
    let floor = Collider::new(Shape::cuboid(v(50, 1, 50))).with_events(Scalar::ONE);
    h.spawn(key(FLOOR), (Transform::IDENTITY, floor));
    for index in 0..SPHERES {
        let lean = Scalar::from_f64(0.03) * Scalar::from_u64(index);
        let height = Scalar::from_i32(2) + Scalar::from_f64(1.1) * Scalar::from_u64(index);
        let at = Transform::at(lactose_math::Vec3::new(lean, height, Scalar::ZERO));
        let collider = sphere().with_events(Scalar::HALF);
        h.spawn(sphere_key(index), (at, Velocity::ZERO, RigidBody::dynamic(), collider, claim));
    }
    for (n, x) in [(BONE_A, 10), (BONE_B, 11)] {
        h.spawn(key(n), (at(x, 3, 0), Velocity::ZERO, RigidBody::kinematic(), sphere(), claim));
    }
    let joint = Joint::new(JointKind::Spherical, key(BONE_A), key(BONE_B));
    let owner = OwnedBy { owner: lactose_core::EntityRef::new(key(BONE_A)) };
    h.spawn(key(JOINT), (joint, owner));
    h.spawn(key(PROXY), (at(-10, 3, 0), Velocity::ZERO, RigidBody::dynamic(), sphere()));
    let drifting = Velocity { linear: v(2, 0, 0), angular: v(0, 0, 0) };
    h.spawn(key(DRIFTER), (at(-30, 3, 0), drifting, floating(), sphere(), claim));
    h.world_mut()
        .spawn(
            key(6),
            OTHER_REGION,
            (at(30, 3, 0), Velocity::ZERO, RigidBody::dynamic(), sphere(), claim),
        )
        .unwrap();
    // The sync's bookkeeping in the hash: every tick's body count and whether the sphere
    // the rollback tick despawns is still a body, so a lost removal shows in the hash.
    // Claimed, so the prediction world runs the kernel over it.
    h.spawn(key(BOOKKEEPING), (Bookkeeping::default(), claim));
    h.script().input_ticks = true;
    let script = h.script();
    script.watched.push(sphere_key(7));
    for tick in 1..=TICKS {
        let index = u64::from(tick) % SPHERES;
        let source = sphere_key(index);
        let bone = Transform::at(lactose_math::Vec3::new(
            Scalar::from_i32(10),
            Scalar::from_i32(3) + Scalar::from_f64(0.05) * Scalar::from_u32(tick % 11),
            Scalar::ZERO,
        ));
        script.at(tick, When::Simulation, key(BONE_A), Action::WriteTransform(bone));
        if tick % 5 == 0 {
            let push = PhysicsCommand::ApplyImpulse { body: source, impulse: v(1, 4, 0) };
            script.at(tick, When::Simulation, source, Action::Command(push));
        }
        if tick % 7 == 0 {
            let spin = PhysicsCommand::ApplyTorqueImpulse { body: source, torque: v(0, 1, 0) };
            script.at(tick + 1, When::Simulation, source, Action::Command(spin));
        }
        if tick % 11 == 0 {
            let sliding = Velocity { linear: v(0, 3, 1), angular: v(0, 0, 0) };
            script.at(tick, When::Simulation, source, Action::WriteVelocity(sliding));
        }
        if tick % 13 == 0 {
            script.at(tick, When::Post, source, Action::Spawn(at(0, 20, 3)));
        }
        if tick % 17 == 0 {
            let victim = sphere_key((index + 1) % SPHERES);
            script.at(tick, When::Post, source, Action::Despawn(victim));
        }
        if tick % 19 == 0 {
            let teleport = PhysicsCommand::Teleport { body: source, transform: at(0, 25, -3) };
            script.at(tick + 1, When::Simulation, source, Action::Command(teleport));
        }
        if tick == 40 {
            script.at(tick, When::Simulation, key(BONE_A), Action::WriteBody(RigidBody::dynamic()));
        }
        if tick == 60 {
            script.at(tick, When::Post, source, Action::Move(sphere_key(39), OTHER_REGION));
        }
        if tick == 70 {
            script.at(tick, When::Post, source, Action::Claim(key(PROXY)));
        }
    }
    // The rollback's snapshot tick holds pending work of every kind, as the conformance
    // scene's does: a post-stage spawn, a despawn and a `cmd.set` of a transform, all left
    // for tick 31's sync when the snapshot is taken; tick 31's simulation stage records a
    // command, which the re-simulated tick records again.
    let source = sphere_key(5);
    let push = PhysicsCommand::ApplyImpulse { body: sphere_key(9), impulse: v(2, 1, 0) };
    let slower = Velocity { linear: v(1, 0, 0), angular: v(0, 0, 0) };
    script
        .at(ROLLBACK_TICK, When::Post, source, Action::Spawn(at(3, 15, 3)))
        .at(ROLLBACK_TICK, When::Post, source, Action::Despawn(sphere_key(7)))
        .at(ROLLBACK_TICK, When::Post, source, Action::SetTransform(sphere_key(8), at(-6, 12, 0)))
        .at(ROLLBACK_TICK + 1, When::Simulation, source, Action::Command(push))
        // Slowed to the velocity it keeps to the end: equal to what the run's last
        // write-back writes, so a restore that kept the live record of the last write-back
        // instead of the carried one would skip this push and the drifter would diverge.
        .at(ROLLBACK_TICK, When::Post, source, Action::SetVelocity(key(DRIFTER), slower));
    h
}

/// One step of the scene: the second region's freeze and thaw through `world_mut`, then the
/// step, recorded.
fn step(
    setup: &SessionSetup,
    mut simulation: Simulation,
    record: &mut dyn FnMut(lactose_core::Tick, u64) -> Result<()>,
) -> Result<Simulation> {
    let next = simulation.tick().0 + 1;
    if next == 80 {
        simulation.world_mut().set_region_mode(OTHER_REGION, RegionMode::Frozen)?;
    } else if next == 95 {
        simulation.world_mut().set_region_mode(OTHER_REGION, RegionMode::Simulated)?;
    }
    run_recorded(&setup.executor, simulation, [TickInput::default()], record)
}

fn record_scene(test: &str, kind: WorldKind) -> Result<()> {
    let setup = SessionSetup::from_env()?;
    let Harness { mut simulation, .. } = scene(kind, setup.layout);
    let mut recorder = setup.recorder(test, simulation.world().layout())?;
    let mut record =
        |tick: lactose_core::Tick, hash: u64| recorder.record(tick.0, hash).map_err(protocol_error);
    let mut saved = None;
    for tick in 1..=TICKS {
        simulation = step(&setup, simulation, &mut record)?;
        if kind == WorldKind::Prediction && tick == ROLLBACK_TICK {
            saved = Some((simulation.world().snapshot(), snapshot(simulation.world())?));
        }
    }
    if let Some((world_snapshot, physics_snapshot)) = saved {
        // A rollback to tick 30 and a re-simulation to the end: every re-simulated tick's
        // hash is recorded again, and the runner requires it equal to the first one's.
        simulation.world_mut().restore(&world_snapshot)?;
        restore(simulation.world_mut(), &physics_snapshot)?;
        corrected(simulation.world_mut(), &[sphere_key(3), key(BONE_A)])?;
        while simulation.tick().0 < TICKS {
            simulation = step(&setup, simulation, &mut record)?;
        }
    }
    setup.finish(test)
}

#[test]
fn a_server_scene() {
    record_scene("lactose_physics_rapier::sour::a_server_scene", WorldKind::Server).unwrap();
}

#[test]
fn a_prediction_scene_with_a_rollback() {
    record_scene(
        "lactose_physics_rapier::sour::a_prediction_scene_with_a_rollback",
        WorldKind::Prediction,
    )
    .unwrap();
}

/// The same scene twice, outside the runner, recording into memory: a rolled-back
/// prediction world ends with the hash of the run that never rolled back.
#[test]
fn the_rolled_back_scene_ends_where_the_straight_one_does() {
    let executor = executor(2);
    let run = |rollback: bool| {
        let Harness { mut simulation, .. } = scene(WorldKind::Prediction, LayoutConfig::default());
        let mut hashes = Vec::new();
        let mut saved = None;
        for tick in 1..=TICKS {
            simulation =
                executor.block_on(simulation.step(&executor, TickInput::default())).unwrap();
            if [80, 95].contains(&(tick + 1)) {
                let mode = if tick + 1 == 80 { RegionMode::Frozen } else { RegionMode::Simulated };
                simulation.world_mut().set_region_mode(OTHER_REGION, mode).unwrap();
            }
            hashes.push(simulation.hash().unwrap());
            if tick == ROLLBACK_TICK {
                // The bookkeeping row is written, so the hashes see it: the sphere the
                // rollback tick despawns is a body until the next sync.
                let book = simulation.world().get::<Bookkeeping>(key(BOOKKEEPING)).unwrap();
                assert_eq!((book.watched, book.bodies > 40), (1, true), "{book:?}");
            }
            if rollback && tick == ROLLBACK_TICK {
                saved =
                    Some((simulation.world().snapshot(), snapshot(simulation.world()).unwrap()));
            }
        }
        if let Some((world_snapshot, physics_snapshot)) = saved {
            simulation.world_mut().restore(&world_snapshot).unwrap();
            restore(simulation.world_mut(), &physics_snapshot).unwrap();
            for tick in ROLLBACK_TICK + 1..=TICKS {
                simulation =
                    executor.block_on(simulation.step(&executor, TickInput::default())).unwrap();
                if [80, 95].contains(&(tick + 1)) {
                    let mode =
                        if tick + 1 == 80 { RegionMode::Frozen } else { RegionMode::Simulated };
                    simulation.world_mut().set_region_mode(OTHER_REGION, mode).unwrap();
                }
                assert_eq!(simulation.hash().unwrap(), hashes[tick as usize - 1], "tick {tick}");
            }
        }
        let book = simulation.world().get::<Bookkeeping>(key(BOOKKEEPING)).unwrap();
        assert_eq!(book.watched, 0, "{book:?}");
        hashes
    };
    assert_eq!(run(true), run(false));
}

/// `physics-api`'s conformance run (architecture section 10) on the session's executor and
/// layout: the first run's world hash after each of its ticks, then, at tick
/// `conformance::TICKS + 1`, the hash of the backend's bytes after its last tick, so
/// architecture mode compares the scene and Rapier's serialised world across machines. The
/// run itself requires the resumed run's hashes equal to the first run's.
#[test]
fn the_conformance_run() {
    let test = "lactose_physics_rapier::sour::the_conformance_run";
    let setup = SessionSetup::from_env().unwrap();
    let config = PhysicsConfig::default();
    let report =
        conformance::snapshot_restore(&setup.executor, backend(), config, setup.layout).unwrap();
    // `snapshot_restore` builds the world and refuses one that reports another layout than
    // the one it was given, so that one is the layout the run applied.
    let mut recorder = setup.recorder(test, &setup.layout).unwrap();
    let ticks: Vec<u32> = report.first.iter().map(|&(tick, _)| tick.0).collect();
    assert_eq!(ticks, (1..=conformance::TICKS).collect::<Vec<_>>());
    for &(tick, hash) in &report.first {
        recorder.record(tick.0, hash).map_err(protocol_error).unwrap();
    }
    let bytes = report.physics_hash.0;
    recorder.record(conformance::TICKS + 1, bytes).map_err(protocol_error).unwrap();
    setup.finish(test).unwrap();
}

const HINGE_BASE: u64 = 2;
const ARM: u64 = 3;
const HINGE: u64 = 4;
/// The hinge's limits in radians, chosen so that the recording tells the vendored Rapier's
/// patch (D123: the limit row's `sin_cos` of half its centre through `libm`) from its
/// absence. Half the centre is 0.3075, a hard rounding case: Apple's `cos` is 0.524 ulp off
/// there and `libm`'s is correctly rounded, so with the patch reverted the recorded hashes
/// differ on macOS from tick 17. An easy case such as (-0.35, 0.8), whose 0.1125 every
/// library under about 0.6 ulp rounds alike, records the same either way. Both limits stay
/// effective: unlimited, the arm swings from the floor at about -0.47 to 2.84.
const HINGE_LIMITS: (f64, f64) = (-0.27, 1.5);
/// The ticks at which the arm is swung, about `+Z` and `-Z` in turn.
const SWINGS: [u32; 3] = [30, 75, 120];

/// A limited revolute joint between two dynamic bodies, driven against both its limits: a
/// heavy base resting on the floor and a light arm hinged at its top edge about `+Z`, which
/// gravity holds against one limit and torque impulses swing against the other and back.
fn hinge_scene(layout: LayoutConfig) -> Harness {
    let mut options = Options::new(WorldKind::Server);
    options.layout = layout;
    let mut h = options.build_with(backend(), CallLog::default());
    let s = Scalar::from_f64;
    h.spawn(key(FLOOR), (Transform::IDENTITY, Collider::new(Shape::cuboid(v(50, 1, 50)))));
    let base = Collider::new(Shape::cuboid(Vec3::new(Scalar::ONE, Scalar::HALF, Scalar::ONE)));
    let on_floor = Transform::at(Vec3::new(Scalar::ZERO, s(1.5), Scalar::ZERO));
    h.spawn(key(HINGE_BASE), (on_floor, Velocity::ZERO, RigidBody::dynamic(), base));
    let arm = Collider::new(Shape::cuboid(Vec3::new(Scalar::ONE, s(0.1), s(0.1))));
    // The hinge stands a quarter clear of the base's edge, so the arm's end never touches
    // the base: a jointed pair keeps its contacts, and the base would stop the arm short.
    let level = Transform::at(Vec3::new(s(2.25), Scalar::TWO, Scalar::ZERO));
    h.spawn(key(ARM), (level, Velocity::ZERO, RigidBody::dynamic(), arm));
    let (lo, hi) = HINGE_LIMITS;
    let hinge = Joint::new(JointKind::Revolute, key(HINGE_BASE), key(ARM))
        .with_anchors(Vec3::new(s(1.25), Scalar::HALF, Scalar::ZERO), -Vec3::X)
        .with_axis(Vec3::Z)
        .with_limits(s(lo), s(hi));
    h.spawn(key(HINGE), (hinge,));
    for (n, tick) in SWINGS.into_iter().enumerate() {
        let about = if n % 2 == 0 { v(0, 0, 1) } else { v(0, 0, -1) };
        let swing = PhysicsCommand::ApplyTorqueImpulse { body: key(ARM), torque: about };
        h.script().at(tick, When::Simulation, key(ARM), Action::Command(swing));
    }
    h
}

/// The arm's angle about the hinge's axis relative to the base, in radians.
fn hinge_angle(world: &World) -> Scalar {
    let rotation = |n| world.get::<Transform>(key(n)).expect("a body's transform").rotation;
    let relative = rotation(HINGE_BASE).conjugate() * rotation(ARM);
    // The same rotation either way; the positive real part keeps the angle in (-π, π].
    let (z, w) = if relative.w < Scalar::ZERO {
        (-relative.z, -relative.w)
    } else {
        (relative.z, relative.w)
    };
    Scalar::TWO * z.atan2(w)
}

/// The hinge scene for [`TICKS`] ticks, recorded; the arm reaches each limit and stops
/// there, so the limit rows enter the solve, and the run counts the ticks it spends within
/// a hundredth of a radian of each.
#[test]
fn a_limited_hinge_driven_against_its_limits() {
    let test = "lactose_physics_rapier::sour::a_limited_hinge_driven_against_its_limits";
    let setup = SessionSetup::from_env().unwrap();
    let Harness { mut simulation, .. } = hinge_scene(setup.layout);
    let mut recorder = setup.recorder(test, simulation.world().layout()).unwrap();
    let mut record =
        |tick: lactose_core::Tick, hash: u64| recorder.record(tick.0, hash).map_err(protocol_error);
    let (lo, hi) = (Scalar::from_f64(HINGE_LIMITS.0), Scalar::from_f64(HINGE_LIMITS.1));
    let near = Scalar::from_f64(0.01);
    let (mut lowest, mut highest) = (Scalar::ZERO, Scalar::ZERO);
    let (mut at_lo, mut at_hi) = (0, 0);
    for _ in 0..TICKS {
        simulation =
            run_recorded(&setup.executor, simulation, [TickInput::default()], &mut record).unwrap();
        let angle = hinge_angle(simulation.world());
        lowest = lowest.min(angle);
        highest = highest.max(angle);
        at_lo += usize::from((angle - lo).abs() < near);
        at_hi += usize::from((angle - hi).abs() < near);
    }
    setup.finish(test).unwrap();
    println!(
        "hinge: from {lowest:?} to {highest:?}; {at_lo} ticks at the lower limit, \
         {at_hi} at the upper"
    );
    // Each limit is reached and holds: the arm overshoots neither by more than a twentieth
    // of a radian.
    let slack = Scalar::from_f64(0.05);
    assert!(lowest < lo + near && lowest > lo - slack, "lowest {lowest:?}");
    assert!(highest > hi - near && highest < hi + slack, "highest {highest:?}");
    assert!(at_lo > 0 && at_hi > 0, "{at_lo} {at_hi}");
}
