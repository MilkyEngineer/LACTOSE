//! Thread lanes: the way workers reach state that lives on one particular thread (the OS
//! main thread, the present thread) without ever waiting on it synchronously.

use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use lactose_core::{Error, ErrorKind, Result};

use crate::lock;
use crate::span::TARGET;

type Call<S> = Box<dyn FnOnce(&mut S) + Send>;

struct Queue<S> {
    calls: VecDeque<Call<S>>,
    /// The pump is gone; nothing queued from now on will run.
    closed: bool,
}

struct Shared<S> {
    name: &'static str,
    wake: Box<dyn Fn() + Send + Sync>,
    queue: Mutex<Queue<S>>,
}

/// Creates a lane for a thread that owns state `S`.
///
/// The owning thread keeps the [`LanePump`] and drains it from its own loop. Everyone else
/// holds clones of the [`Lane`]. `S` never leaves the owning thread, so it need not be
/// `Send`: a window, a swapchain surface or an event loop are all fine.
///
/// `name` appears in tracing spans. `wake` is called from the submitting thread whenever a
/// call is queued, so the owner can interrupt a blocking wait (an event-loop proxy on the
/// main thread, an unpark on the present thread).
pub fn lane<S: 'static>(
    name: &'static str,
    wake: impl Fn() + Send + Sync + 'static,
) -> (Lane<S>, LanePump<S>) {
    let shared = Arc::new(Shared {
        name,
        wake: Box::new(wake),
        queue: Mutex::new(Queue { calls: VecDeque::new(), closed: false }),
    });
    (Lane { shared: shared.clone() }, LanePump { shared, not_send: PhantomData })
}

/// The sending side of a lane.
pub struct Lane<S> {
    shared: Arc<Shared<S>>,
}

impl<S> Clone for Lane<S> {
    fn clone(&self) -> Self {
        Self { shared: self.shared.clone() }
    }
}

impl<S: 'static> Lane<S> {
    /// Queues `f` to run on the lane's thread with exclusive access to its state, in
    /// submission order per sender.
    pub fn run<F, R>(&self, f: F) -> LaneCall<R>
    where
        F: FnOnce(&mut S) -> R + Send + 'static,
        R: Send + 'static,
    {
        let slot = Arc::new(Mutex::new(Slot { result: None, waker: None, settled: false }));
        // If the call is dropped without running, the reply's destructor reports that.
        let reply = Reply { slot: slot.clone() };
        let call: Call<S> = Box::new(move |state| reply.send(Ok(f(state))));

        let mut queue = lock(&self.shared.queue);
        if queue.closed {
            drop(queue);
            drop(call);
        } else {
            queue.calls.push_back(call);
            drop(queue);
            (self.shared.wake)();
        }
        LaneCall { slot }
    }
}

struct Slot<R> {
    result: Option<Result<R>>,
    waker: Option<Waker>,
    /// A result has been stored, whether or not the future has taken it yet.
    settled: bool,
}

/// The lane thread's end of a [`LaneCall`].
struct Reply<R> {
    slot: Arc<Mutex<Slot<R>>>,
}

impl<R> Reply<R> {
    fn send(self, result: Result<R>) {
        self.put(result);
    }

    fn put(&self, result: Result<R>) {
        let waker = {
            let mut slot = lock(&self.slot);
            if slot.settled {
                return;
            }
            slot.settled = true;
            slot.result = Some(result);
            slot.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl<R> Drop for Reply<R> {
    fn drop(&mut self) {
        // A no-op after `send`. Otherwise the call was dropped unrun: the pump is gone, or
        // an earlier call in the same drain panicked.
        self.put(Err(Error::new(ErrorKind::Cancelled, "the lane's thread dropped its pump")));
    }
}

/// The result of a lane call. Resolves to an error of kind `Cancelled` if the pump was
/// dropped before the call ran. Dropping this does not cancel a call already queued.
#[must_use = "futures do nothing unless awaited"]
pub struct LaneCall<R> {
    slot: Arc<Mutex<Slot<R>>>,
}

impl<R> Future for LaneCall<R> {
    type Output = Result<R>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<R>> {
        let mut slot = lock(&self.slot);
        match slot.result.take() {
            Some(result) => Poll::Ready(result),
            None => {
                slot.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// The receiving side of a lane, owned by the lane's thread.
pub struct LanePump<S> {
    shared: Arc<Shared<S>>,
    not_send: PhantomData<*const ()>,
}

impl<S: 'static> LanePump<S> {
    /// Runs every call that was queued when it was called against `state` and returns how
    /// many ran. Never blocks. Calls queued while it runs wait for the next drain; each of
    /// them triggers the lane's `wake` again.
    pub fn drain(&mut self, state: &mut S) -> usize {
        let calls = core::mem::take(&mut lock(&self.shared.queue).calls);
        if calls.is_empty() {
            return 0;
        }
        let _span = tracing::info_span!(target: TARGET, "lane", name = self.shared.name).entered();
        let count = calls.len();
        for call in calls {
            call(state);
        }
        count
    }
}

impl<S> Drop for LanePump<S> {
    fn drop(&mut self) {
        let calls = {
            let mut queue = lock(&self.shared.queue);
            queue.closed = true;
            core::mem::take(&mut queue.calls)
        };
        // Dropping a call resolves its future to `Cancelled`.
        drop(calls);
    }
}
