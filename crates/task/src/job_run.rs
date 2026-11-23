//! One job for a task that is in none: [`crate::Executor::run_job`], the slot its value comes
//! back through and the future that reads it.
//!
//! The slot is modelled under loom (`models.rs`), so it goes through `sync`. A call allocates
//! one cell, which holds the closure, then the value, and is shared by the queue entry and the
//! future; the queue entry's destructor is what cancels a job that never starts, so every
//! path that drops queued work (shutdown's sweep, the replay pool's, a submission that finds
//! the executor shut down) resolves the future.

use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use std::sync::Arc;

use lactose_core::{AllocTag, Error, ErrorKind, Result};

use crate::chaos::{JobKind, LogRecord};
use crate::guard::abort_on_unwind;
use crate::job::JobContext;
use crate::pool::{Arenas, Shared, WorkerLocal};
use crate::span::TARGET;
use crate::sync::{Mutex, MutexGuard};

enum Stage<F, R> {
    /// In a queue, not started.
    Queued(F),
    /// Taken out to be run, or to be dropped unrun.
    Taken,
    /// The closure returned this, and the future has not taken it.
    Done(R),
    /// The closure was dropped unrun.
    Cancelled,
}

struct State<F, R> {
    stage: Stage<F, R>,
    waker: Option<Waker>,
    /// The future was dropped: nobody will take the value.
    abandoned: bool,
}

/// Where a job's closure waits to be taken and its value waits to be read.
///
/// Whoever holds the job's queue entry is the only caller of [`JobSlot::take`]; the future is
/// the only caller of [`JobSlot::poll`] and [`JobSlot::abandon`]. Every closure, value and
/// waker is dropped or woken outside the lock.
pub(crate) struct JobSlot<F, R> {
    state: Mutex<State<F, R>>,
}

