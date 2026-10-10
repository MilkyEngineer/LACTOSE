//! Compile-only: the README's SOUR glue, against the real protocol module and the real
//! `lactose-task` builder. The protocol module lives in the `xtask` leaf crate, which no
//! library can depend on, so it is included by path until it moves into a foundation
//! crate; its own unit tests come along and run here as well. Adapted from the design
//! review's probe.

#![forbid(unsafe_code)]
#![allow(dead_code, unused_variables, missing_docs)]

#[path = "../../../tools/xtask/src/harness/protocol.rs"]
#[expect(
    clippy::wrong_self_convention,
    reason = "the runner's module as written; it is included, not owned, here"
)]
mod protocol;

use std::num::NonZeroU32;

use lactose_ecs::{Executor, LayoutConfig, WorldKind};
use lactose_sim::harness::{ChunkCapacity, layout_mode, run_recorded};
use lactose_sim::{Registration, SimulationBuilder, TickInput, TypeTable};
use lactose_task::ChaosConfig;
use protocol::{Capacity, Layout, RunConfig, Session};

const NAME: &str = "lactose_sim::sketch";

/// Chaos mode: every perturbation and recording on.
fn chaos_mode(seed: u64) -> ChaosConfig {
    ChaosConfig { seed, shuffle_steals: true, inject_yields: true, force_split: true, record: true }
}

/// The layout the world reports, as the runner spells it.
fn applied_layout(layout: &LayoutConfig) -> Layout {
    if *layout == LayoutConfig::default() {
        return Layout::Production;
    }
    let capacity = match layout.chunk_rows {
        Some(n) => Capacity::Rows(n),
        None => Capacity::Bytes(16 * 1024),
    };
    Layout::Perturbed { capacity, seed: layout.seed }
}

fn register(_: &mut Registration) {}

#[expect(
    clippy::disallowed_methods,
    reason = "SOUR's glue is test code outside the step: it reports what the executor was \
              built with and writes the chaos log"
)]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let session = Session::from_env()?;
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
        executor = executor.chaos(chaos_mode(seed));
    }
    let executor = executor.build()?;
    let simulation = SimulationBuilder::new(WorldKind::Server, TypeTable::new())
        .layout(layout)
        .systems(register)
        .build()?;
    let applied = RunConfig {
        workers: config.workers.map(|_| NonZeroU32::new(executor.frame_worker_count()).unwrap()),
        layout: applied_layout(simulation.world().layout()),
        chaos_seed: executor.chaos_config().map(|chaos| chaos.seed),
    };
    let mut recorder = session.recorder(NAME, &applied)?;
    let frames = (0..10).map(|_| TickInput::default());
    let simulation = run_recorded(&executor, simulation, frames, &mut |tick, hash| {
        recorder.record(tick.0, hash).map_err(|e| {
            lactose_core::Error::new(lactose_core::ErrorKind::InvalidData, e.to_string())
        })
    })?;
    if let Some(log) = executor.execution_log() {
        session.write_execution_log(NAME, log.as_bytes())?;
    }
    let _ = simulation.tick();
    Ok(())
}
