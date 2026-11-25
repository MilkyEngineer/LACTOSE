//! The binary's run (`crates/app/README.md`, "What `lactose-app-client` wires", steps 1 to
//! 8; `crates/platform/README.md`, "A client binary, by thread" and "Shutdown"). The main
//! thread makes the window and pumps its events; a driver thread of the binary's own, never a
//! worker, makes the rest, runs the frames and shuts down in order. The paced mode runs its
//! frames from a root task on a frame-critical worker; the fixed-frame mode of decision 5.1
//! steps them from the driver thread, each joined and its point waited for.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use lactose_app::{
    Cooked, PresentSide, RenderSide, Rendered, Server, Shown, ShownError, ShownKept, Start,
    Stopped, TICK_PERIOD, Workers, WorldConfig, build_shown, start_executor,
};
use lactose_asset::{Store, pack_set};
use lactose_core::{Error, ErrorKind, Result, Tick};
use lactose_input_host::{LatestSample, RawSample};
use lactose_io::{Io, IoConfig};
use lactose_platform::{
    ClientPlatform, MainThread, PixelSize, PresentThread, WindowDesc, WindowReports, event_pump,
    present_thread,
};
use lactose_rhi_api::{
    Device, DeviceRequest, Extent, HEAP_SLOTS_DEFAULT, Message, MessageSink, SAMPLERS_DEFAULT,
    Swapchain, SwapchainDesc,
};
use lactose_rhi_vulkan::VulkanDevice;
use lactose_task::{Executor, Lane, Platform, WorkerClass};

use crate::game::{Frames, GAME, InMemory};
use crate::present::ClientPresent;
use crate::reference::{self, Counts};
use crate::render::ClientRender;
use crate::shaders::{CookedShaders, PipelineCache};

/// The log target of the binary's run.
pub const LOG_TARGET: &str = "lactose_app_client";

/// The fixed-frame mode's extent: the window's and the target's.
pub const FIXED_EXTENT: Extent = Extent { width: 512, height: 384 };

/// The fixed-frame mode's fraction: from the second step on, a frame's pair is the last two
/// ticks stepped, so any fixed fraction is reproducible.
pub const FIXED_FRACTION: f32 = 0.5;

/// The fixed-frame mode's sample: the camera turned to no side and looking down at 0.5
/// radians, so that the scene's circle is seen from above its plane.
pub const FIXED_SAMPLE: RawSample = RawSample { yaw: 0.0, pitch: -0.5, ..RawSample::IDLE };

/// The bound of each wait for a frame's point in the fixed-frame mode, and of the root's
/// wait after the pump stopped with an error.
const BOUND: Duration = Duration::from_secs(10);

/// How the binary runs its frames.
#[derive(Clone, Debug)]
pub enum Mode {
    /// Paced by the tick's deadlines, until the window is closed or, with `Some`, that many
    /// seconds have passed.
    Paced {
        /// The run's length, or `None` to run until the window is closed.
        seconds: Option<u32>,
    },
    /// Decision 5.1's fixed-frame mode: `frames` steps at [`FIXED_EXTENT`], each drawn, then
    /// the last frame dumped into `dump` and held to the CPU reference.
    Fixed {
        /// How many steps.
        frames: u32,
        /// Where the dump is written.
        dump: PathBuf,
    },
}

/// What the binary is given.
#[derive(Clone, Debug)]
pub struct Options {
    /// The cooked shaders' directory.
    pub shaders: PathBuf,
    /// The pack set's directory, mounted at start, or `None`.
    pub packs: Option<PathBuf>,
    /// The pipeline cache file.
    pub cache: PathBuf,
    /// How the frames run.
    pub mode: Mode,
}

/// What a run did.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    /// The packs mounted.
    pub packs: usize,
    /// The frames run.
    pub frames: u64,
    /// The frames that drew and were queued to be shown.
    pub drew: u64,
    /// Frames that drew and were held behind a present call on a shared queue, so dropped.
    pub held: u64,
    /// Frames that drew and were shed.
    pub shed: u64,
    /// Frames that ran their ticks alone, the frame before still on the render side.
    pub busy: u64,
    /// The fixed-frame mode's comparison of the last frame with the reference.
    pub counts: Option<Counts>,
    /// The fixed-frame mode's pixels surely drawn in each slot's colour.
    pub surely: Option<[usize; 7]>,
    /// Messages the device's validators sent.
    pub messages: usize,
}

