//! The executor. Foundation tier, outside the determinism boundary.
//!
//! This crate currently holds **signatures only**: the surface `lactose-ecs` and the backends
//! build against, written before the implementation as architecture section 20 requires.
//! `README.md` is the contract the implementation must meet, including the benchmark shapes
//! and pass thresholds of milestone 0.
//!
//! The surface in one paragraph: an [`Executor`] owns frame-critical and background workers
//! created through a [`Platform`]. Per-frame work is a [`Graph`] of [`GraphNode`]s that
//! complete by continuation counters, so no worker ever blocks waiting for a system. Inside a
//! job, [`JobContext::scope`] gives fork-join over borrowed data. Everything that waits (IO,
//! GPU, assets, the tick loop itself) is a `'static` future started with [`Executor::spawn`].
//! Work that must run on one particular thread goes through a [`Lane`].

#![allow(unused_variables, reason = "signatures only until the implementation lands")]

mod chaos;
mod executor;
mod graph;
mod job;
mod lane;
mod platform;
mod time;

pub use chaos::{ChaosConfig, ExecutionLog};
pub use executor::{Executor, ExecutorBuilder, Task, YieldNow, yield_now};
pub use graph::{Graph, GraphBuilder, GraphError, GraphNode, GraphRun, NodeId};
pub use job::{ChildContext, JobContext, Scope};
pub use lane::{Lane, LaneCall, LanePump, lane};
pub use platform::{CoreTopology, Platform, StdPlatform, ThreadHandle, ThreadSpec};
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
