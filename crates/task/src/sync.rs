//! The primitives the scope latch and the sleep-and-wake protocol are built from.
//!
//! With the `loom` feature these are loom's model-checked versions, so `latch` and `sleep`
//! and the modules built on them (`gate`, `quiesce`)
//! compile unchanged against either. Nothing else in the crate goes through this module: the
//! rest of the executor is not modelled, and with the feature on it must not be run, because a
//! loom object panics outside `loom::model`.
//!
//! [`TimedPark`] is one worker's timed wait, the step of the keeper's wait for a deadline
//! (`Sleep::park`). It has three forms: on Windows a high-resolution waitable timer beside an
//! event that a waker sets (D144); elsewhere, and under Miri, `std::thread::park_timeout`;
//! under loom a flag that only a waker sets.

pub(crate) use core::sync::atomic::Ordering;

#[cfg(not(feature = "loom"))]
mod imp {
    pub(crate) use core::sync::atomic::{AtomicU64, AtomicUsize, fence};
    pub(crate) use std::sync::{Mutex, MutexGuard};
    pub(crate) use std::thread::park;

    #[cfg(not(all(windows, not(miri))))]
    pub(crate) use super::portable::TimedPark;
    #[cfg(all(windows, not(miri)))]
    pub(crate) use super::windows::TimedPark;
}

#[cfg(feature = "loom")]
mod imp {
    use core::time::Duration;

    pub(crate) use loom::sync::atomic::{AtomicU64, AtomicUsize, fence};
    pub(crate) use loom::sync::{Mutex, MutexGuard};
    pub(crate) use loom::thread::park;

    /// One worker's timed wait. loom has no clock, so a wait lasts until a waker ends it,
    /// which only removes wake-ups: a model that passes does not depend on a timeout for
    /// liveness.
    ///
    /// An unpark alone does not end it, as it does not end the Windows wait, so a waker that
    /// skips [`TimedPark::wake`] fails the keeper hand-off model. Both sides swap: loom
    /// treats a `SeqCst` store as a release store, which the waiter's swap can read stale,
    /// and a flag behind a loom mutex trips loom's own assertion when an unpark makes the
    /// thread blocked on it runnable.
    pub(crate) struct TimedPark {
        woken: loom::sync::atomic::AtomicBool,
    }

    impl TimedPark {
        pub(crate) fn new() -> std::io::Result<Self> {
            Ok(Self::coarse())
        }

        pub(crate) fn coarse() -> Self {
            Self { woken: loom::sync::atomic::AtomicBool::new(false) }
        }

        pub(crate) fn wait(&self, _timeout: Duration) {
            while !self.woken.swap(false, super::Ordering::SeqCst) {
                park();
            }
        }

        pub(crate) fn wake(&self) {
            self.woken.swap(true, super::Ordering::SeqCst);
        }
    }
}

/// The timed wait wherever an unpark ends a `park_timeout` early and its timeout is as fine
/// as it asks: macOS and Linux take nanoseconds. Under Miri too, which has no Windows calls
/// to make.
#[cfg(all(not(feature = "loom"), not(all(windows, not(miri)))))]
mod portable {
    use core::time::Duration;

    pub(crate) struct TimedPark;

    impl TimedPark {
        pub(crate) fn new() -> std::io::Result<Self> {
            Ok(Self)
        }

        pub(crate) fn coarse() -> Self {
            Self
        }

        /// Parks the calling thread for at most `timeout`. An unpark ends it early.
        pub(crate) fn wait(&self, timeout: Duration) {
            std::thread::park_timeout(timeout);
        }

        /// Nothing to do: the unpark that follows it ends the wait.
        pub(crate) fn wake(&self) {}
    }
}