/// Counts and logs every message a validator sends. Criterion 1 is met only where none is
/// sent.
#[derive(Default)]
struct Messages {
    count: AtomicUsize,
}

impl MessageSink for Messages {
    fn message(&self, message: &Message<'_>) {
        self.count.fetch_add(1, Ordering::Relaxed);
        tracing::warn!(
            target: LOG_TARGET,
            "{:?} {:?} {}: {}",
            message.source,
            message.severity,
            message.id,
            message.text
        );
    }
}

/// Queues the pump's `exit` when dropped, an unwind included. It is not awaited.
struct ExitOnDrop(Lane<MainThread>);

impl Drop for ExitOnDrop {
    fn drop(&mut self) {
        drop(self.0.run(MainThread::exit));
    }
}

/// The window's size as it is made: the fixed-frame mode's extent, or 1280 by 720.
fn window_size(mode: &Mode) -> PixelSize {
    match mode {
        Mode::Fixed { .. } => PixelSize { width: FIXED_EXTENT.width, height: FIXED_EXTENT.height },
        Mode::Paced { .. } => PixelSize { width: 1280, height: 720 },
    }
}

fn extent_of(size: PixelSize) -> Option<Extent> {
    (size.width > 0 && size.height > 0).then_some(Extent { width: size.width, height: size.height })
}

/// Runs the binary on the main thread, which `main` calls it on: winit needs the main thread
/// for the window and its events (decision 5.1).
///
/// # Errors
///
/// The window's, the device's, the frame's and the comparison's; `Cancelled` where the
/// driver thread did not end within its bound after the pump stopped with an error.
pub fn run(options: Options) -> Result<Outcome> {
    let size = window_size(&options.mode);
    let (main, pump) = event_pump(WindowDesc { title: "LACTOSE".to_owned(), size })?;
    let platform: Arc<dyn Platform> = Arc::new(ClientPlatform::new());
    let executor =
        start_executor(Workers { platform: Some(Arc::clone(&platform)), ..Workers::default() })?;
    let (sender, receiver) = mpsc::channel();
    let driver = {
        let executor = executor.clone();
        let main = main.clone();
        std::thread::Builder::new()
            .name("driver".to_owned())
            .spawn(move || {
                let outcome = drive(&executor, main, &platform, &options);
                let _ = sender.send(outcome);
            })
            .map_err(|error| Error::new(ErrorKind::Io, format!("the driver thread: {error}")))?
    };
    drop(main);
    let pumped = pump.run();
    // After an error from `run` the steps' main-lane calls resolve as cancelled, and the
    // driver ends within its own bounds; past this one `main` returns without it.
    let outcome = match receiver.recv_timeout(3 * BOUND) {
        Ok(outcome) => {
            let _ = driver.join();
            outcome
        }
        Err(_) => Err(Error::new(ErrorKind::Cancelled, "the driver thread outlived its bound")),
    };
    executor.shutdown();
    pumped?;
    outcome
}

/// What a run of frames leaves for the shutdown: the render side and the world if it
/// survived, and the error that ended it.
struct Ended {
    server: Option<Server>,
    render: Option<Box<dyn RenderSide>>,
    error: Option<Error>,
}

/// What a failed frame leaves: a `ShownError` does not convert into a plain error, so it is
/// matched on what it kept and the render side is never dropped on a worker.
async fn ended(failure: ShownError) -> Result<Ended> {
    let ShownError { error, kept } = failure;
    match kept {
        ShownKept::Shown(kept) => {
            let Stopped { server, render } = kept.stop().await?;
            Ok(Ended { server: Some(server), render: Some(render), error: Some(error) })
        }
        ShownKept::Render(render) => {
            Ok(Ended { server: None, render: Some(render), error: Some(error) })
        }
        ShownKept::Nothing => Err(error),
    }
}

