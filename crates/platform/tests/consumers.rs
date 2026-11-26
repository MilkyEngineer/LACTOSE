//! Compile-only consumer sketches: what `lactose-app-client`, `lactose-app` and the
//! renderer, a graphics backend, the `swapchain` lane's tests and `platform-impl`'s
//! subprocess test write against this crate, type-checked and never run: the stand-ins for
//! `lactose-rhi-api` are `todo!()`, and the tests that run are `present`, `threads` and
//! `tests/window.rs`. They pin the surface the README promises: a change that breaks one is
//! a signature change to list.

#![allow(dead_code, unused_variables, missing_docs)]

use std::io::Read;
use std::mem::ManuallyDrop;
use std::process::{Command, Stdio};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use lactose_core::{Error, ErrorKind, Result};
use lactose_platform::{
    ClientPlatform, EventPump, MainThread, PixelSize, PresentThread, Window, WindowDesc,
    WindowReports, event_pump, present_thread,
};
use lactose_task::{
    CoreTopology, Executor, Lane, Platform, StdPlatform, ThreadHandle, ThreadSpec, WorkerClass,
};
use raw_window_handle::{HandleError, HasDisplayHandle, HasWindowHandle};

// ---- 0. Stand-ins for `lactose-rhi-api`, which this crate never names ----
//
// Each has the bounds and the receiver of the item it stands for on the branch `rhi-api`
// (`SurfaceSource`, `DeviceRequest::surface`, `Device`, `Swapchain`, `Rule`), and nothing
// else.

mod rhi {
    use std::sync::Arc;

    use lactose_core::{Error, Result};
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
        /// Refused as `Rule::Pending`, which the caller retries, while the submit it waits
        /// for is not yet released; the image is then still the acquired one.
        fn present(&mut self, image: u32) -> Result<Presented>;
        /// Refused as `Rule::Pending` while a present is pending; an acquired image is
        /// given up.
        fn resize(&mut self, extent: Extent) -> Result<()>;
    }

    /// What a backend refuses a call by.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Rule {
        Pending,
    }

    impl Rule {
        pub fn of(error: &Error) -> Option<Self> {
            todo!()
        }
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
    use std::time::{Duration, Instant};

    use lactose_core::Result;
    use lactose_task::{Executor, Lane};

    use super::rhi::{Acquired, Device, Extent, Presented, Rule, Swapchain};

    /// What the present thread owns. `Send` and not `Sync`, as the swapchain is: the lane
    /// asks for no more.
    pub struct Presenter {
        pub device: Option<Arc<dyn Device>>,
        pub swapchain: Option<Box<dyn Swapchain>>,
        /// The size last asked for, which is what a resize is decided against.
        pub asked: Option<Extent>,
        /// An image whose present was refused as pending, presented again at the next call.
        pub acquired: Option<u32>,
        pub presents: Cell<u64>,
    }

    impl Presenter {
        pub fn empty() -> Self {
            Self {
                device: None,
                swapchain: None,
                asked: None,
                acquired: None,
                presents: Cell::new(0),
            }
        }

        /// One present, on the present thread: every call here may block, within a bound,
        /// and none of them is made anywhere else.
        fn present(&mut self, size: Option<Extent>) -> Result<bool> {
            let (Some(device), Some(size)) = (&self.device, size) else { return Ok(false) };
            if self.asked != Some(size) {
                match &mut self.swapchain {
                    Some(swapchain) => match swapchain.resize(size) {
                        Ok(()) => {}
                        // `resize` has waited, within its bound, for the swapchain's
                        // presents; a refusal as pending past that bound is retried at the
                        // next call, not returned.
                        Err(error) if Rule::of(&error) == Some(Rule::Pending) => {
                            return Ok(false);
                        }
                        Err(error) => return Err(error),
                    },
                    None => self.swapchain = Some(device.create_swapchain(size)?),
                }
                // A resize gives up the image an earlier call kept.
                self.acquired = None;
                self.asked = Some(size);
            }
            let Some(swapchain) = &mut self.swapchain else { return Ok(false) };
            let image = match self.acquired.take() {
                Some(image) => image,
                None => match swapchain.acquire()? {
                    Acquired::Image(image) => image,
                    Acquired::None => return Ok(false),
                    Acquired::Stale => {
                        self.asked = None;
                        return Ok(false);
                    }
                },
            };
            match swapchain.present(image) {
                Ok(Presented::Shown) => {
                    self.presents.set(self.presents.get() + 1);
                    Ok(true)
                }
                Ok(Presented::Stale) => {
                    self.asked = None;
                    Ok(false)
                }
                // The image is still the acquired one: the next call presents it again,
                // and the refusal is not returned.
                Err(error) if Rule::of(&error) == Some(Rule::Pending) => {
                    self.acquired = Some(image);
                    Ok(false)
                }
                Err(error) => Err(error),
            }
        }

        /// The present thread's last call at shutdown, queued once every point a pending
        /// submit waits on that no submit made will signal is signalled (step 2). A
        /// client's first signals in the same way the points of a present of its own that
        /// was refused, then waits, within a bound, for the points its presents signalled;
        /// this stand-in keeps none. It drops the swapchain, a drop no caller bounds, which
        /// waits for the submits that use its images, and its clone of the device. What
        /// `stop` then hands back holds nothing of the window.
        pub fn wind_down(&mut self) {
            self.swapchain = None;
            self.device = None;
        }
    }

    /// What the frame reads of the window, as `lactose-app` would declare it.
    pub trait Shown: Send + Sync {
        /// `None` while nothing can be presented.
        fn extent(&self) -> Option<Extent>;
        fn close_requested(&self) -> bool;
    }

    /// The frame's presents: one call on the lane at a time, each awaited, so a present
    /// that blocks holds this task and no worker, and the lane never holds a second. A
    /// call that presented nothing paces nothing, so the frame then waits for its next
    /// deadline; in `lactose-app` the timer root paces the frame (architecture section 5.1).
    pub async fn frames(
        executor: &Executor,
        shown: &dyn Shown,
        present: &Lane<Presenter>,
    ) -> Result<u64> {
        const FRAME: Duration = Duration::from_millis(16);
        let mut presented = 0;
        while !shown.close_requested() {
            let size = shown.extent();
            if present.run(move |presenter| presenter.present(size)).await?? {
                presented += 1;
            } else {
                executor.sleep_until(Instant::now() + FRAME).await?;
            }
        }
        Ok(presented)
    }
}

