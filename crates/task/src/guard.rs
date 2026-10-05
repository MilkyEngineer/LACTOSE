//! The abort-on-unwind guard (architecture section 5.1).

/// Runs `f` and aborts the process if it unwinds.
///
/// Every job and every scope wait runs under this. A job that unwound would leave its node's
/// counter undecremented and its scope's children borrowing a dead stack frame, so there is
/// no state to recover to. The panic message has already been printed by the panic hook when
/// the guard fires; binaries install a hook that also writes a crash dump.
pub(crate) fn abort_on_unwind<R>(f: impl FnOnce() -> R) -> R {
    struct Bomb;

    impl Drop for Bomb {
        fn drop(&mut self) {
            std::process::abort();
        }
    }

    let bomb = Bomb;
    let result = f();
    core::mem::forget(bomb);
    result
}