/// The shown world over a device, and what its shutdown needs (`crates/app/README.md`, "What
/// `lactose-app-client` wires", steps 4 to 6).
pub struct Wired {
    /// The shown world, the render side and the present lane installed.
    pub shown: Shown,
    /// The present thread, which the shutdown stops after `Shown::stop`.
    pub present: PresentThread<Box<dyn PresentSide>>,
    /// The game's input, tick by tick.
    pub frames: Frames,
    /// What the frame's latch reads, which the binary publishes.
    pub input: Arc<LatestSample>,
}

/// Wires the game onto `device`: the render side, the present side over `swapchain` on a
/// present thread `platform` makes, and `build_shown` with `world` and a fresh start. With `readback`, each frame at that extent is also copied for
/// [`ClientRender::last_frame`]. Called off the workers: a failure stops the present
/// thread, which blocks.
///
/// # Errors
///
/// The render side's, the present side's, the present thread's and `build_shown`'s.
pub fn wire(
    executor: &Executor,
    platform: &dyn Platform,
    device: &Arc<dyn Device>,
    swapchain: Box<dyn Swapchain>,
    shaders: &CookedShaders,
    world: WorldConfig,
    readback: Option<Extent>,
) -> Result<Wired> {
    let input = Arc::new(LatestSample::new());
    let render = ClientRender::new(device, shaders, Arc::clone(&input), readback)?;
    let present = ClientPresent::new(Arc::clone(device), swapchain, shaders)?;
    let (lane, thread) = present_thread(platform, Box::new(present) as Box<dyn PresentSide>)?;
    let cooked = Arc::new(InMemory::new()?);
    let built = Frames::new(&cooked).and_then(|frames| {
        let cooked = cooked as Arc<dyn Cooked>;
        let shown =
            build_shown(executor, &GAME, world, Start::Fresh, cooked, Box::new(render), lane)?;
        Ok((shown, frames))
    });
    match built {
        Ok((shown, frames)) => Ok(Wired { shown, present: thread, frames, input }),
        Err(error) => {
            // The lane is gone with the failed build, and the present side's `close` was
            // never called: its state comes back and drops here.
            drop(executor.block_on(thread.stop()));
            Err(error)
        }
    }
}

/// The driver thread: everything but the pump, in `main`'s order, then the shutdown's.
fn drive(
    executor: &Executor,
    main: Lane<MainThread>,
    platform: &Arc<dyn Platform>,
    options: &Options,
) -> Result<Outcome> {
    let exit = ExitOnDrop(main.clone());
    let mut outcome = Outcome::default();
    let shaders = CookedShaders::read(&options.shaders)?;
    // The cooked pack set (D136). Nothing in it is read at milestone 2: the scene enters by
    // `game-sim`'s session applier, and its mesh reference names no asset a pack holds.
    let io = Io::start(&**platform, IoConfig::for_platform(&**platform))?;
    let store = Store::new(executor.clone(), io.clone(), []).and_then(|store| {
        if let Some(directory) = &options.packs {
            for pack in executor.block_on(pack_set(&io, directory.clone()))? {
                executor.block_on(store.mount(pack))?;
                outcome.packs += 1;
            }
        }
        Ok(store)
    });
    let ran = store.and_then(|store| {
        // The device on the window and its swapchain, made in a main-lane call: the window
        // lends its handles on the main thread alone.
        let messages = Arc::new(Messages::default());
        let cache = PipelineCache::at(&options.cache);
        // A window reported empty at its making is swapchained at the size it was asked
        // for, and the first frame's extent resizes it.
        let asked = window_size(&options.mode);
        let fallback = Extent { width: asked.width, height: asked.height };
        let request = DeviceRequest {
            adapter: None,
            surface: None,
            // The capacities the cooked shaders' heap arrays are compiled for
            // (`lactose.heap`).
            heap_slots: HEAP_SLOTS_DEFAULT,
            samplers: SAMPLERS_DEFAULT,
            pipeline_cache: cache.load(),
            messages: Arc::clone(&messages) as Arc<dyn MessageSink>,
        };
        let made = executor.block_on(main.run(move |main: &mut MainThread| {
            let window = main.window();
            let reports = window.reports();
            let extent = extent_of(window.size()).unwrap_or(fallback);
            let device = VulkanDevice::new(DeviceRequest { surface: Some(window), ..request })?;
            let device = Arc::new(device) as Arc<dyn Device>;
            let swapchain = device.create_swapchain(&SwapchainDesc { extent });
            Ok::<_, Error>((device, swapchain, reports))
        }))??;
        let (device, swapchain, reports) = made;
        let swapchain = match swapchain {
            Ok(swapchain) => swapchain,
            Err(error) => {
                // Step 5 of "Shutdown" on this path too.
                drop(executor.block_on(main.run(move |_: &mut MainThread| drop(device))));
                return Err(error);
            }
        };
        let ran = run_on(executor, &device, swapchain, &shaders, &cache, reports, options);
        // Step 5 of "Shutdown": the last device, in a main-lane call, where its surface is
        // dropped.
        let dropped = executor.block_on(main.run(move |_: &mut MainThread| drop(device)));
        drop(store);
        let counted = ran?;
        dropped?;
        outcome = Outcome {
            packs: outcome.packs,
            messages: messages.count.load(Ordering::Relaxed),
            ..counted
        };
        Ok(())
    });
    io.shutdown();
    drop(exit);
    ran.map(|()| outcome)
}

