//! The present thread: a thread of class `ThreadClass::Present` that owns one state and
//! drains one lane, so that whatever blocks in an acquire or a present blocks nothing else.

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

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
/// Whatever `platform` returns for a thread it cannot make. `state` is then dropped on
/// the calling thread.
pub fn present_thread<S: Send + 'static>(
    platform: &dyn Platform,
    state: S,
) -> Result<(Lane<S>, PresentThread<S>)> {
    todo!()
}

/// The present thread, as the one who started it holds it: the way to stop it and to take
/// its state back. `Send`, so a task may hold it.
///
/// Dropping it stops the thread as [`PresentThread::stop`] does, and the state is then
/// dropped on the present thread.
pub struct PresentThread<S> {
    /// Set by `stop`, and by a drop, before the thread is woken.
    stopping: Arc<AtomicBool>,
    /// The lane the thread hands its state back through, drained once, after the last
    /// call.
    hand_back: Lane<Option<S>>,
    /// The thread, as the platform gave it.
    thread: ThreadHandle,
}

impl<S: Send + 'static> PresentThread<S> {
    /// Asks the thread to finish, and returns the future of its state. It never waits.
    ///
    /// The thread runs every call queued before `stop` was called, the one in progress
    /// included, then runs no other, hands the state back and exits. A call queued
    /// after `stop` was called runs before the hand-back or resolves to
    /// `ErrorKind::Cancelled`.
    pub fn stop(self) -> PresentStop<S> {
        todo!()
    }
}

/// The state of a present thread that was asked to stop: it resolves, on whichever thread
/// polls it, once the thread has run its last call, and from then on the present thread
/// runs nothing of the caller's and holds nothing of the caller's.
///
/// It resolves to `ErrorKind::Cancelled` if the thread ended without handing the state
/// back, which only a panic in one of its calls causes. Dropping it does not undo the
/// stop: the state is dropped where the future was, or on the present thread.
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
