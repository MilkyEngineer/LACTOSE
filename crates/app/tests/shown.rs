//! What `lactose-app-client` and `frame-impl`'s tests write against the shown frame of
//! `lactose-app` (D162): the render side over the render graph and the renderer as merged,
//! the present side over rhi-api's `Swapchain`, the binary's root and its shutdown, the
//! fixed-frame mode of decision 5.1, criterion 3(c)'s two runs, and a render side with no
//! device behind it. Sections 1 to 5 are compile-only: they are type-checked and never
//! run, since `Shown`'s bodies are `todo!()` and so are the graph's and the renderer's.
//! They pin the surface `README.md` promises: a change that breaks one is a signature
//! change to list.
//!
//! Section 6 runs. It is the route by which the frame holds the render side and a
//! `JobContext` together with no lock (`crates/render-graph/README.md`, "What each
//! consumer needs"): a world of the frame's own, with no row and no region, whose one
//! service is a `dyn` object and whose schedule is one system that takes `Job` beside
//! `ResMut` of it, run as a pass inside a task, the world moved in and handed back. The
//! trait, the system and the function that builds that world are the amendment D162 asks
//! of `lactose-present-api`; the copies here are the test's own, as CLAUDE.md allows a
//! `#[system]` in tests alone, and they show the route against `lactose-ecs` as merged.

#![forbid(unsafe_code)]
#![allow(dead_code, unused_variables, missing_docs, clippy::too_many_arguments)]

use std::any::Any;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lactose_app::{
    Cooked, Draw, Finished, FrameCounts, FrameTime, Game, KindCounts, PresentLane, PresentReport,
    PresentSide, Region, RenderSide, Rendered, Server, Shown, ShownError, ShownKept, Start,
    Stopped, Submitted, TARGETS, TICK_PERIOD, TargetIndex, WorldConfig, build_server, build_shown,
};
use lactose_core::{Error, ErrorKind, Handle, Result, StableKey, Tick};
use lactose_ecs::{
    ClaimedBy, InRegion, Job, OwnedBy, PredictedOrigin, ResMut, Schedule, World, WorldBuilder,
    WorldKind, system,
};
use lactose_input_host::{LatestSample, RawSample};
use lactose_math::render::RenderPosition;
use lactose_present_api::{ExtractPass, InstanceRow, InstanceSink, PresentFrame};
use lactose_reflect::Registry;
use lactose_render_graph::{FrameDecl, ImageImport, RecordedFrame, RenderGraph};
use lactose_renderer::{
    CameraLatch, FrameStats, FrameView, Opened, Projection, RemoteWriter, Renderer,
};
use lactose_rhi_api::{
    Acquired, Device, Encoder, Extent, ImageHandle, ImageRange, ImageState, ImageTransition,
    Presented, QueueRole, Rule, Sharing, StateOn, Submit, Swapchain, TimelineHandle, TimelinePoint,
};
use lactose_sim::TickInput;
use lactose_task::{Executor, JobContext, WorkerClass};

const GRAPHICS: QueueRole = QueueRole::Graphics;

const PROJECTION: Projection = Projection { fov_y: 1.0, near: 0.1, far: 1000.0 };

fn on<S>(state: S, queue: QueueRole) -> StateOn<S> {
    StateOn { state, queue }
}

fn reached(device: &dyn Device, point: Option<TimelinePoint>) -> Result<bool> {
    match point {
        None => Ok(true),
        Some(point) => Ok(device.timeline_value(point.timeline)? >= point.value),
    }
}

// ---- 1. `lactose-app-client`: the render side, over the graph and the renderer ----

/// One offscreen target as the render side keeps it.
#[derive(Clone, Copy)]
struct Target {
    image: ImageHandle,
    extent: Extent,
    /// The point of the frame that last drew it.
    drawn: Option<TimelinePoint>,
}

/// An image a change of extent left behind, kept until nothing pending names it.
struct Retired {
    image: ImageHandle,
    drawn: Option<TimelinePoint>,
    read: Option<TimelinePoint>,
}

