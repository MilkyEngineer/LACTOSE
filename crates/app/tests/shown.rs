//! What `lactose-app-client` writes against the shown frame of `lactose-app` (D162), and
//! two of the tests of it that `frame-impl` writes. Sections 1 to 4 are compile-only: the
//! render side over the render graph and the renderer as merged, the present side over
//! rhi-api's `Swapchain`, the binary's root and its shutdown, and the fixed-frame mode of
//! decision 5.1. They are type-checked and never run, since what is the binary's own in them
//! (the target's image, `lactose-presentation`'s schedule) is `todo!()`. They pin the surface
//! `README.md` promises: a change that breaks one is a signature change to list.
//!
//! Sections 5 and 6 run. Section 5 is criterion 3's (a) and (b), over the render side with
//! no device of `frame::support`. Section 6 is the route by which the frame holds the render
//! side and a `JobContext` together: a closure run once as one job by `Executor::run_job`,
//! the render side moved in and handed back in its value, with no world, no system and no
//! `Sync` bound, on an executor of two frame workers and of three.

#![forbid(unsafe_code)]
#![allow(dead_code, unused_variables, missing_docs, clippy::too_many_arguments)]

use std::any::Any;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use lactose_app::{
    Cooked, Draw, Finished, FrameCounts, FrameTime, Game, KindCounts, PresentLane, PresentReport,
    PresentSide, Region, RenderSide, Rendered, Server, Shown, ShownError, ShownKept, Start,
    Stopped, Submitted, TARGETS, TICK_PERIOD, TargetIndex, WorldConfig, build_shown,
};
use lactose_core::{Error, ErrorKind, Handle, Result, StableKey, Tick};
use lactose_ecs::WorldKind;
use lactose_input_host::{LatestSample, RawSample};
use lactose_math::render::RenderPosition;
use lactose_present_api::{ExtractPass, InstanceRow, InstanceSink, PresentFrame};
use lactose_render_graph::{FrameDecl, ImageImport, RecordedFrame, RenderGraph};
use lactose_renderer::{
    CameraLatch, FrameStats, FrameView, Opened, Projection, RemoteWriter, Renderer,
};
use lactose_rhi_api::{
    Acquired, Device, Encoder, Extent, HeapSlot, ImageHandle, ImageRange, ImageState,
    ImageTransition, Presented, QueuePath, QueueRole, Rule, Sharing, StateOn, Submit, Swapchain,
    TimelineHandle, TimelinePoint,
};
use lactose_sim::TickInput;
use lactose_task::{ChaosConfig, Executor, JobContext, JobRun, WorkerClass};

