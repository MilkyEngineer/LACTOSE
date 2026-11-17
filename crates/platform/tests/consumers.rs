//! Compile-only consumer sketches: what `lactose-app-client`, `lactose-app` and the
//! renderer, a graphics backend, the `swapchain` lane's tests and `platform-impl`'s
//! subprocess test write against this crate, type-checked and never run (every deciding
//! body is `todo!()`). They pin the surface the README promises: a change that breaks one
//! is a signature change to list.

#![allow(dead_code, unused_variables, missing_docs)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use lactose_core::{Error, ErrorKind, Result};
use lactose_platform::{
    ClientPlatform, EventPump, MainThread, PixelSize, PresentThread, Window, WindowDesc,
    event_pump, present_thread,
};
use lactose_task::{
    CoreTopology, Executor, Lane, Platform, StdPlatform, ThreadHandle, ThreadSpec, WorkerClass,
};
use raw_window_handle::{HandleError, HasDisplayHandle, HasWindowHandle};

// ---- 0. Stand-ins for `lactose-rhi-api`, which this crate never names ----
//
// Each has the bounds and the receiver of the item it stands for on the branch `rhi-api`
// (`SurfaceSource`, `DeviceRequest::surface`, `Device`, `Swapchain`), and nothing else.

mod rhi {
    use std::sync::Arc;

    use lactose_core::Result;
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

    pub trait SurfaceSource: HasWindowHandle + HasDisplayHandle + Send + Sync {}
    impl<T: HasWindowHandle + HasDisplayHandle + Send + Sync + ?Sized> SurfaceSource for T {}

    /// Each dimension above zero.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub struct Extent {
        pub width: u32,
        pub height: u32,
    }

    pub struct DeviceRequest {
        pub surface: Option<Arc<dyn SurfaceSource>>,
    }

    pub trait Device: Send + Sync {
        fn create_swapchain(&self, extent: Extent) -> Result<Box<dyn Swapchain>>;
    }

    pub enum Acquired {
        Image(u32),
        None,
        Stale,
    }

    pub enum Presented {
        Shown,
        Stale,
    }

    /// `Send` and not `Sync`: one thread holds it.
    pub trait Swapchain: Send {
        fn extent(&self) -> Extent;
        fn acquire(&mut self) -> Result<Acquired>;
        fn present(&mut self, image: u32) -> Result<Presented>;
        fn resize(&mut self, extent: Extent) -> Result<()>;
    }

    /// A backend's constructor. It reads the window's two handles, once each, on the
    /// thread that calls it, and keeps the `Arc` until its surface is destroyed.
    pub fn create_device(request: DeviceRequest) -> Result<Arc<dyn Device>> {
        todo!()
    }
}

// ---- 1. `lactose-app` and the renderer: neither names this crate ----
//
// `lactose-platform` is on the server's denylist and a backend crate may not name another,
// so what drives the frame holds `lactose_task::Lane<S>` over a state of its own and is
// told of the window through a trait of its own. Nothing in this module names
// `lactose_platform`.

mod frame {
    use core::cell::Cell;
    use std::sync::Arc;

    use lactose_core::Result;
    use lactose_task::Lane;

    use super::rhi::{Acquired, Device, Extent, Presented, Swapchain};

    /// What the present thread owns. `Send` and not `Sync`, as the swapchain is: the lane
    /// asks for no more.
    pub struct Presenter {
        pub device: Option<Arc<dyn Device>>,
        pub swapchain: Option<Box<dyn Swapchain>>,
        /// The size last asked for, which is what a resize is decided against.
        pub asked: Option<Extent>,
        pub presents: Cell<u64>,
    }

    impl Presenter {
        pub fn empty() -> Self {
            Self { device: None, swapchain: None, asked: None, presents: Cell::new(0) }
        }

        /// One present, on the present thread: every call here may block, and none of
        /// them is made anywhere else.
        fn present(&mut self, size: Option<Extent>) -> Result<bool> {
            let (Some(device), Some(size)) = (&self.device, size) else { return Ok(false) };
            if self.asked != Some(size) {
                match &mut self.swapchain {
                    Some(swapchain) => swapchain.resize(size)?,
                    None => self.swapchain = Some(device.create_swapchain(size)?),
                }
                self.asked = Some(size);
            }
            let Some(swapchain) = &mut self.swapchain else { return Ok(false) };
            match swapchain.acquire()? {
                Acquired::Image(image) => match swapchain.present(image)? {
                    Presented::Shown => {
                        self.presents.set(self.presents.get() + 1);
                        Ok(true)
                    }
                    Presented::Stale => {
                        self.asked = None;
                        Ok(false)
                    }
                },
                Acquired::None => Ok(false),
                Acquired::Stale => {
                    self.asked = None;
                    Ok(false)
                }
            }
        }
    }