/// What the presentation schedule leaves for the frame's draw.
struct Presentation {
    pivot: RenderPosition,
    boom: f32,
    sampled: Vec<InstanceRow>,
}

/// What `lactose-app-client` builds and hands to `build_shown`. `lactose-app` names
/// neither crate under it (decision 5.11).
struct ClientRender {
    graph: RenderGraph,
    renderer: Renderer,
    targets: [Option<Target>; TARGETS],
    retired: Vec<Retired>,
    /// The frame recorded and not yet submitted, and the target it drew.
    recorded: Option<(RecordedFrame, TargetIndex)>,
    /// What the binary's main thread publishes and the latch reads. In the fixed-frame
    /// mode the binary publishes the fixed sample before each step.
    input: Arc<LatestSample>,
    presentation: Presentation,
    fraction: f32,
}

/// The binary's: the target's format and usage.
fn make_target(device: &dyn Device, extent: Extent) -> Result<ImageHandle> {
    todo!(
        "`lactose-app-client`: `Device::create_image`, exclusive, a colour attachment that is sampled"
    )
}

/// `lactose-presentation`'s schedule on one acquired pair: interpolation at the frame's
/// fraction, the camera rig, the remote sample, the delivery of the pair's events.
fn presentation(
    ctx: &mut JobContext<'_>,
    frame: &PresentFrame<'_>,
    time: FrameTime,
) -> Presentation {
    todo!("`lactose-presentation`")
}

/// The remote batch, from the presentation schedule's forked jobs (renderer, "Writers").
fn upload_remote(writer: &RemoteWriter<'_>, ctx: &mut JobContext<'_>, sampled: &[InstanceRow]) {
    ctx.scope(|scope| {
        for rows in sampled.chunks(256) {
            scope.fork(move |_| writer.write(rows));
        }
    });
}

fn counts_of(stats: &FrameStats) -> FrameCounts {
    let of = |kind| KindCounts {
        lost: stats.lost(kind),
        dropped: stats.dropped_in(kind),
        unplaced: stats.unplaced_in(kind),
    };
    FrameCounts {
        server: of(WorldKind::Server),
        client: of(WorldKind::Client),
        prediction: of(WorldKind::Prediction),
    }
}

impl ClientRender {
    /// The image of `draw.target` at `draw.extent`, and whether it is new. A retired image
    /// is destroyed here once the frame that last drew it and the present thread's last
    /// read of it have both completed, which `timeline_value` says without a wait, and the
    /// device takes the destroy: it refuses one while a heap slot holds the image
    /// (rhi-api, "Handles and lifetimes"), and the present side's slot lets go at the
    /// first call that names another image for the index, or at `close`.
    fn target(&mut self, draw: Draw) -> Result<(ImageHandle, bool)> {
        let device = Arc::clone(self.graph.device());
        let mut kept = Vec::new();
        for retired in self.retired.drain(..) {
            if !(reached(&*device, retired.drawn)? && reached(&*device, retired.read)?) {
                kept.push(retired);
                continue;
            }
            match device.destroy_image(retired.image) {
                Ok(()) => {}
                Err(error) if Rule::of(&error) == Some(Rule::Pending) => kept.push(retired),
                Err(error) => return Err(error),
            }
        }
        self.retired = kept;
        let slot = &mut self.targets[draw.target.index()];
        match *slot {
            Some(target) if target.extent == draw.extent => Ok((target.image, false)),
            old => {
                if let Some(old) = old {
                    // `draw.ready` is the present thread's last read of the old image.
                    self.retired.push(Retired {
                        image: old.image,
                        drawn: old.drawn,
                        read: draw.ready,
                    });
                }
                let image = make_target(&*device, draw.extent)?;
                *slot = Some(Target { image, extent: draw.extent, drawn: None });
                Ok((image, true))
            }
        }
    }

