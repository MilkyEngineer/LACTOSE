//! The conformance suite a backend's test crate runs (architecture sections 10 and 20):
//! the snapshot-and-restore run that `lactose-physics-rapier` must pass before this crate
//! is frozen, as a function so that every backend runs the same scene.
//!
//! ```no_run
//! use lactose_ecs::Executor;
//! use lactose_physics_api::conformance::snapshot_restore;
//! use lactose_physics_api::{PhysicsBackend, PhysicsConfig};
//!
//! # fn backend(_threads: u32) -> Box<dyn PhysicsBackend> { unimplemented!() }
//! # fn main() -> lactose_core::Result<()> {
//! let executor = Executor::builder().build()?;
//! // The backend's test builds its pool outside the boundary, here with one thread and
//! // with several, and requires the two reports equal.
//! let one = snapshot_restore(&executor, backend(1), PhysicsConfig::default())?;
//! let many = snapshot_restore(&executor, backend(4), PhysicsConfig::default())?;
//! assert_eq!(one, many);
//! # Ok(()) }
//! ```

use lactose_core::{Result, Tick};
use lactose_ecs::Executor;

use crate::backend::PhysicsBackend;
use crate::service::PhysicsConfig;

/// How many ticks the run simulates.
pub const TICKS: u32 = 10;

/// The tick after which the snapshot is taken.
pub const SNAPSHOT_TICK: u32 = 3;

/// How many dynamic bodies the scene drops.
pub const BODIES: u32 = 64;

/// What [`snapshot_restore`] records. Two reports from backends that differ only in their
/// pool's thread count, or that ran on different architectures, are equal.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ConformanceReport {
    /// `Simulation::hash` after each of the [`TICKS`] ticks of the first run.
    pub first: Vec<(Tick, u64)>,
    /// `Simulation::hash` after each tick from [`SNAPSHOT_TICK`] + 1 to [`TICKS`] of the
    /// run resumed from the snapshot. Equal, tick for tick, to the tail of `first`.
    pub resumed: Vec<(Tick, u64)>,
    /// The physics snapshot's size, what milestone 1 measures against the prediction-world
    /// budget.
    pub snapshot_bytes: usize,
}

/// Runs ten ticks, snapshots at three, restores and re-runs, and requires the hash equal
/// at every tick after the restore (architecture section 10), with `backend` behind this
/// crate's two systems in a `Prediction` world under `Directed` region control.
///
/// The scene: one region created directly, a fixed cuboid floor (a `Collider` row), and
/// [`BODIES`] dynamic spheres of unit radius spawned directly with `Net` keys and a
/// `ClaimedBy` for player one in a stack that topples, each with a kernel-free impulse
/// recorded through the channel on the first tick by a once-per-tick system of this
/// module. Every step takes `TickInput::default()`. After tick [`SNAPSHOT_TICK`] a
/// `World::snapshot` and a [`snapshot`](crate::snapshot) are taken; after tick [`TICKS`]
/// both are restored and the run continues to [`TICKS`] again. Driven with
/// `Executor::block_on`, so it is called from a test's main thread.
///
/// Fails with `InvalidData` naming the first tick whose hash differs, with what building
/// the simulation fails with, and as a step fails.
pub fn snapshot_restore(
    executor: &Executor,
    backend: Box<dyn PhysicsBackend>,
    config: PhysicsConfig,
) -> Result<ConformanceReport> {
    todo!()
}
