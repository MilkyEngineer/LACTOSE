//! The simulation tests that record under SOUR (architecture section 8.3): each builds from
//! the runner's session through the glue the README shows, runs the program for a few
//! hundred ticks with the frames fed back from the lifecycle, and records `(tick, hash)`
//! after every step. Outside the runner the session is production and nothing is recorded.

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
use lactose_core::{Error, ErrorKind, Result};
use lactose_ecs::{Executor, LayoutConfig, RegionMode};
use lactose_sim::harness::{ChunkCapacity, layout_mode, run_recorded};
use lactose_sim::{RegionRequest, Simulation};
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

/// What the program did, so a test shows it exercised what it claims.
#[derive(Default)]
struct Coverage {
    loads: usize,
    releases: usize,
    simulated: usize,
    frozen: usize,
}

/// Runs the program for `ticks` ticks under the session, recording every hash.
fn record_program(test: &str, ticks: u32) -> Result<Coverage> {
    let sour = Sour::from_env()?;
    let mut simulation = program(sour.layout);
    let mut recorder = sour.recorder(test, &simulation)?;
    let mut driver = Driver::default();
    let mut coverage = Coverage::default();
    let mut record =
        |tick: lactose_core::Tick, hash: u64| recorder.record(tick.0, hash).map_err(protocol_error);
    for tick in 1..=ticks {
        let frame = program_frame(tick, &mut driver);
        simulation = run_recorded(&sour.executor, simulation, [frame], &mut record)?;
        driver.observe(&mut simulation);
        let modes = loaded(&simulation);
        coverage.simulated =
            coverage.simulated.max(modes.values().filter(|&&m| m == RegionMode::Simulated).count());
        coverage.frozen =
            coverage.frozen.max(modes.values().filter(|&&m| m == RegionMode::Frozen).count());
    }
    for request in &driver.requests {
        match request {
            RegionRequest::Load { .. } => coverage.loads += 1,
            RegionRequest::Release { .. } => coverage.releases += 1,
            RegionRequest::Cancel { .. } => {}
        }
    }
    sour.finish(test)?;
    Ok(coverage)
}

#[test]
fn anchored_world() {
    let coverage = record_program("lactose_sim::sour::anchored_world", 300).unwrap();
    assert!(coverage.loads > 10, "regions streamed in: {}", coverage.loads);
    assert!(coverage.releases > 0, "regions released");
    assert!(coverage.simulated > 1 && coverage.frozen > 1);
}

#[test]
fn run_recorded_records_after_every_step_and_hands_the_simulation_back() {
    let executor = executor();
    let simulation = program(LayoutConfig::default());
    let mut records = Vec::new();
    let frames = (1..=5).map(script);
    let simulation = run_recorded(&executor, simulation, frames, &mut |tick, hash| {
        records.push((tick.0, hash));
        Ok(())
    })
    .unwrap();
    assert_eq!(records.iter().map(|&(tick, _)| tick).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
    assert_eq!(records.last().unwrap().1, simulation.hash().unwrap());
    // An error from `record` ends the run with it.
    let error = run_recorded(&executor, simulation, (6..=9).map(script), &mut |tick, _| {
        if tick.0 == 7 { Err(Error::new(ErrorKind::Io, "disk full")) } else { Ok(()) }
    })
    .err()
    .unwrap();
    assert_eq!(error.kind(), ErrorKind::Io);
}