    /// Declare, compile and record against a shared borrow of the renderer, which ends
    /// when the recorded frame is returned.
    fn recorded_frame(
        renderer: &Renderer,
        graph: &mut RenderGraph,
        ctx: &mut JobContext<'_>,
        target: &ImageImport<'_>,
        extent: Extent,
        fraction: f32,
    ) -> Result<RecordedFrame> {
        let mut decl = FrameDecl::new(1);
        let target = decl.import_image(target);
        renderer.declare(&mut decl, &FrameView { target, extent, fraction })?;
        let compiled = graph.compile(decl)?;
        graph.record(compiled, ctx)
    }
}

impl RenderSide for ClientRender {
    fn open(&mut self, ctx: &mut JobContext<'_>) -> Result<Region> {
        loop {
            match self.renderer.open()? {
                Opened::Ready => return Ok(Region::Ready),
                // The frame asks again at its next frame: nothing here waits.
                Opened::Wait(point) => return Ok(Region::Wait(point)),
                // After torn frames alone: a frame with no pass still signals a point,
                // and the `open` after it answers `Ready` or `Wait`.
                Opened::Unsubmitted => {
                    let empty = self.graph.compile(FrameDecl::new(1))?;
                    let recorded = self.graph.record(empty, ctx)?;
                    let point = self.graph.submit(recorded)?;
                    self.renderer.submitted(point);
                }
            }
        }
    }

    fn sink(&mut self) -> Box<dyn InstanceSink> {
        Box::new(self.renderer.sink())
    }

    fn present(
        &mut self,
        ctx: &mut JobContext<'_>,
        frame: &PresentFrame<'_>,
        time: FrameTime,
    ) -> Result<()> {
        self.presentation = presentation(ctx, frame, time);
        self.fraction = time.fraction;
        Ok(())
    }

    fn draw(&mut self, ctx: &mut JobContext<'_>, draw: Draw) -> Result<FrameCounts> {
        // Nothing is prepared yet: an error here ends no frame.
        let (image, new) = self.target(draw)?;
        // The `Client` world's pass ended inside `Simulation::run_passes`, before this
        // job began, so the batch begins after it (renderer, "Writers").
        {
            let writer = self.renderer.remote();
            upload_remote(&writer, ctx, &self.presentation.sampled);
        }
        let stats = match self.renderer.prepare() {
            Ok(stats) => stats,
            Err(error) => {
                // A device's error left the frame prepared, to be ended here; after a
                // refusal `torn` does nothing (renderer, "Errors and drops").
                self.renderer.torn();
                return Err(error);
            }
        };
        // Taken from `Undefined` in a submit that waits for the present thread's last
        // read of this image, and handed on to the present role (rhi-api, "The present
        // thread"; graph, "Imports"). A new image has no read to wait for.
        let import = ImageImport {
            label: "target",
            image,
            mips: 1,
            layers: 1,
            sharing: Sharing::Exclusive,
            initial: on(ImageState::Undefined, GRAPHICS),
            last: on(ImageState::ColourAttachment, GRAPHICS),
            handoff: Some(on(ImageState::SampledRead, QueueRole::Present)),
            ready: if new { None } else { draw.ready },
        };
        let fraction = self.fraction;
        match Self::recorded_frame(
            &self.renderer,
            &mut self.graph,
            ctx,
            &import,
            draw.extent,
            fraction,
        ) {
            Ok(recorded) => {
                self.recorded = Some((recorded, draw.target));
                Ok(counts_of(&stats))
            }
            Err(error) => {
                // The frame was prepared and will not be submitted.
                self.renderer.torn();
                Err(error)
            }
        }
    }

