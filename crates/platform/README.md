# PAIL (`lactose-platform`)

The client's operating-system layer (architecture section 4, `platform`; section 5.1): the
window and the handles a graphics backend makes its surface from; the event pump that owns
the main thread and drains the main-thread lane; the present thread, which owns whatever
presents and runs every call that touches it; and the `Platform` a client binary gives its
executor. A client binary names this crate. `lactose-app`, the renderer and the graphics
backends reach what it makes through `lactose_task::Lane` and `raw-window-handle`'s traits,
and never name it.

| | |
|---|---|
| Tier | backend; on `[server_denylist]`, so no server binary links it |
| Determinism boundary | outside |
| Unsafe | none, `#![forbid(unsafe_code)]`; architecture section 21 lists the crate, and `SAFETY.md` says where the implementation's `unsafe` will live |

**Status: facade.** Every public signature is final and every body that decides anything is
`todo!()`; `PixelSize::is_empty`, `ClientPlatform::new` and the plain values run, and nothing
else does. This file is the contract the implementation lane (`platform-impl`) meets, and
every sentence under "Behaviour the signatures cannot express" is one a dependent crate may
rely on. Architecture sections 2, 3, 4, 5.1 and 21 are the design, with the milestone 2
plan's decisions 5.6 and 5.11 (`docs/plans/milestone-2.md`); where this file and the
architecture disagree, the architecture wins and this file is wrong. Decision D159 records
the choices those left open. `crates/task/README.md` (lanes, `Platform`, `block_on`) and
`crates/core/README.md` (errors, `ThreadClass`) are the contracts this crate builds on and
never restates. The window is handed to a graphics backend as `raw-window-handle` 0.6's two
traits, which is what `lactose-rhi-api` makes a surface from; this crate does not name
`lactose-rhi-api`, and no backend names this crate.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `lactose-app-client`, the `swapchain` lane's
tests and, through the lanes, `lactose-app` and the renderer are written against this
surface. `tests/consumers.rs` is a compile-only sketch of what each of them writes: a change
that breaks one is a signature change to list.

## Surface

| Item | Purpose |
|---|---|
| `event_pump` | Makes the process's one event pump and the main-thread lane, on the main thread, from the description of the window |
| `EventPump`, `EventPump::run` | The main thread's loop: opens the window, pumps the operating system's events, keeps the window's reports current, drains the main-thread lane, and returns when the process is done with the window |
| `MainThread`, `MainThread::{window, request_size, set_minimised, exit}` | What a main-lane call is lent: the window, two requests of it, and the end of the pump |
| `Window`, `Window::{size, close_requested}`, its `HasWindowHandle` and `HasDisplayHandle` | The window as any thread holds it: what a resize and a close report, and the handles a surface is made from, lent on the main thread alone |
| `WindowDesc`, `PixelSize`, `PixelSize::is_empty` | What the window is opened with, and a size in physical pixels |
| `present_thread`, `PresentThread`, `PresentThread::stop`, `PresentStop` | The present thread over a state of the caller's: the lane that carries every call to it, and the stop that hands the state back as a future |
| `ClientPlatform`, `ClientPlatform::new` | The `lactose_task::Platform` a client passes as `Workers::platform` |
| `LOG_TARGET` | The `tracing` target of everything the crate logs, `pail` |

