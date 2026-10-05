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
}

impl ThreadClass {
    /// Applies this class to the calling thread.
    ///
    /// The floating-point state is always set (see [`fp::set_deterministic`]). The scheduling
    /// class is best effort: an operating system that refuses it yields an error of kind
    /// `Platform`, and the thread still runs correctly at its default priority.
    ///
    /// Enter a class once, at thread start. On Linux the background class lowers priority
    /// in a way an unprivileged thread cannot undo, so a later `enter` of another class
    /// leaves it lowered. On Windows the class sets thread priority only; steering
    /// background threads to efficiency cores is an open item in `docs/verification.md`.
    pub fn enter(self) -> Result<()> {
        fp::set_deterministic();
        os::set_scheduling(self)
    }
}

/// Floating-point control state (MXCSR on x86-64, FPCR on aarch64).
///
/// The deterministic state is the IEEE default every thread starts with: round to nearest,
/// denormals honoured, no flush to zero, no default-NaN mode, exceptions masked. Engine
/// threads re-assert it because foreign code (audio drivers, plug-ins, some game overlays)
/// changes it and does not always put it back.
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
            ThreadClass::Background => libc::qos_class_t::QOS_CLASS_UTILITY,
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
        // Raising priority needs privileges a game does not have, so only the background
        // class is adjusted. On Linux a nice value applies to one thread when given its ID.
        if class != ThreadClass::Background {
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

#[cfg(windows)]
mod os {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
        THREAD_PRIORITY_BELOW_NORMAL, THREAD_PRIORITY_TIME_CRITICAL,
    };

    use super::ThreadClass;
    use crate::error::{Error, ErrorKind, Result};

    pub(super) fn set_scheduling(class: ThreadClass) -> Result<()> {
        let priority = match class {
            ThreadClass::Background => THREAD_PRIORITY_BELOW_NORMAL,
            ThreadClass::Audio => THREAD_PRIORITY_TIME_CRITICAL,
            ThreadClass::FrameCritical
            | ThreadClass::Network
            | ThreadClass::Present
            | ThreadClass::PhysicsSolver => THREAD_PRIORITY_ABOVE_NORMAL,
        };
        // SAFETY: `GetCurrentThread` returns a pseudo-handle that is always valid for the
        // calling thread, and `SetThreadPriority` takes plain values.
        let ok = unsafe { SetThreadPriority(GetCurrentThread(), priority) };
        if ok != 0 {
            Ok(())
        } else {
            let source = std::io::Error::last_os_error();
            Err(Error::new(ErrorKind::Platform, "setting thread priority failed")
                .with_source(source))
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
    fn every_class_can_be_entered() {
        for class in [
            ThreadClass::FrameCritical,
            ThreadClass::Network,
            ThreadClass::Present,
            ThreadClass::Audio,
            ThreadClass::PhysicsSolver,
            ThreadClass::Background,
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