    fn submit(&mut self) -> Result<Submitted> {
        let (recorded, target) = self
            .recorded
            .take()
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "no frame is recorded"))?;
        // At the extent `draw` declared the frame with.
        let latch = CameraLatch {
            pivot: self.presentation.pivot,
            input: &self.input,
            boom: self.presentation.boom,
            projection: PROJECTION,
        };
        if let Err(error) = self.renderer.latch(&latch) {
            self.renderer.torn();
            return Err(error);
        }
        let point = match self.graph.submit(recorded) {
            Ok(point) => point,
            Err(error) => {
                self.renderer.torn();
                return Err(error);
            }
        };
        self.renderer.submitted(point);
        let slot = self.targets[target.index()].as_mut().expect("`draw` made the target");
        slot.drawn = Some(point);
        let image = slot.image;
        // The next frame's region. After a `submitted` every held region has a point, so
        // `Unsubmitted` cannot be the answer; an error is the next frame's `open`'s to
        // report.
        let region = match self.renderer.open() {
            Ok(Opened::Ready) => Region::Ready,
            Ok(Opened::Wait(point)) => Region::Wait(point),
            Ok(Opened::Unsubmitted) | Err(_) => Region::Wait(point),
        };
        Ok(Submitted { image, point, region })
    }
}

/// Dropped by the binary off the frame-critical workers, after the present thread has
/// stopped: the graph that submitted the renderer's frames before the renderer (renderer,
/// "Errors and drops"), then the targets, which nothing pending names by then.
fn drop_render(render: ClientRender) {
    let ClientRender { graph, renderer, targets, retired, .. } = render;
    let device = Arc::clone(graph.device());
    drop(graph);
    drop(renderer);
    for image in targets.iter().flatten().map(|target| target.image) {
        let _ = device.destroy_image(image);
    }
    for retired in retired {
        let _ = device.destroy_image(retired.image);
    }
}

// ---- 2. `lactose-app-client`: the present side, over rhi-api's `Swapchain` ----

/// The state of `lactose_platform::present_thread`'s lane. `Send` and not `Sync`, as the
/// swapchain is: the lane asks for no more.
struct ClientPresent {
    device: Option<Arc<dyn Device>>,
    swapchain: Option<Box<dyn Swapchain>>,
    /// A timeline of the present side's own: each present that reads a target signals
    /// its next value, which is the report's `read`.
    reads: TimelineHandle,
    next: u64,
    /// The last point a present signalled: what a resize waits for first.
    last: Option<TimelinePoint>,
    /// Whether the swapchain said it no longer matches its surface.
    stale: bool,
    /// The extent the swapchain was last resized to. The swapchain's own may differ: a
    /// resize makes images at about the extent asked for (rhi-api, "The swapchain").
    asked: Option<Extent>,
    /// A swapchain image whose present was refused as pending, presented by the next
    /// call.
    acquired: Option<ImageHandle>,
}

/// The bound the present side gives its own waits, on the present thread alone.
const PRESENT_WAIT: Duration = Duration::from_millis(100);

const NOTHING: PresentReport = PresentReport { read: None, shown: false };

/// The pass that samples the target into the swapchain image, with a pipeline made for
/// `Swapchain::format` (rhi-api, "The present thread").
fn draw_target(encoder: &mut dyn Encoder, target: ImageHandle, image: ImageHandle) {
    todo!("`lactose-app-client`: the heap slot, the pass, the transition to `Present`")
}

