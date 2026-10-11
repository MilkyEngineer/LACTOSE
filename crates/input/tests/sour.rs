//! The input tests that record under SOUR (architecture section 8.3): each builds its
//! simulation from the runner's session through the glue `lactose-sim`'s README shows (the
//! protocol module by `#[path]`, the executor from `lactose-task`), runs one of the
//! programs of `common` through this crate's applier and writer declaration and a kernel
//! that records every value it derives from `&PlayerInput`, and records `(tick, hash)`
//! after every step. The hash covers the transient column and the derived floats, so the
//! architecture mode compares the decoded angles and directions across targets. The
//! prediction program also rolls back and records the re-simulated ticks again, which the
//! runner requires equal to the first run's. Outside the runner the session is production
//! and nothing is recorded.

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
use lactose_core::{Error, ErrorKind, Result, Tick};
use lactose_ecs::{Executor, LayoutConfig};
use lactose_sim::Simulation;
use lactose_sim::harness::{ChunkCapacity, layout_mode, run_recorded};
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

/// The tick after which the prediction program is snapshotted and later restored.
const ROLLBACK_TICK: u32 = 30;

#[test]
fn the_server_program() -> Result<()> {
    let test = "lactose_input::sour::the_server_program";
    let sour = Sour::from_env()?;
    let simulation = build_server(sour.layout);
    let mut recorder = sour.recorder(test, &simulation)?;
    let mut record = |tick: Tick, hash: u64| recorder.record(tick.0, hash).map_err(protocol_error);
    let frames = (1..=TICKS).map(|tick| server_frame(Program::FULL, tick));
    let simulation = run_recorded(&sour.executor, simulation, frames, &mut record)?;
    assert_eq!(simulation.tick(), Tick(TICKS));
    sour.finish(test)
}

#[test]
fn the_prediction_program_with_a_rollback() -> Result<()> {
    let test = "lactose_input::sour::the_prediction_program_with_a_rollback";
    let sour = Sour::from_env()?;
    let simulation = build_prediction(sour.layout);
    let mut recorder = sour.recorder(test, &simulation)?;
    let mut record = |tick: Tick, hash: u64| recorder.record(tick.0, hash).map_err(protocol_error);
    let frames = (1..=ROLLBACK_TICK).map(prediction_frame);
    let simulation = run_recorded(&sour.executor, simulation, frames, &mut record)?;
    let snapshot = simulation.world().snapshot();
    let frames = (ROLLBACK_TICK + 1..=TICKS).map(prediction_frame);
    let mut simulation = run_recorded(&sour.executor, simulation, frames, &mut record)?;
    // A rollback to the snapshot and a re-simulation to the end: every re-simulated tick's
    // hash is recorded again, and the runner requires it equal to the first one's.
    simulation.world_mut().restore(&snapshot)?;
    let frames = (ROLLBACK_TICK + 1..=TICKS).map(prediction_frame);
    let simulation = run_recorded(&sour.executor, simulation, frames, &mut record)?;
    assert_eq!(simulation.tick(), Tick(TICKS));
    sour.finish(test)
}
