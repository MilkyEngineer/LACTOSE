//! The conformance suite a backend's test crate runs (architecture sections 10 and 20):
//! the snapshot-and-restore run that `lactose-physics-rapier` must pass before this crate
//! is frozen, as a function so that every backend runs the same scene under every layout.
//!
//! ```no_run
//! use lactose_ecs::{Executor, LayoutConfig};
//! use lactose_physics_api::conformance::snapshot_restore;
//! use lactose_physics_api::{PhysicsBackend, PhysicsConfig};
//!
//! # fn backend(_threads: u32) -> Box<dyn PhysicsBackend> { unimplemented!() }
//! # fn main() -> lactose_core::Result<()> {
//! let executor = Executor::builder().build()?;
//! // The backend's test builds its pool outside the boundary, here with one thread and
//! // with several, under the production layout and under SOUR's, and requires every
//! // report equal.
//! let layout = LayoutConfig::default();
//! let one = snapshot_restore(&executor, backend(1), PhysicsConfig::default(), layout)?;
//! let many = snapshot_restore(&executor, backend(4), PhysicsConfig::default(), layout)?;
//! assert_eq!(one, many);
//! # Ok(()) }
//! ```

use lactose_core::{Result, Tick};
use lactose_ecs::{Executor, LayoutConfig};

use crate::backend::PhysicsBackend;
use crate::service::PhysicsConfig;

/// How many ticks the run simulates.
pub const TICKS: u32 = 10;

/// The tick after which the snapshot is taken.
pub const SNAPSHOT_TICK: u32 = 3;

/// How many dynamic bodies the scene drops.
pub const BODIES: u32 = 64;

/// What [`snapshot_restore`] records. Two reports from backends that differ only in their
/// pool's thread count, from two layouts, or from two architectures are equal.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ConformanceReport {
    /// `Simulation::hash` after each of the [`TICKS`] ticks of the first run.
    pub first: Vec<(Tick, u64)>,
    /// `Simulation::hash` after each tick from [`SNAPSHOT_TICK`] + 1 to [`TICKS`] of the
    /// run resumed from the snapshot. Equal, tick for tick, to the tail of `first`.
    pub resumed: Vec<(Tick, u64)>,
    /// The physics snapshot's size at [`SNAPSHOT_TICK`], what milestone 1 measures against
    /// the prediction-world budget.
    pub snapshot_bytes: usize,
    /// [`PhysicsSnapshot::hash`](crate::PhysicsSnapshot::hash) of a snapshot taken after
    /// tick [`TICKS`] of the first run and of the resumed run. Reported, not required
    /// equal, until the verification item on the bytes' order stability decides.
    pub physics_hash: (u64, u64),
}

/// Runs ten ticks, snapshots at three, restores and re-runs, and requires the hash equal
/// at every tick after the restore (architecture section 10), with `backend` behind this
/// crate's three systems in a `Prediction` world under `Directed` region control and the
/// storage `layout` (SOUR's `harness::layout_mode`, or the production default).
///
/// The scene, written so that the restore has pending work of every kind to carry: one
/// region created directly, a fixed cuboid floor (a `Collider` row), [`BODIES`] dynamic
/// spheres of unit radius spawned directly with `Net` keys and a `ClaimedBy` for player
/// one in a stack that topples, with an impulse each recorded through the channel on the
/// first tick; two bones as kinematic bodies and a limited spherical `Joint` row between
/// them, driven by `Transform` writes from a kernel; and a proxy, a `RigidBody` row
/// without a claim. An exclusive `Stage::Input` system after `sim`'s `sets::INPUT` calls
/// `World::set_input_tick(player one, tick)` every tick, and the `Stage::Post` spawn at
/// tick [`SNAPSHOT_TICK`] comes from a kernel on a claimed body, so the spawn is predicted
/// rather than dropped; the same `Stage::Post` system despawns a body and writes a
/// `Transform` through `cmd.set`, so the snapshot point holds pending log entries, pending
/// keys and a pending command. After the restore a kernel writes a `Velocity` through
/// `Mut`, records a command, switches a bone to dynamic through `Mut<RigidBody>`, and the
/// region is frozen for one tick and thawed; every write, command, freeze and thaw
/// described as after the restore happens at the same ticks in the first run. Every step
/// takes `TickInput::default()` (the mode changes go through `world_mut`, tests only).
/// After tick [`SNAPSHOT_TICK`] a `World::snapshot` and a [`snapshot`](crate::snapshot) are
/// taken; after tick [`TICKS`] both are restored and the run continues to [`TICKS`] again.
/// Driven with `Executor::block_on`, so it is called from a test's main thread.
///
/// Fails with `InvalidData` naming the first tick whose hash differs, with what building
/// the simulation fails with, and as a step fails.
pub fn snapshot_restore(
    executor: &Executor,
    backend: Box<dyn PhysicsBackend>,
    config: PhysicsConfig,
    layout: LayoutConfig,
) -> Result<ConformanceReport> {
    todo!()
}
