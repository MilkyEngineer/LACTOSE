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
//!   backend makes a surface from, there and nowhere else; [`Window::size`] and
//!   [`Window::close_requested`] are what a resize and a close report, to any thread.
//! - [`present_thread`] starts the one thread that acquires and presents, and gives the
//!   lane that carries every call to it; [`PresentThread::stop`] takes its state back.
//! - [`ClientPlatform`] is what the client passes as `Workers::platform`.
//!
//! A client binary, by thread:
//!
//! ```no_run
//! use std::sync::Arc;
//!
//! use lactose_core::Result;
//! use lactose_platform::{
//!     ClientPlatform, MainThread, PixelSize, WindowDesc, event_pump, present_thread,
//! };
//! use lactose_task::{Executor, Lane, Platform, WorkerClass};
//!
//! /// What the present thread owns: the caller's type, which holds the swapchain and
//! /// whatever draws a finished target into its images.
//! struct Presenter {
//!     presented: u64,
//! }
//!
//! /// Everything but the two pumps is a task, and waits for either thread with `.await`.
//! async fn client(main: &Lane<MainThread>, present: &Lane<Presenter>) -> Result<()> {
//!     // The device is made in a call like this one, where the window lends its handles.
//!     let window = main.run(|main| main.window()).await?;
//!     while !window.close_requested() {
//!         let size = window.size();
//!         // One call at a time: the next is queued when this one has returned.
//!         present
//!             .run(move |presenter| {
//!                 if !size.is_empty() {
//!                     presenter.presented += 1;
//!                 }
//!             })
//!             .await?;
//!     }
//!     Ok(())
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
//!     let (present, thread) = present_thread(&*platform, Presenter { presented: 0 })?;
//!
//!     let root = executor.spawn(WorkerClass::FrameCritical, async move {
//!         let ran = client(&main, &present).await;
//!         // On every path: take the present thread's state back, destroy on the main
//!         // thread what was made from the window, and let the pump go.
//!         let presenter = thread.stop().await;
//!         let left = main
//!             .run(move |main| {
//!                 drop(presenter);
//!                 main.exit();
//!             })
//!             .await;
//!         ran.and(left)
//!     });
//!
//!     pump.run()?;
//!     let ran = executor.block_on(root);
//!     executor.shutdown();
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
pub use window::{PixelSize, Window, WindowDesc};

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
    // The bounds of `lactose-rhi-api`'s `SurfaceSource`, which is implemented for every
    // type that meets them, and `'static`, which an `Arc<dyn SurfaceSource>` needs.
    const fn surface_source<T: HasWindowHandle + HasDisplayHandle + Send + Sync + 'static>() {}
    // What `Workers::platform` holds.
    fn workers_platform<T: Platform>(platform: std::sync::Arc<T>) -> std::sync::Arc<dyn Platform> {
        platform
    }
    let _ = workers_platform::<ClientPlatform>;

    // The window goes to a backend as a surface source, and to any thread for its reports.
    surface_source::<Window>();
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
