//! The present thread: a thread of class `ThreadClass::Present` that owns one state and
//! drains one lane, so that whatever blocks in an acquire or a present blocks nothing else.

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use lactose_core::Result;
use lactose_task::{Lane, LaneCall, Platform, ThreadHandle};

/// Starts the present thread and moves `state` onto it.
///
/// The thread is made through `platform`, which a client passes as `Executor::platform`,
/// with the name `present`, the class `ThreadClass::Present` and index zero. `state` is
/// whatever presents: the swapchain and what draws into its images, as a type of the
/// caller's, never of this crate's. The [`Lane`] is the only way to it. Each call queued
/// on the lane runs on the present thread with the state lent to it, one at a time, in the
/// order `lactose-task`'s lanes give; a call may block for as long as an acquire or a
/// present does, and holds only the calls queued behind it (`README.md`, "The present
/// thread and its pump").
///
/// It returns once the thread has started: a setup call, for `main` and for tests, and
/// never for a worker or a task.
///
/// # Errors
///
/// Whatever `platform` returns for a thread it cannot make. `ErrorKind::Cancelled` if
/// `platform` accepted the thread's body and dropped it unrun. Either way `state` is
/// dropped, and no call of the lane will run.
pub fn present_thread<S: Send + 'static>(
    platform: &dyn Platform,
    state: S,
) -> Result<(Lane<S>, PresentThread<S>)> {
    todo!()
}

/// The present thread, as the one who started it holds it: the way to stop it and to take
/// its state back. `Send`, so a task may hold it.
///
/// [`stop`](Self::stop) is the way to end the thread. Dropping it ends the thread too, for
/// a caller that unwinds: the thread runs what was queued before the drop and drops the
/// state on the present thread, and nothing says when that is done (`README.md`, "The
/// present thread and its pump").
pub struct PresentThread<S> {
    /// The lane every call reaches the state through. The stop is a call on it, queued
    /// after every call before `stop`, which marks the end of the thread's last batch by a
    /// flag of the present thread's own: a thread-local the body clears when it starts,
    /// since a platform may run a body on a thread that ran another.
    present: Lane<S>,
    /// The lane the thread hands its state back through. `stop` queues the take here
    /// before it queues the stop, and the thread drains it once, after it has dropped the
    /// present lane's pump.
    hand_back: Lane<Option<S>>,
    /// The thread, as the platform gave it. It is never joined.
    thread: ThreadHandle,
}

impl<S: Send + 'static> PresentThread<S> {
    /// Asks the thread to finish, and returns the future of its state. It never waits.
    ///
    /// It queues the take of the state on the hand-back lane, then the stop on the present
    /// lane, so "queued before `stop`" is the present lane's own order. The thread runs
    /// every call queued before the stop, the one in progress included, and every call
    /// that reached the same drain after it; it then drops its pump, which resolves every
    /// call not run to `ErrorKind::Cancelled`, hands the state back and exits.
    pub fn stop(self) -> PresentStop<S> {
        todo!()
    }
}

/// The state of a present thread that was asked to stop: it resolves, on whichever thread
/// polls it, once the thread has run its last call, and from then on the present thread
/// runs nothing of the caller's and holds nothing of the caller's.
///
/// It resolves to `ErrorKind::Cancelled` if the thread ended without handing the state
/// back, which only a panic in one of its calls causes. It does not resolve while a call
/// queued before the stop has not returned, which is why the caller bounds every wait
/// inside a present call. Dropping it does not undo the stop: a state not yet handed back
/// is dropped on the present thread, and one handed back is dropped with the future, on
/// the thread that drops it.
#[must_use = "futures do nothing unless awaited"]
pub struct PresentStop<S> {
    /// The hand-back, as a call on the thread's second lane.
    state: LaneCall<Option<S>>,
}

impl<S: Send + 'static> Future for PresentStop<S> {
    type Output = Result<S>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<S>> {
        todo!()
    }
}
