//! Thread lanes: the way workers reach state that lives on one particular thread (the OS
//! main thread, the present thread) without ever waiting on it synchronously.

use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll};

use lactose_core::Result;

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
    todo!()
}

/// The sending side of a lane.
pub struct Lane<S> {
    marker: PhantomData<fn(&mut S)>,
}

impl<S> Clone for Lane<S> {
    fn clone(&self) -> Self {
        Self { marker: PhantomData }
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
        todo!()
    }
}

/// The result of a lane call. Resolves to an error of kind `Cancelled` if the pump was
/// dropped before the call ran. Dropping this does not cancel a call already queued.
#[must_use = "futures do nothing unless awaited"]
pub struct LaneCall<R> {
    marker: PhantomData<fn() -> R>,
}

impl<R> Future for LaneCall<R> {
    type Output = Result<R>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<R>> {
        todo!()
    }
}

/// The receiving side of a lane, owned by the lane's thread.
pub struct LanePump<S> {
    state: PhantomData<fn(&mut S)>,
    not_send: PhantomData<*const ()>,
}

impl<S: 'static> LanePump<S> {
    /// Runs every queued call against `state` and returns how many ran. Never blocks.
    pub fn drain(&mut self, state: &mut S) -> usize {
        todo!()
    }
}
