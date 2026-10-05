//! The executor. Foundation tier, outside the determinism boundary.
//!
//! `README.md` states the behaviour the signatures cannot express and the proof the crate
//! carries; `SAFETY.md` states the invariants its `unsafe` relies on.
//!
//! The surface in one paragraph: an [`Executor`] owns frame-critical and background workers
//! created through a [`Platform`]. Per-frame work is a [`Graph`] of [`GraphNode`]s that
//! complete by continuation counters, so no worker ever blocks waiting for a system. Inside a
//! job, [`JobContext::scope`] gives fork-join over borrowed data. Everything that waits (IO,
//! GPU, assets, the tick loop itself) is a `'static` future started with [`Executor::spawn`].
//! Work that must run on one particular thread goes through a [`Lane`].
//!
//! Where things live: `pool` is the worker loop, stealing and idling; `graph` and `job` are
//! the two kinds of work a frame-critical worker runs; `task` is the third, on either class;
//! `latch` and `sleep` are the two wait protocols, written against `sync` so loom can model
//! them (`models`).

mod chaos;
mod deque;
mod executor;
mod graph;
mod guard;
mod job;
mod lane;
mod latch;
#[cfg(all(test, feature = "loom"))]
mod models;
mod platform;
mod pool;
mod sleep;
mod span;
mod sync;
mod task;
mod time;

pub use chaos::{ChaosConfig, ExecutionLog};
pub use executor::{Executor, ExecutorBuilder};
pub use graph::{Graph, GraphBuilder, GraphError, GraphNode, GraphRun, NodeId};
pub use job::{ChildContext, JobContext, Scope};
pub use lane::{Lane, LaneCall, LanePump, lane};
pub use platform::{CoreTopology, Platform, StdPlatform, ThreadHandle, ThreadSpec};
pub use task::{Task, YieldNow, yield_now};
pub use time::{Sleep, TickBudget, TickPacer};

/// Which pool a task runs on.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WorkerClass {
    /// Performance cores, user-interactive priority. Simulation, extract, graph compile and
    /// record. Never runs background work, even when idle.
    FrameCritical,
    /// Efficiency cores, utility priority. Asset decode, compression and long futures.
    /// CPU-bound work here yields every 0.5 ms through [`yield_now`].
    Background,
}

/// Locks `mutex`, ignoring poisoning: every mutex in this crate guards state that is left
/// consistent at each unlock, and a panic on an engine thread aborts anyway.
fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