use crate::frame::support::{
    Answer, Call, E1, Rig, SERIAL, TICK_HOOK, WAIT_BOUND, When, bounded, lock,
};

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
    fn present_queue(&self) -> QueuePath {
        self.graph.device().capabilities().queues.present
    }

    fn open(&mut self, ctx: &mut JobContext<'_>) -> Result<Region> {
        loop {
            match self.renderer.open()? {
                Opened::Ready => return Ok(Region::Ready),
                // The frame asks again at its next frame: nothing here waits.
                Opened::Wait(point) => return Ok(Region::Wait(point)),
                // After torn frames alone: a frame with no pass still signals a point,
                // and the `open` after it answers `Ready` or `Wait`. This is the one
                // submit outside `submit`, which is why the frame calls `open` with no
                // present call in flight where the roles share a queue.
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
    /// The heap slots the binary gives the present pass, one a target, and the image this
    /// side last put in each.
    heap: [HeapSlot; TARGETS],
    slots: [Option<ImageHandle>; TARGETS],
}

/// The bound the present side gives its own waits, on the present thread alone.
const PRESENT_WAIT: Duration = Duration::from_millis(100);

const NOTHING: PresentReport = PresentReport { read: None, shown: false, waiting: None };

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
        // The image is not the one this target's slot holds. A slot is written only once
        // no pending pass declares it, and the pass that last declared this one is covered
        // by the last read this side reported, since calls name the targets in turn. The
        // resize's wait above was for the same point, but a new image can come with no
        // resize: an extent that changed and came back, the frame between dropped or torn.
        let slot = finished.target.index();
        if self.slots[slot] != Some(finished.image) {
            if let Some(last) = self.last
                && !device.wait_timeline(last, PRESENT_WAIT)?
            {
                return Ok(NOTHING);
            }
            device.write_heap_slot(self.heap[slot], finished.image)?;
            self.slots[slot] = Some(finished.image);
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
                // No swapchain reports a display time yet, so nothing is counted as
                // waiting and no frame is shed.
                Ok(PresentReport {
                    read: Some(read),
                    shown: presented == Presented::Shown,
                    waiting: None,
                })
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
        // The heap slots this side wrote are emptied first, behind a bounded wait for its
        // last read, so the render side's drop may destroy the targets. Past the bound
        // the device refuses the clear (`uuwp-used`) and the slot is left: the device
        // then refuses the destroy of the target it holds, and that image is freed with
        // the device. Nothing here retries. On a lost device the wait returns at once.
        if let Some(device) = &self.device {
            if let Some(last) = self.last {
                let _ = device.wait_timeline(last, PRESENT_WAIT);
            }
            for (slot, held) in self.heap.iter().zip(&mut self.slots) {
                if held.take().is_some() {
                    let _ = device.clear_heap_slot(*slot);
                }
            }
        }
        // The swapchain's drop waits for the submits that use its images.
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

/// What a failed run leaves, for the binary's shutdown. A `ShownError` does not convert
/// into a plain error, so a caller cannot `?` it and drop the render side where it stands,
/// which for the root is a frame-critical worker: it matches `kept`.
async fn ended(failure: ShownError) -> Result<Ended> {
    let ShownError { error, kept } = failure;
    match kept {
        // A device loss, or a world's run that handed its simulation back: the world is
        // kept and the render side is ended in order.
        ShownKept::Shown(kept) => {
            let Stopped { server, render } = kept.stop().await?;
            Ok(Ended { server: Some(server), render: Some(render), error: Some(error) })
        }
        // The world is lost; the render side was ended as `stop` ends it.
        ShownKept::Render(render) => {
            Ok(Ended { server: None, render: Some(render), error: Some(error) })
        }
        // The executor shut down first: nothing is left to end.
        ShownKept::Nothing => Err(error),
    }
}

/// The timer root paces the frame and nothing else does (architecture section 5.1): one
/// sleep to the next tick's deadline, one frame. No tick waits for the render side: a
/// frame waits for the frame before it only until `JOIN_MARGIN` before the next tick's
/// deadline, and one that finds it still there then has run its ticks, defers its passes
/// and says `Rendered::Busy`. Where the present role has a queue of its own the
/// present thread holds no frame; where it shares the graphics queue a present call still
/// in flight holds one frame's submit, and that frame is dropped after it.
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
                if let Some(Rendered::Drew { counts, queued, held, shed, .. }) = output.joined {
                    // A result file's: what was drawn, what was queued to be shown, and
                    // what was held and so dropped.
                    let _ = (counts.server.stale(), queued, held, shed);
                }
                // A result file's as well: a frame that ran its ticks alone because the
                // frame before it was still on the render side.
                let _busy = matches!(output.deferred, Some(Rendered::Busy));
            }
            Err(failure) => return ended(failure).await,
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
/// done the binary takes its own render side back, as its own type (`own_render`), for
/// the dump of decision 5.1 and the bytes of the last target, which its last frame copied
/// to a buffer in a pass of the frame's own declaration, before the hand-off: a target
/// taken back from the present role afterwards would have no defined contents (rhi-api,
/// "States"). A step that fails ends as the root's frame does, by what it kept.
fn fixed_frames(
    executor: &Executor,
    device: &dyn Device,
    mut shown: Shown,
    input: &LatestSample,
    fixed: RawSample,
    extent: Extent,
    fraction: f32,
    steps: u32,
) -> Result<Ended> {
    let start = Instant::now();
    for step in 0..steps {
        // The fixed sample, published by the binary before the frame that latches it.
        input.publish(fixed);
        let time = FrameTime { now: start + TICK_PERIOD * step, fraction };
        let stepped =
            executor.block_on(shown.step(executor, TickInput::default(), time, Some(extent)));
        let (next, output, rendered) = match stepped {
            Ok(stepped) => stepped,
            Err(failure) => return executor.block_on(ended(failure)),
        };
        shown = next;
        // The mode is reproducible or it has failed: a frame that waited or was torn drew
        // nothing for its tick.
        let Rendered::Drew { point, counts, .. } = rendered else {
            let Stopped { server, render } = executor.block_on(shown.stop())?;
            let error = Error::new(ErrorKind::Unsupported, "a fixed frame did not draw");
            return Ok(Ended { server: Some(server), render: Some(render), error: Some(error) });
        };
        assert!(!counts.server.stale());
        // Bounded, and off every worker.
        let _reached = device.wait_timeline(point, Duration::from_secs(1))?;
    }
    let Stopped { server, render } = executor.block_on(shown.stop())?;
    Ok(Ended { server: Some(server), render: Some(render), error: None })
}

/// After `stop`, the render side as the type the binary built it with (`RenderSide: Any`).
fn own_render(render: Box<dyn RenderSide>) -> Box<ClientRender> {
    let render: Box<dyn Any> = render;
    render.downcast::<ClientRender>().expect("the object `build_shown` was given")
}

// ---- 5. Criterion 3 (a) and (b): the frame's passes and its ticks, with no device ----

fn point(value: u64) -> TimelinePoint {
    let timeline: TimelineHandle = Handle::from_parts(0, core::num::NonZeroU32::MIN);
    TimelinePoint { timeline, value }
}

struct NoSink;

impl InstanceSink for NoSink {
    fn begin(&mut self, pass: ExtractPass) {}
    fn removed(&mut self, keys: &[StableKey]) {}
    fn write(&self, rows: &[InstanceRow]) {}
    fn end(&mut self) {}
}

/// The target a frame draws into is never the one a call in flight names, and a target's
/// index is below `TARGETS`.
fn other(target: TargetIndex) -> TargetIndex {
    TargetIndex::new((target.index() + 1) % TARGETS).expect("below `TARGETS`")
}

/// Criterion 3(a) over one log: no tick's system starts between a pass's `begin` and its
/// `end`; no `present` or `draw` is called there; and none is called between that `end` and
/// the publication after it, which the log shows by the call that follows: the next
/// `present` reads a snapshot of the pass's tick or a later one, and a `draw` follows such
/// a `present`. The publication itself is no call of a side, so the log cannot place it.
fn passes_hold_the_frame_apart(calls: &[Call]) {
    let mut open: Option<Tick> = None;
    let mut last_pass: Option<Tick> = None;
    // Whether a `present` that read the last pass's publication has been called since it.
    let mut presented = true;
    for call in calls {
        match call {
            Call::Begin(pass) => {
                assert_eq!(open, None, "a pass began inside another");
                open = Some(pass.tick);
            }
            Call::End { .. } => {
                last_pass = open.take();
                assert!(last_pass.is_some(), "an end with no begin");
                presented = false;
            }
            Call::Tick(tick) => assert_eq!(open, None, "tick {tick:?} started inside a pass"),
            Call::Present { newer, .. } => {
                assert_eq!(open, None, "a present inside a pass");
                if let Some(pass) = last_pass {
                    assert!(*newer >= pass, "a present before the publication of its pass");
                }
                presented = true;
            }
            Call::Draw(_) => {
                assert_eq!(open, None, "a draw inside a pass");
                assert!(presented, "a draw before the present of the pass's publication");
            }
            _ => {}
        }
    }
}

#[test]
fn criterion_3a_no_tick_or_draw_runs_inside_a_pass() {
    let _serial = lock(&SERIAL);
    let mut rig = Rig::new(2, false);
    *lock(&TICK_HOOK) = Some(Arc::clone(&rig.script));
    // Stepped: each state.
    rig.step(Some(E1));
    rig.step(None);
    rig.script.submit_answers([Ok(Region::Wait(point(70)))]);
    rig.step(Some(E1));
    rig.script.open_answers(&[Answer::Wait]);
    rig.step(Some(E1));
    rig.script.open_answers(&[Answer::Fail(ErrorKind::Platform)]);
    rig.script.submit_answers([Ok(Region::Wait(point(71)))]);
    rig.step(Some(E1));
    rig.step(Some(E1));
    rig.step(Some(E1));
    // Paced: drawn, hidden, and a busy frame behind a held draw.
    rig.frame(When::Ahead, 2, Some(E1));
    rig.frame(When::Ahead, 1, None);
    rig.flush();
    rig.script.hold_draws(true);
    rig.frame(When::Ahead, 1, Some(E1));
    rig.script.await_draws(1);
    let busy = rig.frame(When::Passed, 2, Some(E1));
    assert_eq!(busy.deferred, Some(Rendered::Busy));
    rig.script.hold_draws(false);
    rig.frame(When::Ahead, 1, Some(E1));
    rig.flush();
    *lock(&TICK_HOOK) = None;
    let calls = rig.script.calls();
    assert!(calls.iter().any(|call| matches!(call, Call::Tick(_))));
    passes_hold_the_frame_apart(&calls);
    let stats = rig.stats();
    assert!(stats.hidden >= 2 && stats.waited >= 1 && stats.torn >= 1 && stats.busy >= 1);
    rig.stop();
}

#[test]
fn criterion_3b_a_frame_s_draw_overlaps_the_next_tick() {
    let _serial = lock(&SERIAL);
    let mut rig = Rig::new(2, false);
    *lock(&TICK_HOOK) = Some(Arc::clone(&rig.script));
    rig.frame(When::Ahead, 1, Some(E1));
    rig.flush();
    // The next frame runs one tick and draws; its draw meets the system of the tick after.
    let tick = rig.shown().server().tick();
    let next = Tick(tick.0 + 2);
    rig.script.with(|plan| plan.rendezvous = Some(next));
    rig.frame(When::Ahead, 1, Some(E1));
    // The frame that begins while that draw waits runs the tick that meets it. Whether it
    // is busy is not asserted.
    rig.frame(When::Ahead, 1, Some(E1));
    rig.flush();
    *lock(&TICK_HOOK) = None;
    assert!(rig.script.with(|plan| plan.draw_met && plan.tick_met));
    assert_eq!(rig.shown().server().tick(), next);
    rig.stop();
}

// ---- 6. The frame's job: a closure run as one job, with its `JobContext` (runs) ----

/// How many units the render side below forks over.
const UNITS: usize = 4096;

/// A render side that is `Send` and not `Sync`, and that uses the context its job lends:
/// `open` forks over a buffer of its own, as a real `draw` forks the remote batch and the
/// graph's recording.
struct ForkingRender {
    /// One mark a unit, raised once by each `open`.
    marks: Vec<u32>,
    /// Calls of `open`. A `Cell`, which a `Sync` bound on the trait would refuse.
    opens: Cell<u32>,
    /// The thread each `open` ran on.
    threads: Vec<std::thread::ThreadId>,
    /// Each thread that ran a child of an `open`'s scopes, with the thread of its job.
    children: Vec<(std::thread::ThreadId, std::thread::ThreadId)>,
    /// Whether a third scope's two children found each other on two threads.
    shared: Vec<bool>,
    /// The most children of one `open`'s first two scopes that ran at once.
    peak: u32,
    /// The largest slot a job's context reported.
    slot: u32,
    /// `Executor::run_job_width` of the executor the jobs run on.
    width: u32,
}

/// How many children run at once, and the most that have.
struct Running {
    now: AtomicU32,
    peak: AtomicU32,
}

/// A child running, counted until it is dropped.
struct Child<'a>(&'a AtomicU32);

impl Running {
    fn enter(&self) -> Child<'_> {
        let now = self.now.fetch_add(1, Ordering::AcqRel) + 1;
        self.peak.fetch_max(now, Ordering::AcqRel);
        Child(&self.now)
    }
}

