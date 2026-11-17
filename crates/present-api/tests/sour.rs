//! Extract writes nothing to the world (`README.md`, "Extract writes nothing to the world";
//! D152): the tests that record under SOUR (architecture section 8.3). Each builds its
//! simulation from the runner's session through the glue `lactose-sim`'s README shows (the
//! protocol module by `#[path]`, the executor from `lactose-task`), runs the scripted scene
//! of `extract.rs` (rows that move, teleport, spawn, despawn, change region, lose and gain a
//! mesh) with a recording sink and a snapshot channel installed, and after every step takes
//! the hash, runs the passes twice (a pass that follows the tick, then one that does not),
//! and takes the hash after each: the three are equal, and so are each region's. The hash
//! recorded for the tick is the one after the passes. The prediction scene rolls back and
//! records the re-simulated ticks again, which the runner requires equal to the first run's.
//! Outside the runner the session is production and nothing is recorded, and a last test
//! shows the streams equal, tick for tick, those of the same scenes stepped with no pass.

#![forbid(unsafe_code)]

use crate::common;
use crate::extract::scene;

#[path = "../../../tools/xtask/src/harness/protocol.rs"]
#[expect(
    clippy::wrong_self_convention,
    reason = "the runner's module as written; it is included, not owned, here"
)]
mod protocol;

use std::num::NonZeroU32;

use common::{OTHER_REGION, REGION, Rig, simulation};
use lactose_core::{Error, ErrorKind, Result, Tick};
use lactose_ecs::{Executor, LayoutConfig, WorldKind};
use lactose_sim::Simulation;
use lactose_sim::harness::{ChunkCapacity, layout_mode, layout_mode_of};
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

const START: u32 = 100;
const FRAMES: u32 = 48;
/// The frame after whose step the prediction scene is snapshotted and later restored.
const ROLLBACK_FRAME: u32 = 20;

/// The world hash and each region's.
fn hashes(rig: &Rig) -> [u64; 3] {
    let region = |id| rig.simulation().world_hash(id).expect("the region hashes");
    [rig.hash(), region(REGION), region(OTHER_REGION)]
}

/// What the passes of a run wrote: rows, flagged rows, removed keys and full passes, so a
/// test can show the scene exercised them.
#[derive(Default, Debug)]
struct Seen {
    written: usize,
    flagged: usize,
    removed: usize,
    full: usize,
}

/// One frame: the scene's edits, a step, then, if `passes`, the two passes with the hashes
/// before and after each equal, the events published, and every second frame an acquire.
/// Returns the tick and the world hash after it all.
fn frame(rig: &mut Rig, frame: u32, passes: bool, seen: &mut Seen) -> (u32, u64) {
    scene(rig, frame);
    rig.step();
    if passes {
        let before = hashes(rig);
        rig.passes();
        let after_one = hashes(rig);
        rig.passes();
        let after_two = hashes(rig);
        assert_eq!(after_one, before, "a pass that follows a tick, at {:?}", rig.tick());
        assert_eq!(after_two, before, "a pass that follows none, at {:?}", rig.tick());
        rig.publish();
        for log in rig.recording.take() {
            seen.written += log.rows.len();
            seen.flagged += log.flagged().len();
            seen.removed += log.removed.map_or(0, |removed| removed.len());
            seen.full += usize::from(log.pass.full);
        }
        if frame.is_multiple_of(2) {
            let _ = rig.reader.acquire();
        }
    }
    (rig.tick().0, rig.hash())
}

/// The scene in `rig`, with or without the passes: every `(tick, hash)` to `record`, the
/// prediction world's re-simulated ticks after its rollback included.
fn run(mut rig: Rig, passes: bool, mut record: impl FnMut(u32, u64) -> Result<()>) -> Result<Seen> {
    let prediction = rig.world().kind() == WorldKind::Prediction;
    let mut seen = Seen::default();
    let mut saved = None;
    for n in 0..FRAMES {
        let (tick, hash) = frame(&mut rig, n, passes, &mut seen);
        record(tick, hash)?;
        if prediction && n == ROLLBACK_FRAME {
            saved = Some(rig.world().snapshot());
        }
    }
    if let Some(snapshot) = saved {
        // A rollback and a re-simulation to the end: every re-simulated tick's hash is
        // recorded again, and the runner requires it equal to the first one's.
        rig.world_mut().restore(&snapshot)?;
        for n in ROLLBACK_FRAME + 1..FRAMES {
            let (tick, hash) = frame(&mut rig, n, passes, &mut seen);
            record(tick, hash)?;
        }
    }
    Ok(seen)
}

fn record_scene(test: &str, kind: WorldKind) -> Result<()> {
    let setup = SessionSetup::from_env()?;
    let rig = Rig::new(simulation(kind, setup.layout, Tick(START)), setup.executor.clone());
    let mut recorder = setup.recorder(test, rig.simulation())?;
    run(rig, true, |tick, hash| recorder.record(tick, hash).map_err(protocol_error))?;
    setup.finish(test)
}

#[test]
fn a_server_scene_with_extract() {
    record_scene("lactose_present_api::sour::a_server_scene_with_extract", WorldKind::Server)
        .unwrap();
}

#[test]
fn a_prediction_scene_with_extract_and_a_rollback() {
    record_scene(
        "lactose_present_api::sour::a_prediction_scene_with_extract_and_a_rollback",
        WorldKind::Prediction,
    )
    .unwrap();
}

/// Outside the runner: each scene's stream with the passes equals, tick for tick, the stream
/// of the same scene stepped with no pass run; and the passes wrote, flagged and removed
/// rows, and ran full passes, so the hashes held over each kind of pass.
#[test]
fn the_streams_equal_those_with_no_pass() {
    for kind in [WorldKind::Server, WorldKind::Prediction] {
        let stream = |passes: bool| {
            let mut stream = Vec::new();
            let simulation = simulation(kind, LayoutConfig::default(), Tick(START));
            let rig = Rig::new(simulation, common::executor(2));
            let seen = run(rig, passes, |tick, hash| {
                stream.push((tick, hash));
                Ok(())
            })
            .unwrap();
            (stream, seen)
        };
        let ((with, seen), (without, _)) = (stream(true), stream(false));
        let rerun = if kind == WorldKind::Prediction { FRAMES - ROLLBACK_FRAME - 1 } else { 0 };
        assert_eq!(with.len(), usize::try_from(FRAMES + rerun).unwrap());
        assert_eq!(with, without, "{kind:?}");
        assert!(seen.written > 50 && seen.flagged > 0 && seen.removed > 0, "{kind:?}: {seen:?}");
        let fulls = if kind == WorldKind::Prediction { 2 } else { 1 };
        assert_eq!(seen.full, fulls, "the first pass, and the one after the restore: {seen:?}");
    }
}