There is no hidden plumbing: nothing here is `#[doc(hidden)]`. `Lane`, `LaneCall` and
`Platform` in these signatures are `lactose-task`'s and are not re-exported, and the two
handle traits are `raw-window-handle`'s.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Given by |
|---|---|---|
| `lactose-app-client` (`frame-impl`) | A `main` that gives its thread to the operating system and gets it back; a window to make the device from; the thread that presents; the platform for `Workers::platform`; a shutdown that cannot destroy the window under a surface | `event_pump`, `EventPump::run`; a main-lane call with `MainThread::window`; `present_thread`; `ClientPlatform`; `PresentThread::stop`, `MainThread::exit`, and `run` returning only once the window is let go ("Shutdown") |
| `lactose-app` (`frame-design`), which may not name this crate (decision 5.11) | To queue a present and learn when it returned, with no blocking wait; to pace the frame against the present thread; to learn of a resize and a close | `lactose_task::Lane<S>` over a state of its own or the renderer's, handed to it by the binary; `LaneCall` resolving when the call returned ("The present thread and its pump"); `Window::size` and `Window::close_requested`, which the binary adapts to whatever `app` declares, two reads with no queue to drain |
| `lactose-renderer`, or whichever crate owns what presents | The swapchain and the pass that draws a finished target into its image on one thread, without naming this crate | The state `S` of `present_thread` is any `Send + 'static` type; a call is lent `&mut S` on the present thread and nowhere else |
| `lactose-rhi-vulkan` and every graphics backend (`rhi-api`) | A window without naming this crate; which thread reads the handles; a window that outlives every use of them; a window thread that answers while a swapchain is made, resized or destroyed | `Window` is `HasWindowHandle + HasDisplayHandle + Send + Sync + 'static`; the handles are lent inside a main-lane call, where the client calls the backend's constructor; the pump destroys the window only after the last `Arc<Window>` is dropped; the main thread never waits for another thread ("Threads") |
| The `swapchain` lane's tests (this crate's `tests/`, as a subprocess) | A window and a present thread in a process of their own; a resize it can ask for; a drawable it can make block; a block it can time; a shutdown with frames in flight | `event_pump` and `present_thread` from the subprocess's `main`; `MainThread::request_size`, `MainThread::set_minimised`; a call that returns how long it took; `PresentThread::stop`, ordered behind every call queued before it |
| `platform-impl` | A contract for every body; where its `unsafe` goes; what it may choose | "Behaviour the signatures cannot express"; `SAFETY.md`; the windowing library, its pin and its features are its own, and no signature names them |
| `lactose-task` | Its two sentences on the lanes' owners kept: the platform thread owns the main-thread pump and drains it from its event loop, the present thread owns its own, and dedicated threads are made through the executor's platform | `EventPump` holds the `LanePump<MainThread>`; the present thread makes its `LanePump<S>` itself; `present_thread` takes `&dyn Platform` |
| A profiler | The two pumps' work by name | `lactose-task`'s `lane` span carries `name = "main"` and `name = "present"` |
| `lactose-input-host`'s device sources, action mapping, `lactose-debug-ui` (milestone 5) | Keys, pointers, pads and text as they arrive | Not at milestone 2: "Not in this crate yet" |

## A client binary, by thread

| Thread | Made by | Runs | Never |
|---|---|---|---|
| The main thread | The operating system | `event_pump`, the setup that follows it, then `EventPump::run`: the operating system's events, the window's reports, the main-lane calls | Waits for another thread; runs a tick, a frame or a present |
| The present thread | `present_thread`, through the `Platform` it is given | The calls of its lane, one at a time, each lent the state; a call may block in an acquire or a present | Is waited for by the main thread or by a worker |
| The workers | The executor, through `ClientPlatform` | Everything else, as tasks: they hold the two lanes and the window's `Arc`, and wait for a call with `.await` | Block on either thread |

```rust
// `main`, on the main thread.
let (main, pump) = event_pump(WindowDesc { title, size })?;
let platform: Arc<dyn Platform> = Arc::new(ClientPlatform::new());
let executor = Executor::builder().platform(platform.clone()).build()?;
let (present, thread) = present_thread(&*platform, Presenter::empty())?;

let root = executor.spawn(WorkerClass::FrameCritical, async move {
    // A task. The device is made on the main thread, where the window lends its handles.
    let made = main.run(|main| {
        let window = main.window();
        let device = create_device(DeviceRequest { surface: Some(window.clone()), .. })?;
        Ok((device, window))
    });
    let ran = match made.await {
        Ok(Ok((device, window))) => frames(&device, &window, &present).await,
        Ok(Err(error)) | Err(error) => Err(error),
    };
    // On every path: the present thread's state comes back, what was made from the window
    // is destroyed on the main thread, and the pump is told to go.
    let presenter = thread.stop().await;
    let left = main.run(move |main| { drop(presenter); main.exit(); }).await;
    ran.and(left)
});

pump.run()?;                    // until `exit`, and until the window is let go
let ran = executor.block_on(root);
executor.shutdown();
```

