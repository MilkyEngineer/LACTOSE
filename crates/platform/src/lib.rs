//! The client's operating-system layer (architecture sections 4 and 5.1): the window and
//! the handles a surface is made from, the event pump that owns the main thread, the
//! present thread, and the `Platform` a client binary gives its executor.
//!
//! Backend tier, outside the determinism boundary, and on the server's denylist: a client
//! binary names this crate, and `lactose-app`, the renderer and every graphics backend
//! reach what it makes through `lactose_task::Lane` and `raw-window-handle`'s traits
//! without naming it. The `README.md` is the contract: it states what the signatures
//! cannot, and what a caller may rely on.
//!
//! # Status
//!
//! Implemented over winit 0.30 on macOS and Windows; on Linux it builds over X11 and is
//! not run. It contains no `unsafe` (`SAFETY.md`).
//!
//! # Where to start
//!
//! - [`event_pump`] is called first, on the main thread. It gives the [`EventPump`], which
//!   `main` runs last and which returns when the process is done with its window, and
//!   the main-thread lane, whose calls are lent a [`MainThread`].
//! - [`Window`] is reached inside a main-lane call. Its handles are what a graphics
//!   backend makes a surface from, there and nowhere else; [`Window::reports`] is what a
//!   resize and a close report, to any thread, in a handle that keeps nothing open.
//! - [`present_thread`] starts the one thread that acquires and presents, and gives the
//!   lane that carries every call to it; [`PresentThread::stop`] takes its state back.
//! - [`ClientPlatform`] is what the client passes as `Workers::platform`.
//!
//! A client binary, by thread, with the shutdown of `README.md`, "Shutdown", step by step:
//!
//! ```no_run
//! use std::mem::ManuallyDrop;
//! use std::sync::{Arc, mpsc};
//! use std::time::{Duration, Instant};
//!
//! use lactose_core::{Error, ErrorKind, Result, ThreadClass};
//! use lactose_platform::{
//!     ClientPlatform, MainThread, PixelSize, PresentThread, Window, WindowDesc, WindowReports,
//!     event_pump, present_thread,
//! };
//! use lactose_task::{Executor, Lane, Platform, ThreadSpec, WorkerClass};
//!
//! /// Stands for a graphics backend's device, which keeps the window's `Arc` until its
//! /// surface is destroyed.
//! struct Device {
//!     _window: Arc<Window>,
//! }
//!
//! impl Device {
//!     /// Stands for the backend's waits on the device's last points: `false` if the bound
//!     /// expired, so that dropping the device may block.
//!     fn last_points(&self, _bound: Duration) -> bool {
//!         true
//!     }
//! }
//!
//! /// What the present thread owns: the caller's type, which holds the swapchain, a clone
//! /// of the device, and whatever draws a finished target into the swapchain's images.
//! struct Presenter {
//!     device: Option<Arc<Device>>,
//!     presented: u64,
//! }
//!
//! /// Queues `exit` when it is dropped, a panic's unwinding included, and does not wait.
//! struct ExitOnDrop(Lane<MainThread>);
//!
//! impl Drop for ExitOnDrop {
//!     fn drop(&mut self) {
//!         drop(self.0.run(|main| main.exit()));
//!     }
//! }
//!
//! /// A main-lane call, where the window lends its handles: the device is made from the
//! /// window, and the frame keeps the reports, which hold nothing open.
//! fn start(main: &mut MainThread) -> (Arc<Device>, WindowReports) {
//!     let window = main.window();
//!     let reports = window.reports();
//!     (Arc::new(Device { _window: window }), reports)
//! }
//!
//! /// One present call in flight, the next queued when the last has returned. A call that
//! /// presented nothing paces nothing, so the frame then waits for its next deadline.
//! async fn frames(
//!     executor: &Executor,
//!     reports: &WindowReports,
//!     present: &Lane<Presenter>,
//! ) -> Result<()> {
//!     while !reports.close_requested() {
//!         let size = reports.size();
//!         let presented = present
//!             .run(move |presenter| {
//!                 let shown = presenter.device.is_some() && !size.is_empty();
//!                 presenter.presented += u64::from(shown);
//!                 shown
//!             })
//!             .await?;
//!         if !presented {
//!             executor.sleep_until(Instant::now() + Duration::from_millis(16)).await?;
//!         }
//!     }
//!     Ok(())
//! }
//!
//! /// The root task: start, frames, and the shutdown every path takes.
//! async fn root(
//!     executor: Executor,
//!     main: Lane<MainThread>,
//!     present: Lane<Presenter>,
//!     thread: PresentThread<Presenter>,
//! ) -> Result<()> {
//!     let exit = ExitOnDrop(main.clone());
//!     let mut device = None;
//!     let ran = async {
//!         let (made, reports) = main.run(start).await?;
//!         let on_thread = made.clone();
//!         device = Some(made);
//!         present.run(move |presenter| presenter.device = Some(on_thread)).await?;
//!         frames(&executor, &reports, &present).await
//!     }
//!     .await;
//!     // 1. `exit` first, not awaited: it destroys nothing while an `Arc<Window>` is held,
//!     //    and the pump drains main-lane calls until the last one goes.
//!     drop(exit);
//!     // 2. No present and no frame submit is queued from here, and every point a pending
//!     //    submit waits on that no submit made will signal is signalled from the host
//!     //    first: what the host owes, and the points of a submit that failed or will now
//!     //    not be made; the swapchain's drop waits for the submits that use its images.
//!     //    Then the last present-thread call signals in the same way the points of a
//!     //    refused present of its own, waits, within a bound, for the points its presents
//!     //    signalled, and drops the swapchain; here the thread holds only a device clone.
//!     let last = present.run(|presenter| presenter.device = None).await;
//!     // 3. The state comes back with nothing of the window in it.
//!     let stopped = thread.stop().await.map(drop);
//!     // 4. On a thread of its own: the device's last points are waited for within a
//!     //    bound, and every encoder and clone but one go.
//!     let settled = match device {
//!         Some(device) => settle(&executor, device).await,
//!         None => Ok(None),
//!     };
//!     // 5. A main-lane call drops the last, which then waits for nothing. After an expired
//!     //    step-4 bound none comes back: step 4's thread drops it.
//!     let dropped = match settled {
//!         Ok(Some(device)) => main.run(move |_| drop(device)).await,
//!         Ok(None) => Ok(()),
//!         Err(error) => Err(error),
//!     };
//!     ran.and(last).and(stopped).and(dropped)
//! }
//!
//! /// How long a step of the shutdown waits; `main` waits three after an error from `run`.
//! const BOUND: Duration = Duration::from_secs(2);
//!
//! /// Stands for the client's warning on `lactose_platform::LOG_TARGET`.
//! fn warn(message: &str) {
//!     eprintln!("{message}");
//! }
//!
//! /// Step 4 on a thread of its own, made through the executor's platform, since it may
//! /// block and no worker may. It hands the device back if its last points were reached
//! /// within the bound; otherwise it logs, hands back nothing and drops the device itself,
//! /// a drop that may block while the main thread answers. The root looks on a timer. If
//! /// the platform makes no thread, its error ends step 4 and the device is dropped on the
//! /// calling worker, where its drop may block; the pump's warning shows the stall.
//! async fn settle(executor: &Executor, device: Arc<Device>) -> Result<Option<Arc<Device>>> {
//!     let (answer, answered) = mpsc::channel();
//!     let spec = ThreadSpec {
//!         name: "settle".to_owned(),
//!         class: ThreadClass::Background,
//!         index_in_class: 0,
//!     };
//!     executor.platform().spawn_thread(spec, Box::new(move || {
//!         if device.last_points(BOUND) {
//!             let _ = answer.send(Some(device));
//!         } else {
//!             warn("the device's last points passed the shutdown's bound");
//!             let _ = answer.send(None);
//!             drop(device);
//!         }
//!     }))?;
//!     loop {
//!         match answered.try_recv() {
//!             Ok(device) => return Ok(device),
//!             Err(mpsc::TryRecvError::Empty) => {
//!                 executor.sleep_until(Instant::now() + Duration::from_millis(10)).await?;
//!             }
//!             Err(mpsc::TryRecvError::Disconnected) => {
//!                 return Err(Error::new(ErrorKind::Cancelled, "step 4's thread ended"));
//!             }
//!         }
//!     }
//! }
//!
//! fn main() -> Result<()> {
//!     // The main thread makes the pump first and runs it last.
//!     let (main, pump) = event_pump(WindowDesc {
//!         title: "LACTOSE".to_owned(),
//!         size: PixelSize { width: 1280, height: 720 },
//!     })?;
//!     let platform: Arc<dyn Platform> = Arc::new(ClientPlatform::new());
//!     let executor = Executor::builder().platform(platform.clone()).build()?;
//!     let (present, thread) =
//!         present_thread(&*platform, Presenter { device: None, presented: 0 })?;
//!     let task =
//!         executor.spawn(WorkerClass::FrameCritical, root(executor.clone(), main, present, thread));
//!
//!     // 6. When `run` returns a value the last `Arc<Window>` is gone, and nothing after it
//!     //    holds the device or the window.
//!     if let Err(error) = pump.run() {
//!         // The steps run with no pump, and their main-lane calls resolve to `Cancelled`.
//!         // The root is awaited on a thread of its own within a bound longer than steps
//!         // 2 and 4's together; past it `main` logs and returns the error with the root
//!         // neither dropped nor shut down, and the process ends with it, as it does with
//!         // the leaked window. A platform that makes no thread drops the body unrun,
//!         // which leaks the root rather than dropping it here.
//!         let (answer, answered) = mpsc::channel();
//!         let waiter = executor.clone();
//!         let task = ManuallyDrop::new(task);
//!         let spec = ThreadSpec {
//!             name: "root".to_owned(),
//!             class: ThreadClass::Background,
//!             index_in_class: 0,
//!         };
//!         let waited = platform.spawn_thread(spec, Box::new(move || {
//!             let task = ManuallyDrop::into_inner(task);
//!             let _ = answer.send(waiter.block_on(task));
//!         }));
//!         match waited.map(|_| answered.recv_timeout(3 * BOUND)) {
//!             Ok(Ok(_)) => executor.shutdown(),
//!             _ => warn("the root did not end within its bound after `run` failed"),
//!         }
//!         return Err(error);
//!     }
//!     let ran = executor.block_on(task);
//!     executor.shutdown();
//!     ran?
//! }
//! ```

