# lactose-platform safety

This crate contains no `unsafe` and forbids it: `#![forbid(unsafe_code)]` is an attribute of
every crate root. Architecture section 21 lists `platform` for "present-thread QoS and
window handles", and neither is written yet: the crate is a facade, whose bodies are
`todo!()`, and its public surface is safe to call and stays so.

When the implementation lands, whatever `unsafe` it needs lives in one module of its own, and
the crate-level `forbid` becomes `#![deny(unsafe_code)]` with `#![allow(unsafe_code)]` on
that module alone, so the two pumps and the public surface stay provably safe. Every `unsafe`
block carries a `// SAFETY:` comment, and this file then carries the invariants they rely
on. The contract in `README.md` already fixes the ones a caller can see:

- **A lent handle is the handle of a live window.** The operating system's window is
  destroyed by the pump alone, on the main thread, and only after the last `Arc<Window>` has
  been dropped (`README.md`, "The window and its handles"). A handle is lent for the borrow
  of the `Window` it came from, so no handle outlives the window it names. The pump decides
  that the last other `Arc<Window>` is gone by `Arc::try_unwrap` or `Arc::into_inner` on
  its own `Arc`, which takes the count from one to none in one step, after which no `Weak`
  upgrades; never by `Arc::strong_count`, which a `Weak::upgrade` on another thread races,
  and never by `Arc::get_mut`. A panic that unwinds out of `run` leaves the window to the
  end of the process, as an error does, so no surface outlives it. If the
  implementation makes a `WindowHandle` or a `DisplayHandle` from a raw one, this is what
  makes the borrow sound; if it forwards the windowing library's own borrowed handles, there
  is no `unsafe` here and this entry says so.
- **A handle is lent on the main thread alone.** Each method compares the calling thread
  with the pump's before it touches the window, and returns `HandleError::Unavailable` on
  any other, so nothing that an operating system ties to its window thread is reached from
  elsewhere.
- **A thread's treatment is its own.** A call that sets a quality of service, a priority or
  a scheduling class is made by the thread it applies to, before its body runs, with no
  pointer the call keeps. `ThreadClass::enter` in `lactose-core` is safe to call and is what
  every thread gets; only what `ClientPlatform` adds for `ThreadClass::Present` could be
  `unsafe` here, and a refusal is a warning, never undefined behaviour.