// ---- 2. `lactose-app-client`: the one crate that names this one ----

/// The adapter from this crate's window to what the frame reads. It holds the reports and
/// no `Arc<Window>`, so wherever `app` keeps it, it keeps nothing open.
struct ShownWindow(WindowReports);

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

/// Queues `exit` when it is dropped, a panic's unwinding included, and does not wait for
/// it: the first step of every shutdown.
struct ExitOnDrop(Lane<MainThread>);

impl Drop for ExitOnDrop {
    fn drop(&mut self) {
        drop(self.0.run(|main| main.exit()));
    }
}

/// The device is made inside a main-lane call, which is where the window lends its
/// handles; the window's `Arc` goes to the backend as its surface source, and the frame
/// takes the reports alone.
async fn start(main: &Lane<MainThread>) -> Result<(Arc<dyn rhi::Device>, WindowReports)> {
    main.run(|main| {
        let window = main.window();
        let reports = window.reports();
        let surface: Arc<dyn rhi::SurfaceSource> = window;
        let device = rhi::create_device(rhi::DeviceRequest { surface: Some(surface) })?;
        Ok((device, reports))
    })
    .await?
}

/// The client's root task: start, frames, and the shutdown that every path takes, in the
/// order of `README.md`, "Shutdown".
async fn root(
    executor: Executor,
    main: Lane<MainThread>,
    present: Lane<frame::Presenter>,
    thread: PresentThread<frame::Presenter>,
) -> Result<u64> {
    let exit = ExitOnDrop(main.clone());
    let mut device = None;
    let ran = async {
        let (made, reports) = start(&main).await?;
        let on_thread = made.clone();
        device = Some(made);
        present.run(move |presenter| presenter.device = Some(on_thread)).await?;
        frame::frames(&executor, &ShownWindow(reports), &present).await
    }
    .await;
    // 1. `exit` first, not awaited: it destroys nothing while an `Arc<Window>` is held, and
    //    the pump drains main-lane calls until the last one goes.
    drop(exit);
    // 2. No present and no frame submit is queued from here. Every point a pending submit
    //    waits on that no submit made will signal is signalled from the host first: what
    //    the host owes, and the points of a submit that failed or will now not be made; the
    //    drop of an encoder the present side keeps waits for its submit. Then the present
    //    thread's last call, which signals the points of a refused present of its own.
    let last = present.run(frame::Presenter::wind_down).await;
    // 3. The state comes back with nothing of the window in it.
    let stopped = thread.stop().await.map(drop);
    // 4. On a thread of its own: the device's last points are waited for within a bound,
    //    and every encoder and every `Device` clone but one is dropped.
    let settled = match device {
        Some(device) => settle(&executor, device).await,
        None => Ok(None),
    };
    // 5. Only when a device came back. A main-lane call drops the last, which then waits
    //    for nothing, and MoltenVK's view observation is removed on the view's thread. After
    //    an expired step-4 bound none comes back: step 4's thread drops it.
    let dropped = match settled {
        Ok(Some(device)) => main.run(move |_| drop(device)).await,
        Ok(None) => Ok(()),
        Err(error) => Err(error),
    };
    // Every step has run. The start's or the frame's error first: a later step's
    // `Cancelled` follows from it.
    let frames = ran?;
    last.and(stopped).and(dropped)?;
    Ok(frames)
}

