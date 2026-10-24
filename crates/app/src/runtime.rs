//! The executor every binary starts, the solver pool each world gets, and the constants of
//! the timer root (architecture sections 5.1, 11 and 16; D111, D112).

use std::fmt;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use lactose_core::Result;
use lactose_sim::TICK_RATE_HZ;
use lactose_task::{ChaosConfig, ExecutionLog, Executor, Platform};

/// The catch-up cap: at most this many new ticks run in one frame, in every shipped
/// configuration (architecture section 5.1). What every pacer is built with. A rollback's
/// re-simulation does not count against it (D122).
pub const CATCH_UP_CAP: u32 = 4;

/// The structural-log retention every world is built with, in ticks: the builder's default,
/// which is at least [`CATCH_UP_CAP`], as `crates/sim/README.md` requires of the timer
/// root; a consumer that reads the log once per frame then never finds its cursor stale
/// after a catch-up frame.
pub const LOG_RETENTION: u32 = 120;

/// What `crates/sim/README.md` requires of the timer root, pinned at compile time.
const _: () = assert!(LOG_RETENTION >= CATCH_UP_CAP);

/// The tick period every pacer is built with: one second over
/// [`TICK_RATE_HZ`](lactose_sim::TICK_RATE_HZ), truncated to whole nanoseconds.
pub const TICK_PERIOD: Duration = Duration::from_nanos(1_000_000_000 / TICK_RATE_HZ as u64);

/// What the assembly decides for the executor, beyond the builder's defaults: what SOUR's
/// workers and chaos modes vary, the replay of a recorded chaos run, and the platform a
/// binary supplies. Everything left `None` is the builder's default (D111): a frame-critical
/// worker per performance core, capped by the hardware threads the process may use, and
/// the background workers those leave.
#[derive(Clone, Default)]
pub struct Workers {
    /// Frame-critical workers, or the default. SOUR's workers mode sets one and N.
    pub frame_workers: Option<u32>,
    /// Background workers, or the default.
    pub background_workers: Option<u32>,
    /// Chaos scheduling, or free scheduling. A runtime option, never a Cargo feature.
    pub chaos: Option<ChaosConfig>,
    /// A recorded execution log to replay instead of scheduling freely: for step-driven
    /// runs only, since a paced run interleaves steps and passes by wall time, which no
    /// recorded log can follow. `Headless::run` refuses an executor built with one
    /// (`README.md`, "The executor and the solver pool").
    pub replay: Option<ExecutionLog>,
    /// The thread and topology provider for the executor and the IO pool: the standard one
    /// when `None`, which the headless binary uses; the platform crate's instance from the
    /// client binary at milestone 4.
    pub platform: Option<Arc<dyn Platform>>,
}

impl fmt::Debug for Workers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Workers")
            .field("frame_workers", &self.frame_workers)
            .field("background_workers", &self.background_workers)
            .field("chaos", &self.chaos)
            .field("replay", &self.replay.is_some())
            .field("platform", &self.platform.is_some())
            .finish()
    }
}

/// Starts the executor every binary runs on: `Executor::builder()` with each `Some` of
/// `workers` applied, then `build`. The one place an executor is built; the solver pool of
/// each world is built beside its world from this executor's frame worker count
/// (`README.md`, "The executor and the solver pool"). Fails as `ExecutorBuilder::build`
/// fails, which the policy treats as fatal: no binary runs without its executor.
pub fn start_executor(workers: Workers) -> Result<Executor> {
    todo!("apply the overrides to the builder and build")
}

/// The solver pool of one world and the handle its world waits on at shutdown:
/// `frame_worker_count - reserved` threads, at least one, named `solver-<n>`, built with
/// rayon's builder and never with `use_current_thread`, so the pool is the backend's alone
/// (`crates/physics-rapier/README.md`), with an `exit_handler` that raises the handle's
/// count as each thread exits. Fails with `Platform` if rayon cannot start a thread.
pub(crate) fn solver_pool(
    executor: &Executor,
    reserved: u32,
) -> Result<(rayon::ThreadPool, SolverExit)> {
    todo!("size the pool from the frame worker count and build it with the exit handler")
}

/// What a world keeps of its solver pool once the pool itself has gone to the backend: the
/// thread count, and the count of threads that have exited, which every thread's
/// `exit_handler` raises. Hidden plumbing, changeable.
pub(crate) struct SolverExit {
    threads: usize,
    exited: Arc<(Mutex<usize>, Condvar)>,
}

impl SolverExit {
    /// Blocks until every solver thread has exited: what `Server::shutdown` and
    /// `Client::shutdown` call after dropping the world, whose backend's drop terminated the
    /// pool, so that "shut down" means the threads are gone and not only told to go. Never
    /// before the world is dropped (nothing would be exiting) and never from a solver thread
    /// (it would wait for itself).
    pub(crate) fn wait(&self) {
        todo!("wait on the condition variable until exited == threads")
    }
}
