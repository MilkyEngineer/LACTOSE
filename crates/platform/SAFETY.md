# lactose-platform safety

The library contains no `unsafe` and forbids it: `#![forbid(unsafe_code)]` is an attribute of
its root. Architecture section 21 lists `platform` for "present-thread QoS and window
handles", and neither needs `unsafe` here: the window's handles are winit's own borrowed
handles, forwarded, and the present thread gets no treatment beyond `ThreadClass::enter`,
which `lactose-core` makes safe to call. The public surface is safe to call.

The invariants the contract in `README.md` fixes, and what holds each without `unsafe`:

- **A lent handle is the handle of a live window.** The operating system's window is
  destroyed by the pump alone, on the main thread, and only after the last `Arc<Window>` has
  been dropped (`README.md`, "The window and its handles"). A handle is lent for the borrow
  of the `Window` it came from, so no handle outlives the window it names. The pump decides
  that the last other `Arc<Window>` is gone by `Arc::try_unwrap` on its own `Arc`, which
  takes the count from one to none in one step, after which no `Weak` upgrades; never by
  `Arc::into_inner`, which drops the pump's `Arc` when it fails; never by
  `Arc::strong_count`, which a `Weak::upgrade` on another thread races, and which it reads
  for its warning alone; and never by `Arc::get_mut`. A panic that unwinds out of `run`
  leaves the window to the end of the process, as an error does (`mem::forget` of the
  pump's `Arc`), so no surface outlives it. The handles are winit 0.30's
  `HasWindowHandle` and `HasDisplayHandle` of its `Window`, borrowed from the field the
  `Window` holds: no handle is made from a raw one, so there is no `unsafe` here.
- **A handle is lent on the main thread alone.** Each method compares the calling thread
  with the pump's before it touches the window, and returns `HandleError::Unavailable` on
  any other, so nothing that an operating system ties to its window thread is reached from
  elsewhere.
- **A thread's treatment is its own.** A call that sets a quality of service, a priority or
  a scheduling class is made by the thread it applies to, before its body runs, with no
  pointer the call keeps. `ClientPlatform` makes every thread enter its class through
  `ThreadClass::enter`, which gives `ThreadClass::Present` the user-interactive quality of
  service on macOS and the above-normal priority on Windows, and adds nothing: Windows'
  multimedia class scheduler (`AvSetMmThreadCharacteristicsW`), the one further treatment
  either offers an unprivileged thread, would be the first `unsafe` here and a new
  dependency, and no milestone 2 measurement asks for it. Applying one moves this crate to
  `#![deny(unsafe_code)]` with `#![allow(unsafe_code)]` on one module of its own, and this
  entry then carries its invariant.

## In the tests

`tests/window.rs` does to a window what a user does, and reads back what the operating
system gave a thread, through one module, `os`: the binary is `#![deny(unsafe_code)]` and
that module alone allows it. Every block carries a `// SAFETY:` comment, and each relies on
the window's handle being lent, which is on the main thread inside a main-lane call, for
the borrow of a `Window` the call holds:

- macOS: the content view of winit's handle as an `NSView`, its window's `performClose:`,
  which a close button sends; `NSApplication`'s `stop:` and the application-defined event
  that makes it take effect, as winit's own stop posts; `pthread_get_qos_class_np` of the
  calling thread into two locals.
- Windows: `PostMessageW` of `WM_CLOSE` to the window's handle, `IsWindowVisible` of it,
  and `GetThreadPriority` of the calling thread's pseudo-handle.
