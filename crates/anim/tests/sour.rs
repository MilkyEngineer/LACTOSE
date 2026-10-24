//! The animation tests that record under SOUR (architecture section 8.3): each builds its
//! simulation from the runner's session through the glue `lactose-sim`'s README shows (the
//! protocol module by `#[path]`, the executor from `lactose-task`), runs a crowd through
//! this crate's systems and a game sketch that drives the parameters and applies the root
//! motion, and records `(tick, hash)` after every step. The server scene also spawns,
//! limps and recovers ragdolls over the physics test backend; the prediction scene rolls
//! back and records the re-simulated ticks again, which the runner requires equal to the
//! first run's. Outside the runner the session is production and nothing is recorded.

#![forbid(unsafe_code)]

mod common;

#[path = "../../../tools/xtask/src/harness/protocol.rs"]
#[expect(
    clippy::wrong_self_convention,
    reason = "the runner's module as written; it is included, not owned, here"
)]
mod protocol;

use std::num::NonZeroU32;

use common::{
    Fixture, Library, MACHINE, Options, Ragdoll, RagdollAction, SKELETON, chain, clip, clip_ref,
    key, s, state, transition, v, with_events, with_root,
};
use lactose_anim::{AnimParams, AnimRig, Clip, Condition, Phase, RootMotion, StateMachine, sets};
use lactose_asset_api::AssetRef;
use lactose_core::{Error, ErrorKind, Result, StableKey};
use lactose_ecs::{Executor, LayoutConfig, Mut, Res, StableKeyOf, SystemSet, WorldKind, system};
use lactose_math::Scalar;
use lactose_physics_api::Transform;
use lactose_sim::harness::{ChunkCapacity, layout_mode};
use lactose_sim::{CurrentTick, Registration, Simulation, Stage};
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

const TICKS: u32 = 300;
/// The tick after which the prediction scene is snapshotted and later restored.
const ROLLBACK_TICK: u32 = 100;
const CHARACTERS: u64 = 200;
const BONES: u8 = 16;

const WALK: AssetRef<Clip> = clip_ref("sour/walk");
const RUN: AssetRef<Clip> = clip_ref("sour/run");
const JUMP: AssetRef<Clip> = clip_ref("sour/jump");
const JUMP_TRIGGER: u8 = 0;
const RESTART_TRIGGER: u8 = 1;
const SPEED: u8 = 0;

/// Walking and running loops with root motion and events, a one-shot jump on a trigger
/// from any state, a restart at half on another, speed thresholds both ways with a blend,
/// a kept phase from run back to walk.
fn library() -> Library {
    let mut walk = state("walk", WALK, 1.0);
    walk.speed_param = Some(SPEED);
    let mut run = state("run", RUN, 1.0);
    run.speed_param = Some(SPEED);
    let mut to_run = transition(Some(0), 1, &[Condition::Above { param: SPEED, value: s(1.5) }]);
    to_run.blend_ticks = 4;
    let mut to_walk =
        transition(Some(1), 0, &[Condition::Below { param: SPEED, value: Scalar::ONE }]);
    to_walk.blend_ticks = 4;
    to_walk.keep_phase = true;
    let mut jump = transition(None, 2, &[Condition::Trigger(JUMP_TRIGGER)]);
    jump.blend_ticks = 2;
    let mut land = transition(Some(2), 0, &[Condition::Finished]);
    land.blend_ticks = 6;
    let mut restart = transition(Some(0), 0, &[Condition::Trigger(RESTART_TRIGGER)]);
    restart.entry_phase = Phase::HALF;
    let machine = StateMachine {
        states: vec![walk, run, state("jump", JUMP, 1.0)],
        transitions: vec![jump, to_run, to_walk, land, restart],
        initial: 0,
    };
    let marks = [Phase::ZERO, Phase::from_fraction(1, 3), Phase::HALF, Phase(u32::MAX - 7)];
    Library::new()
        .with(SKELETON, chain(usize::from(BONES), |index| index % 5 == 0))
        .with(MACHINE, machine)
        .with(WALK, with_events(with_root(clip(BONES, 6, 40, true, 0.0), 0.2, 0.01), &marks))
        .with(RUN, with_events(with_root(clip(BONES, 4, 24, true, 0.6), 0.4, -0.02), &marks[1..3]))
        .with(JUMP, with_root(clip(BONES, 5, 12, false, 1.1), 0.1, 0.0))
}

/// The salt of a character: its key's bits.
fn salt(key: StableKey) -> u64 {
    match key {
        StableKey::Net(id) => id.to_bits(),
        _ => 0,
    }
}

/// The game's parameter writer: a speed that climbs and falls, the two triggers at ticks
/// of each character's own.
#[system(name = "test::sour_drive", worlds = [server, prediction], in_set = sets::DRIVE)]
fn sour_drive(now: Res<CurrentTick>, key: StableKeyOf, params: Mut<AnimParams>) {
    let tick = u64::from(now.get().0);
    let salt = salt(*key) % 1_000;
    let mut next = *params;
    next.values[usize::from(SPEED)] =
        Scalar::from_u64((tick * 3 + salt) % 25) / Scalar::from_u64(10);
    if (tick + salt).is_multiple_of(41) {
        next.trigger(JUMP_TRIGGER);
    }
    if (tick + salt).is_multiple_of(29) {
        next.trigger(RESTART_TRIGGER);
    }
    params.set_if_neq(next);
}