/// Steps 4 to 7 of "What `lactose-app-client` wires" on `device`, then steps 2 to 4 of
/// "Shutdown": every device clone but the caller's is gone when it returns.
fn run_on(
    executor: &Executor,
    device: &Arc<dyn Device>,
    swapchain: Box<dyn Swapchain>,
    shaders: &CookedShaders,
    cache: &PipelineCache,
    reports: WindowReports,
    options: &Options,
) -> Result<Outcome> {
    let mut outcome = Outcome::default();
    let readback = matches!(options.mode, Mode::Fixed { .. }).then_some(FIXED_EXTENT);
    let world = WorldConfig::shown();
    let Wired { shown, present, frames, input } =
        wire(executor, &**executor.platform(), device, swapchain, shaders, world, readback)?;
    // Written once the renderer's pipelines are made; a cache that cannot be written costs
    // the next run its pipelines' time and nothing else.
    if let Err(error) = device.pipeline_cache().and_then(|bytes| cache.store(&bytes)) {
        tracing::warn!(target: LOG_TARGET, "the pipeline cache was not written: {error}");
    }
    let ran = match &options.mode {
        Mode::Fixed { frames: steps, .. } => {
            fixed_frames(executor, &**device, shown, &input, frames, *steps, &mut outcome)
        }
        Mode::Paced { seconds } => {
            let root = paced(executor.clone(), shown, reports, frames, *seconds);
            executor.block_on(executor.spawn(WorkerClass::FrameCritical, root)).and_then(|result| {
                let (ended, counted) = result?;
                outcome = counted;
                Ok(ended)
            })
        }
    };
    // "Shutdown", step 2 was `Shown::stop`, whose last present-thread call was
    // `PresentSide::close`. Step 3: the present thread.
    drop(executor.block_on(present.stop()));
    let Ended { server, render, error } = ran?;
    let compared = match (&options.mode, &render) {
        (Mode::Fixed { dump, .. }, Some(render)) if error.is_none() => {
            compare_last(&**render, dump, &mut outcome)
        }
        _ => Ok(()),
    };
    // Step 4, on this thread, which is no worker: the render side's drop waits, within a
    // bound, for its last points.
    drop(render);
    if let Some(server) = server {
        server.shutdown();
    }
    if let Some(error) = error {
        return Err(error);
    }
    compared.map(|()| outcome)
}