The crate's documentation carries this as a doctest, and `tests/consumers.rs` in full.

## Behaviour the signatures cannot express

**Threads** (architecture sections 2, 3 and 5.1; D18, D90)

- The main thread is the process's main thread, on every operating system. `event_pump`
  panics on any other, and `EventPump` is neither `Send` nor `Sync`, so the thread that made
  the pump runs it. A test harness's thread is not the main thread: a test that opens a
  window runs a subprocess whose `main` is the test (the milestone 2 plan's rows
  `platform-impl` and `swapchain`).
- The main thread pumps the operating system's events and runs main-lane calls, and does
  nothing else from `run`'s start to its return. `run` never waits for another thread: not
  for a worker, not for the present thread, not for a lock another thread holds across a
  wait. Between events it waits in the operating system, and it does not spin.
- A main-lane call is the caller's code on the main thread, and keeps the same rule: it
  never waits for another thread. It joins nothing, calls no `Executor::block_on`, and
  receives from no channel. Every event and every other call waits behind it. A graphics
  driver may also need the window's thread to answer while another thread makes, resizes
  or destroys a swapchain; the main thread answers only while it is not waiting.
- Workers never wait synchronously on either thread. A lane call is a future, and
  `lactose-task` has one blocking wait, `Executor::block_on`, which asserts on a worker in
  debug builds. This crate adds none.
- The present thread is the one thread a call may block on: that is what it is for
  (section 5.1). It may wait for anything but itself.
- `Window` is `Send + Sync`. `Lane<MainThread>` is `Send + Sync`, though a `MainThread`
  never leaves the main thread. `PresentThread<S>` and `PresentStop<S>` are `Send` for
  every `S` the function accepts, so a task holds them across an `.await`. `src/lib.rs`
  asserts each of these, and that `EventPump` and `MainThread` are neither `Send` nor
  `Sync`.
- This crate takes no lock of its own on per-frame work. The window's reports are atomic
  reads. The two pumps are `lactose-task`'s lanes, whose queue and whose reply slots are
  locks: a call takes the queue's once to be queued and its reply's once to be answered,
  each held for a push, a take or a store and never across a call. A frame crosses them
  once for each call it queues to the present thread, which architecture section 3
  permits as D159's amendment words it. What this crate locks beside them it locks in
  setup and teardown.

**The main thread and its pump** (architecture section 5.1)

- `event_pump` makes the operating system's event loop and shows nothing. A process has
  one pump: a second call returns `Unsupported`, whether or not the first has finished.
  Where the operating system gives the process no window system, it returns `Platform`.
- `run` opens the window before it drains a call. If the operating system refuses the
  window, `run` returns `Platform` at once and no call has run. Otherwise every call sees
  the window: `MainThread::window` neither fails nor waits.
- A call queued before `run` waits for it, and runs when the window is open. Calls run in
  the order `lactose-task`'s lanes give, one at a time, each lent the one `MainThread`.
  The lane is named `main`, which is the `name` its `lane` spans carry.
- A queued call wakes the pump through the operating system, and the pump drains the lane
  when it next runs. Nothing bounds how soon that is: the operating system may hold the
  main thread, as it does while a menu is open or a window is dragged. So nothing a frame
  needs goes through the main lane. What goes through it is the device's creation and its
  destruction, a request of the window, and `exit`.
- A panic in a main-lane call is not caught. A binary aborts (section 5.1); in a test's
  subprocess the panic reaches the operating system's event loop, which may abort too.
- When `run` returns, with a value or an error, every call still queued and every call
  queued later resolves to `Cancelled`. An `EventPump` dropped before `run` has opened no
  window, and its lane's calls resolve to `Cancelled`.
- An error from `run` is `Platform`: the window was refused, or the operating system's
  event loop failed. `lactose-app`'s policy for it is the binary's to choose; nothing can
  be shown after it.

**The window and its handles** (architecture section 4; decision 5.6)

