//! What a simulation test under SOUR uses (architecture section 8.3). The runner's side of
//! the protocol, `Session`, `RunConfig` and `Recorder`, lives in
//! `tools/xtask/src/harness/protocol.rs` and is written to move into a foundation crate
//! outside the boundary that tests depend on; this module is the deterministic side, in
//! terms of the world and executor types alone, so the glue between the two is a few lines
//! in each test crate's common module (the README shows them).
//!
//! A test, in order: reads its session; builds the executor with the session's worker
//! count and, under a chaos seed, a `ChaosConfig` with every perturbation and recording on;
//! builds its simulation with [`layout_mode`] for a perturbed layout; runs its ticks through
//! [`run_recorded`], recording [`Simulation::hash`] after every step; writes the executor's
//! execution log under a chaos seed. Outside the runner the session is the production
//! configuration and records nothing, so the same test runs under plain `cargo nextest run`.

use lactose_core::{Result, Tick};
use lactose_ecs::{Executor, LayoutConfig};

use crate::input::TickInput;
use crate::simulation::Simulation;

/// SOUR's layout mode (architecture section 8.3) for one chunk capacity and seed: rows
/// permuted at every sync point, chunk iteration shuffled by the seed, batches split at
/// random, identities preserved. `chunk_rows` is the test capacity (1, 7 or 64 in the
/// matrix), or `None` for the production 16 KiB capacity with the perturbations on. A
/// no-op for results: a simulation under it hashes as one under the default.
#[must_use]
pub fn layout_mode(chunk_rows: Option<u32>, seed: u64) -> LayoutConfig {
    todo!()
}

/// Steps `simulation` once per frame of `inputs` on `executor`, calling `record` with the
/// tick and [`Simulation::hash`] after every step, and hands the simulation back. Drives
/// each step with [`Executor::block_on`], so it is called from a test's main thread, never
/// from a worker. An error from a step, a hash or `record` ends the run with it.
pub fn run_recorded(
    executor: &Executor,
    simulation: Simulation,
    inputs: impl IntoIterator<Item = TickInput>,
    record: &mut dyn FnMut(Tick, u64) -> Result<()>,
) -> Result<Simulation> {
    todo!()
}