/// The fixed-frame mode's steps: one step and its frame at a time, each joined and its point
/// waited for, so that a region is free at every step and every tick's frame is drawn. A
/// step that did not draw fails the mode.
fn fixed_frames(
    executor: &Executor,
    device: &dyn Device,
    mut shown: Shown,
    input: &LatestSample,
    mut frames: Frames,
    steps: u32,
    outcome: &mut Outcome,
) -> Result<Ended> {
    let start = Instant::now();
    for step in 0..steps {
        input.publish(FIXED_SAMPLE);
        let tick = shown.server().tick();
        let time =
            lactose_app::FrameTime { now: start + TICK_PERIOD * step, fraction: FIXED_FRACTION };
        let stepped =
            executor.block_on(shown.step(executor, frames.next(tick), time, Some(FIXED_EXTENT)));
        let (next, _output, rendered) = match stepped {
            Ok(stepped) => stepped,
            Err(failure) => return executor.block_on(ended(failure)),
        };
        shown = next;
        outcome.frames += 1;
        let Rendered::Drew { point, .. } = rendered else {
            let Stopped { server, render } = executor.block_on(shown.stop())?;
            let error = Error::new(
                ErrorKind::Unsupported,
                format!("fixed frame {step} drew nothing: {rendered:?}"),
            );
            return Ok(Ended { server: Some(server), render: Some(render), error: Some(error) });
        };
        outcome.drew += 1;
        if !device.wait_timeline(point, BOUND)? {
            return Err(Error::new(
                ErrorKind::Cancelled,
                format!("fixed frame {step}'s point was not reached"),
            ));
        }
    }
    let Stopped { server, render } = executor.block_on(shown.stop())?;
    Ok(Ended { server: Some(server), render: Some(render), error: None })
}

/// Decision 5.1's comparison: the last frame dumped, rasterised and compared.
fn compare_last(
    render: &dyn RenderSide,
    dump: &std::path::Path,
    outcome: &mut Outcome,
) -> Result<()> {
    let render: &dyn std::any::Any = render;
    let render = render
        .downcast_ref::<ClientRender>()
        .ok_or_else(|| Error::new(ErrorKind::InvalidData, "the render side is not the client's"))?;
    let last = render
        .last_frame()?
        .ok_or_else(|| Error::new(ErrorKind::InvalidData, "no fixed frame was drawn"))?;
    reference::dump(&last, dump)?;
    let expected = reference::reference(&last.records, &last.camera, last.fraction, last.extent)?;
    outcome.surely = Some(reference::surely_drawn(&expected));
    outcome.counts = Some(reference::compare(&last.pixels, &expected, last.extent)?);
    Ok(())
}

/// The paced mode's root, on a frame-critical worker: one sleep to the next tick's deadline,
/// one frame, until the window is closed or the run's seconds have passed.
async fn paced(
    executor: Executor,
    mut shown: Shown,
    reports: WindowReports,
    mut frames: Frames,
    seconds: Option<u32>,
) -> Result<(Ended, Outcome)> {
    let mut outcome = Outcome::default();
    let until = seconds.map(|seconds| Instant::now() + Duration::from_secs(u64::from(seconds)));
    let mut next = move |tick: Tick| Some(frames.next(tick));
    let mut deadline = Instant::now();
    loop {
        executor.sleep_until(deadline).await?;
        if reports.close_requested() || until.is_some_and(|until| Instant::now() >= until) {
            break;
        }
        let extent = extent_of(reports.size());
        match shown.frame(&executor, Instant::now(), &mut next, extent).await {
            Ok((stepped, output)) => {
                shown = stepped;
                outcome.frames += 1;
                deadline = output.world.next_deadline;
                if let Some(Rendered::Drew { queued, held, shed, .. }) = output.joined {
                    outcome.drew += u64::from(queued);
                    outcome.held += u64::from(held);
                    outcome.shed += u64::from(shed);
                }
                outcome.busy += u64::from(matches!(output.deferred, Some(Rendered::Busy)));
            }
            Err(failure) => return Ok((ended(failure).await?, outcome)),
        }
    }
    let Stopped { server, render } = shown.stop().await?;
    Ok((Ended { server: Some(server), render: Some(render), error: None }, outcome))
}