impl Drop for Child<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl ForkingRender {
    /// Two children that each wait, within the bound, until both have started: on a width
    /// of two they run on two threads at once, and the second thread is a worker other
    /// than the job's. Never called on a width of one, where the second would wait for a
    /// worker the job holds.
    fn meet(ctx: &mut JobContext<'_>) -> bool {
        let started = AtomicU32::new(0);
        let met = AtomicBool::new(true);
        ctx.scope(|scope| {
            for _ in 0..2 {
                scope.fork(|_| {
                    started.fetch_add(1, Ordering::AcqRel);
                    let start = Instant::now();
                    while started.load(Ordering::Acquire) < 2 {
                        if start.elapsed() > WAIT_BOUND {
                            met.store(false, Ordering::Relaxed);
                            return;
                        }
                        std::thread::yield_now();
                    }
                });
            }
        });
        met.into_inner()
    }
}

impl RenderSide for ForkingRender {
    fn present_queue(&self) -> QueuePath {
        QueuePath::Own
    }

    fn open(&mut self, ctx: &mut JobContext<'_>) -> Result<Region> {
        self.opens.set(self.opens.get() + 1);
        let job = std::thread::current().id();
        self.threads.push(job);
        self.slot = self.slot.max(ctx.slot());
        let children = Mutex::new(Vec::new());
        let running = Running { now: AtomicU32::new(0), peak: AtomicU32::new(0) };
        // One scope over disjoint `&mut` chunks, a child each.
        let marks = &mut self.marks;
        ctx.scope(|scope| {
            for chunk in marks.chunks_mut(256) {
                let children = &children;
                let running = &running;
                scope.fork(move |_| {
                    let _child = running.enter();
                    lock(children).push((std::thread::current().id(), job));
                    for mark in chunk {
                        *mark += 1;
                    }
                });
            }
        });
        // A second scope of the same job, over a range: each unit once, however the
        // range is split.
        let seen: Vec<AtomicU32> = (0..UNITS).map(|_| AtomicU32::new(0)).collect();
        ctx.scope(|scope| {
            scope.fork_range(0..UNITS as u32, |units, _| {
                let _child = running.enter();
                lock(&children).push((std::thread::current().id(), job));
                for unit in units {
                    seen[unit as usize].fetch_add(1, Ordering::Relaxed);
                }
            });
        });
        assert!(seen.iter().all(|count| count.load(Ordering::Relaxed) == 1));
        self.children.extend(children.into_inner().unwrap_or_else(PoisonError::into_inner));
        self.peak = self.peak.max(running.peak.into_inner());
        if self.width > 1 {
            self.shared.push(Self::meet(ctx));
        }
        Ok(Region::Wait(point(u64::from(self.opens.get()))))
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
        Ok(FrameCounts::default())
    }