    /// What the frame reads of the window, as `lactose-app` would declare it.
    pub trait Shown: Send + Sync {
        /// `None` while nothing can be presented.
        fn extent(&self) -> Option<Extent>;
        fn close_requested(&self) -> bool;
    }

    /// The frame's presents: one call on the lane at a time, each awaited, so a present
    /// that blocks holds this task and no worker, and the lane never holds a second.
    pub async fn frames(shown: &dyn Shown, present: &Lane<Presenter>) -> Result<u64> {
        let mut presented = 0;
        while !shown.close_requested() {
            let size = shown.extent();
            if present.run(move |presenter| presenter.present(size)).await?? {
                presented += 1;
            }
        }
        Ok(presented)
    }
}

// ---- 2. `lactose-app-client`: the one crate that names this one ----

/// The adapter from this crate's window to what the frame reads.
struct ShownWindow(Arc<Window>);

impl frame::Shown for ShownWindow {
    fn extent(&self) -> Option<rhi::Extent> {
        let size = self.0.size();
        (!size.is_empty()).then_some(rhi::Extent { width: size.width, height: size.height })
    }

    fn close_requested(&self) -> bool {
        self.0.close_requested()
    }
}

/// `Workers::platform` as `lactose-app` declares it: the client passes this crate's.
struct Workers {
    platform: Option<Arc<dyn Platform>>,
}

fn workers() -> Workers {
    Workers { platform: Some(Arc::new(ClientPlatform::new())) }
}

/// The device is made inside a main-lane call, which is where the window lends its
/// handles; the window's `Arc` goes to the backend as its surface source and to the
/// frame for its reports.
async fn start(main: &Lane<MainThread>) -> Result<(Arc<dyn rhi::Device>, Arc<Window>)> {
    main.run(|main| {
        let window = main.window();
        let surface: Arc<dyn rhi::SurfaceSource> = window.clone();
        let device = rhi::create_device(rhi::DeviceRequest { surface: Some(surface) })?;
        Ok((device, window))
    })
    .await?
}

/// The client's root task: start, frames, and the shutdown that every path takes.
async fn root(
    main: Lane<MainThread>,
    present: Lane<frame::Presenter>,
    thread: PresentThread<frame::Presenter>,
) -> Result<u64> {
    let ran = async {
        let (device, window) = start(&main).await?;
        let on_thread = device.clone();
        present.run(move |presenter| presenter.device = Some(on_thread)).await?;
        let shown = ShownWindow(window);
        frame::frames(&shown, &present).await
        // `device`, `shown` and with it the window's `Arc` are dropped here, on a worker.
    }
    .await;
    // The present thread runs what was queued, hands its state back and is gone. The
    // state holds the swapchain and a clone of the device, which holds the window: both
    // are destroyed on the main thread, and only then is the window no one else's.
    let presenter = thread.stop().await;
    let left = main
        .run(move |main| {
            drop(presenter);
            main.exit();
        })
        .await;
    left.and(ran)
}

/// `main`: the pump is made first and run last, and nothing after `run` touches the
/// window, which is gone.
fn client_main() -> Result<()> {
    let (main, pump) = event_pump(WindowDesc {
        title: "client".to_owned(),
        size: PixelSize { width: 1280, height: 720 },
    })?;
    let workers = workers();
    let executor =
        Executor::builder().platform(workers.platform.clone().expect("the client's")).build()?;
    // The present thread is made through the executor's platform, with an empty state
    // that the first call fills.
    let (present, thread) =
        present_thread(executor.platform().as_ref(), frame::Presenter::empty())?;
    let task = executor.spawn(WorkerClass::FrameCritical, root(main, present, thread));
    pump.run()?;
    let frames = executor.block_on(task);
    executor.shutdown();
    frames?.map(|_| ())
}

// ---- 3. A graphics backend: the window's handles, and where they are lent ----

/// What a backend's constructor does with the source it was given, on the thread that
/// called it.
fn read_handles(surface: &dyn rhi::SurfaceSource) -> Result<()> {
    let unavailable = |error: HandleError| Error::new(ErrorKind::Unsupported, error.to_string());
    let window = surface.window_handle().map_err(unavailable)?;
    let display = surface.display_handle().map_err(unavailable)?;
    let (_window, _display) = (window.as_raw(), display.as_raw());
    Ok(())
}

/// Off the main thread the window lends nothing, on every operating system.
fn handles_off_the_main_thread(window: &Window) {
    assert!(matches!(window.window_handle(), Err(HandleError::Unavailable)));
    assert!(matches!(window.display_handle(), Err(HandleError::Unavailable)));
}

