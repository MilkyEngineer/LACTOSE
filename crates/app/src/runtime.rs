//! The executor every binary starts, the solver pool each world gets, and the constants of
//! the timer root (architecture sections 5.1, 11 and 16; D111, D112).

use std::fmt;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use lactose_core::{Error, ErrorKind, Result};
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
    let Workers { frame_workers, background_workers, chaos, replay, platform } = workers;
    let mut builder = Executor::builder();
    if let Some(count) = frame_workers {
        builder = builder.frame_workers(count);
    }
    if let Some(count) = background_workers {
        builder = builder.background_workers(count);
    }
    if let Some(config) = chaos {
        builder = builder.chaos(config);
    }
    if let Some(log) = replay {
        builder = builder.replay(log);
    }
    if let Some(platform) = platform {
        builder = builder.platform(platform);
    }
    builder.build()
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
    let threads = executor.frame_worker_count().saturating_sub(reserved).max(1);
    #[cfg(test)]
    if tests::SLOW_EXIT.load(std::sync::atomic::Ordering::Relaxed) {
        return pool(usize::try_from(threads).unwrap_or(1), tests::slow);
    }
    pool(usize::try_from(threads).unwrap_or(1), || {})
}

/// The pool of [`solver_pool`] with `threads` threads, `before_exit` running on each thread
/// as it exits, before the count is raised: nothing in production, a delay in the test
/// that proves the wait outlasts a slow exit.
fn pool(threads: usize, before_exit: fn()) -> Result<(rayon::ThreadPool, SolverExit)> {
    let exited = Arc::new((Mutex::new(0_usize), Condvar::new()));
    let raised = Arc::clone(&exited);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|index| format!("solver-{index}"))
        .exit_handler(move |_| {
            before_exit();
            let (count, changed) = &*raised;
            *count.lock().unwrap_or_else(PoisonError::into_inner) += 1;
            changed.notify_all();
        })
        .build()
        .map_err(|error| {
            Error::new(ErrorKind::Platform, format!("the solver pool did not start: {error}"))
        })?;
    Ok((pool, SolverExit { threads, exited }))
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
        let (count, changed) = &*self.exited;
        let mut exited = count.lock().unwrap_or_else(PoisonError::into_inner);
        while *exited < self.threads {
            exited = changed.wait(exited).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// The thread count and the shared exit count, for a test that reads the count after the
    /// handle is gone.
    #[cfg(test)]
    pub(crate) fn counts(&self) -> (usize, Arc<(Mutex<usize>, Condvar)>) {
        (self.threads, Arc::clone(&self.exited))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::atomic::AtomicBool;

    use super::*;

    /// Set by a test that builds a world: every solver pool built after it exits slowly.
    pub(crate) static SLOW_EXIT: AtomicBool = AtomicBool::new(false);

    /// A slow exit: the count is raised well after `ThreadPool::drop` returned.
    pub(crate) fn slow() {
        std::thread::sleep(Duration::from_millis(50));
    }

    #[test]
    fn the_wait_outlasts_a_slow_exit() {
        let (pool, exit) = pool(3, slow).unwrap();
        let (threads, exited) = exit.counts();
        assert_eq!(threads, 3);
        drop(pool);
        exit.wait();
        assert_eq!(*exited.0.lock().unwrap(), 3);
    }

    #[test]
    fn a_pool_is_sized_from_the_frame_workers_less_the_reservation() {
        for (workers, reserved, expected) in [(4, 1, 3), (1, 1, 1), (2, 5, 1)] {
            let executor =
                Executor::builder().frame_workers(workers).background_workers(1).build().unwrap();
            let (pool, exit) = solver_pool(&executor, reserved).unwrap();
            assert_eq!(pool.current_num_threads(), expected);
            let names: Vec<String> = pool.broadcast(|_| {
                std::thread::current().name().map(str::to_owned).unwrap_or_default()
            });
            assert!(names.iter().all(|name| name.starts_with("solver-")), "{names:?}");
            drop(pool);
            exit.wait();
            assert_eq!(*exit.counts().1.0.lock().unwrap(), expected);
            executor.shutdown();
        }
    }
}