    fn submit(&mut self) -> Result<Submitted> {
        let image: ImageHandle = Handle::from_parts(0, core::num::NonZeroU32::MIN);
        Ok(Submitted { image, point: point(0), region: Region::Ready })
    }
}

/// What `lactose-app` moves into a job and takes back from it: `Side`, in the shape it
/// has, with no lock and no world around the render side.
struct Moved {
    render: Box<dyn RenderSide>,
    /// What the job was asked and what it answered, written in the job and read by the
    /// task once the job has resolved.
    opened: Option<Result<Region>>,
}

/// One job: the render side moved in, asked, and handed back.
fn ask(executor: &Executor, mut side: Box<Moved>) -> JobRun<Box<Moved>> {
    executor.run_job(move |ctx| {
        side.opened = Some(side.render.open(ctx));
        side
    })
}

fn the_frame_job_on(frame_workers: u32) {
    // Every range split to single units, every steal and yield seeded: the range of the
    // job's second scope is split into many children whatever the machine is doing.
    let chaos = ChaosConfig {
        seed: 162,
        shuffle_steals: true,
        inject_yields: true,
        force_split: true,
        record: false,
    };
    let executor = Executor::builder()
        .frame_workers(frame_workers)
        .background_workers(1)
        .chaos(chaos)
        .build()
        .expect("executor");
    let width = executor.run_job_width();
    assert_eq!(width, frame_workers - 1);
    let driver = std::thread::current().id();
    let render: Box<dyn RenderSide> = Box::new(ForkingRender {
        marks: vec![0; UNITS],
        opens: Cell::new(0),
        threads: Vec::new(),
        children: Vec::new(),
        shared: Vec::new(),
        peak: 0,
        slot: 0,
        width,
    });
    let side = Box::new(Moved { render, opened: None });

    // The frame's render task: it owns the side, moves it into the job, takes it back
    // and then calls what needs no job from its own poll, as it calls `submit`.
    let handle = executor.clone();
    let task = executor.spawn(WorkerClass::FrameCritical, async move {
        let mut side = ask(&handle, side).await?;
        let submitted = side.render.submit();
        Ok::<_, Error>((side, submitted))
    });
    let (mut side, submitted) =
        bounded(&executor, task, "the render task").expect("the task ran").expect("the job ran");
    assert_eq!(submitted.expect("no error").region, Region::Ready);
    // Between frames the task that drives the frame reads what the job left, with no
    // lock of its own.
    assert_eq!(
        side.opened.take().expect("the job asked").expect("no error"),
        Region::Wait(point(1))
    );

    // A second job with the same object, from the thread that drives a stepped run: the
    // job that asks `open` at the start of a frame. That thread is no worker, so the job
    // cannot run on it.
    let asked = ask(&executor, side);
    let mut side = bounded(&executor, asked, "the job that asks `open`").expect("the job ran");
    assert_eq!(
        side.opened.take().expect("the job asked").expect("no error"),
        Region::Wait(point(2))
    );

    // At `Shown::stop` the render side leaves by value, as the type it was built with.
    let render: Box<dyn Any> = side.render;
    let render = render.downcast::<ForkingRender>().expect("the object that was moved in");
    assert_eq!(render.opens.get(), 2);
    assert!(render.marks.iter().all(|&mark| mark == 2), "each unit once in each job");
    assert_eq!(render.threads.len(), 2);
    // Both jobs ran off the thread that drives the test.
    assert!(render.threads.iter().all(|&thread| thread != driver));
    assert!(render.slot < executor.slot_count());
    // Width one: the job's own worker runs every child it forks. Width two: a second
    // worker shares them, and no more than two run at any moment. Which worker is the
    // second may change over a scope, since the width counts workers at a moment
    // (`crates/task/README.md`, "The width").
    assert!(render.peak >= 1 && render.peak <= width, "{} at once", render.peak);
    if width == 1 {
        assert!(render.children.iter().all(|(child, job)| child == job));
    } else {
        assert_eq!(render.shared, vec![true, true], "a second worker shared the children");
    }
    drop(render);
    executor.shutdown();
}

#[test]
fn the_frame_job_is_one_job_with_the_render_side_moved_in_and_out_on_two_workers() {
    the_frame_job_on(2);
}

#[test]
fn the_frame_job_is_one_job_with_the_render_side_moved_in_and_out_on_three_workers() {
    the_frame_job_on(3);
}
