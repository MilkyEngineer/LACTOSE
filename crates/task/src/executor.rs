//! The executor handle, its builder and stackless tasks.

use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::sync::Arc;
use std::time::Instant;

use lactose_core::Result;

use crate::WorkerClass;
use crate::chaos::{ChaosConfig, ExecutionLog};
use crate::graph::{Graph, GraphRun};
use crate::platform::Platform;
use crate::time::Sleep;

/// A handle to the worker pools. Cheap to clone; every clone refers to the same pools.
#[derive(Clone)]
pub struct Executor {
    _private: (),
}

impl Executor {
    /// Starts configuring an executor.
    #[must_use]
    pub fn builder() -> ExecutorBuilder {
        ExecutorBuilder { _private: () }
    }

    /// Starts `future` on a worker of `class` and returns a handle to its result.
    ///
    /// The task inherits the caller's `lactose_core::AllocTag`, and the executor re-enters
    /// that tag on whichever worker polls it. A task is cancelled by dropping its [`Task`].
    pub fn spawn<F>(&self, class: WorkerClass, future: F) -> Task<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        todo!()
    }

    /// Runs every node of `graph` on the frame-critical workers.
    ///
    /// The returned future resolves when every node has finalized. A node is released when
    /// all nodes it depends on have finalized; among nodes released together, lower
    /// [`crate::NodeId`]s are pushed first, though execution order between independent nodes
    /// is unspecified. Dropping the future does not cancel the run: nodes own their data, so
    /// the run finishes on its own. One graph has at most one run in flight.
    pub fn run_graph(&self, graph: Arc<Graph>) -> GraphRun {
        todo!()
    }

    /// A future that resolves at `deadline`. The tick loop paces itself with this.
    pub fn sleep_until(&self, deadline: Instant) -> Sleep {
        todo!()
    }

    /// Drives `future` to completion on the calling thread while the workers run whatever it
    /// spawns. For `main`, tests and tools.
    ///
    /// This is the crate's only blocking wait. Calling it on a worker thread is a debug
    /// assertion, because a worker that blocks stalls the frame.
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        todo!()
    }

    /// The execution log recorded so far, if [`ChaosConfig::record`] was set.
    #[must_use]
    pub fn take_execution_log(&self) -> Option<ExecutionLog> {
        todo!()
    }

    /// Stops accepting work, lets running jobs finish, cancels unfinished tasks and joins
    /// every worker. Returns once all threads have exited.
    pub fn shutdown(self) {
        todo!()
    }
}

/// Configures and starts an [`Executor`].
pub struct ExecutorBuilder {
    _private: (),
}

impl ExecutorBuilder {
    /// Overrides the number of frame-critical workers. The default is the platform's
    /// performance-core count.
    #[must_use]
    pub fn frame_workers(self, count: u32) -> Self {
        todo!()
    }

    /// Overrides the number of background workers. The default is the platform's
    /// efficiency-core count, or two on homogeneous CPUs.
    #[must_use]
    pub fn background_workers(self, count: u32) -> Self {
        todo!()
    }

    /// Replaces the thread and topology provider. The default is [`crate::StdPlatform`].
    #[must_use]
    pub fn platform(self, platform: Arc<dyn Platform>) -> Self {
        todo!()
    }

    /// Enables chaos scheduling for the determinism harness. A runtime option, never a Cargo
    /// feature, and a no-op when not set.
    #[must_use]
    pub fn chaos(self, config: ChaosConfig) -> Self {
        todo!()
    }

    /// Replays a recorded execution log instead of scheduling freely, so a chaos failure
    /// reproduces exactly.
    #[must_use]
    pub fn replay(self, log: ExecutionLog) -> Self {
        todo!()
    }

    /// Starts the worker threads.
    pub fn build(self) -> Result<Executor> {
        todo!()
    }
}

/// A spawned task's result. Dropping it cancels the task at its next suspension point.
#[must_use = "dropping a Task cancels it; call detach() to let it run unobserved"]
pub struct Task<T> {
    marker: PhantomData<fn() -> T>,
}

impl<T> Task<T> {
    /// Lets the task run to completion with nobody waiting for its result.
    pub fn detach(self) {
        todo!()
    }
}

impl<T> Future for Task<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        todo!()
    }
}

/// Gives other tasks of the same class a turn. See [`yield_now`].
#[must_use = "futures do nothing unless awaited"]
pub struct YieldNow {
    _private: (),
}

impl Future for YieldNow {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        todo!()
    }
}

/// Reschedules the current task behind other ready tasks of its class. Background CPU work
/// awaits this at least every 0.5 ms.
pub fn yield_now() -> YieldNow {
    YieldNow { _private: () }
}