static MOVEMENT: SystemSet = SystemSet::new("test::sour_movement").after(&[&sets::EVALUATE]);

#[system(name = "test::sour_move", worlds = [server, prediction], in_set = MOVEMENT)]
fn sour_move(motion: &RootMotion, root: Mut<Transform>) {
    let next = motion.apply(&root);
    root.set_if_neq(next);
}

fn register_game(registration: &mut Registration) {
    registration.stage(Stage::Simulation).sets(&[&MOVEMENT]).add(&SOUR_DRIVE).add(&SOUR_MOVE);
}

/// The crowd: every character on its own spot; in a server world every twentieth one
/// grows bones early, goes limp at 120 and recovers at 200.
fn scene(kind: WorldKind, layout: LayoutConfig, executor: Executor) -> Fixture {
    let mut options = Options::new(kind);
    options.layout = layout;
    options.systems = Some(register_game);
    options.physics = kind == WorldKind::Server;
    let mut fixture = options.build_on(executor);
    fixture.assets(library().pin());
    for n in 1..=CHARACTERS {
        let spot = v((n % 20) as f64 * 3.0, 0.0, (n / 20) as f64 * 3.0);
        fixture.spawn_at(key(n), AnimRig::new(MACHINE, SKELETON), Transform::at(spot));
    }
    if kind == WorldKind::Server {
        let script = fixture.world_mut().service_mut::<Ragdoll>().expect("the setup");
        for n in (20..=CHARACTERS).step_by(20) {
            let spawn = 10 + u32::try_from(n % 7).expect("small");
            script.actions.push((spawn, key(n), RagdollAction::Spawn));
            script.actions.push((120, key(n), RagdollAction::Limp(true)));
            script.actions.push((200, key(n), RagdollAction::Limp(false)));
        }
    }
    fixture
}

fn record_scene(test: &str, kind: WorldKind) -> Result<()> {
    let sour = Sour::from_env()?;
    let mut fixture = scene(kind, sour.layout, sour.executor.clone());
    let mut recorder = sour.recorder(test, fixture.simulation())?;
    let mut saved = None;
    for _ in 1..=TICKS {
        fixture.step();
        recorder.record(fixture.tick(), fixture.hash()).map_err(protocol_error)?;
        if kind == WorldKind::Prediction && fixture.tick() == ROLLBACK_TICK {
            saved = Some(fixture.world().snapshot());
        }
    }
    if let Some(snapshot) = saved {
        // A rollback to tick 100 and a re-simulation to the end: every re-simulated tick's
        // hash is recorded again, and the runner requires it equal to the first one's.
        fixture.world_mut().restore(&snapshot)?;
        while fixture.tick() < TICKS {
            fixture.step();
            recorder.record(fixture.tick(), fixture.hash()).map_err(protocol_error)?;
        }
    }
    sour.finish(test)
}

#[test]
fn a_server_crowd_with_ragdolls() {
    record_scene("lactose_anim::sour::a_server_crowd_with_ragdolls", WorldKind::Server).unwrap();
}

#[test]
fn a_prediction_crowd_with_a_rollback() {
    record_scene("lactose_anim::sour::a_prediction_crowd_with_a_rollback", WorldKind::Prediction)
        .unwrap();
}

/// The prediction scene twice, outside the runner: a rolled-back world ends with the hash
/// of the run that never rolled back, and the scene does what it is for (transitions of
/// every kind, events, ragdolls in the server scene).
#[test]
fn the_rolled_back_crowd_ends_where_the_straight_one_does() {
    let executor = common::executor(2);
    let run = |rollback: bool| {
        let mut fixture = scene(WorldKind::Prediction, LayoutConfig::default(), executor.clone());
        let mut hashes = Vec::new();
        let mut saved = None;
        for _ in 1..=TICKS {
            fixture.step();
            hashes.push(fixture.hash());
            if rollback && fixture.tick() == ROLLBACK_TICK {
                saved = Some(fixture.world().snapshot());
            }
        }
        if let Some(snapshot) = saved {
            fixture.world_mut().restore(&snapshot).unwrap();
            while fixture.tick() < TICKS {
                fixture.step();
                hashes.push(fixture.hash());
            }
        }
        (hashes, fixture.log().events.len())
    };
    let (straight, events) = run(false);
    let (rolled, _) = run(true);
    assert_eq!(rolled.len(), straight.len() + (TICKS - ROLLBACK_TICK) as usize);
    assert_eq!(rolled[..straight.len()], straight[..]);
    assert_eq!(rolled[straight.len()..], straight[ROLLBACK_TICK as usize..]);
    assert!(events > 1_000, "{events} events");
}

#[test]
fn the_server_crowd_moves_its_ragdolls() {
    let mut fixture = scene(WorldKind::Server, LayoutConfig::default(), common::executor(2));
    let mut states = [0u32; 3];
    let mut moved = Vec::new();
    for _ in 1..=TICKS {
        fixture.step();
        for n in 1..=CHARACTERS {
            if let Some(state) = fixture.state(key(n)).state() {
                states[usize::from(state)] += 1;
            }
        }
        if [120, 201, TICKS].contains(&fixture.tick()) {
            moved.push(fixture.log().moved.len());
        }
    }
    assert!(states.iter().all(|&count| count > 1_000), "{states:?}");
    assert_eq!(moved[0], 0, "animated bones report nothing");
    assert!(moved[1] > 0, "the limp ragdolls report their bones");
    assert_eq!(moved[2], moved[1], "the recovered ones report nothing again");
}