impl PresentSide for ClientPresent {
    fn present(&mut self, finished: Finished) -> Result<PresentReport> {
        let (Some(device), Some(swapchain)) = (&self.device, &mut self.swapchain) else {
            return Ok(NOTHING);
        };
        if self.stale || self.asked != Some(finished.extent) {
            // The caller's wait before a resize is for its own points (rhi-api, "The
            // swapchain"). A timeout or a refusal as pending is the next call's to retry.
            if let Some(last) = self.last
                && !device.wait_timeline(last, PRESENT_WAIT)?
            {
                return Ok(NOTHING);
            }
            match swapchain.resize(finished.extent) {
                Ok(()) => {}
                Err(error) if Rule::of(&error) == Some(Rule::Pending) => return Ok(NOTHING),
                Err(error) => return Err(error),
            }
            // A resize destroys every handle the swapchain gave before it. At an empty
            // surface it keeps its images and stays stale, which the acquire below says.
            self.acquired = None;
            self.stale = false;
            self.asked = Some(finished.extent);
        }
        let image = match self.acquired.take() {
            Some(image) => image,
            None => match swapchain.acquire()? {
                Acquired::Image(image) => image,
                Acquired::None => return Ok(NOTHING),
                Acquired::Stale => {
                    self.stale = true;
                    return Ok(NOTHING);
                }
            },
        };
        let mut encoder = device.create_encoder(QueueRole::Present, "present")?;
        // The acquire half of the hand-off the frame's submit released, with equal
        // values; where one queue serves both roles it is the whole change (graph,
        // "Imports").
        encoder.transition_images(&[ImageTransition {
            image: finished.image,
            range: ImageRange::FIRST,
            from: on(ImageState::ColourAttachment, GRAPHICS),
            to: on(ImageState::SampledRead, QueueRole::Present),
        }]);
        draw_target(&mut *encoder, finished.image, image);
        let read = TimelinePoint { timeline: self.reads, value: self.next };
        let mut encoders = [encoder];
        let submit = Submit { encoders: &mut encoders, waits: &[finished.point], signals: &[read] };
        match swapchain.present(image, submit) {
            Ok(presented) => {
                self.next += 1;
                self.last = Some(read);
                self.stale = presented == Presented::Stale;
                Ok(PresentReport { read: Some(read), shown: presented == Presented::Shown })
            }
            // Nothing was submitted and the image is still the acquired one: the frame's
            // release of the target stays unmet, and the frame takes the target back from
            // `Undefined` behind the read it already knows.
            Err(error) if Rule::of(&error) == Some(Rule::Pending) => {
                self.acquired = Some(image);
                Ok(NOTHING)
            }
            Err(error) => Err(error),
        }
    }

    fn close(&mut self) {
        // The heap slots this side wrote are emptied first (`Device::clear_heap_slot`,
        // behind a bounded wait for its last read), so the render side's drop may destroy
        // the targets. The swapchain's drop waits for the submits that use its images.
        self.swapchain = None;
        self.device = None;
    }
}

// ---- 3. `lactose-app-client`: the root task, and the order of the shutdown ----

/// What the root reads of the window (`lactose_platform::WindowReports`).
trait Window: Send + Sync {
    /// `None` while the window has no area.
    fn extent(&self) -> Option<Extent>;
    fn close_requested(&self) -> bool;
}

fn client_builds(
    executor: &Executor,
    game: &Game,
    start: Start,
    cooked: Arc<dyn Cooked>,
    render: ClientRender,
    lane: PresentLane,
) -> Result<Shown> {
    build_shown(executor, game, WorldConfig::shown(), start, cooked, Box::new(render), lane)
}

/// What the root hands the binary's shutdown: the render side, and the world if it
/// survived.
struct Ended {
    server: Option<Server>,
    render: Option<Box<dyn RenderSide>>,
    error: Option<Error>,
}

/// The timer root paces the frame and nothing else does (architecture section 5.1): one
/// sleep to the next tick's deadline, one frame. The present thread paces nothing.
async fn root(
    executor: &Executor,
    mut shown: Shown,
    window: &dyn Window,
    frames: &mut (dyn FnMut(Tick) -> Option<TickInput> + Send),
) -> Result<Ended> {
    let mut deadline = Instant::now();
    loop {
        executor.sleep_until(deadline).await?;
        if window.close_requested() {
            break;
        }
        match shown.frame(executor, Instant::now(), &mut *frames, window.extent()).await {
            Ok((next, output)) => {
                shown = next;
                deadline = output.world.next_deadline;
                if let Some(Rendered::Drew { counts, queued, .. }) = output.joined {
                    // A result file's: what was drawn and what was shown.
                    let _ = (counts.server.stale(), queued);
                }
            }
            // A device loss, or a world's run that handed its simulation back: the world
            // is kept and the render side is ended in order.
            Err(ShownError { error, kept: ShownKept::Shown(kept) }) => {
                let Stopped { server, render } = kept.stop().await?;
                return Ok(Ended {
                    server: Some(server),
                    render: Some(render),
                    error: Some(error),
                });
            }
            // The world is lost; the render side was ended as `stop` ends it.
            Err(ShownError { error, kept: ShownKept::Render(render) }) => {
                return Ok(Ended { server: None, render: Some(render), error: Some(error) });
            }
            Err(ShownError { error, kept: ShownKept::Nothing }) => return Err(error),
        }
    }
    let Stopped { server, render } = shown.stop().await?;
    Ok(Ended { server: Some(server), render: Some(render), error: None })
}

