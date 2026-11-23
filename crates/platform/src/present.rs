//! The present thread: a thread of class `ThreadClass::Present` that owns one state and
//! drains one lane, so that whatever blocks in an acquire or a present blocks nothing else.

use core::cell::Cell;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::sync::mpsc;

use lactose_core::{Error, ErrorKind, Result, ThreadClass};
use lactose_task::{Lane, LaneCall, Platform, ThreadSpec, lane, parking_lane};

thread_local! {
    /// Set by the stop call, read by the body that runs it once its drain is done. The
    /// present thread's own: the body clears it when it starts, since a platform may run a
    /// body on a thread that ran another.
    static STOPPING: Cell<bool> = const { Cell::new(false) };
}

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
    // The lanes are made on the present thread, which owns their pumps, and come back here.
    // The body holds the sender, so a body dropped unrun ends the wait below.
    let (started, start) = mpsc::channel();
    let spec =
        ThreadSpec { name: "present".to_owned(), class: ThreadClass::Present, index_in_class: 0 };
    // The handle is let go: the thread is never joined (`README.md`, "The present thread
    // and its pump").
    drop(platform.spawn_thread(spec, Box::new(move || present(state, started)))?);
    let Ok((present, hand_back)) = start.recv() else {
        return Err(Error::new(
            ErrorKind::Cancelled,
            "the platform dropped the present thread's body unrun",
        ));
    };
    let stopping: Lane<S> = Lane::clone(&present);
    let stop = Box::new(move || drop(stopping.run(|_| STOPPING.set(true))));
    Ok((present, PresentThread { hand_back, stop }))
}

/// The present thread's body: it runs the present lane's calls until a stop call has run,
/// closes that lane, hands the state back once, and returns.
fn present<S: 'static>(mut state: S, started: mpsc::Sender<(Lane<S>, Lane<Option<S>>)>) {
    STOPPING.set(false);
    // `lactose-task`'s parking lane: a call that parks, as a channel's receive and a lock's
    // wait do, may take the thread's unpark, and the lane's wait does not lose that wake.
    let (present, mut pump) = parking_lane("present");
    // Drained once, after the stop: the take is queued before the stop, so it needs no wake.
    let (hand_back, mut handing_back) = lane("present hand-back", || {});
    if started.send((present, hand_back)).is_err() {
        return;
    }
    drop(started);
    loop {
        pump.drain(&mut state);
        if STOPPING.get() {
            break;
        }
        // A call queued after the drain began ends the wait at once.
        pump.wait();
    }
    // Every call not run, and every call queued from here on, resolves to `Cancelled`.
    drop(pump);
    let mut slot = Some(state);
    handing_back.drain(&mut slot);
    // `slot` still holds the state if no take was queued, as when the `PresentThread` was
    // dropped: it is dropped here, on the present thread.
}

/// The present thread, as the one who started it holds it: the way to stop it and to take
/// its state back. `Send`, so a task may hold it.
///
/// [`stop`](Self::stop) is the way to end the thread. Dropping it ends the thread too, for
/// a caller that unwinds: the thread runs what was queued before the drop and drops the
/// state on the present thread, and nothing says when that is done (`README.md`, "The
/// present thread and its pump").
pub struct PresentThread<S> {
    /// The lane the thread hands its state back through. `stop` queues the take here
    /// before it queues the stop, and the thread drains it once, after it has dropped the
    /// present lane's pump.
    hand_back: Lane<Option<S>>,
    /// Queues the stop call on the present lane, after every call queued before it. The
    /// call sets the present thread's own flag, a thread-local the body clears when it
    /// starts. Run by the drop, on every path; a closure, because the drop of a type
    /// without bounds cannot reach `Lane::run`.
    stop: Box<dyn Fn() + Send>,
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
        let state = self.hand_back.run(Option::take);
        // The drop queues the stop, behind the take.
        drop(self);
        PresentStop { state }
    }
}

impl<S> Drop for PresentThread<S> {
    fn drop(&mut self) {
        (self.stop)();
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
        let ended =
            || Error::new(ErrorKind::Cancelled, "the present thread ended without handing back");
        match Pin::new(&mut self.get_mut().state).poll(cx) {
            Poll::Ready(Ok(Some(state))) => Poll::Ready(Ok(state)),
            // The one take finds the state, so `None` is not reached; it reads as the end.
            Poll::Ready(Ok(None) | Err(_)) => Poll::Ready(Err(ended())),
            Poll::Pending => Poll::Pending,
        }
    }
}