/// How long a step of the shutdown waits before it logs and goes on. After an error from
/// `run`, `main` waits three of them, longer than steps 2 and 4's bounds together.
const BOUND: Duration = Duration::from_secs(2);

/// Step 4, on a thread of its own made through the executor's platform, since it may block
/// and no worker may. The thread waits within the bound for the device's last points and
/// hands the device back for step 5. If the bound expires it logs that, hands back nothing,
/// and drops the last `Device` itself: a drop that then blocks holds that thread and the
/// window, and while the pump runs the main thread goes on answering under its warning.
/// The root looks for the answer on a timer, so no worker waits for the thread. If the
/// platform makes no thread, its error ends step 4 and the device is dropped on the calling
/// worker, where its drop may block; the pump's warning shows the stall.
async fn settle(
    executor: &Executor,
    device: Arc<dyn rhi::Device>,
) -> Result<Option<Arc<dyn rhi::Device>>> {
    let (answer, answered) = mpsc::channel();
    executor.platform().spawn_thread(
        ThreadSpec {
            name: "settle".to_owned(),
            class: lactose_core::ThreadClass::Background,
            index_in_class: 0,
        },
        Box::new(move || {
            if last_points(&*device, BOUND) {
                let _ = answer.send(Some(device));
            } else {
                warn_on_pail("the device's last points passed the shutdown's bound");
                let _ = answer.send(None);
                drop(device);
            }
        }),
    )?;
    loop {
        match answered.try_recv() {
            Ok(device) => return Ok(device),
            Err(mpsc::TryRecvError::Empty) => {
                executor.sleep_until(Instant::now() + POLL).await?;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err(Error::new(ErrorKind::Cancelled, "step 4's thread ended unanswered"));
            }
        }
    }
}

/// Stands for the backend's waits for the device's last points: `false` if the bound
/// expired, after which dropping the device may block.
fn last_points(device: &dyn rhi::Device, bound: Duration) -> bool {
    todo!()
}

/// Stands for the client's `tracing::warn!` on `lactose_platform::LOG_TARGET`.
fn warn_on_pail(message: &str) {
    todo!()
}

