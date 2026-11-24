//! Thread classes: scheduling priority and floating-point state for every thread that runs
//! engine work, including threads the executor does not own (the physics solver pool, the
//! audio mixer). Those crates call [`ThreadClass::enter`] instead of writing their own unsafe.

use crate::error::Result;

/// What a thread is for, which decides its scheduling class.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ThreadClass {
    /// Simulation, extract, graph compile and record. Performance cores.
    FrameCritical,
    /// Asset decode, compression and long futures. Efficiency cores.
    Background,
    /// Packet parsing, at frame-critical priority so input lateness does not grow with load.
    Network,
    /// The thread that calls present and may block in it.
    Present,
    /// The audio mixer.
    Audio,
    /// A thread of the physics solver pool.
    PhysicsSolver,
    /// A thread of the file IO pool (`lactose-io`): it makes blocking file system calls and
    /// decodes nothing, since asset decode runs on background workers. Utility priority like
    /// `Background`, as its own class so a platform that pins by class and index never puts
    /// it on a background worker's core.
    Io,
}

impl ThreadClass {
    /// Applies this class to the calling thread.
    ///
    /// The floating-point state is always set (see [`fp::set_deterministic`]). The scheduling
    /// class is best effort: an operating system that refuses it yields an error of kind
    /// `Platform`, whose message names the call refused and on Windows the system's error
    /// code, and the thread still runs correctly under the operating system's default for
    /// what was refused.
    ///
    /// Enter a class at thread start; entering the class a thread is already in is
    /// harmless. On Linux the two utility classes, `Background` and `Io`, lower priority in a
    /// way an unprivileged thread cannot undo, so a later `enter` of another class leaves it
    /// lowered. On Windows every class sets thread priority and then the thread's
    /// power-throttling state. `Background` opts into execution-speed throttling (EcoQoS),
    /// which on Windows 11 steers the thread to efficiency cores at their most efficient
    /// frequency, on mains power as on battery. Every other class, `Io` included, hands the
    /// state back to the system's own management, so there a later `enter` of another class
    /// undoes `Background`'s. It also clears a throttling state that other code set on the
    /// thread, such as a middleware's on the mixer thread it created. Windows 10 before
    /// version 1709 has no such state: there `Background` reports the refusal and the other
    /// classes do not.
    pub fn enter(self) -> Result<()> {
        fp::set_deterministic();
        os::set_scheduling(self)
    }
}

/// Floating-point control state (MXCSR on x86-64, FPCR on aarch64).
///
/// The deterministic state is the IEEE default: round to nearest, denormals honoured, no
/// flush to zero, no default-NaN mode, exceptions masked. On Linux a new thread starts in
/// its creator's floating-point state (POSIX), so a thread that runs boundary work sets the
/// state at start, whatever thread created it. Engine threads re-assert it because foreign
/// code (audio drivers, plug-ins, some game overlays) changes it and does not always put it
/// back.
pub mod fp {
    /// Puts the calling thread's floating-point control register in the deterministic state.
    pub fn set_deterministic() {
        arch::set_deterministic();
    }

    /// Whether the calling thread's floating-point control register is in the deterministic
    /// state. Boundary jobs assert this in debug builds.
    #[must_use]
    pub fn is_deterministic() -> bool {
        arch::is_deterministic()
    }

    #[cfg(all(target_arch = "x86_64", not(miri)))]
    mod arch {
        use core::arch::asm;

        const DENORMALS_ARE_ZERO: u32 = 1 << 6;
        const EXCEPTION_MASKS: u32 = 0b11_1111 << 7;
        const ROUNDING: u32 = 0b11 << 13;
        const FLUSH_TO_ZERO: u32 = 1 << 15;
        const MUST_BE_CLEAR: u32 = DENORMALS_ARE_ZERO | ROUNDING | FLUSH_TO_ZERO;

        fn read() -> u32 {
            let mut value = 0u32;
            // SAFETY: `stmxcsr` stores the 32-bit MXCSR to the given address, which is a
            // live local, and has no other effect.
            unsafe {
                asm!("stmxcsr [{}]", in(reg) &raw mut value, options(nostack, preserves_flags));
            }
            value
        }