/// The timed wait on Windows (D144). Every timed wait of the standard library that another
/// thread can end rounds its timeout up to whole milliseconds and ends on the system's timer
/// tick, 15.6 ms by default; this one waits on a high-resolution waitable timer and on an
/// auto-reset event that a waker sets. `SAFETY.md` states what its `unsafe` relies on.
#[cfg(all(windows, not(miri), not(feature = "loom")))]
mod windows {
    use core::ptr::null;
    use core::sync::atomic::{AtomicBool, Ordering};
    use core::time::Duration;
    use std::io;

    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE, WAIT_FAILED};
    use windows_sys::Win32::System::Threading::{
        CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateEventW, CreateWaitableTimerExW, INFINITE,
        SetEvent, SetWaitableTimer, TIMER_ALL_ACCESS, WaitForMultipleObjects,
    };

    use crate::span::TARGET;

    pub(crate) struct TimedPark {
        /// The event and the timer, in the order `WaitForMultipleObjects` takes them, or
        /// `None` for a wait that falls back to `std::thread::park_timeout`.
        handles: Option<[HANDLE; 2]>,
        /// Whether a failed call has been logged.
        reported: AtomicBool,
    }

    // SAFETY: a handle is a process-wide value, valid on every thread; `SAFETY.md`, "The
    // keeper's timed wait", gives which thread may make which call on it, and when.
    unsafe impl Send for TimedPark {}
    // SAFETY: as for `Send`: any thread may set the event, only the owning worker touches
    // the timer or waits, and only `Drop`, which has the value to itself, closes either.
    unsafe impl Sync for TimedPark {}

    impl TimedPark {
        /// An auto-reset event and a high-resolution timer that is not manual-reset. Fails
        /// where either cannot be made, as before Windows 10 version 1803, which has no
        /// high-resolution timer.
        pub(crate) fn new() -> io::Result<Self> {
            // SAFETY: null attributes and a null name are allowed; auto-reset, not signalled.
            let event = unsafe { CreateEventW(null(), FALSE, FALSE, null()) };
            if event.is_null() {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: null attributes and a null name are allowed. Without
            // `CREATE_WAITABLE_TIMER_MANUAL_RESET` the timer is a synchronization timer,
            // which the wait it ends resets.
            let timer = unsafe {
                CreateWaitableTimerExW(
                    null(),
                    null(),
                    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                    TIMER_ALL_ACCESS,
                )
            };
            if timer.is_null() {
                let error = io::Error::last_os_error();
                // SAFETY: `event` was made above, is closed once, here, and was never shared.
                unsafe { CloseHandle(event) };
                return Err(error);
            }
            Ok(Self { handles: Some([event, timer]), reported: AtomicBool::new(false) })
        }

        /// A wait that is `std::thread::park_timeout`, for a worker whose event or timer
        /// could not be made.
        pub(crate) fn coarse() -> Self {
            Self { handles: None, reported: AtomicBool::new(false) }
        }

        /// Waits for at most about `timeout`, or until [`TimedPark::wake`] or an unpark of a
        /// coarse wait. Called only by the worker this belongs to.
        ///
        /// It may return early. The timer cannot cause that: it is set again before every
        /// wait, which clears a signal it raised with nothing waiting. The event can: a wake
        /// that came while this worker was not waiting on it, such as one by
        /// `Sleep::wake_claimed` while it was awake or in an untimed park, ends the next wait
        /// at once. The caller, `Sleep::park`, takes every return as a possible spurious
        /// wake: it reads its bit and the clock again and waits again.
        /// `sleep::tests::a_stale_wake_does_not_end_a_timed_park_early` holds it to that.
        ///
        /// A failed call is logged once, and that wait is a `park_timeout` instead, so the
        /// caller never spins on a call that keeps failing.
        pub(crate) fn wait(&self, timeout: Duration) {
            let Some([event, timer]) = self.handles else {
                return std::thread::park_timeout(timeout);
            };
            // Negative is relative, in 100 ns units, rounded up so the wait is not asked for
            // less than `timeout`.
            let intervals = timeout.as_nanos().div_ceil(100).max(1);
            let due = -i64::try_from(intervals).unwrap_or(i64::MAX);
            // SAFETY: the timer is open, since only `Drop` closes it, and only this worker
            // sets it; `due` outlives the call, and there is no completion routine.
            if unsafe { SetWaitableTimer(timer, &raw const due, 0, None, null(), FALSE) } == 0 {
                self.report("SetWaitableTimer");
                return std::thread::park_timeout(timeout);
            }
            let handles = [event, timer];
            // SAFETY: both handles are open, and the array of two outlives the call. Ends
            // when either is signalled; the timer always will be.
            let ended = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), FALSE, INFINITE) };
            if ended == WAIT_FAILED {
                self.report("WaitForMultipleObjects");
                std::thread::park_timeout(timeout);
            }
        }

        /// Ends this worker's wait, or the next one if it is not waiting. Any thread may call
        /// it; the unpark that follows it ends a coarse wait.
        pub(crate) fn wake(&self) {
            let Some([event, _]) = self.handles else { return };
            // SAFETY: the event is open: the caller reached this through the pool, which
            // outlives every reference to it, and only `Drop` closes the event.
            if unsafe { SetEvent(event) } == 0 {
                self.report("SetEvent");
            }
        }

        fn report(&self, call: &'static str) {
            let error = io::Error::last_os_error();
            if !self.reported.swap(true, Ordering::Relaxed) {
                tracing::warn!(
                    target: TARGET,
                    %error,
                    call,
                    "the keeper's timed wait failed; such a wait is a std park_timeout, in whole \
                     milliseconds",
                );
            }
        }
    }

    impl Drop for TimedPark {
        fn drop(&mut self) {
            for handle in self.handles.take().into_iter().flatten() {
                // SAFETY: each handle was made by `new` and is closed once, here. `&mut self`
                // is the only reference to it: the pool that holds it is being dropped with
                // its last reference (`SAFETY.md`).
                unsafe { CloseHandle(handle) };
            }
        }
    }
}

pub(crate) use imp::*;