- A process has one window. `run` opens it visible, with `WindowDesc::title`, resizable by
  the user, at about `WindowDesc::size`: the operating system may give another size (its
  minimum, the screen's bounds, whole points on a scaled display), and `Window::size` is
  what it gave.
- The window is reached as an `Arc<Window>`, from `MainThread::window` and from clones of
  what it returned. Every one of them is the one window.
- The operating system's window lives for as long as any such `Arc` does. The pump
  destroys it on the main thread, inside `run`, once `exit` has been called and the last
  of them has been dropped, and at no other time and on no other thread. If `run` returns
  an error while one is still held, the window is never destroyed, and its handles stay
  valid until the process ends. A `Weak<Window>` keeps nothing open, and upgrades to
  nothing once the window is destroyed.
- So whoever keeps the `Arc` for as long as it uses the handles, as a backend keeps it
  until its surface is destroyed, has a live window at every use, and the pump waits for
  it.
- The handles are `raw-window-handle` 0.6's, through `HasWindowHandle` and
  `HasDisplayHandle`; the workspace pins the crate at 0.6.2. They are lent on the main
  thread alone, which is inside a main-lane call. On any other thread each method returns
  `HandleError::Unavailable`, on every operating system, including those whose handles
  could be read elsewhere: a client is written one way and fails the same way everywhere.
  A backend's constructor reads them on the thread that calls it, so the client calls
  that constructor inside a main-lane call, which holds the main thread for as long as
  the constructor takes, once, at start.
- A lent handle is valid for the borrow of the window it was lent from, and every call
  lends the handle of the same window and the same display.
- `Window` meets the bounds `lactose-rhi-api` asks of a surface source, `HasWindowHandle +
  HasDisplayHandle + Send + Sync`, and is `'static`, so an `Arc<Window>` is handed to a
  device request as it is. `src/lib.rs` asserts the bounds.

**What a resize reports**

- `Window::size` is the size of the window's drawable area in physical pixels, which is
  what a swapchain's extent is asked as. It is the size the operating system gave when the
  window opened, and then the size of the newest resize the operating system has delivered
  to the main thread, whatever caused it: the user, `request_size`, a display's scale, a
  minimise.
- That is the whole report. There is no event to take and no callback, and this crate adds
  no delay and no coalescing of its own: the value changes when the operating system
  delivers the resize, and a reader sees the newest size and may never see one between two
  of its reads.
- A read returns one whole size, never the width of one resize with the height of another,
  and never waits.
- A size with a zero dimension is reported as it is, which is what Windows gives a
  minimised window. Whether an operating system changes a minimised window's size is its
  own, and the report follows it. `PixelSize::is_empty` is true for such a size, and
  nothing is presented at it: no swapchain is made at it or resized to it.
- The report and a swapchain's own word that it is stale are two reports of one change
  and arrive in either order. A reader decides a resize against the size it last asked
  for, never against the extent the surface gave, which may differ from what was asked
  and stay different.

**What a close reports**

- `Window::close_requested` is `false` until the operating system first asks for the
  window to be closed and `true` from then on. The request is whatever the operating
  system delivers to the window as one: its close button, Alt+F4, a window menu's Close.
- Nothing else happens. The window stays open, the pump runs on, and nothing in this
  crate ends the process. The client reads the report, winds down and calls
  `MainThread::exit`.
- What an operating system addresses to the application and not to the window is not a
  close request and is not reported at milestone 2: macOS's Quit, from the Dock or from a
  menu that offers it, and a session's end. The operating system ends the process as it
  ends any process, and nothing here runs.

**Main-thread requests**

- `request_size` and `set_minimised` ask the operating system and return. Neither waits
  for the answer, and a request the operating system refuses or changes is not an error:
  the answer to `request_size` is a resize report or none, and a size it gives may be
  another than the one asked for.
- `exit` says the process is done with the window. The pump runs on as before until the
  window is let go ("Shutdown"). A second `exit` does nothing, and `exit` is never undone.

**The present thread and its pump** (architecture section 5.1; decision 5.6, D153)

- `present_thread` makes one thread through the `Platform` it is given, with the spec
  `name: "present"`, `class: ThreadClass::Present`, `index_in_class: 0`, and moves `state`
  onto it. The platform enters the class before the thread runs anything. The call returns
  once the thread has started: it is a setup call, for `main` and for a test, and never
  for a worker or a task.
- The state is reached through the lane alone: a call is lent `&mut S` on the present
  thread, and nothing else holds it until `stop` hands it back. So a value that is `Send`
  and not `Sync`, as `lactose-rhi-api`'s swapchain is, is used by the present thread alone
  once it is in the state. That is how every submit that touches a swapchain image comes
  to be the present thread's (decision 5.6): by the type of what the caller puts in the
  state, with no check here.
- The thread runs the lane's calls one at a time, in the order `lactose-task`'s lanes
  give, and parks between them. It has no clock, it runs nothing but the calls, and it
  skips, merges and reorders none. The lane is named `present`.
- A call may block for as long as an acquire or a present does. It holds the calls queued
  behind it and nothing else: not the main thread, not a worker, and not its caller, which
  holds a future.
- The lane has no bound. A caller that queues a call for every frame while one blocks
  adds a call a frame, and each of them then runs. The frame therefore queues its next
  present when its last has returned, one in flight, and chooses the newest finished
  target when it queues; that rule is the frame's to keep (`frame-design`).
- `PresentThread::stop` never waits. The thread runs every call that was queued before
  `stop` was called, the one in progress included, then hands the state back and exits. A
  call queued after `stop` was called either runs before the hand-back or resolves to
  `Cancelled`; a call queued after the hand-back resolves to `Cancelled`.
- `PresentStop` resolves to the state once the thread has run its last call. From then on
  the present thread runs nothing of the caller's and holds nothing of the caller's; the
  operating system's thread ends by itself and is not joined. Dropping a `PresentStop`
  does not undo the stop: the state is dropped where the future was, or on the present
  thread.
- Dropping a `PresentThread` stops the thread as `stop` does, and the state is dropped on
  the present thread.
- A panic in a call is not caught. A binary aborts (section 5.1); in a test the present
  thread ends, the call that panicked and every call not yet run resolve to `Cancelled`,
  and so does the `PresentStop`.
- If the platform makes no thread, `present_thread` returns its error and `state` is
  dropped on the calling thread.
- The engine makes one present thread in a process. A second call is not refused: it
  makes a second thread with the same spec.

**Shutdown** (the milestone 2 plan's row `swapchain`: a shutdown with frames in flight)

- The order a client keeps: it stops queuing presents; it awaits `PresentThread::stop`,
  which runs what was queued and hands the state back; it drops what was made from the
  window, the swapchain before the device; it calls `exit`; `run` returns; `main` waits
  for its root task and shuts the executor down.
- What was made from the window may be dropped on any thread while the pump runs. The
  sketch drops it inside the main-lane call that then calls `exit`, where a driver that
  needs the window's thread is already on it and nothing is left to order.
- `run` returns once `exit` has been called and the window is let go: every `Arc<Window>`
  that `MainThread::window` returned, and every clone, has been dropped. Until both hold
  it pumps events and drains calls as before, and a later `MainThread::window` gives the
  window again and is waited for again. It does not wait for an operating-system event to
  see the last drop: from `exit` until the window is let go it wakes on a timer of its
  own, the one time it does.
- So when `run` returns nothing made from the window is alive, provided a backend holds
  its `Arc` for as long as its surface, and everything that was made from the window was
  destroyed while the main thread could answer.
- A client calls `exit` on every path, a failed start included. A pump that is never told
  to exit, or whose window is never let go, never returns. While it waits after `exit` it
  logs a warning on `pail` once a second with the number of `Arc`s still held.
- This crate waits for no present and no submit. What a swapchain asks before it is
  resized or destroyed, the caller does in its last calls or with the state it took back.

**`ClientPlatform`** (architecture section 5.1; `crates/task/README.md`, "Workers")

- `topology` and `hardware_threads` return what `StdPlatform`'s return in the same
  process, so a client's worker counts are the headless binary's on the same machine.
- `spawn_thread` makes a thread as `StdPlatform` does: named `spec.name`, in `spec.class`
  through `ThreadClass::enter` before `body` runs, with its errors. For
  `ThreadClass::Present` it also applies what the operating system offers a thread that
  presents, where `platform-impl` finds one worth applying; `SAFETY.md` then records it,
  and no caller relies on it. A class or a treatment the operating system refuses is a
  warning on `pail`, and the thread runs.
- A value holds nothing. It is made on any thread, before or after the pump, and any
  number of them behave alike.
- The worker counts are `lactose-task`'s, from that topology: its frame-critical workers
  stop one short of the usable hardware threads, and the main thread shares the last with
  the background workers (`crates/task/README.md`, "Workers"). No hardware thread is kept
  for the present thread, which waits.

**Errors** (architecture section 19)

| Kind | When |
|---|---|
| `Unsupported` | `event_pump`, a second time in one process |
| `Platform` | `event_pump`, where there is no window system to reach; `EventPump::run`, if the window is refused or the event loop fails |
| `Cancelled` | A lane call that will not run: the pump was dropped or `run` returned; the present thread handed its state back, or ended. A `PresentStop` whose thread ended without handing back |
| As `StdPlatform` reports it | `ClientPlatform::spawn_thread` and `present_thread`, if the operating system makes no thread |

No other kind comes from this crate, and no request of the window is an error.

## Not in this crate yet

- **Device sources and action mapping**: keys, pointers, pads, text, the cursor's capture,
  and the stream of events they need. Milestone 5's, with `lactose-input-host` and
  `lactose-debug-ui`, whose tier exception to read window events is already written. At
  milestone 2 a close and a resize are states, and they stay states when a stream arrives.
- **Declining a close, and an application's quit.** `close_requested` never returns to
  `false`, and macOS's Quit is not intercepted. Both arrive with the game's menus, at
  milestone 5.
- **Fullscreen and borderless modes, displays and their refresh, a display's scale, focus
  and occlusion, more than one window.** No milestone 2 row reads one. `frame-design` names
  a display's refresh here if the frame's pacing needs it.
- **The windowing library.** `winit`, its version, its features and its vendor check are
  `platform-impl`'s. No signature names it, and `Cargo.lock` does not hold it yet.
- **The tests with a device.** `swapchain` writes them here, with `lactose-rhi-vulkan` as
  a test-support dev-dependency, as subprocesses.
- **The crash hook** is `lactose-app`'s (D155), not this crate's.

## Proof

The facade proves what the compiler can. `src/lib.rs` asserts the auto traits under
"Threads" and the surface-source bounds; `tests/consumers.rs` type-checks what each consumer
writes, with stand-ins for `lactose-rhi-api`'s items that carry their bounds; `tests/values.rs`
runs the plain values; the crate's doctest type-checks a client's `main`.

`platform-impl` shows the rest, each as a named test. A test that needs a window is a
subprocess whose `main` is the test, on the M1 and on the laptop.

- A window opens, events are pumped, one call runs on the main thread and one on the
  present thread, and the process closes: `run` returns and the exit status is zero.
- A call queued before `run` runs after the window opens; a call queued after `run`
  returned is `Cancelled`; a dropped pump cancels its calls and opens nothing.
- `event_pump` on a second thread panics; a second `event_pump` is `Unsupported`.
- The handles are lent inside a main-lane call and are `Unavailable` on every other thread.
- `request_size` is answered by `Window::size`; a minimised window on Windows reports
  empty; a read on another thread never sees a torn size, under a writer that alternates
  two sizes.
- `close_requested` turns `true` on a close the test sends the window, and the window
  stays open.
- `run` does not return after `exit` while an `Arc<Window>` is held, warns on `pail`, and
  returns when the last is dropped on another thread.
- A worker that waits on the main thread fails: `Executor::block_on` of a main-lane call
  on a worker is `lactose-task`'s debug assertion, and the subprocess aborts.
- The present thread has the spec above, through a spy `Platform`; a call that blocks
  holds the calls behind it and no other thread; `stop` runs every call queued before it
  and hands the state back; a call queued after the hand-back is `Cancelled`; a dropped
  `PresentThread` drops the state on the present thread.
- `ClientPlatform` reports `StdPlatform`'s topology and hardware threads, and enters the
  class of each thread it makes.
- Clippy passes for `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu` from the M1.
