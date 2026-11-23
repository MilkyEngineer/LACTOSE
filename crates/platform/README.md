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
| Unsafe | none, `#![forbid(unsafe_code)]`; architecture section 21 lists the crate, and `SAFETY.md` says why none is needed and where the tests' is |

**Status: implemented** over winit 0.30 on macOS and Windows (`platform-impl`). On Linux it
builds over X11 and is not run, save the check that `event_pump` panics off the main thread.
This file is the contract the implementation meets, and every sentence under "Behaviour the signatures cannot express" is one a dependent crate may
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

The windowing library is winit 0.30.13, pinned exactly in the root `Cargo.toml` with its
default features off and `rwh_06` on, and on Linux `x11` as well, whose libraries it loads at
run time, so the crate builds and lints where no window system's libraries are installed. No
signature names it. It is outside the determinism boundary and carries no patch, so
`cargo xtask vendor` has nothing of it to check.

## Surface

| Item | Purpose |
|---|---|
| `event_pump` | Makes the process's one event pump and the main-thread lane, on the main thread, from the description of the window |
| `EventPump`, `EventPump::run` | The main thread's loop: opens the window, pumps the operating system's events, keeps the window's reports current, drains the main-thread lane, and returns when the process is done with the window |
| `MainThread`, `MainThread::{window, request_size, set_minimised, is_minimised, exit}` | What a main-lane call is lent: the window, two requests of it and what the operating system reports of the second, and the end of the pump |
| `Window`, `Window::{size, close_requested, reports}`, its `HasWindowHandle` and `HasDisplayHandle` | The window as any thread holds it: what a resize and a close report, and the handles a surface is made from, lent on the main thread alone |
| `WindowReports`, `WindowReports::{size, close_requested}` | The window's two reports in a handle that keeps nothing open, for whatever reads them and does not use the handles |
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
| `lactose-app-client` (`frame-impl`) | A `main` that gives its thread to the operating system and gets it back; a window to make the device from; the thread that presents; the platform for `Workers::platform`; a shutdown that cannot destroy the window under a surface, and that cannot hang without a warning | `event_pump`, `EventPump::run`; a main-lane call with `MainThread::window`; `present_thread`; `ClientPlatform`; `MainThread::exit` queued first, `PresentThread::stop`, and `run` returning only once the window is let go ("Shutdown") |
| `lactose-app` (`frame-design`), which may not name this crate (decision 5.11) | To queue a present and learn when it returned, with no blocking wait; to keep at most one present in flight; to learn of a resize and a close without keeping the window open | `lactose_task::Lane<S>` over a state of its own, handed to it by the binary; `LaneCall` resolving when the call returned ("The present thread and its pump"); `WindowReports`, which the binary adapts to whatever `app` declares, two reads with no queue to drain and no `Arc<Window>` held |
| `lactose-renderer`, or whichever crate owns what presents | The swapchain and the pass that draws a finished target into its image on one thread, without naming this crate | The state `S` of `present_thread` is any `Send + 'static` type; a call is lent `&mut S` on the present thread and nowhere else |
| `lactose-rhi-vulkan` and every graphics backend (`rhi-api`) | A window without naming this crate; which thread reads the handles; a window that outlives every use of them; a window thread that answers while a swapchain is made, resized or destroyed | `Window` is `HasWindowHandle + HasDisplayHandle + Send + Sync + 'static`; the handles are lent inside a main-lane call, where the client calls the backend's constructor; the pump destroys the window only after the last `Arc<Window>` is dropped; the main thread never waits for another thread while it pumps ("Threads") |
| The `swapchain` lane's tests (this crate's `tests/`, as a subprocess) | A window and a present thread in a process of their own; a resize it can ask for; a drawable it can make block; a block it can time; a shutdown with frames in flight | `event_pump` and `present_thread` from the subprocess's `main`; `MainThread::request_size`, `MainThread::set_minimised` and `MainThread::is_minimised`, which tells a refused minimise from a drawable that does not block; a call that returns how long it took; `PresentThread::stop`, ordered behind every call queued before it |
| `platform-impl` | A contract for every body; where its `unsafe` goes; what it may choose | "Behaviour the signatures cannot express"; `SAFETY.md`; the windowing library, its pin and its features are its own, and no signature names them |
| `lactose-task` | Its two sentences on the lanes' owners kept: the platform thread owns the main-thread pump and drains it from its event loop, the present thread owns its own, and dedicated threads are made through the executor's platform | `EventPump` holds the `LanePump<MainThread>`; the present thread makes its `LanePump<S>` itself; `present_thread` takes `&dyn Platform` |
| A profiler | The two pumps' work by name | `lactose-task`'s `lane` span carries `name = "main"` and `name = "present"` |
| `lactose-input-host`'s device sources, action mapping, `lactose-debug-ui` (milestone 5) | Keys, pointers, pads and text as they arrive | Not at milestone 2: "Not in this crate yet" |