#![forbid(unsafe_code)]

mod main_thread;
mod present;
mod threads;
mod window;

pub use main_thread::{EventPump, MainThread, event_pump};
pub use present::{PresentStop, PresentThread, present_thread};
pub use threads::ClientPlatform;
pub use window::{PixelSize, Window, WindowDesc, WindowReports};

/// The `tracing` target of everything this crate logs: a thread class the operating
/// system refused, and a pump that has been told to exit and is still waiting for the
/// window to be let go (`README.md`, "Shutdown").
pub const LOG_TARGET: &str = "pail";

// What the contract promises at the type level, so that the implementation or an amendment
// cannot lose it silently (`README.md`, "Threads").
const _: () = {
    use lactose_task::{Lane, Platform};
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

    const fn send_sync<T: ?Sized + Send + Sync>() {}
    const fn send<T: ?Sized + Send>() {}
    const fn plain<T: Copy + Send + Sync + 'static>() {}
    const fn shared<T: Clone + Send + Sync + 'static>() {}
    // The bounds of `lactose-rhi-api`'s `SurfaceSource`, which is implemented for every
    // type that meets them, and `'static`, which an `Arc<dyn SurfaceSource>` needs.
    const fn surface_source<T: HasWindowHandle + HasDisplayHandle + Send + Sync + 'static>() {}
    // What `Workers::platform` holds.
    fn workers_platform<T: Platform>(platform: std::sync::Arc<T>) -> std::sync::Arc<dyn Platform> {
        platform
    }
    let _ = workers_platform::<ClientPlatform>;

    // The window goes to a backend as a surface source, and its reports to any thread,
    // cloned as often as their readers want.
    surface_source::<Window>();
    shared::<WindowReports>();
    plain::<PixelSize>();
    send_sync::<WindowDesc>();
    send_sync::<ClientPlatform>();

    // The main-thread lane goes anywhere, though the state it reaches never leaves the
    // main thread; and what a task holds across an `.await` is `Send`, for a state that
    // is `Send` and not `Sync`, as a swapchain is.
    send_sync::<Lane<MainThread>>();
    send_sync::<Lane<core::cell::Cell<u8>>>();
    send::<PresentThread<core::cell::Cell<u8>>>();
    send::<PresentStop<core::cell::Cell<u8>>>();

    // Neither `Send` nor `Sync`: the pump runs on the thread that made it, and a
    // `MainThread` stays where it is lent. Were either `Send`, both impls below would
    // apply and the inference would be ambiguous; the same for `Sync`.
    trait NotSend<A> {
        fn holds() {}
    }
    impl<T: ?Sized> NotSend<()> for T {}
    impl<T: ?Sized + Send> NotSend<u8> for T {}
    trait NotSync<A> {
        fn holds() {}
    }
    impl<T: ?Sized> NotSync<()> for T {}
    impl<T: ?Sized + Sync> NotSync<u8> for T {}
    let _ = <EventPump as NotSend<_>>::holds;
    let _ = <EventPump as NotSync<_>>::holds;
    let _ = <MainThread as NotSend<_>>::holds;
    let _ = <MainThread as NotSync<_>>::holds;
};
