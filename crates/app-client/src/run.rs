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
use lactose_core::{Error, ErrorKind, Result, ThreadClass, Tick};
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
use lactose_task::{Executor, Lane, Platform, ThreadSpec, WorkerClass};

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

/// The bound of each wait for a frame's point in the fixed-frame mode, of the render side's
/// drop (step 4 of "Shutdown"), and of the root's wait after the pump stopped with an error.
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

/// What a run did. The four counts of frames that drew are the present side's, in both
/// modes: each such frame is queued, held, shed or dropped.
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
    /// Frames that drew and were dropped, neither queued, held nor shed: behind a present
    /// call in flight on a present queue of its own.
    pub dropped: u64,
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
        exit(&self.0);
    }
}

/// Queues the pump's `exit`, step 1 of "Shutdown", as the frames end: from here on the pump
/// warns of a stall in the steps after it. It is not awaited; a second call does nothing.
fn exit(main: &Lane<MainThread>) {
    drop(main.run(MainThread::exit));
}

/// Drops `value` on a thread of its own, made through `platform`, and waits within `bound`
/// for the drop to end: step 4 of "Shutdown", since the drop may block and no worker may.
/// Returns whether it ended. Past the bound the thread is left with `value`, which is never
/// freed. If the platform makes no thread, `value` is dropped on the calling thread, where
/// the drop may block.
fn drop_within<T: Send + 'static>(platform: &dyn Platform, value: T, bound: Duration) -> bool {
    let (dropped, ended) = mpsc::channel();
    let spec = ThreadSpec {
        name: "render-drop".to_owned(),
        class: ThreadClass::Background,
        index_in_class: 0,
    };
    let body = Box::new(move || {
        drop(value);
        let _ = dropped.send(());
    });
    // A platform that makes no thread drops the body, and `value` with it, before it
    // returns: the sender is gone, and the wait below ends at once.
    drop(platform.spawn_thread(spec, body));
    !matches!(ended.recv_timeout(bound), Err(mpsc::RecvTimeoutError::Timeout))
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
    // driver ends within its own bounds, a frame's wait and step 4's each `BOUND`, a step's
    // and `Shown::stop`'s each the present side's: past this one `main` returns without it.
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
    /// The present lane, which the shutdown does not need: a call queued on it runs once
    /// every call queued before it has run and been replied to, so a test awaits one as a
    /// barrier behind the present calls so far.
    pub barrier: Lane<Box<dyn PresentSide>>,
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
    let barrier = lane.clone();
    let cooked = Arc::new(InMemory::new()?);
    let built = Frames::new(&cooked).and_then(|frames| {
        let cooked = cooked as Arc<dyn Cooked>;
        let shown =
            build_shown(executor, &GAME, world, Start::Fresh, cooked, Box::new(render), lane)?;
        Ok((shown, frames))
    });
    match built {
        Ok((shown, frames)) => Ok(Wired { shown, present: thread, frames, input, barrier }),
        Err(error) => {
            // The lane's calls are gone with the failed build, and the present side's
            // `close` was never called: its state comes back and drops here.
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
    let exit_on_drop = ExitOnDrop(main.clone());
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
        // Which driver ran, and whether the roles share a queue: what a run's record needs.
        tracing::info!(target: LOG_TARGET, "{:?}", device.info());
        tracing::info!(target: LOG_TARGET, "{:?}", device.capabilities().queues);
        let swapchain = match swapchain {
            Ok(swapchain) => swapchain,
            Err(error) => {
                // Steps 1 and 5 of "Shutdown" on this path too.
                exit(&main);
                drop(executor.block_on(main.run(move |_: &mut MainThread| drop(device))));
                return Err(error);
            }
        };
        let ran = run_on(executor, &main, &device, swapchain, &shaders, &cache, reports, options);
        // Step 1, which `run_on` queued already on every path that ran frames, and step 5 of
        // "Shutdown": the last device, in a main-lane call, where its surface is dropped.
        exit(&main);
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
    drop(exit_on_drop);
    ran.map(|()| outcome)
}

/// Steps 4 to 7 of "What `lactose-app-client` wires" on `device`, then steps 1 to 4 of
/// "Shutdown": every device clone but the caller's is gone when it returns. A render side
/// whose drop outlives step 4's bound ends the process instead, in either mode.
#[expect(clippy::too_many_arguments, reason = "the run's parts, each made by `drive`")]
fn run_on(
    executor: &Executor,
    main: &Lane<MainThread>,
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
    let Wired { shown, present, frames, input, .. } =
        wire(executor, &**executor.platform(), device, swapchain, shaders, world, readback)?;
    // Written once the renderer's pipelines are made; a cache that cannot be written costs
    // the next run its pipelines' time and nothing else.
    if let Err(error) = device.pipeline_cache().and_then(|bytes| cache.store(&bytes)) {
        tracing::warn!(target: LOG_TARGET, "the pipeline cache was not written: {error}");
    }
    let ran = match &options.mode {
        Mode::Fixed { frames: steps, .. } => {
            fixed_frames(executor, main, &**device, shown, &input, frames, *steps, &mut outcome)
        }
        Mode::Paced { seconds } => {
            let root = paced(executor.clone(), main.clone(), shown, reports, frames, *seconds);
            executor.block_on(executor.spawn(WorkerClass::FrameCritical, root)).and_then(|result| {
                let (ended, counted) = result?;
                outcome = counted;
                Ok(ended)
            })
        }
    };
    // "Shutdown": step 1, `exit`, was queued as the frames ended, and step 2 was
    // `Shown::stop`, whose last present-thread call was `PresentSide::close`. Step 3: the
    // present thread. A root that failed to join queues `exit` here.
    exit(main);
    drop(executor.block_on(present.stop()));
    let Ended { server, render, error } = ran?;
    let compared = match (&options.mode, &render) {
        (Mode::Fixed { dump, .. }, Some(render)) if error.is_none() => {
            compare_last(&**render, dump, &mut outcome)
        }
        _ => Ok(()),
    };
    // Step 4: the render side's drop, which waits for its last points (the graph's drop
    // waits for its timeline with no bound), on a thread of its own within `BOUND`.
    if let Some(render) = render
        && !drop_within(&**executor.platform(), render, BOUND)
    {
        let error = error.as_ref().map_or_else(String::new, |error| format!(" after {error}"));
        tracing::error!(
            target: LOG_TARGET,
            "the render side's drop did not end within {BOUND:?}{error}: the device is not \
             answering"
        );
        // What the drop holds keeps the window, so the pump never returns
        // (`crates/platform/README.md`, "Shutdown"): in either mode the process ends here,
        // its teardown not run, rather than wait under the pump's warning to be killed.
        eprintln!("lactose-app-client: the device did not answer within {BOUND:?}{error}");
        std::process::exit(1);
    }
    if let Some(server) = server {
        server.shutdown();
    }
    if let Some(error) = error {
        return Err(error);
    }
    compared.map(|()| outcome)
}

/// What a run of frames leaves where it ends early with `error` and the world is still
/// whole: `Shown::stop`'s.
fn stop_with(executor: &Executor, shown: Shown, error: Error) -> Result<Ended> {
    let Stopped { server, render } = executor.block_on(shown.stop())?;
    Ok(Ended { server: Some(server), render: Some(render), error: Some(error) })
}

/// The fixed-frame mode's steps: one step and its frame at a time, each joined and its point
/// waited for, so that a region is free at every step and every tick's frame is drawn. A
/// step that did not draw fails the mode, as does a frame whose point is not reached within
/// `BOUND`; on every path `exit` is queued as the frames end and the world stopped.
#[expect(clippy::too_many_arguments, reason = "the run's parts, each made by `run_on`")]
fn fixed_frames(
    executor: &Executor,
    main: &Lane<MainThread>,
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
            Err(failure) => {
                exit(main);
                return executor.block_on(ended(failure));
            }
        };
        shown = next;
        outcome.frames += 1;
        let Rendered::Drew { point, queued, held, shed, .. } = rendered else {
            exit(main);
            let error = Error::new(
                ErrorKind::Unsupported,
                format!("fixed frame {step} drew nothing: {rendered:?}"),
            );
            return stop_with(executor, shown, error);
        };
        outcome.count(queued, held, shed);
        let waited = device.wait_timeline(point, BOUND).and_then(|reached| {
            reached.then_some(()).ok_or_else(|| {
                Error::new(
                    ErrorKind::Cancelled,
                    format!("fixed frame {step}'s point was not reached within {BOUND:?}"),
                )
            })
        });
        if let Err(error) = waited {
            exit(main);
            return stop_with(executor, shown, error);
        }
    }
    exit(main);
    let Stopped { server, render } = executor.block_on(shown.stop())?;
    Ok(Ended { server: Some(server), render: Some(render), error: None })
}