## A client binary, by thread

| Thread | Made by | Runs | Never |
|---|---|---|---|
| The main thread | The operating system | `event_pump`, the setup that follows it, then `EventPump::run`: the operating system's events, the window's reports, the main-lane calls; after an error from `run`, a bounded wait for the root | Waits for another thread while it pumps; runs a tick, a frame or a present |
| The present thread | `present_thread`, through the `Platform` it is given | The calls of its lane, one at a time, each lent the state; a call may block in an acquire or a present | Is waited for by the main thread or by a worker |
| A shutdown's own threads | The client, through `Executor::platform` | Step 4 of "Shutdown", which may block; after an error from `run`, the root's `block_on` | Is waited for by a worker |
| The workers | The executor, through `ClientPlatform` | Everything else, as tasks: they hold the two lanes, the window's reports and, until shutdown, what was made from the window, and wait for a call with `.await` | Block on either thread |

```rust
// `main`, on the main thread.
let (main, pump) = event_pump(WindowDesc { title, size })?;
let platform: Arc<dyn Platform> = Arc::new(ClientPlatform::new());
let executor = Executor::builder().platform(platform.clone()).build()?;
let (present, thread) = present_thread(&*platform, Presenter::empty())?;

let root = executor.spawn(WorkerClass::FrameCritical, async move {
    let exit = ExitOnDrop(main.clone());   // its `Drop` queues `exit`, unwinding included
    // A task. The device is made on the main thread, where the window lends its handles;
    // the frame keeps the reports, which hold nothing open.
    let made = main.run(|main| {
        let window = main.window();
        let reports = window.reports();
        let device = create_device(DeviceRequest { surface: Some(window), .. })?;
        Ok((device, reports))
    });
    let (device, ran) = match made.await {
        Ok(Ok((device, reports))) => {
            (Some(device.clone()), frames(device, &reports, &present).await)
        }
        Ok(Err(error)) | Err(error) => (None, Err(error)),
    };
    // On every path, in the order of "Shutdown".
    drop(exit);                                          // 1. `exit`, not awaited
    signal_unsignalled_points();                         // 2. no submit will signal them;
    let last = present.run(Presenter::wind_down).await;  //    bounded waits; the swapchain
    let presenter = thread.stop().await;                 // 3. nothing of the window in it
    let settled = match device {                         // 4. a thread of its own, bounded:
        Some(device) => settle(&executor, device).await, //    the device back, or dropped
        None => Ok(None),                                //    there if the bound expired
    };
    let dropped = match settled {                        // 5. the last `Device`
        Ok(Some(device)) => main.run(move |_| drop(device)).await, // waits for nothing
        Ok(None) => Ok(()),
        Err(error) => Err(error),
    };
    ran.and(last).and(presenter.map(drop)).and(dropped)
});

if let Err(error) = pump.run() {  // 6. until `exit`, and until the window is let go
    // No pump now: the steps' main-lane calls resolve to `Cancelled`, and the root is
    // awaited on a thread of its own within a bound longer than steps 2 and 4's together,
    // past which `main` logs and returns. A thread that is never made leaks the root.
    let root = on_a_thread_of_its_own(&executor, move |executor| executor.block_on(root));
    match root.recv_timeout(3 * BOUND) {
        Ok(_) => executor.shutdown(),
        Err(_) => tracing::warn!(target: LOG_TARGET, "the root outlived `run`'s error"),
    }
    return Err(error);
}
let ran = executor.block_on(root);
executor.shutdown();
ran?
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
  driver may also need the window's thread to answer another thread's call: MoltenVK reads
  a display's refresh (`vkGetRefreshCycleDurationGOOGLE`) by a synchronous dispatch to the
  main thread, and a Windows driver may send the window a message while a swapchain is
  made, resized or destroyed; the main thread answers only while it is not waiting.
- Workers never wait synchronously on either thread. A lane call is a future, and
  `lactose-task` has one blocking wait, `Executor::block_on`, which asserts on a worker in
  debug builds. This crate adds none.
- The present thread is the one thread a call may block on: that is what it is for
  (section 5.1). It may wait for anything but itself.
- `Window` is `Send + Sync`, and `WindowReports` is `Clone + Send + Sync`.
  `Lane<MainThread>` is `Send + Sync`, though a `MainThread`
  never leaves the main thread. `PresentThread<S>` and `PresentStop<S>` are `Send` for
  every `S` the function accepts, so a task holds them across an `.await`. `src/lib.rs`
  asserts each of these, and that `EventPump` and `MainThread` are neither `Send` nor
  `Sync`.
- This crate takes no lock of its own on per-frame work. The window's reports are atomic
  reads. The two pumps are `lactose-task`'s lanes, which have two locks
  (`crates/task/README.md`, "Lanes"): the lane's queue, held to push a call and to take
  what is queued, and each call's reply, held to store or take its result or the awaiting
  task's waker. Neither is held while a call runs or while a waker is woken. A frame
  crosses both for each call it queues to the present thread, and keeps at most one such
  call in flight and queues at most one a frame, which architecture section 3 permits as
  D159's amendment words it. A call to the main thread crosses the same two and is never
  per-frame work. What this crate locks beside them it locks in setup and teardown.

**The main thread and its pump** (architecture section 5.1)

- `event_pump` makes the operating system's event loop and shows nothing. A process has
  one pump: a second call returns `Unsupported`, whether or not the first has finished. A
  first call that panicked, off the main thread, was the process's one call.
  Where the operating system gives the process no window system, it returns `Platform`.
- `run` opens the window before it drains a call. If the operating system refuses the
  window, `run` returns `Platform` at once and no call has run. Otherwise every call sees
  the window: `MainThread::window` neither fails nor waits.
- A call queued before `run` waits for it, and runs when the window is open. Calls run in
  the order `lactose-task`'s lanes give, one at a time, each lent the one `MainThread`.
  The lane is named `main`, which is the `name` its `lane` spans carry.
- A queued call wakes the pump through the operating system, and the pump drains the lane
  when it next runs. With winit 0.30 the wake is its event-loop proxy, which is
  `Send + Sync` for a `Send` payload, so it takes no lock of this crate's. Nothing bounds
  how soon that is: the operating system may hold the main thread, as it does while a
  menu is open or a window is dragged. So nothing a frame needs goes through the main
  lane. What goes through it is the device's creation, the drop of its last holder at
  shutdown, a request of the window, and `exit`.
- A panic in a main-lane call is not caught. A binary aborts (section 5.1); in a test's
  subprocess the panic reaches the operating system's event loop, which may abort too or
  resume the panic after its loop ends. A panic that unwinds out of `run` leaves the
  window to the end of the process, as an error does, so a surface another thread holds
  stays valid and the test fails with its panic's message.
- When `run` returns, with a value or an error, `Window::close_requested` turns `true`,
  and every call still queued and every call queued later resolves to `Cancelled`. A frame
  that loops until a close therefore ends after a failed `run` too. `run` turns
  `close_requested` true before it drops its pump, which closes the lane. A call the close
  cancels has its reply dropped as the pump drops, which releases the call's slot lock,
  and the `LaneCall`'s poll acquires that lock. A call queued after the close finds the
  lane closed under the queue's lock, which the pump's drop released after it marked the
  lane closed, and is cancelled on the thread that queued it. So in a binary, a task whose
  main-lane call came back `Cancelled` reads the close, and so does the waker the cancel
  wakes. In a test, a main-lane call that panics drops the rest of its batch as it
  unwinds, before the close is set, so those calls' tasks may read it `false`. The lane's
  wake ignores an event loop that has ended (winit's `EventLoopClosed`), even after the
  lane is closed, since `Lane::run` wakes after it releases the queue: `exit` is queued
  from destructors, unwinding included, where a panic in the wake would abort. An
  `EventPump` dropped before `run` has opened no window, and its lane's calls resolve to
  `Cancelled`.
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
  of them has been dropped, and at no other time and on no other thread. The pump keeps
  an `Arc` of its own and decides that the last other one is gone by taking the window
  back with `Arc::try_unwrap` on it, which leaves no `Weak` able to upgrade; never by
  `Arc::into_inner`, which drops the pump's `Arc` when it fails; never by
  `Arc::strong_count`, which races a `Weak::upgrade` on another thread; and never by
  `Arc::get_mut`, which waits for every `Weak` as well. If `run` returns an error, or a
  panic unwinds out of it, while one is still held, the window is never destroyed, and
  its handles stay valid until the process ends. A `Weak<Window>` keeps nothing open, and
  upgrades to nothing once the window is taken back.
- So whoever keeps the `Arc` for as long as it uses the handles, as a backend keeps it
  until its surface is destroyed, has a live window at every use, and the pump waits for
  it. Whatever only reads the window's reports holds a `WindowReports` instead, from
  `Window::reports`, which keeps nothing open.
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
  what a swapchain's extent is asked as; on macOS this holds while the layer a backend
  presents to has the window's backing scale (`lactose-rhi-api`). It is the size the
  operating system gave when the window opened, and then the size of the newest resize
  the operating system has delivered to the main thread, whatever caused it: the user,
  `request_size`, a display's scale, a minimise. `WindowReports::size` reads the same
  value, and after the window is destroyed the last one delivered.
- That is the whole report. There is no event to take and no callback, and this crate adds
  no delay and no coalescing of its own: the value changes when the operating system
  delivers the resize, and a reader sees the newest size and may never see one between two
  of its reads. A resize wakes no one, and neither does a close, so whoever waits for
  either polls on a timer (`Executor::sleep_until`), under a deadline where the operating
  system may refuse or change what was asked.
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

- `Window::close_requested` and `WindowReports::close_requested` are `false` until the
  operating system first asks for the window to be closed, or `run` returns, and `true`
  from then on. The request is whatever the operating system delivers to the window as
  one: its close button, Alt+F4, a window menu's Close.
- Nothing else happens. The window stays open, the pump runs on, and nothing in this
  crate ends the process. The client reads the report and winds down in the order of
  "Shutdown", which calls `MainThread::exit` first.
- What an operating system addresses to the application and not to the window is not a
  close request and is not reported at milestone 2: macOS's Quit, from the Dock or from a
  menu that offers it, and a session's end. The operating system ends the process as it
  ends any process, and nothing here runs.

**Main-thread requests**

- `request_size` and `set_minimised` ask the operating system and return. Neither waits
  for the answer, and a request the operating system refuses or changes is not an error:
  the answer to `request_size` is a resize report or none, and a size it gives may be
  another than the one asked for. The answer to `set_minimised` is what `is_minimised`
  reports once the operating system has carried it out, or none; `is_minimised` is `None`
  where the operating system does not say.
- `exit` says the process is done with the window. It destroys nothing while an
  `Arc<Window>` is held: the pump runs on as before, and drains every main-lane call,
  until the window is let go ("Shutdown"). A second `exit` does nothing, and `exit` is
  never undone.

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
  skips, merges and reorders none. The lane is named `present`, and is `lactose-task`'s
  `parking_lane`: the thread parks in the lane's own wait, which a call that parks inside
  its body, on a channel or a fence's wait, cannot rob of the wake of a call queued
  meanwhile. That wait is `lactose-task`'s, with its loom model, and this crate keeps no
  flag or park for it.
- A call may block for as long as an acquire or a present does. It holds the calls queued
  behind it and nothing else: not the main thread, not a worker, and not its caller, which
  holds a future. Every wait inside a present call is bounded, by the caller, save the
  swapchain's drop in the last call, which waits for the submits that use its images
  (`lactose-rhi-api`, "Handles and lifetimes") and so returns once step 2's host signals
  are made ("Shutdown"). A call that never returns holds every call behind it, and the
  stop.
- The lane has no bound. A caller that queues a call for every frame while one blocks
  adds a call a frame, and each of them then runs. So the frame keeps at most one present
  call in flight: it queues the next when the last has returned, and chooses the newest
  finished target when it queues. That is back-pressure, not pacing. A call that
  presented nothing returns at once and paces nothing, and the timer root paces the frame
  (architecture section 5.1); both are the frame's to keep (`frame-design`).
- A call's return says its submit was taken, not that its target or its image is free:
  the frame takes a target back by the point the present thread signals
  (`lactose-rhi-api`, "The present thread").
- `PresentThread::stop` never waits. It queues the take of the state on a second lane,
  the hand-back lane, and then a stop call on the present lane. Both go through
  `lactose-task`'s lanes and no flag shared between threads, so "queued before `stop`" is
  the present lane's own order. The flag the stop call sets is the present thread's own:
  a thread-local the body clears when it starts, so a platform that runs a body on a
  thread that ran another does not stop the second early. The thread runs every call
  queued before the stop, the one in progress included, and whatever reached the same
  drain after it. Then it drops its pump, which closes the present lane; drains the
  hand-back lane once, which hands the state back; and exits. It is not joined. From this
  order:
  - A call queued after `stop` runs in that last drain or resolves to `Cancelled`. A call
    that never ran is dropped, with what it captured, on the present thread when the pump
    is dropped, or, if it was queued after that, on the caller's thread. A call queued
    after the hand-back resolves to `Cancelled`.
  - The lane is closed before the state is handed back, so from the hand-back on the
    present thread runs nothing of the caller's and holds nothing of the caller's.
  - `PresentStop` does not resolve while a call queued before the stop has not returned.
    A task that keeps queuing while the thread stops does not delay it: the thread stops
    after one last drain, whatever is queued behind it.
  - The take is queued before the stop, so the hand-back always finds it.
- `stop` is the supported way to end the thread; a drop is for unwinding, and gives no
  signal when it is done. Dropping a `PresentThread` queues the stop alone: the thread
  runs what was queued before it, drops its pump, and drops the state on the present
  thread, and the process may exit while it does. Dropping a `PresentStop` does not undo
  the stop: a state not yet handed back is dropped on the present thread, and one already
  handed back is dropped with the future, on the thread that drops it, a frame worker
  included. A state that holds nothing of the window, which "Shutdown" gives `stop`, makes
  either drop wait for nothing.
- A panic in a call is not caught. A binary aborts (section 5.1); in a test the present
  thread ends, the call that panicked and every call not yet run resolve to `Cancelled`,
  and so does the `PresentStop`.
- If the platform makes no thread, `present_thread` returns its error and `state` is
  dropped on the calling thread. If the platform accepts the thread's body and drops it
  unrun, `present_thread` sees the body go and returns `Cancelled` rather than wait for a
  start that never comes; `state` is dropped where the body was.
- The engine makes one present thread in a process. A second call is not refused: it
  makes a second thread with the same spec.

**Shutdown** (the milestone 2 plan's row `swapchain`: a shutdown with frames in flight)

- The order a client keeps, on every path, a failed start included:
  1. `exit` is queued first and not awaited, by a guard whose `Drop` queues it, held from
     its start by whatever drives the client (the root task, or a test's driver thread),
     so a path that returns early or unwinds queues it too. `exit` destroys nothing while
     an `Arc<Window>` is held, and the pump keeps draining main-lane calls until the last
     one goes, so when `run` returns a value every step below still reached the main
     thread, and every stall below happened under the pump's warning.
  2. The client stops queuing presents and frame submits, and signals from the host
     every point a pending submit waits on that no submit made will signal: what the host
     owes, and the points of a submit that failed or will now not be made. The swapchain's
     drop below waits for the submits that use its images. Then a last present-thread
     call signals in the same way the points of a present of its own that was refused,
     waits, within a bound, for the points its presents signalled, and drops the
     swapchain.
  3. It awaits `PresentThread::stop`, which hands back a state with nothing of the window
     in it.
  4. On a thread of its own, made through `Executor::platform`, since it may block and no
     worker may, it waits within a bound for the device's last points, and drops every
     encoder and every `Device` clone but one. If the bound is met, the thread hands the
     last `Device` back for step 5; if it expires, the thread drops it (below). If the
     platform makes no thread, its error ends step 4 and the device is dropped on the
     calling worker, where its drop may block; the pump's warning shows the stall.
  5. A main-lane call drops the last one, which then waits for nothing; MoltenVK's
     observation of the view is removed on the view's thread (off it after an expired
     step-4 bound, below).
  6. `run` returns once the last `Arc<Window>` goes, or with an error; `main` waits for
     its root task, within a bound after an error (below), and shuts the executor down.
     When `run` returns a value, nothing that runs after it holds the device or an
     `Arc<Window>`.
- When a bound expires, the client logs it and goes on to the next step, save one: a last
  `Device` whose step-4 bound expired is dropped on step 4's thread, a thread of its own,
  never in a main-lane call. A drop that then blocks holds that thread and keeps the
  window, and the pump's warning, already running because `exit` came first, shows the
  stall, and the main thread goes on answering. On that path MoltenVK's observation of the
  view is removed off the view's thread, which is accepted: the view's layer is not
  replaced during a shutdown. `run` does not instead return and leak the window: a
  teardown after `run` returned would have no main thread to answer a driver, and would
  stall with no warning.
- After an error from `run`, the steps run with no pump, their main-lane calls resolve to
  `Cancelled`, and `main` waits for them within a bound: it awaits the root on a thread of
  its own, made through `Executor::platform`, which sends the root's result on a channel,
  and takes it with a timeout. `main`'s bound is longer than steps 2 and 4's bounds
  together, so it does not give up on a root still inside them. Past it `main` logs on
  `pail` and returns `run`'s error with the root neither dropped nor shut down, and the
  process ends with it, as it does with the leaked window; how the process ends then is
  `lactose-app`'s choice. The waiter's body holds the root so that a platform that makes
  no thread leaks it, never drops it on the main thread.
- A main-lane call that drops what was made from the window drops only what has nothing
  pending (`lactose-rhi-api`, "Handles and lifetimes"). Whatever waits is done on the
  present thread or on a thread of its own before it, while the pump runs.
- `run` returns once `exit` has been called and the window is let go: every `Arc<Window>`
  that `MainThread::window` returned, and every clone, has been dropped. Until both hold
  it pumps events and drains calls as before, and a later `MainThread::window` gives the
  window again and is waited for again. It does not wait for an operating-system event to
  see the last drop: from `exit` until the window is let go it wakes every 50 ms on a
  timer of its own, the one time it does, so it returns within about 50 ms of the last
  drop. The last drop does not wake it instead: the pump keeps its own `Arc`, so the last
  other one runs no destructor. Once it has dropped the window it waits for the operating
  system's report that the window is destroyed, which Windows defers to a message, and
  returns on it; past a second without one it warns on `pail` and returns.
- No `Arc<Window>` is returned from the root task, kept in a world or resource that
  outlives `exit`, or dropped by `executor.shutdown`. What reads the window's reports
  past `exit` holds a `WindowReports`, which keeps nothing open.
- So when `run` returns nothing made from the window is alive, provided a backend holds
  its `Arc` for as long as its surface, and everything that was made from the window was
  destroyed while the main thread could answer.
- A pump that is never told to exit, or whose window is never let go, never returns.
  While it waits after `exit` it logs a warning on `pail` once a second with the number
  of `Arc`s still held.
- This crate waits for no present and no submit. What a swapchain asks before it is
  resized or destroyed, the caller does in its last calls or with the state it took back.

**`ClientPlatform`** (architecture section 5.1; `crates/task/README.md`, "Workers")

- `topology` and `hardware_threads` return what `StdPlatform`'s return in the same
  process, so a client's worker counts are the headless binary's on the same machine.
- `spawn_thread` makes a thread as `StdPlatform` does: named `spec.name`, in `spec.class`
  through `ThreadClass::enter` before `body` runs, with its errors. For
  `ThreadClass::Present` it applies nothing more: `ThreadClass::enter` gives that class
  macOS's user-interactive quality of service and Windows's above-normal priority, and
  `SAFETY.md` says why the one further treatment is not applied. No caller relies on a
  treatment. A class the operating system refuses is a warning on `pail`, and the thread
  runs.
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
| `Cancelled` | A lane call that will not run: the pump was dropped or `run` returned; the present thread handed its state back, or ended. A `PresentStop` whose thread ended without handing back. `present_thread`, if the platform dropped the thread's body unrun |
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
  and occlusion, more than one window.** No milestone 2 row reads one. The frame's pacing
  needs no display's refresh at milestone 2: the timer root paces it at the tick's deadline
  (D162), and the change that gives the frame a deadline between ticks asks for one here.
- **The tests with a device.** `swapchain` writes them here, with `lactose-rhi-vulkan` as
  a test-support dev-dependency, as subprocesses.
- **The crash hook** is `lactose-app`'s (D155), not this crate's.

## Proof

The compiler proves what it can. `src/lib.rs` asserts the auto traits under "Threads" and the
surface-source bounds; `tests/consumers.rs` type-checks what each consumer writes, with
stand-ins for `lactose-rhi-api`'s items that carry their bounds; the crate's doctest
type-checks a client's `main`.

The rest is a named test each, in three places. `tests/integration.rs` (`present`, `threads`,
`values`) and the unit tests of `src/window.rs` need no window and run everywhere CI does,
and nightly Miri runs them, save the present thread's CPU test, which reads a clock
through the operating system. `tests/window.rs` is the tests that open a window, a binary of
its own with its own `main`: a test harness's thread is not the main thread, so each case is
that binary run again as a subprocess whose `main` is the case, and the binary speaks enough
of the harness's arguments for `cargo nextest` to list and run it. Every case is ignored, so
CI lists them and runs none and no case passes where no window opens; on the M1 and on the
laptop they run by

```text
cargo nextest run -p lactose-platform --test window --run-ignored only
```

Every wait in a test is bounded, and each test shows the interleaving it names, by a barrier
or a count and never by a sleep's length alone. The two CPU tests idle for a set length and
bound the CPU the idle thread spent in it, read from that thread's own clock, which a loaded
machine does not raise. Each window subprocess runs under a deadline its parent keeps, as
`tests/consumers.rs`, `under_a_deadline`, sketches it: the parent reads the child's output
while it waits, and past the deadline it kills the child and fails, naming the last stage
the child printed. It takes the output within a bound too, and fails with "stdout held open"
past it, since a process the child started may keep the pipe. The subprocess's driver holds
the `exit` guard of "Shutdown" from its start, so a check that fails ends the pump too.
`ThreadHandle::join` discards a panic, so the driver sends its result on a channel as its
last act and the subprocess's exit status carries it; the parent checks the status and that
the last stage printed is the final one, or, for a case that must fail or abort, the stage
and the text it must end with.

- A window opens, events are pumped, one call runs on the main thread and one on the
  present thread, and the process closes: `run` returns and the exit status is zero.
  (`window`: `opens_pumps_and_closes`, which also reads the present thread's scheduling
  back from the operating system.)
- A call queued before `run` runs after the window opens; a call queued after `run`
  returned is `Cancelled`; a dropped pump cancels its calls. A dropped pump opens no
  window, which no case checks: the window is made in winit's `resumed` alone, which only
  `run_app` reaches, and a dropped pump never calls it. (`window`:
  `a_call_queued_before_run_runs_and_one_after_is_cancelled`,
  `a_dropped_pump_cancels_its_calls_and_a_second_pump_is_unsupported`.)
- `event_pump` on a second thread panics; a second `event_pump` is `Unsupported`.
  (`integration`: `threads::event_pump_panics_off_the_main_thread_and_a_second_is_unsupported`,
  on Linux too; `window`: the second call on the main thread, in the dropped pump's case.)
- The handles are lent inside a main-lane call and are `Unavailable` on every other thread.
  (`window`: `the_handles_are_lent_on_the_main_thread_alone`, on the driver and the present
  thread, and the same handles at every call.)
- `request_size` is answered by `Window::size` and by a `WindowReports`, polled under a
  deadline; a minimised window on Windows reports empty, and `is_minimised` reports it; a
  read on another thread never sees a torn size, under a writer that alternates two sizes.
  (`window`: `a_requested_size_and_a_minimise_are_reported`; `src/window.rs`:
  `a_read_never_sees_a_torn_size`.)
- `close_requested` turns `true` on a close the test sends the window, and the window
  stays open; it turns `true` when `run` returns, by value and by error; and it is `true`
  before the lane is closed: the waker of a main-lane call that the close cancels reads it
  `true` when the cancel wakes it, on the main thread, as the pump drops. That call is
  queued and polled while the main thread is still inside the call that runs `exit`, so
  the lane is open then however late the test's thread runs. (`window`:
  `a_close_sent_to_the_window_is_reported_and_the_window_stays_open`, and the case of the
  call queued before `run` for the waker; by error, the case after an error from `run`.)
- `run` does not return after `exit` while an `Arc<Window>` is held: it has not returned
  a set time after `exit`, and has warned on `pail`; it returns within a bound of the last
  drop, made on another thread, of 500 ms, which a pump that looked once a second or
  waited for the second's fallback overshoots. A `WindowReports` held past `exit` does not
  keep it. (`window`: `run_waits_for_the_last_window_and_warns`.)
- The pump spends under 200 ms of the main thread's CPU in each of two 2 s spells: idle
  with the window open, and after `exit` while an `Arc<Window>` is held. The main thread
  reads its own CPU clock in a main-lane call at each end. (`window`:
  `the_pump_idles_open_and_while_a_window_is_held`.)
- A driver that panics before its last step still lets the pump return: its `exit` guard
  queues `exit` while it unwinds, and the subprocess's exit status is not zero. (`window`:
  `a_driver_that_panics_still_lets_the_pump_return`.)
- A last device whose drop would block, after step 4's bound expired, is dropped on step
  4's thread, a thread of its own: the pump warns, a main-lane call queued meanwhile
  runs, and no worker is held. (`window`:
  `a_last_device_whose_drop_blocks_holds_its_own_thread_alone`.)
- After an error from `run`, with a root whose last step blocks, `main` returns `run`'s
  error within its bound, having logged on `pail`, and the root is not dropped on the main
  thread. The close is reported after the error, and the window, held by the pump's `Arc`
  alone and by the test's `Weak`, is still alive. (`window`:
  `after_an_error_from_run_main_returns_within_its_bound`, on macOS alone: the error is
  the event loop ended by the application's own stop, and winit's loop on Windows ends
  through its `exit` alone.)
- A worker that waits on the main thread fails: `Executor::block_on` of a main-lane call
  on a worker is `lactose-task`'s debug assertion, and the subprocess aborts. In a release
  build there is no assertion, the main thread runs the call and the wait resolves, which
  the case expects and prints. (`window`: `a_worker_that_waits_on_the_main_thread_aborts`.)
- The present thread has the spec above, through a spy `Platform`. A call that blocks
  holds the calls behind it and no other thread: a bounded handshake shows the blocking
  call in progress while a main-lane call and a worker's task complete, and the call
  behind it not yet run. `stop` runs every call queued before it and hands the state back,
  and a drop runs them and drops the state on the present thread: in 3,000 rounds that
  alternate the two, three threads queue calls in a loop while the stop or the drop is
  made; every call queued before it runs, in order, each later one runs in the last drain
  or is `Cancelled`, none runs after the hand-back, and across the rounds both outcomes
  occur; so a stop queued before the take fails it, in about one stop round in a hundred,
  and so in all but about one run in 10^8 (Miri runs four rounds, which check the order
  but do not catch that stop). The lane is closed before the state is handed back: a call
  that the close cancels is in its drop while the stop's future is polled pending. An idle
  present thread spends under 50 ms of its CPU in each of three 500 ms spells: after plain
  calls, after a call that parked with one behind it, and after a call that unparked its
  own thread; it reads its own CPU clock in a call at each end. A call queued after the
  hand-back is `Cancelled`. A platform that runs every body on one
  thread makes a second present thread after the first stopped, and the second runs its
  calls. A dropped `PresentThread` drops the state on the present thread, which the test
  sees by polling a flag the state's drop sets, under a deadline. A platform that drops
  the body unrun makes `present_thread` return `Cancelled`. A call queued while a call
  that parks the present thread is in progress runs after it, its wake not lost to the
  park, and a call that panics ends the thread with the stop `Cancelled`. (`integration`:
  each of `present::`; `window`:
  `a_present_call_that_blocks_holds_only_the_calls_behind_it`.)
- `ClientPlatform` reports `StdPlatform`'s topology and hardware threads, and enters the
  class of each thread it makes. (`integration`: `threads::`; the present class read back,
  in `opens_pumps_and_closes`.)
- Clippy passes for `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu` from the M1.