/// After the root has resolved: steps 3 to 6 of `crates/platform/README.md`, "Shutdown".
/// Step 2 was `Shown::stop`: no present and no frame submit is queued after it, and
/// `PresentSide::close` was the present thread's last call. `stop_present_thread` stands
/// for awaiting `lactose_platform::PresentThread::stop`, and `on_own_thread` for a thread
/// made through `Executor::platform`, since the render side's drop waits for the device
/// and no worker may.
fn shut_down(
    executor: Executor,
    ended: Ended,
    stop_present_thread: impl FnOnce(),
    on_own_thread: impl FnOnce(Box<dyn FnOnce() + Send>),
) -> Result<()> {
    let Ended { server, render, error } = ended;
    stop_present_thread();
    if let Some(render) = render {
        // The graph, the renderer, their encoders and their clones of the device.
        on_own_thread(Box::new(move || drop(render)));
    }
    if let Some(server) = server {
        server.shutdown();
    }
    executor.shutdown();
    error.map_or(Ok(()), Err)
}

// ---- 4. The fixed-frame mode of decision 5.1, and a replay: step-driven, joined ----

/// One step and its frame at a time, each joined before the next: no clock is read, the
/// sample is fixed, the extent is fixed, and the fraction is the mode's own constant, which
/// decision 5.1 dumps: from the second step on a frame's pair is the last two ticks
/// stepped, so any fixed fraction is reproducible. The driver is a thread of the binary's
/// own and never a worker (`Executor::block_on`), so it may wait for each frame's point: a
/// region is then free at every step and every tick's frame is drawn. When the steps are
/// done the binary takes its own render side back, as its own type, for the dump of
/// decision 5.1 and the bytes of the last target, which its last frame copied to a buffer
/// in a pass of the frame's own declaration, before the hand-off: a target taken back
/// from the present role afterwards would have no defined contents (rhi-api, "States").
fn fixed_frames(
    executor: &Executor,
    device: &dyn Device,
    mut shown: Shown,
    input: &LatestSample,
    fixed: RawSample,
    extent: Extent,
    fraction: f32,
    steps: u32,
) -> Result<(Server, Box<ClientRender>)> {
    let start = Instant::now();
    for step in 0..steps {
        // The fixed sample, published by the binary before the frame that latches it.
        input.publish(fixed);
        let time = FrameTime { now: start + TICK_PERIOD * step, fraction };
        let (next, output, rendered) =
            executor.block_on(shown.step(executor, TickInput::default(), time, Some(extent)))?;
        shown = next;
        // The mode is reproducible or it has failed: a frame that waited or was torn drew
        // nothing for its tick.
        let Rendered::Drew { point, counts, .. } = rendered else {
            return Err(Error::new(ErrorKind::Unsupported, "a fixed frame did not draw"));
        };
        assert!(!counts.server.stale());
        // Bounded, and off every worker.
        let _reached = device.wait_timeline(point, Duration::from_secs(1))?;
    }
    let Stopped { server, render } = executor.block_on(shown.stop())?;
    let render: Box<dyn Any> = render;
    let render = render.downcast::<ClientRender>().expect("the object `build_shown` was given");
    Ok((server, render))
}

// ---- 5. `frame-impl`'s tests: a render side with no device, and criterion 3(c) ----

