//! Stackless tasks: a `'static` future, the state machine that decides who may poll it, and
//! the handle that returns its output.

use core::cell::UnsafeCell;
use core::pin::Pin;
use core::sync::atomic::{AtomicU8, Ordering};
use core::task::{Context, Poll, Waker};
use std::sync::{Arc, Mutex, Weak};
use std::task::Wake;

use lactose_core::{AllocTag, Error, ErrorKind, Result};

use crate::guard::abort_on_unwind;

use crate::pool::Shared;
use crate::{WorkerClass, lock};

/// Waiting for a wake-up. Nobody holds the future.
const IDLE: u8 = 0;
/// In a queue. The queue entry holds the right to poll.
const SCHEDULED: u8 = 1;
/// Being polled. The poller holds the future.
const RUNNING: u8 = 2;
/// Being polled, and woken meanwhile: the poller reschedules instead of going idle.
const NOTIFIED: u8 = 3;
/// The future is gone: it completed or was cancelled.
const DONE: u8 = 4;
const PHASE: u8 = 0x0F;
/// Set alongside `SCHEDULED`, `RUNNING` or `NOTIFIED`: whoever holds the future drops it
/// instead of polling it again.
const CANCEL: u8 = 0x10;

/// A task as the queues and the shutdown path see it.
pub(crate) trait RawTask: Send + Sync {
    /// Polls the task, or with `cancel` drops its future. The caller holds the queue entry
    /// of a task in the `SCHEDULED` phase.
    fn run(self: Arc<Self>, cancel: bool);

    /// Drops the future at the task's next suspension point, or now if it is suspended.
    fn cancel(&self);
}

/// A task as its [`Task`] handle sees it.
trait Joinable<T>: Send + Sync {
    fn poll_output(&self, cx: &mut Context<'_>) -> Poll<Result<T>>;
    fn cancel(&self);
}

struct Join<T> {
    output: Option<T>,
    /// The future was dropped without completing; no output will come.
    cancelled: bool,
    waker: Option<Waker>,
}

struct TaskCell<F: Future> {
    state: AtomicU8,
    class: WorkerClass,
    /// The spawner's allocation tag, re-entered around every poll and around the drop.
    tag: AllocTag,
    id: u64,
    shared: Weak<Shared>,
    /// Accessed only by the thread that holds the `RUNNING` or `NOTIFIED` phase.
    future: UnsafeCell<Option<F>>,
    join: Mutex<Join<F::Output>>,
}

// SAFETY: `future` is the only field that is not `Sync` by itself. The phase in `state`
// admits one thread at a time to it (see `run` and `cancel`), and that thread may differ
// from poll to poll, which `F: Send` allows. The output crosses threads through a mutex,
// which `F::Output: Send` allows.
unsafe impl<F: Future + Send> Sync for TaskCell<F> where F::Output: Send {}

impl<F> TaskCell<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    fn schedule(self: &Arc<Self>) {
        match self.shared.upgrade() {
            Some(shared) => shared.submit_task(self.class, self.clone()),
            // The executor is gone and nothing will ever poll this task.
            None => self.clone().run(true),
        }
    }

    /// Drops the future and publishes `output`. The caller holds the `RUNNING` or
    /// `NOTIFIED` phase.
    fn finish(&self, output: Option<F::Output>) {
        // A panic in the future's destructor is a panic in the task, whichever thread the
        // cancellation happens to run on.
        abort_on_unwind(|| {
            let _tag = self.tag.scope();
            // SAFETY: the caller holds the phase that grants exclusive access to `future`.
            unsafe { *self.future.get() = None };
        });
        self.state.store(DONE, Ordering::Release);
        if let Some(shared) = self.shared.upgrade() {
            shared.forget_task(self.id);
        }
        let waker = {
            let mut join = lock(&self.join);
            join.cancelled = output.is_none();
            join.output = output;
            join.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl<F: Future> Drop for TaskCell<F> {
    fn drop(&mut self) {
        // Reached with the future still in place when a suspended task loses its last
        // waker and its handle: nothing can ever poll it again.
        abort_on_unwind(|| {
            let _tag = self.tag.scope();
            *self.future.get_mut() = None;
        });
        if let Some(shared) = self.shared.upgrade() {
            shared.forget_task(self.id);
        }
    }
}

impl<F> RawTask for TaskCell<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    fn run(self: Arc<Self>, cancel: bool) {
        let previous = self
            .state
            .try_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                Some(RUNNING | (state & CANCEL))
            })
            .expect("the update always applies");
        debug_assert_eq!(previous & PHASE, SCHEDULED, "a queue entry for an unscheduled task");
        if cancel || previous & CANCEL != 0 {
            self.finish(None);
            return;
        }

        let waker = Waker::from(self.clone());
        let poll = {
            let _tag = self.tag.scope();
            // SAFETY: this thread moved the task into the `RUNNING` phase, which grants
            // exclusive access to `future` until it leaves the phase below.
            let future = unsafe { &mut *self.future.get() };
            let future = future.as_mut().expect("the future lives until the task is done");
            // SAFETY: the future sits inside an `Arc` allocation and is never moved out of
            // it; it is dropped in place by `finish`.
            let future = unsafe { Pin::new_unchecked(future) };
            future.poll(&mut Context::from_waker(&waker))
        };
        if let Poll::Ready(output) = poll {
            self.finish(Some(output));
            return;
        }

        let mut state = self.state.load(Ordering::Acquire);
        loop {
            if state & CANCEL != 0 {
                self.finish(None);
                return;
            }
            let next = if state == RUNNING { IDLE } else { SCHEDULED };
            match self.state.compare_exchange_weak(state, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => {
                    if next == SCHEDULED {
                        self.schedule();
                    }
                    return;
                }
                Err(actual) => state = actual,
            }
        }
    }