impl<F, R> JobSlot<F, R> {
    pub(crate) fn new(job: F) -> Self {
        Self {
            state: Mutex::new(State { stage: Stage::Queued(job), waker: None, abandoned: false }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State<F, R>> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Takes the closure to run it, or with `shutting_down` drops it through `discard` and
    /// resolves the future to `Cancelled`. `None` means it was dropped.
    pub(crate) fn take(&self, shutting_down: bool, discard: impl FnOnce(F)) -> Option<F> {
        if shutting_down {
            self.cancel(discard);
            return None;
        }
        match core::mem::replace(&mut self.lock().stage, Stage::Taken) {
            Stage::Queued(job) => Some(job),
            _ => unreachable!("a job is taken once, by whoever holds its entry"),
        }
    }

    /// Drops the closure through `discard`, if it has not been taken, and then resolves the
    /// future to `Cancelled`: once the future says so, the closure is gone.
    pub(crate) fn cancel(&self, discard: impl FnOnce(F)) {
        let job = {
            let mut state = self.lock();
            match core::mem::replace(&mut state.stage, Stage::Taken) {
                Stage::Queued(job) => job,
                // Taken by a worker, which hands its value back.
                other => {
                    state.stage = other;
                    return;
                }
            }
        };
        discard(job);
        let waker = {
            let mut state = self.lock();
            state.stage = Stage::Cancelled;
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    /// Hands the closure's `value` to the future. Returns it if the future was dropped, for
    /// the caller to drop.
    #[must_use]
    pub(crate) fn finish(&self, value: R) -> Option<R> {
        let waker = {
            let mut state = self.lock();
            if state.abandoned {
                return Some(value);
            }
            state.stage = Stage::Done(value);
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        None
    }

    /// What the future reads: the value, `Cancelled`, or nothing yet, in which case `cx`'s
    /// waker is kept for the job's end.
    pub(crate) fn poll(&self, cx: &mut Context<'_>) -> Poll<Result<R>> {
        let mut state = self.lock();
        match core::mem::replace(&mut state.stage, Stage::Taken) {
            Stage::Done(value) => Poll::Ready(Ok(value)),
            Stage::Cancelled => Poll::Ready(Err(Error::new(
                ErrorKind::Cancelled,
                "the executor shut down before the job started",
            ))),
            pending => {
                state.stage = pending;
                if state.waker.as_ref().is_some_and(|waker| waker.will_wake(cx.waker())) {
                    return Poll::Pending;
                }
                // The waker it replaces is dropped outside the lock, as every waker here is
                // woken: a foreign waker's drop may run anything, this slot's own calls too.
                let old = state.waker.replace(cx.waker().clone());
                drop(state);
                drop(old);
                Poll::Pending
            }
        }
    }

    /// The future is being dropped. Returns the value if the job has finished, for the
    /// caller to drop; otherwise the worker drops it when the closure returns.
    #[must_use]
    pub(crate) fn abandon(&self) -> Option<R> {
        let mut state = self.lock();
        state.abandoned = true;
        let waker = state.waker.take();
        let value = match core::mem::replace(&mut state.stage, Stage::Taken) {
            Stage::Done(value) => Some(value),
            other => {
                state.stage = other;
                None
            }
        };
        drop(state);
        drop(waker);
        value
    }
}

/// A job as the queues see it.
pub(crate) trait RawJob: Send + Sync {
    /// Which run of its executor this is: the ordinal it takes beside the graph runs, which
    /// identifies it in an execution log.
    fn ordinal(&self) -> u64;

    /// Runs the job on this worker, or drops it unrun if shutdown has begun. Called once,
    /// by the holder of its entry.
    fn run(&self, local: &WorkerLocal, arenas: &mut Arenas, position: u64);

    /// Drops the job unrun on this thread if it has not started.
    fn cancel(&self);
}

/// A job as its future sees it.
trait Outcome<R>: Send + Sync {
    fn poll(&self, cx: &mut Context<'_>) -> Poll<Result<R>>;
    fn abandon(&self);
}

struct JobCell<F, R> {
    ordinal: u64,
    slot: JobSlot<F, R>,
}

/// Drops a closure that will never run, under the abort guard a job runs under.
fn discard<F>(job: F) {
    abort_on_unwind(|| drop(job));
}

impl<F, R> RawJob for JobCell<F, R>
where
    F: FnOnce(&mut JobContext<'_>) -> R + Send + 'static,
    R: Send + 'static,
{
    fn ordinal(&self) -> u64 {
        self.ordinal
    }

    fn run(&self, local: &WorkerLocal, arenas: &mut Arenas, position: u64) {
        let shared = &*local.shared;
        // A job that started finishes, shutdown or not; one that has not, never starts once
        // shutdown has begun.
        let Some(job) = self.slot.take(shared.is_shut_down(), discard) else {
            return;
        };
        let position = match &shared.chaos {
            Some(chaos) if chaos.recording() && !chaos.replaying => chaos.record(LogRecord {
                worker: local.index as u32,
                kind: JobKind::Job,
                key: self.ordinal,
                index: 0,
                units: 0..1,
            }),
            // In replay the position came with the entry, and the take recorded it.
            _ => position,
        };
        let _span = tracing::info_span!(target: TARGET, "run_job").entered();
        // From here until the closure returns, the children of its scopes wait on the pool's
        // queue for such work (`JobContext::scope`).
        local.wide_job.set(true);
        let value = job(&mut JobContext::new(local, arenas, position));
        local.wide_job.set(false);
        arenas.job.reset();
        drop(self.slot.finish(value));
    }

    fn cancel(&self) {
        self.slot.cancel(discard);
    }
}

impl<F: Send, R: Send> Outcome<R> for JobCell<F, R> {
    fn poll(&self, cx: &mut Context<'_>) -> Poll<Result<R>> {
        self.slot.poll(cx)
    }

    fn abandon(&self) {
        drop(self.slot.abandon());
    }
}

/// The queue entry of a job. Dropping it unrun cancels the job.
pub(crate) struct JobEntry {
    job: Option<Arc<dyn RawJob>>,
}

impl JobEntry {
    pub(crate) fn ordinal(&self) -> u64 {
        self.job.as_ref().expect("an entry holds its job until it runs").ordinal()
    }

    /// Runs the job. See [`RawJob::run`].
    pub(crate) fn run(mut self, local: &WorkerLocal, arenas: &mut Arenas, position: u64) {
        let job = self.job.take().expect("an entry holds its job until it runs");
        job.run(local, arenas, position);
    }
}

impl Drop for JobEntry {
    fn drop(&mut self) {
        if let Some(job) = self.job.take() {
            job.cancel();
        }
    }
}

/// Queues `job` on `shared` and returns its future.
pub(crate) fn start<F, R>(shared: &Shared, job: F) -> JobRun<R>
where
    F: FnOnce(&mut JobContext<'_>) -> R + Send + 'static,
    R: Send + 'static,
{
    let ordinal = shared.next_run_ordinal();
    let cell = {
        let _tag = AllocTag::Task.scope();
        let cell = Arc::new(JobCell { ordinal, slot: JobSlot::new(job) });
        // Where std's mutex is a pthread mutex, as on Apple targets, its first lock
        // allocates it: lock it here, so that that allocation is the task tag's too.
        drop(cell.slot.lock());
        cell
    };
    let run = JobRun { job: Some(cell.clone()) };
    shared.submit_job(JobEntry { job: Some(cell) });
    run
}

/// A job in flight. See [`crate::Executor::run_job`].
///
/// Resolves to the closure's value once the closure has returned, which is after every scope
/// it opened has closed: no child of it is executing or ever will, and what they and the
/// closure wrote happens before the resolution. A job that started finishes and hands its
/// value back, shutdown or not. Resolves to an error of kind `Cancelled` when shutdown began
/// before the job started; the closure has then been dropped unrun.
///
/// Dropping it does not cancel the job. The value is then dropped by the worker that ran it
/// or by the thread that dropped this, whichever comes last. Polling it again after it
/// resolved panics.
#[must_use = "futures do nothing unless awaited"]
pub struct JobRun<R> {
    job: Option<Arc<dyn Outcome<R>>>,
}

impl<R: Send + 'static> Future for JobRun<R> {
    type Output = Result<R>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<R>> {
        let job = self.job.as_ref().expect("a job run is not polled after it resolved");
        let poll = job.poll(cx);
        if poll.is_ready() {
            self.job = None;
        }
        poll
    }
}

impl<R> Drop for JobRun<R> {
    fn drop(&mut self) {
        if let Some(job) = self.job.take() {
            job.abandon();
        }
    }
}