        pub(super) fn set_deterministic() {
            let value = (read() & !MUST_BE_CLEAR) | EXCEPTION_MASKS;
            // SAFETY: `ldmxcsr` loads MXCSR from a live local. The value keeps reserved bits
            // as read, masks every exception, and selects the IEEE default modes, which is
            // the state Rust assumes floating-point code runs in.
            unsafe {
                asm!("ldmxcsr [{}]", in(reg) &raw const value, options(nostack, readonly, preserves_flags));
            }
        }

        pub(super) fn is_deterministic() -> bool {
            let value = read();
            value & MUST_BE_CLEAR == 0 && value & EXCEPTION_MASKS == EXCEPTION_MASKS
        }

        #[cfg(test)]
        pub(super) fn set_flush_to_zero() {
            let value = read() | FLUSH_TO_ZERO;
            // SAFETY: as in `set_deterministic`; the test restores the default state before
            // any floating-point code runs.
            unsafe {
                asm!("ldmxcsr [{}]", in(reg) &raw const value, options(nostack, readonly, preserves_flags));
            }
        }
    }

    #[cfg(all(target_arch = "aarch64", not(miri)))]
    mod arch {
        use core::arch::asm;

        const TRAP_ENABLES: u64 = 0b1001_1111 << 8;
        const ROUNDING: u64 = 0b11 << 22;
        const FLUSH_TO_ZERO: u64 = 1 << 24;
        const DEFAULT_NAN: u64 = 1 << 25;
        // Alternate modes: flush inputs to zero, alternate handling and NaN element
        // propagation (FEAT_AFP), half-precision flush to zero, alternate half-precision.
        // They read as zero and ignore writes on cores without the feature.
        const ALTERNATE: u64 = 0b111 | 1 << 19 | 1 << 26;
        const MUST_BE_CLEAR: u64 =
            TRAP_ENABLES | ROUNDING | FLUSH_TO_ZERO | DEFAULT_NAN | ALTERNATE;

        fn read() -> u64 {
            let value: u64;
            // SAFETY: reading FPCR has no side effect.
            unsafe {
                asm!("mrs {}, fpcr", out(reg) value, options(nomem, nostack, preserves_flags));
            }
            value
        }

        fn write(value: u64) {
            // SAFETY: writing FPCR changes only floating-point control state. Callers pass
            // a value read from FPCR with mode bits changed.
            unsafe {
                asm!("msr fpcr, {}", in(reg) value, options(nomem, nostack, preserves_flags));
            }
        }

        pub(super) fn set_deterministic() {
            // The cleared bits select the IEEE default modes, which is the state Rust
            // assumes floating-point code runs in.
            write(read() & !MUST_BE_CLEAR);
        }

        pub(super) fn is_deterministic() -> bool {
            read() & MUST_BE_CLEAR == 0
        }

        #[cfg(test)]
        pub(super) fn set_flush_to_zero() {
            // The test restores the default state before any floating-point code runs.
            write(read() | FLUSH_TO_ZERO);
        }
    }

    #[cfg(any(miri, not(any(target_arch = "x86_64", target_arch = "aarch64"))))]
    mod arch {
        pub(super) fn set_deterministic() {}

        pub(super) fn is_deterministic() -> bool {
            true
        }
    }

    #[cfg(all(test, not(miri), any(target_arch = "x86_64", target_arch = "aarch64")))]
    mod tests {
        use super::*;

        #[test]
        fn detects_and_repairs_flush_to_zero() {
            assert!(is_deterministic(), "threads start in the IEEE default state");
            arch::set_flush_to_zero();
            assert!(!is_deterministic());
            set_deterministic();
            assert!(is_deterministic());
        }
    }
}

#[cfg(target_vendor = "apple")]
mod os {
    use super::ThreadClass;
    use crate::error::{Error, ErrorKind, Result};