    fn cancel(&self) {
        let mut state = self.state.load(Ordering::Acquire);
        loop {
            let next = match state & PHASE {
                DONE => return,
                // Suspended: nobody holds the future, so take it and drop it here.
                IDLE => RUNNING | CANCEL,
                _ => state | CANCEL,
            };
            match self.state.compare_exchange_weak(state, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => {
                    if state & PHASE == IDLE {
                        self.finish(None);
                    }
                    return;
                }
                Err(actual) => state = actual,
            }
        }
    }
}

impl<F> Wake for TaskCell<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let mut state = self.state.load(Ordering::Acquire);
        loop {
            let next = match state & PHASE {
                IDLE => SCHEDULED,
                RUNNING => NOTIFIED | (state & CANCEL),
                _ => return,
            };
            match self.state.compare_exchange_weak(state, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => {
                    if next == SCHEDULED {
                        self.schedule();
                    }
                    return;
                }
                Err(actual) => state = actual,
            }
        }
    }
}

impl<F> Joinable<F::Output> for TaskCell<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    fn poll_output(&self, cx: &mut Context<'_>) -> Poll<Result<F::Output>> {
        let mut join = lock(&self.join);
        match join.output.take() {
            Some(output) => Poll::Ready(Ok(output)),
            None if join.cancelled => Poll::Ready(Err(Error::new(
                ErrorKind::Cancelled,
                "the executor shut down before the task finished",
            ))),
            None => {
                join.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }

    fn cancel(&self) {
        RawTask::cancel(self);
    }
}

/// Creates a task for `future` and queues its first poll.
pub(crate) fn spawn<F>(shared: &Arc<Shared>, class: WorkerClass, future: F) -> Task<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let tag = AllocTag::current();
    let cell = {
        let _tag = AllocTag::Task.scope();
        Arc::new_cyclic(|cell: &Weak<TaskCell<F>>| TaskCell {
            state: AtomicU8::new(SCHEDULED),
            class,
            tag,
            id: shared.register_task(cell.clone()),
            shared: Arc::downgrade(shared),
            future: UnsafeCell::new(Some(future)),
            join: Mutex::new(Join { output: None, cancelled: false, waker: None }),
        })
    };
    shared.submit_task(class, cell.clone());
    Task { inner: Some(cell) }
}

/// A spawned task's result. Dropping it cancels the task at its next suspension point.
///
/// Resolves to the task's output, or to an error of kind `Cancelled` if the executor shut
/// down before the task completed.
#[must_use = "dropping a Task cancels it; call detach() to let it run unobserved"]
pub struct Task<T> {
    inner: Option<Arc<dyn Joinable<T>>>,
}

impl<T> Task<T> {
    /// Lets the task run to completion with nobody waiting for its result.
    pub fn detach(mut self) {
        self.inner = None;
    }
}

impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            inner.cancel();
        }
    }
}

impl<T> Future for Task<T> {
    type Output = Result<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<T>> {
        self.inner.as_ref().expect("the handle holds its task until dropped").poll_output(cx)
    }
}

/// Gives other tasks of the same class a turn. See [`yield_now`].
#[must_use = "futures do nothing unless awaited"]
pub struct YieldNow {
    yielded: bool,
}

impl Future for YieldNow {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.yielded {
            return Poll::Ready(());
        }
        self.yielded = true;
        // Waking a task while it is being polled sends it to the back of its class's queue
        // when the poll returns.
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

/// Reschedules the current task behind other ready tasks of its class. Background CPU work
/// awaits this at least every 0.5 ms.
pub fn yield_now() -> YieldNow {
    YieldNow { yielded: false }
}