/// `main`: the pump is made first and run last. When `run` returns a value nothing after it
/// touches the window, which is gone.
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
    let task =
        executor.spawn(WorkerClass::FrameCritical, root(executor.clone(), main, present, thread));
    // 6. When `run` returns a value the last `Arc<Window>` is gone; `main` waits for its
    //    root task and shuts the executor down, and nothing that runs from here holds the
    //    device or the window.
    if let Err(error) = pump.run() {
        // The steps now run with no pump, and their main-lane calls resolve to `Cancelled`.
        // The root is awaited on a thread of its own, within a bound longer than steps 2
        // and 4's together; past it `main` logs and returns `run`'s error with the root
        // neither dropped nor shut down, and the process ends with it, as it does with the
        // leaked window. How the process ends past the bound is `app`'s choice. A platform
        // that makes no thread drops the body unrun, which leaks the root rather than
        // dropping it here.
        let (answer, answered) = mpsc::channel();
        let waiter = executor.clone();
        let task = ManuallyDrop::new(task);
        let waited = executor.platform().spawn_thread(
            ThreadSpec {
                name: "root".to_owned(),
                class: lactose_core::ThreadClass::Background,
                index_in_class: 0,
            },
            Box::new(move || {
                let task = ManuallyDrop::into_inner(task);
                let _ = answer.send(waiter.block_on(task));
            }),
        );
        match waited.map(|_| answered.recv_timeout(3 * BOUND)) {
            Ok(Ok(_)) => executor.shutdown(),
            _ => warn_on_pail("the root did not end within its bound after `run` failed"),
        }
        return Err(error);
    }
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

/// How long a test waits for the operating system to answer a request.
const ANSWER: Duration = Duration::from_secs(2);
/// How often it looks: a resize and a minimise wake no one.
const POLL: Duration = Duration::from_millis(10);

/// Frames presented across a resize: the test asks for a size on the main thread and
/// reads the report from where it stands. It returns the size given, or `None` if the
/// operating system gave none before the deadline, which the lane records as a refusal and
/// not as a failure.
async fn across_a_resize(
    executor: &Executor,
    main: &Lane<MainThread>,
    present: &Lane<frame::Presenter>,
    reports: &WindowReports,
) -> Result<Option<PixelSize>> {
    let before = reports.size();
    let asked = PixelSize { width: before.width / 2, height: before.height / 2 };
    main.run(move |main| main.request_size(asked)).await?;
    let deadline = Instant::now() + ANSWER;
    while reports.size() == before {
        if Instant::now() >= deadline {
            return Ok(None);
        }
        executor.sleep_until(Instant::now() + POLL).await?;
    }
    let size = reports.size();
    let presents = present
        .run(move |presenter| {
            presenter.asked = None;
            presenter.presents.get()
        })
        .await?;
    Ok(Some(size))
}

/// The drawable made to block: the window is minimised, and the call that blocks returns
/// how long it took, which is how the present thread records a block of a second. What
/// the operating system reported of the minimise comes back with it, so a refused
/// minimise is not read as a drawable that does not block.
async fn a_blocked_drawable(
    executor: &Executor,
    main: &Lane<MainThread>,
    present: &Lane<frame::Presenter>,
) -> Result<(Option<bool>, Duration)> {
    main.run(|main| main.set_minimised(true)).await?;
    let deadline = Instant::now() + ANSWER;
    let minimised = loop {
        let reported = main.run(|main| main.is_minimised()).await?;
        if reported != Some(false) || Instant::now() >= deadline {
            break reported;
        }
        executor.sleep_until(Instant::now() + POLL).await?;
    };
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
    Ok((minimised, blocked))
}

/// A shutdown with frames in flight, run on a background worker or on the test's own
/// thread and never on a frame worker: the present thread's last call is ordered behind
/// every call queued before it, the stop behind that, and the state that comes back holds
/// nothing of the window.
async fn with_frames_in_flight(
    present: Lane<frame::Presenter>,
    thread: PresentThread<frame::Presenter>,
) -> Result<u64> {
    let in_flight = present.run(|presenter| presenter.presents.get());
    let last = present.run(frame::Presenter::wind_down);
    let presenter = thread.stop().await?;
    let presents = in_flight.await?;
    last.await?;
    assert!(presenter.swapchain.is_none() && presenter.device.is_none());
    Ok(presents)
}

// ---- 5. `platform-impl`'s subprocess test, and its parent ----

/// The stage `subprocess_main` prints last when every check passed.
const FINAL_STAGE: &str = "the driver's result";

