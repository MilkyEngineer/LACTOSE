//! The calling thread's CPU time, read through the operating system, for the tests that hold
//! a pump idle and bound what it spends (`SAFETY.md`, "In the tests"). A module of both
//! test binaries.

#![allow(unsafe_code, reason = "a thread's CPU clock has no safe reading in std (`SAFETY.md`)")]

use std::time::Duration;

/// The CPU time the calling thread has used, in user and kernel mode.
#[cfg(unix)]
pub fn thread_cpu() -> Duration {
    let mut time = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: the pointer is to a live local the call writes, and the clock is the calling
    // thread's, which macOS and Linux both have.
    let status = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &raw mut time) };
    assert_eq!(status, 0, "the thread's CPU clock could not be read");
    let seconds = u64::try_from(time.tv_sec).expect("a clock that starts at zero");
    let nanos = u32::try_from(time.tv_nsec).expect("under a second of nanoseconds");
    Duration::new(seconds, nanos)
}

/// The CPU time the calling thread has used, in user and kernel mode.
#[cfg(windows)]
pub fn thread_cpu() -> Duration {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};

    let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: the pseudo-handle of the calling thread is always valid, and each pointer is to
    // a live local the call writes.
    let ok = unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &raw mut created,
            &raw mut exited,
            &raw mut kernel,
            &raw mut user,
        )
    };
    assert_ne!(ok, 0, "the thread's times could not be read");
    let ticks =
        |time: FILETIME| u64::from(time.dwHighDateTime) << 32 | u64::from(time.dwLowDateTime);
    // In units of 100 ns.
    Duration::from_nanos((ticks(kernel) + ticks(user)) * 100)
}
