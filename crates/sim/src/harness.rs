//! What a simulation test under SOUR uses (architecture section 8.3). The runner's side of
//! the protocol, `Session`, `RunConfig` and `Recorder`, lives in
//! `tools/xtask/src/harness/protocol.rs` and is written to move into a foundation crate
//! outside the boundary that tests depend on; this module is the deterministic side, in
//! terms of the world and executor types alone, so the glue between the two is a few lines
//! in each test crate's common module (the README shows them, and `tests/sour_glue.rs`
//! compiles them) until one test-support crate outside the boundary, `lactose-sour`, holds
//! the glue once the protocol module moves.
//!
//! A test, in order: reads its session; builds the executor with the session's worker
//! count and, under a chaos seed, a `ChaosConfig` with every perturbation and recording on;
//! builds its simulation with [`layout_mode`] for the session's layout; runs its ticks
//! through [`run_recorded`], recording [`Simulation::hash`] after every step; writes the
//! executor's execution log under a chaos seed. Outside the runner the session is the
//! production configuration and records nothing, so the same test runs under plain
//! `cargo nextest run`.

use lactose_core::{Result, Tick};
use lactose_ecs::{Executor, LayoutConfig};

use crate::input::TickInput;
use crate::simulation::Simulation;

/// A chunk capacity as the runner names it (`protocol::Capacity`, plus production).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ChunkCapacity {
    /// A fixed number of rows per chunk, in a slab sized to match: 1, 7 and 64 in the
    /// matrix.
    Rows(u32),
    /// A slab of this many bytes; the row count follows from the row size. The ECS has one
    /// slab size, 16 KiB, so that is the only value accepted.
    Bytes(u32),
    /// The production layout: the builder's defaults, nothing permuted.
    Production,
}

/// SOUR's layout mode (architecture section 8.3) for one capacity and seed: rows permuted
/// at every sync point and batches split at random from the seed, identities preserved,
/// and the chunk iteration order chosen by the capacity so that the matrix covers both
/// orders whatever seeds the runner hands out: `ChunkOrder::Reversed` for `Rows(7)` and
/// `Bytes`, `ChunkOrder::Shuffled` by the seed for every other `Rows(n)`. `Production` is
/// `LayoutConfig::default()` whatever the seed. A no-op for results: a simulation under any
/// of these hashes as one under the default.
///
/// Fails with `Unsupported` for a byte capacity other than 16 KiB, which the ECS cannot
/// honour, so a test under a capacity the runner sent and the storage cannot apply fails
/// rather than running something else.
pub fn layout_mode(capacity: ChunkCapacity, seed: u64) -> Result<LayoutConfig> {
    todo!()
}

/// Steps `simulation` once per frame of `inputs` on `executor`, calling `record` with the
/// tick and [`Simulation::hash`] after every step, and hands the simulation back. Runs no
/// pass; a `Server` world's presentation events are drained by the step itself. Drives
/// each step with [`Executor::block_on`], so it is called from a test's main thread, never
/// from a worker. An error from a step, a hash or `record` ends the run with it; the
/// simulation is then gone.
pub fn run_recorded(
    executor: &Executor,
    simulation: Simulation,
    inputs: impl IntoIterator<Item = TickInput>,
    record: &mut dyn FnMut(Tick, u64) -> Result<()>,
) -> Result<Simulation> {
    todo!()
}