/// It opens a window, pumps events, runs one call on the main thread and one on the
/// present thread, and closes. The process's `main` is the test: a harness thread is not
/// the main thread. It prints each stage it reaches, for the parent, and returns the
/// driver's result, which is the subprocess's exit status.
fn subprocess_main() -> Result<()> {
    let (main, pump): (Lane<MainThread>, EventPump) = event_pump(WindowDesc {
        title: "pump".to_owned(),
        size: PixelSize { width: 320, height: 240 },
    })?;
    let platform = ClientPlatform::new();
    let (present, thread) = present_thread(&platform, 0u32)?;
    // `ThreadHandle::join` discards a panic, so the driver's result comes back on a channel.
    let (result, outcome) = mpsc::channel::<Result<()>>();
    // A plain thread stands where the client's tasks do.
    let driver = StdPlatform.spawn_thread(
        ThreadSpec {
            name: "driver".to_owned(),
            class: lactose_core::ThreadClass::Background,
            index_in_class: 0,
        },
        Box::new(move || {
            // Held before anything can fail: a check that fails below, by an error or by a
            // panic's unwinding, still queues `exit`, and the pump returns.
            let exit = ExitOnDrop(main.clone());
            let executor = Executor::builder().build().expect("an executor");
            let checked = executor.block_on(async move {
                let on_main = main.run(|main| main.window().size()).await?;
                println!("stage: a main-lane call ran");
                let on_present = present
                    .run(|count| {
                        *count += 1;
                        std::thread::current().name().map(str::to_owned)
                    })
                    .await?;
                println!("stage: a present-thread call ran");
                assert_eq!(on_present.as_deref(), Some("present"));
                // 1., then 3.: nothing was made from the window.
                drop(exit);
                assert_eq!(thread.stop().await?, 1);
                println!("stage: stopped");
                Ok::<_, Error>(())
            });
            executor.shutdown();
            // The driver's last act. A panic above drops `result` unsent, which the main
            // thread reads as a failure.
            let _ = result.send(checked);
        }),
    )?;
    println!("stage: pumping");
    let pumped = pump.run();
    println!("stage: the pump returned");
    driver.join();
    let checked = outcome.try_recv().unwrap_or_else(|_| {
        Err(Error::new(ErrorKind::Cancelled, "the driver ended without sending its result"))
    });
    pumped?;
    checked?;
    println!("stage: {FINAL_STAGE}");
    Ok(())
}

/// The parent's side, in the test harness: every window subprocess runs under a deadline.
/// It reads the child's output while it waits, so a child that writes more than a pipe
/// holds is not blocked into a false hang. Past the deadline it kills the child and fails,
/// naming the last stage the child printed; otherwise it checks the exit status and that
/// the last stage printed is the final one.
fn under_a_deadline(mut command: Command, deadline: Duration) {
    let mut child = command.stdout(Stdio::piped()).spawn().expect("the subprocess starts");
    let mut stdout = child.stdout.take().expect("the subprocess's stdout is piped");
    let (read, reader) = mpsc::channel();
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stdout.read_to_end(&mut output);
        let _ = read.send(String::from_utf8_lossy(&output).into_owned());
    });
    let until = Instant::now() + deadline;
    let status = loop {
        if let Some(status) = child.try_wait().expect("the subprocess is waited for") {
            break Some(status);
        }
        if Instant::now() >= until {
            child.kill().expect("the subprocess is killed");
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // The child has ended, but a process it started may still hold its end of the pipe, so
    // the read is waited for within a bound as well.
    let output = reader.recv_timeout(ANSWER).expect("stdout held open");
    let stage = output.lines().rev().find_map(|line| line.strip_prefix("stage: "));
    let Some(status) = status else {
        panic!(
            "the subprocess passed its deadline of {deadline:?} after the stage {:?}",
            stage.unwrap_or("none")
        )
    };
    assert!(status.success(), "the subprocess failed with {status} after the stage {stage:?}");
    assert_eq!(stage, Some(FINAL_STAGE), "the subprocess ended early");
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

/// A platform that accepts the body and drops it unrun: `present_thread` returns
/// `Cancelled` and does not hang.
struct Dropping;

impl Platform for Dropping {
    fn topology(&self) -> CoreTopology {
        StdPlatform.topology()
    }

    fn spawn_thread(
        &self,
        spec: ThreadSpec,
        body: Box<dyn FnOnce() + Send + 'static>,
    ) -> Result<ThreadHandle> {
        drop(body);
        StdPlatform.spawn_thread(spec, Box::new(|| {}))
    }
}

fn through_a_platform_that_drops_the_body() {
    let refused = present_thread(&Dropping, String::new());
    assert!(matches!(refused, Err(error) if error.kind() == ErrorKind::Cancelled));
}