fn point(value: u64) -> TimelinePoint {
    let timeline: TimelineHandle = Handle::from_parts(0, core::num::NonZeroU32::MIN);
    TimelinePoint { timeline, value }
}

/// A render side that draws nothing: what `lactose-app`'s own tests of the frame's
/// protocol drive, with no backend linked.
#[derive(Default)]
struct CountingRender {
    frames: u64,
    /// The frames whose `open` answers `Wait`, for the tests of a deferred frame.
    busy: Vec<u64>,
    draws: Vec<Draw>,
}

struct NoSink;

impl InstanceSink for NoSink {
    fn begin(&mut self, pass: ExtractPass) {}
    fn removed(&mut self, keys: &[StableKey]) {}
    fn write(&self, rows: &[InstanceRow]) {}
    fn end(&mut self) {}
}

impl RenderSide for CountingRender {
    fn open(&mut self, ctx: &mut JobContext<'_>) -> Result<Region> {
        if self.busy.contains(&self.frames) {
            return Ok(Region::Wait(point(self.frames)));
        }
        Ok(Region::Ready)
    }

    fn sink(&mut self) -> Box<dyn InstanceSink> {
        Box::new(NoSink)
    }

    fn present(
        &mut self,
        ctx: &mut JobContext<'_>,
        frame: &PresentFrame<'_>,
        time: FrameTime,
    ) -> Result<()> {
        Ok(())
    }

    fn draw(&mut self, ctx: &mut JobContext<'_>, draw: Draw) -> Result<FrameCounts> {
        self.draws.push(draw);
        Ok(FrameCounts::default())
    }

    fn submit(&mut self) -> Result<Submitted> {
        self.frames += 1;
        let target = self.draws.last().expect("a frame was drawn").target;
        let image: ImageHandle =
            Handle::from_parts(target.index() as u32, core::num::NonZeroU32::MIN);
        Ok(Submitted { image, point: point(self.frames), region: Region::Ready })
    }
}

/// A present side that shows nothing and reports a read of every target it is handed.
#[derive(Default)]
struct CountingPresent {
    presents: u64,
    closed: bool,
}

impl PresentSide for CountingPresent {
    fn present(&mut self, finished: Finished) -> Result<PresentReport> {
        self.presents += 1;
        Ok(PresentReport { read: Some(point(1 << 32 | self.presents)), shown: true })
    }

    fn close(&mut self) {
        self.closed = true;
    }
}

/// Criterion 3(c): the same world stepped with a render side and with none has the same
/// hash after every tick. `frame-impl`'s recorded test is this under the harness, with
/// the real render side on null, in `lactose-app-client`'s module `recorded`.
async fn hashes_match(
    executor: &Executor,
    mut plain: Server,
    mut shown: Shown,
    extent: Extent,
    steps: u32,
) -> Result<()> {
    let start = Instant::now();
    for step in 0..steps {
        let (next, _) = plain.step(executor, TickInput::default()).await?;
        plain = next;
        let time = FrameTime { now: start + TICK_PERIOD * step, fraction: 1.0 };
        let (next, _, _) = shown.step(executor, TickInput::default(), time, Some(extent)).await?;
        shown = next;
        assert_eq!(plain.tick(), shown.server().tick());
        assert_eq!(plain.hash()?, shown.server().hash()?);
    }
    Ok(())
}

fn same_world_twice(
    executor: &Executor,
    game: &Game,
    start: impl Fn() -> Start,
    cooked: Arc<dyn Cooked>,
    lane: PresentLane,
) -> Result<(Server, Shown)> {
    let plain = build_server(executor, game, WorldConfig::shown(), start(), Arc::clone(&cooked))?;
    let render = Box::new(CountingRender::default());
    let shown = build_shown(executor, game, WorldConfig::shown(), start(), cooked, render, lane)?;
    Ok((plain, shown))
}

/// The target a frame draws into is never the one a call in flight names, and a target's
/// index is below `TARGETS`.
fn other(target: TargetIndex) -> TargetIndex {
    TargetIndex::new((target.index() + 1) % TARGETS).expect("below `TARGETS`")
}