    pub(super) fn set_scheduling(class: ThreadClass) -> Result<()> {
        let qos = match class {
            ThreadClass::Background | ThreadClass::Io => libc::qos_class_t::QOS_CLASS_UTILITY,
            ThreadClass::FrameCritical
            | ThreadClass::Network
            | ThreadClass::Present
            | ThreadClass::Audio
            | ThreadClass::PhysicsSolver => libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE,
        };
        // SAFETY: the call affects only the calling thread and takes plain values.
        let status = unsafe { libc::pthread_set_qos_class_self_np(qos, 0) };
        if status == 0 {
            Ok(())
        } else {
            Err(Error::new(ErrorKind::Platform, format!("setting thread QoS failed: {status}")))
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod os {
    use super::ThreadClass;
    use crate::error::{Error, ErrorKind, Result};

    const BACKGROUND_NICE: libc::c_int = 10;

    pub(super) fn set_scheduling(class: ThreadClass) -> Result<()> {
        // Raising priority needs privileges a game does not have, so only the two utility
        // classes are adjusted. On Linux a nice value applies to one thread when given its ID.
        if !matches!(class, ThreadClass::Background | ThreadClass::Io) {
            return Ok(());
        }
        // SAFETY: `gettid` has no preconditions, and `setpriority` takes plain values and
        // affects only the calling thread.
        let status = unsafe {
            let thread = libc::gettid() as libc::id_t;
            libc::setpriority(libc::PRIO_PROCESS as _, thread, BACKGROUND_NICE)
        };
        if status == 0 {
            Ok(())
        } else {
            let source = std::io::Error::last_os_error();
            Err(Error::new(ErrorKind::Platform, "setting thread priority failed")
                .with_source(source))
        }
    }
}

/// Windows' rules for a class, in plain values so that every host's tests hold them; the
/// Windows module checks the values against `windows-sys` at compile time.
#[cfg(any(windows, test))]
mod windows_rules {
    use super::ThreadClass;
    use crate::error::{Error, ErrorKind, Result};

    pub(super) const PRIORITY_BELOW_NORMAL: i32 = -1;
    pub(super) const PRIORITY_ABOVE_NORMAL: i32 = 1;
    pub(super) const PRIORITY_TIME_CRITICAL: i32 = 15;
    /// `THREAD_POWER_THROTTLING_EXECUTION_SPEED`.
    pub(super) const EXECUTION_SPEED: u32 = 1;
    pub(super) const ERROR_INVALID_PARAMETER: u32 = 87;

    /// Applies `class` through Windows' two calls. `set_priority` takes a thread priority;
    /// `set_throttling` takes the mask that both masks of the thread's power-throttling state
    /// are set to. Each returns the system's error code when it is refused. Priority goes
    /// first, and each call is made whatever the other returned.
    ///
    /// `Background` opts into execution-speed throttling, which Windows 11 calls EcoQoS: the
    /// most efficient frequency and the efficiency cores, on mains power as on battery
    /// (Windows 10, which has no EcoQoS, applies it as its low QoS, on battery only). Every
    /// other class hands the state back to the system, which infers a level from window
    /// focus, audibility and priority. No class opts out of throttling: that would hold its
    /// thread at high QoS while the game is minimised.
    pub(super) fn apply(
        class: ThreadClass,
        set_priority: impl FnOnce(i32) -> Result<(), u32>,
        set_throttling: impl FnOnce(u32) -> Result<(), u32>,
    ) -> Result<()> {
        let priority = set_priority(match class {
            ThreadClass::Background | ThreadClass::Io => PRIORITY_BELOW_NORMAL,
            ThreadClass::Audio => PRIORITY_TIME_CRITICAL,
            ThreadClass::FrameCritical
            | ThreadClass::Network
            | ThreadClass::Present
            | ThreadClass::PhysicsSolver => PRIORITY_ABOVE_NORMAL,
        });
        let mask = match class {
            ThreadClass::Background => EXECUTION_SPEED,
            ThreadClass::FrameCritical
            | ThreadClass::Network
            | ThreadClass::Present
            | ThreadClass::Audio
            | ThreadClass::PhysicsSolver
            | ThreadClass::Io => 0,
        };
        let throttling = match set_throttling(mask) {
            // Windows 10 before version 1709 refuses the information class. A thread there is
            // under the system's management already, which is all a reset asks for, so only a
            // refused opt-in is reported, and only by the threads it concerns.
            Err(ERROR_INVALID_PARAMETER) if mask == 0 => Ok(()),
            other => other,
        };
        // `Error`'s display shows the message alone, so the codes go into it.
        let message = match (priority, throttling) {
            (Ok(()), Ok(())) => return Ok(()),
            (Err(code), Ok(())) => format!("setting thread priority failed (Windows error {code})"),
            (Ok(()), Err(code)) => {
                format!("setting the thread's power-throttling state failed (Windows error {code})")
            }
            (Err(priority), Err(throttling)) => format!(
                "setting thread priority failed (Windows error {priority}), and setting the \
                 thread's power-throttling state failed (Windows error {throttling})"
            ),
        };
        Err(Error::new(ErrorKind::Platform, message))
    }

    #[cfg(test)]
    mod tests {
        use std::cell::RefCell;

        use super::*;

        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        enum Call {
            Priority(i32),
            Throttling(u32),
        }

        /// Applies `class` with the two calls answering `priority` and `throttling`, and
        /// returns the calls in the order they were made.
        fn run(
            class: ThreadClass,
            priority: Result<(), u32>,
            throttling: Result<(), u32>,
        ) -> (Vec<Call>, Result<()>) {
            let calls = RefCell::new(Vec::new());
            let result = apply(
                class,
                |value| {
                    calls.borrow_mut().push(Call::Priority(value));
                    priority
                },
                |mask| {
                    calls.borrow_mut().push(Call::Throttling(mask));
                    throttling
                },
            );
            (calls.into_inner(), result)
        }

        fn message(result: Result<()>) -> String {
            let error = result.unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Platform);
            error.message().to_owned()
        }

        #[test]
        fn each_class_sets_its_priority_then_its_throttling_state() {
            for (class, priority, mask) in [
                (ThreadClass::FrameCritical, PRIORITY_ABOVE_NORMAL, 0),
                (ThreadClass::Background, PRIORITY_BELOW_NORMAL, EXECUTION_SPEED),
                (ThreadClass::Network, PRIORITY_ABOVE_NORMAL, 0),
                (ThreadClass::Present, PRIORITY_ABOVE_NORMAL, 0),
                (ThreadClass::Audio, PRIORITY_TIME_CRITICAL, 0),
                (ThreadClass::PhysicsSolver, PRIORITY_ABOVE_NORMAL, 0),
                (ThreadClass::Io, PRIORITY_BELOW_NORMAL, 0),
            ] {
                let (calls, result) = run(class, Ok(()), Ok(()));
                assert_eq!(calls, [Call::Priority(priority), Call::Throttling(mask)], "{class:?}");
                result.unwrap();
            }
        }

        #[test]
        fn a_refused_priority_still_sets_the_throttling_state_and_is_named() {
            let (calls, result) = run(ThreadClass::Background, Err(5), Ok(()));
            assert_eq!(
                calls,
                [Call::Priority(PRIORITY_BELOW_NORMAL), Call::Throttling(EXECUTION_SPEED)]
            );
            assert_eq!(message(result), "setting thread priority failed (Windows error 5)");
            for class in [
                ThreadClass::FrameCritical,
                ThreadClass::Network,
                ThreadClass::Present,
                ThreadClass::Audio,
                ThreadClass::PhysicsSolver,
                ThreadClass::Io,
            ] {
                let (_, result) = run(class, Err(5), Err(ERROR_INVALID_PARAMETER));
                assert_eq!(
                    message(result),
                    "setting thread priority failed (Windows error 5)",
                    "{class:?}"
                );
            }
        }

        #[test]
        fn a_refused_opt_in_is_an_error_that_names_the_call() {
            for code in [ERROR_INVALID_PARAMETER, 5] {
                let (_, result) = run(ThreadClass::Background, Ok(()), Err(code));
                assert_eq!(
                    message(result),
                    format!(
                        "setting the thread's power-throttling state failed (Windows error {code})"
                    )
                );
            }
        }

        #[test]
        fn a_reset_refused_as_an_unknown_class_is_success_and_no_other_refusal_is() {
            for class in [
                ThreadClass::FrameCritical,
                ThreadClass::Network,
                ThreadClass::Present,
                ThreadClass::Audio,
                ThreadClass::PhysicsSolver,
                ThreadClass::Io,
            ] {
                run(class, Ok(()), Err(ERROR_INVALID_PARAMETER)).1.unwrap();
                let (_, refused) = run(class, Ok(()), Err(5));
                assert_eq!(
                    message(refused),
                    "setting the thread's power-throttling state failed (Windows error 5)",
                    "{class:?}"
                );
            }
        }

        #[test]
        fn both_refusals_are_reported_with_both_codes() {
            let (calls, result) = run(ThreadClass::Background, Err(5), Err(1450));
            assert_eq!(calls.len(), 2);
            assert_eq!(
                message(result),
                "setting thread priority failed (Windows error 5), and setting the thread's \
                 power-throttling state failed (Windows error 1450)"
            );
        }
    }
}

#[cfg(windows)]
mod os {
    use windows_sys::Win32::Foundation::{ERROR_INVALID_PARAMETER, GetLastError};
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadInformation, SetThreadPriority,
        THREAD_POWER_THROTTLING_CURRENT_VERSION, THREAD_POWER_THROTTLING_EXECUTION_SPEED,
        THREAD_POWER_THROTTLING_STATE, THREAD_PRIORITY_ABOVE_NORMAL, THREAD_PRIORITY_BELOW_NORMAL,
        THREAD_PRIORITY_TIME_CRITICAL, ThreadPowerThrottling,
    };

    use super::{ThreadClass, windows_rules};
    use crate::error::Result;

    const _: () = {
        assert!(windows_rules::PRIORITY_BELOW_NORMAL == THREAD_PRIORITY_BELOW_NORMAL);
        assert!(windows_rules::PRIORITY_ABOVE_NORMAL == THREAD_PRIORITY_ABOVE_NORMAL);
        assert!(windows_rules::PRIORITY_TIME_CRITICAL == THREAD_PRIORITY_TIME_CRITICAL);
        assert!(windows_rules::EXECUTION_SPEED == THREAD_POWER_THROTTLING_EXECUTION_SPEED);
        assert!(windows_rules::ERROR_INVALID_PARAMETER == ERROR_INVALID_PARAMETER);
    };

    pub(super) fn set_scheduling(class: ThreadClass) -> Result<()> {
        windows_rules::apply(class, set_priority, set_power_throttling)
    }

    fn set_priority(priority: i32) -> Result<(), u32> {
        // SAFETY: `GetCurrentThread` returns a pseudo-handle that is always valid for the
        // calling thread, and `SetThreadPriority` takes plain values.
        let ok = unsafe { SetThreadPriority(GetCurrentThread(), priority) };
        // SAFETY: `GetLastError` reads the calling thread's last error and has no
        // preconditions.
        if ok != 0 { Ok(()) } else { Err(unsafe { GetLastError() }) }
    }

    fn set_power_throttling(mask: u32) -> Result<(), u32> {
        let state = THREAD_POWER_THROTTLING_STATE {
            Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: mask,
            StateMask: mask,
        };
        // SAFETY: the pseudo-handle is valid for the calling thread, `state` is a live local
        // of the structure this information class reads, and the size passed is its size.
        let ok = unsafe {
            SetThreadInformation(
                GetCurrentThread(),
                ThreadPowerThrottling,
                (&raw const state).cast(),
                size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
            )
        };
        // SAFETY: as in `set_priority`.
        if ok != 0 { Ok(()) } else { Err(unsafe { GetLastError() }) }
    }

    #[cfg(test)]
    mod tests {
        use windows_sys::Wdk::System::Threading::{NtQueryInformationThread, THREADINFOCLASS};
        use windows_sys::Win32::Foundation::{
            STATUS_INVALID_INFO_CLASS, STATUS_NOT_IMPLEMENTED, STATUS_NOT_SUPPORTED,
        };
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, THREAD_POWER_THROTTLING_CURRENT_VERSION,
            THREAD_POWER_THROTTLING_EXECUTION_SPEED, THREAD_POWER_THROTTLING_STATE,
        };

        use crate::ThreadClass;

        /// `ThreadPowerThrottlingState`, the kernel's class behind `ThreadPowerThrottling`,
        /// which `windows-sys` does not define. The number is phnt's, whose note says the
        /// kernel answers a query of it from Windows 11 22H2; System Informer reads a thread's
        /// state through it. `GetThreadInformation` is not used: Microsoft documents that it
        /// does not support querying this class.
        const POWER_THROTTLING_STATE_CLASS: THREADINFOCLASS = 49;

        const SET: (u32, u32) =
            (THREAD_POWER_THROTTLING_EXECUTION_SPEED, THREAD_POWER_THROTTLING_EXECUTION_SPEED);
        const SYSTEM_MANAGED: (u32, u32) = (0, 0);

        /// The calling thread's control mask and the state bits under it, or `None`, said on
        /// standard error, where the kernel refuses to answer the class. Any other failure
        /// means this call is wrong, and fails the test.
        fn throttling() -> Option<(u32, u32)> {
            let mut state = THREAD_POWER_THROTTLING_STATE {
                Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
                ..THREAD_POWER_THROTTLING_STATE::default()
            };
            // SAFETY: the pseudo-handle is valid for the calling thread, `state` is a live
            // local of the size passed, the most the call writes, and no returned length is
            // asked for.
            let status = unsafe {
                NtQueryInformationThread(
                    GetCurrentThread(),
                    POWER_THROTTLING_STATE_CLASS,
                    (&raw mut state).cast(),
                    size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
                    std::ptr::null_mut(),
                )
            };
            match status {
                // Microsoft documents a state bit as meaningful only under its control bit, so
                // the state bits outside the control mask are dropped: a kernel that reports
                // the effective state in them would otherwise fail a reset step.
                0 => Some((state.ControlMask, state.StateMask & state.ControlMask)),
                STATUS_INVALID_INFO_CLASS | STATUS_NOT_IMPLEMENTED | STATUS_NOT_SUPPORTED => {
                    eprintln!(
                        "the kernel refused to read the power-throttling state (status \
                         {status:#010x}): only the set calls are checked"
                    );
                    None
                }
                other => panic!("reading the power-throttling state failed: status {other:#010x}"),
            }
        }

        /// Each class but `Background`, on a fresh thread: the class, then `Background`, then
        /// the class again, each `enter` returning `Ok`.
        ///
        /// Where the kernel answers the read, the control mask and the state bits under it are
        /// asserted exactly after each step, which catches a missing throttling call, `Io`
        /// throttled, a missing reset, an opt-out for `Background` and a reset written as an
        /// opt-out. Where it refuses, the test says so on standard error (shown by
        /// `--no-capture`) and holds only that each set call succeeded, which catches none of
        /// those.
        #[test]
        #[cfg_attr(miri, ignore = "Miri has no shim for the thread information calls")]
        #[expect(
            clippy::disallowed_methods,
            reason = "each class is entered on a fresh thread, as a pool outside the boundary does"
        )]
        fn background_alone_opts_into_throttling_and_other_classes_reset_it() {
            for class in [
                ThreadClass::FrameCritical,
                ThreadClass::Network,
                ThreadClass::Present,
                ThreadClass::Audio,
                ThreadClass::PhysicsSolver,
                ThreadClass::Io,
            ] {
                std::thread::spawn(move || {
                    // `enter` reports a refused set call as an error.
                    let check = |entered: ThreadClass, masks: (u32, u32)| {
                        entered.enter().unwrap();
                        if let Some(read) = throttling() {
                            assert_eq!(read, masks, "{class:?} after {entered:?}");
                        }
                    };
                    check(class, SYSTEM_MANAGED);
                    check(ThreadClass::Background, SET);
                    check(class, SYSTEM_MANAGED);
                })
                .join()
                .unwrap();
            }
        }
    }
}

#[cfg(not(any(target_vendor = "apple", target_os = "linux", target_os = "android", windows)))]
mod os {
    use super::ThreadClass;
    use crate::error::Result;

    pub(super) fn set_scheduling(_class: ThreadClass) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg_attr(miri, ignore = "calls operating-system scheduling APIs")]
    #[expect(
        clippy::disallowed_methods,
        reason = "each class is entered on a fresh thread, as a pool outside the boundary does"
    )]
    fn every_class_can_be_entered() {
        for class in [
            ThreadClass::FrameCritical,
            ThreadClass::Network,
            ThreadClass::Present,
            ThreadClass::Audio,
            ThreadClass::PhysicsSolver,
            ThreadClass::Background,
            ThreadClass::Io,
        ] {
            std::thread::spawn(move || {
                class.enter().unwrap();
                assert!(fp::is_deterministic());
            })
            .join()
            .unwrap();
        }
    }
}