// ---- 4. The `swapchain` lane's tests: a resize, a minimised window, a shutdown ----

/// A fixed number of frames presented across a resize: the test asks for a size on the
/// main thread and reads the report from where it stands.
async fn across_a_resize(
    main: &Lane<MainThread>,
    present: &Lane<frame::Presenter>,
    window: &Window,
) -> Result<()> {
    let before = window.size();
    let asked = PixelSize { width: before.width / 2, height: before.height / 2 };
    main.run(move |main| main.request_size(asked)).await?;
    while window.size() == before {
        lactose_task::yield_now().await;
    }
    let size = window.size();
    let extent = rhi::Extent { width: size.width, height: size.height };
    let presents = present
        .run(move |presenter| {
            presenter.asked = None;
            presenter.presents.get()
        })
        .await?;
    Ok(())
}

/// The drawable made to block: the window is minimised, and the call that blocks returns
/// how long it took, which is how the present thread records a block of a second.
async fn a_blocked_drawable(
    main: &Lane<MainThread>,
    present: &Lane<frame::Presenter>,
) -> Result<Duration> {
    main.run(|main| main.set_minimised(true)).await?;
    let blocked = present
        .run(|presenter| {
            let start = Instant::now();
            if let Some(swapchain) = &mut presenter.swapchain {
                let _ = swapchain.acquire();
            }
            start.elapsed()
        })
        .await?;
    main.run(|main| main.set_minimised(false)).await?;
    Ok(blocked)
}

/// A shutdown with frames in flight: the stop is ordered behind every call queued before
/// it, and the state comes back to be torn down in the caller's order.
async fn with_frames_in_flight(
    present: Lane<frame::Presenter>,
    thread: PresentThread<frame::Presenter>,
) -> Result<()> {
    let last = present.run(|presenter| presenter.presents.get());
    let presenter = thread.stop().await?;
    let presents = last.await?;
    let frame::Presenter { swapchain, device, .. } = presenter;
    drop(swapchain);
    drop(device);
    Ok(())
}

// ---- 5. `platform-impl`'s subprocess test ----

/// It opens a window, pumps events, runs one call on the main thread and one on the
/// present thread, and closes. The process's `main` is the test: a harness thread is not
/// the main thread.
fn subprocess_main() -> Result<()> {
    let (main, pump): (Lane<MainThread>, EventPump) = event_pump(WindowDesc {
        title: "pump".to_owned(),
        size: PixelSize { width: 320, height: 240 },
    })?;
    let platform = ClientPlatform::new();
    let (present, thread) = present_thread(&platform, 0u32)?;
    // A plain thread stands where the client's tasks do.
    let driver = StdPlatform.spawn_thread(
        ThreadSpec {
            name: "driver".to_owned(),
            class: lactose_core::ThreadClass::Background,
            index_in_class: 0,
        },
        Box::new(move || {
            let executor = Executor::builder().build().expect("an executor");
            executor
                .block_on(async {
                    let on_main = main.run(|main| main.window().size()).await?;
                    let on_present = present
                        .run(|count| {
                            *count += 1;
                            std::thread::current().name().map(str::to_owned)
                        })
                        .await?;
                    assert_eq!(on_present.as_deref(), Some("present"));
                    assert_eq!(thread.stop().await?, 1);
                    main.run(|main| main.exit()).await
                })
                .expect("both calls ran");
            executor.shutdown();
        }),
    )?;
    pump.run()?;
    driver.join();
    Ok(())
}

// ---- 6. A test's own platform: the present thread is made through any `Platform` ----

/// A spy that records what it was asked for and makes the thread as the standard
/// platform does.
struct Spy {
    specs: std::sync::Mutex<Vec<ThreadSpec>>,
}

impl Platform for Spy {
    fn topology(&self) -> CoreTopology {
        StdPlatform.topology()
    }

    fn spawn_thread(
        &self,
        spec: ThreadSpec,
        body: Box<dyn FnOnce() + Send + 'static>,
    ) -> Result<ThreadHandle> {
        self.specs.lock().expect("no test panics with it held").push(spec.clone());
        StdPlatform.spawn_thread(spec, body)
    }
}

fn through_a_spy(spy: &Spy) -> Result<()> {
    let (lane, thread) = present_thread(spy, String::new())?;
    let specs = spy.specs.lock().expect("no test panics with it held");
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].name, "present");
    assert_eq!(specs[0].class, lactose_core::ThreadClass::Present);
    assert_eq!(specs[0].index_in_class, 0);
    drop(lane);
    drop(thread);
    Ok(())
}
