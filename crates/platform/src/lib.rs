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
//! A facade: every public signature is final and every body that decides anything is
//! `todo!()`. The implementation lane replaces the bodies behind this surface and removes
//! the crate-level `allow` with the last `todo!()`.
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
//! use std::sync::Arc;
//! use std::time::{Duration, Instant};
//!
//! use lactose_core::Result;
//! use lactose_platform::{
//!     ClientPlatform, MainThread, PixelSize, PresentThread, Window, WindowDesc, WindowReports,
//!     event_pump, present_thread,
//! };
//! use lactose_task::{Executor, Lane, Platform, WorkerClass};
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
//!     fn last_points(&self, bound: Duration) -> bool {
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
//!     // 2. No present and no frame submit is queued from here, and every point the host
//!     //    owes that a pending submit waits on is signalled first: the swapchain's drop
//!     //    waits for the submits that use its images. Then the last present-thread call
//!     //    waits, within a bound, for the points its presents signalled, and drops the
//!     //    swapchain; here the thread holds only a clone of the device.
//!     let last = present.run(|presenter| presenter.device = None).await;
//!     // 3. The state comes back with nothing of the window in it.
//!     let stopped = thread.stop().await.map(drop);
//!     // 4. On a background worker: the device's last points are waited for within a
//!     //    bound, and every encoder and clone but one go.
//!     let bound = Duration::from_secs(2);
//!     let settled = executor.spawn(WorkerClass::Background, async move {
//!         let settled = device.as_ref().is_none_or(|device| device.last_points(bound));
//!         (device, settled)
//!     });
//!     // 5. A main-lane call drops the last, which then waits for nothing; after an
//!     //    expired step-4 bound it is dropped on a background worker instead, where its
//!     //    drop may block while the main thread answers.
//!     let dropped = match settled.await {
//!         Ok((Some(device), true)) => main.run(move |_| drop(device)).await,
//!         Ok((Some(device), false)) => {
//!             executor.spawn(WorkerClass::Background, async move { drop(device) }).await
//!         }
//!         Ok((None, _)) => Ok(()),
//!         Err(error) => Err(error),
//!     };
//!     ran.and(last).and(stopped).and(dropped)
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
//!     // 6. `run` returns once the last `Arc<Window>` is gone, and nothing after it holds
//!     //    the device or the window. Its result waits until the root has finished and
//!     //    the executor is shut down, so the root's device is never dropped here.
//!     let pumped = pump.run();
//!     let ran = executor.block_on(task);
//!     executor.shutdown();
//!     pumped?;
//!     ran?
//! }
//! ```

#![forbid(unsafe_code)]
#![allow(
    dead_code,
    unused_variables,
    reason = "facade: every deciding body is `todo!()` until the implementation lane replaces it, and the fields and parameters it will use are declared now"
)]

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