// ---- 6. The frame's job: a world of the frame's own lends it a `JobContext` (runs) ----

/// What D162 asks `lactose-present-api` to declare: the frame's work, run once in a job.
trait FrameJob: Any + Send + Sync {
    fn run(&mut self, job: &mut Job<'_, '_>);
}

/// The one system of the frame's own world.
#[system(name = "sketch::frame_job", worlds = [client])]
fn frame_job(job: Job, work: ResMut<dyn FrameJob>) {
    work.run(&mut job);
}

/// What D162 asks `lactose-present-api` to offer: a world that exists to run `work` in a
/// job, and its schedule. Kind `Client`; no row, no region, no resource but the service.
fn frame_world(work: Box<dyn FrameJob>) -> Result<(World, Schedule)> {
    let mut registry = Registry::new();
    registry.register::<InRegion>()?;
    registry.register::<OwnedBy>()?;
    registry.register::<ClaimedBy>()?;
    registry.register::<PredictedOrigin>()?;
    let mut world = WorldBuilder::new(WorldKind::Client, Arc::new(registry)).build()?;
    world.insert_service::<dyn FrameJob>(work);
    let mut builder = Schedule::builder(WorldKind::Client);
    builder.stage("frame", &[]).add(&FRAME_JOB);
    let schedule =
        builder.build().map_err(|error| Error::new(ErrorKind::InvalidData, error.to_string()))?;
    Ok((world, schedule))
}

/// What `lactose-app` keeps in that service: the render side and what one frame asks of
/// it, written before the pass and read after it.
struct Work {
    render: Option<Box<dyn RenderSide>>,
    opened: Option<Result<Region>>,
    runs: u32,
}

impl FrameJob for Work {
    fn run(&mut self, job: &mut Job<'_, '_>) {
        self.runs += 1;
        if let Some(render) = &mut self.render {
            self.opened = Some(render.open(job.context()));
        }
    }
}

/// The service, as the type `lactose-app` put there: upcast to `Any`, then downcast.
fn work_of(world: &mut World) -> &mut Work {
    let work: &mut dyn FrameJob = world.service_mut::<dyn FrameJob>().expect("the service");
    let any: &mut dyn Any = work;
    any.downcast_mut::<Work>().expect("`lactose-app` put a `Work` there")
}

#[test]
fn the_frame_job_runs_in_a_world_of_its_own() {
    let executor =
        Executor::builder().frame_workers(2).background_workers(1).build().expect("executor");
    let render: Box<dyn RenderSide> =
        Box::new(CountingRender { busy: vec![0], ..Default::default() });
    let work = Box::new(Work { render: Some(render), opened: None, runs: 0 });
    let (world, schedule) = frame_world(work).expect("the frame's world");

    // The frame's render task: it owns the world and the schedule, and hands both back.
    let handle = executor.clone();
    let task = executor.spawn(WorkerClass::FrameCritical, async move {
        let mut schedule = schedule;
        let world = schedule.run_pass(world, &handle).await;
        (world, schedule)
    });
    let (world, mut schedule) = executor.block_on(task).expect("the task ran");
    let mut world = world.expect("the pass ran");

    // Between frames the frame's task reads what the job left, with no lock.
    let work = work_of(&mut world);
    assert_eq!(work.runs, 1);
    assert_eq!(
        work.opened.take().expect("the job asked").expect("no error"),
        Region::Wait(point(0))
    );

    // A second pass of the same world: the tick has not advanced and the service is the
    // same object.
    let tick = world.tick();
    let world = executor.block_on(schedule.run_pass(world, &executor)).expect("the pass ran");
    let mut world = world;
    assert_eq!(world.tick(), tick);
    assert_eq!(work_of(&mut world).runs, 2);

    // At `Shown::stop` the render side leaves the world by value.
    let render = work_of(&mut world).render.take().expect("still there");
    drop(world);
    drop(render);
    executor.shutdown();
}
