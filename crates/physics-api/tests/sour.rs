//! The physics tests that record under SOUR (architecture section 8.3): each builds its
//! simulation from the runner's session through the glue `lactose-sim`'s README shows (the
//! protocol module by `#[path]`, the executor from `lactose-task`), runs a scene through
//! this crate's three systems over the test backend, and records `(tick, hash)` after every
//! step. The prediction scene also rolls back and records the re-simulated ticks again,
//! which the runner requires equal to the first run's. Outside the runner the session is
//! production and nothing is recorded.

#![forbid(unsafe_code)]

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
    ClaimedBy, Executor, LayoutConfig, OwnedBy, RegionConfig, RegionMode, WorldKind,
};
use lactose_math::Scalar;
use lactose_physics_api::{
    Collider, Joint, JointKind, PhysicsCommand, RigidBody, Shape, Transform, Velocity, corrected,
    restore, snapshot,
};
use lactose_sim::harness::{ChunkCapacity, layout_mode, run_recorded};
use lactose_sim::{Simulation, TickInput};
use lactose_task::ChaosConfig;
use protocol::{Capacity, Layout, Recorder, RunConfig, Session};

/// What a test builds from its session, and reports back.
struct Sour {
    session: Session,
    executor: Executor,
    layout: LayoutConfig,
}

impl Sour {
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

    /// Opens the recorder with what the executor and the world report they were built
    /// with.
    #[expect(
        clippy::disallowed_methods,
        reason = "SOUR's glue is test code outside the step: it reports what the executor \
                  was built with"
    )]
    fn recorder(&self, test: &str, simulation: &Simulation) -> Result<Recorder> {
        let config = self.session.config();
        let layout = simulation.world().layout();
        let applied = RunConfig {
            workers: config.workers.map(|_| {
                NonZeroU32::new(self.executor.frame_worker_count()).expect("at least one worker")
            }),
            layout: if *layout == LayoutConfig::default() {
                Layout::Production
            } else {
                let capacity = match layout.chunk_rows {
                    Some(n) => Capacity::Rows(n),
                    None => Capacity::Bytes(16 * 1024),
                };
                Layout::Perturbed { capacity, seed: layout.seed }
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

/// The scene both tests run: a floor reporting contacts, a falling stack, two bones joined
/// and driven, a second region frozen and thawed, and game code that pushes, teleports,
/// writes velocities, spawns, despawns and moves through the run.
fn scene(kind: WorldKind, layout: LayoutConfig) -> Harness {
    let mut options = Options::new(kind);
    options.layout = layout;
    let mut h = options.build();
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
    // the rollback tick despawns is still a body, so a lost removal shows as a ghost body
    // although the test backend, with no collision response, never moves a row for it.
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
    sour: &Sour,
    mut simulation: Simulation,
    record: &mut dyn FnMut(lactose_core::Tick, u64) -> Result<()>,
) -> Result<Simulation> {
    let next = simulation.tick().0 + 1;
    if next == 80 {
        simulation.world_mut().set_region_mode(OTHER_REGION, RegionMode::Frozen)?;
    } else if next == 95 {
        simulation.world_mut().set_region_mode(OTHER_REGION, RegionMode::Simulated)?;
    }
    run_recorded(&sour.executor, simulation, [TickInput::default()], record)
}

fn record_scene(test: &str, kind: WorldKind) -> Result<()> {
    let sour = Sour::from_env()?;
    let Harness { mut simulation, .. } = scene(kind, sour.layout);
    let mut recorder = sour.recorder(test, &simulation)?;
    let mut record =
        |tick: lactose_core::Tick, hash: u64| recorder.record(tick.0, hash).map_err(protocol_error);
    let mut saved = None;
    for tick in 1..=TICKS {
        simulation = step(&sour, simulation, &mut record)?;
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
            simulation = step(&sour, simulation, &mut record)?;
        }
    }
    sour.finish(test)
}

#[test]
fn a_server_scene() {
    record_scene("lactose_physics_api::sour::a_server_scene", WorldKind::Server).unwrap();
}

#[test]
fn a_prediction_scene_with_a_rollback() {
    record_scene(
        "lactose_physics_api::sour::a_prediction_scene_with_a_rollback",
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