impl Outcome {
    /// A frame that drew, as the present side took it.
    fn count(&mut self, queued: bool, held: bool, shed: bool) {
        self.drew += u64::from(queued);
        self.held += u64::from(held);
        self.shed += u64::from(shed);
        self.dropped += u64::from(!(queued || held || shed));
    }
}

/// Decision 5.1's comparison: the last frame dumped; the records the device's scatter wrote
/// held to the placements extract handed the renderer, of which the scene's
/// `game_sim::SCENE_ROWS` are live; the reference rasterised from those placements, every
/// instance surely drawn somewhere in it; and the pixels compared.
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
    let live = reference::held_to(&last.records, &last.expected)?;
    if live != game_sim::SCENE_ROWS as usize {
        let rows = game_sim::SCENE_ROWS;
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("{live} instances are live, not the scene's {rows}"),
        ));
    }
    let expected = reference::reference(&last.expected, &last.camera, last.fraction, last.extent)?;
    let surely = reference::surely_drawn(&expected);
    outcome.surely = Some(surely);
    if surely.contains(&0) {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("an instance is surely drawn at no pixel: {surely:?}"),
        ));
    }
    outcome.counts = Some(reference::compare(&last.pixels, &expected, last.extent)?);
    Ok(())
}

/// The paced mode's root, on a frame-critical worker: one sleep to the next tick's deadline,
/// one frame, until the window is closed or the run's seconds have passed. On every path
/// `exit` is queued as the frames end and the world stopped.
async fn paced(
    executor: Executor,
    main: Lane<MainThread>,
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
        if let Err(error) = executor.sleep_until(deadline).await {
            exit(&main);
            let Stopped { server, render } = shown.stop().await?;
            let ended = Ended { server: Some(server), render: Some(render), error: Some(error) };
            return Ok((ended, outcome));
        }
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
                    outcome.count(queued, held, shed);
                }
                outcome.busy += u64::from(matches!(output.deferred, Some(Rendered::Busy)));
            }
            Err(failure) => {
                exit(&main);
                return Ok((ended(failure).await?, outcome));
            }
        }
    }
    exit(&main);
    let Stopped { server, render } = shown.stop().await?;
    Ok((Ended { server: Some(server), render: Some(render), error: None }, outcome))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use lactose_task::StdPlatform;

    use super::drop_within;

    /// A value whose drop waits for a word on a channel, then says it ended.
    struct Blocking {
        go: mpsc::Receiver<()>,
        ended: mpsc::Sender<()>,
    }

    impl Drop for Blocking {
        fn drop(&mut self) {
            let _ = self.go.recv();
            let _ = self.ended.send(());
        }
    }

    /// Step 4's bound: a drop that ends is waited for; one that blocks past the bound is
    /// left on its thread, which ends with it once it is let go.
    #[test]
    fn a_drop_is_waited_for_within_its_bound_and_left_past_it() {
        let (go, wait) = mpsc::channel();
        let (sent, ended) = mpsc::channel();
        go.send(()).expect("a word");
        let value = Blocking { go: wait, ended: sent.clone() };
        assert!(drop_within(&StdPlatform, value, Duration::from_secs(60)));
        ended.try_recv().expect("the drop ended before the wait did");

        let (go, wait) = mpsc::channel();
        let value = Blocking { go: wait, ended: sent };
        assert!(!drop_within(&StdPlatform, value, Duration::from_millis(50)));
        assert!(ended.try_recv().is_err(), "the drop is still blocked");
        go.send(()).expect("the blocked drop let go");
        ended.recv_timeout(Duration::from_secs(60)).expect("the left thread ends");
    }
}
