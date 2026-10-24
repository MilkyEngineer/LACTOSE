//! The glue of the tests that record under SOUR (architecture section 8.3), as
//! `lactose-sim`'s README shows it: the executor and layout built from the runner's
//! session, the recorder opened with what they report, the execution log of a chaos run
//! written at the end. Outside the runner the session is production and nothing is
//! recorded, so the same tests run under a plain `cargo nextest run`.

#![allow(dead_code, reason = "each test file uses part of the glue")]

#[path = "../../../../tools/xtask/src/harness/protocol.rs"]
#[expect(
    clippy::wrong_self_convention,
    reason = "the runner's module as written; it is included, not owned, here"
)]
mod protocol;

use std::num::NonZeroU32;

use lactose_core::{Error, ErrorKind, Result, Tick};
use lactose_ecs::{Executor, LayoutConfig};
use lactose_sim::harness::{ChunkCapacity, layout_mode};
use lactose_task::ChaosConfig;
use protocol::{Capacity, Layout, RunConfig, Session};

pub use protocol::Recorder;

/// What a test builds from its session.
pub struct Sour {
    session: Session,
    /// The executor the session asks for: its worker count and chaos seed.
    pub executor: Executor,
    /// The layout the session asks for.
    pub layout: LayoutConfig,
}

impl Sour {
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
        let mut executor = Executor::builder().background_workers(1);
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

    /// Opens the recorder of `test` with what the executor and `layout`, the layout a
    /// world of the test reports, were built with.
    pub fn recorder(&self, test: &str, layout: &LayoutConfig) -> Result<Recorder> {
        let config = self.session.config();
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
    pub fn finish(&self, test: &str) -> Result<()> {
        if let Some(log) = self.executor.execution_log() {
            self.session.write_execution_log(test, log.as_bytes()).map_err(protocol_error)?;
        }
        Ok(())
    }
}

/// Records `(tick, hash)` through `recorder`, as `run_recorded` hands them over.
pub fn record(recorder: &mut Recorder, tick: Tick, hash: u64) -> Result<()> {
    recorder.record(tick.0, hash).map_err(protocol_error)
}

fn protocol_error(error: protocol::ProtocolError) -> Error {
    Error::new(ErrorKind::InvalidData, error.to_string())
}
