//! The glue the README shows between a test and the determinism runner: read the session,
//! build the executor and the layout it names, report back what was built, record
//! `(tick, hash)` and write the execution log a chaos run records. Outside the runner the
//! session is production and nothing is recorded.

#![allow(dead_code, reason = "each test file uses part of the shared module")]

#[path = "../../../../tools/xtask/src/harness/protocol.rs"]
#[expect(
    clippy::wrong_self_convention,
    reason = "the runner's module as written; it is included, not owned, here"
)]
pub mod protocol;

use std::num::NonZeroU32;

use lactose_core::{Error, ErrorKind, Result};
use lactose_ecs::{Executor, LayoutConfig};
use lactose_sim::Simulation;
use lactose_sim::harness::{ChunkCapacity, layout_mode, layout_mode_of};
use lactose_task::ChaosConfig;
use protocol::{Capacity, Layout, Recorder, RunConfig, Session};

/// What a test builds from its session, and reports back.
pub struct Harness {
    session: Session,
    pub executor: Executor,
    pub layout: LayoutConfig,
}

impl Harness {
    /// Reads the session and builds the executor and layout it asks for.
    pub fn from_env() -> Result<Self> {
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
    pub fn recorder(&self, test: &str, simulation: &Simulation) -> Result<Recorder> {
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
    pub fn finish(&self, test: &str) -> Result<()> {
        if let Some(log) = self.executor.execution_log() {
            self.session.write_execution_log(test, log.as_bytes()).map_err(protocol_error)?;
        }
        Ok(())
    }
}

/// A protocol failure as the engine's error.
pub fn protocol_error(error: protocol::ProtocolError) -> Error {
    Error::new(ErrorKind::InvalidData, error.to_string())
}
