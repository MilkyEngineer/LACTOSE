//! The render side (`crates/app/README.md`, "What `lactose-app-client` wires", step 4): the
//! render graph, the renderer and the presentation schedule, with the two offscreen targets
//! the present thread samples. `tests/shown.rs`, section 1, of `lactose-app` is its sketch.

use std::sync::Arc;

use lactose_app::TargetIndex;
use lactose_app::{
    Draw, FrameCounts, FrameTime, KindCounts, Region, RenderSide, Submitted, TARGETS,
};
use lactose_core::{Error, ErrorKind, Result};
use lactose_ecs::{PresentEmission, PresentEventKey, WorldKind};
use lactose_input_host::LatestSample;
use lactose_math::Vec3;
use lactose_math::render::RenderPosition;
use lactose_present_api::{InstanceSink, PresentFrame};
use lactose_presentation::{EventSink, PresentView, Presentation};
use lactose_render_graph::{
    BufferImport, CopyPassDecl, FrameDecl, GraphDesc, ImageImport, RecordedFrame, RenderGraph,
};
use lactose_renderer::{
    CameraData, CameraLatch, FrameStats, FrameView, InstanceRecord, Opened, Projection, Renderer,
    RendererDesc, Shaders,
};
use lactose_rhi_api::{
    BufferCopy, BufferDesc, BufferHandle, BufferImageCopy, BufferState, BufferUsage, Device,
    Extent, Format, ImageDesc, ImageHandle, ImageMemory, ImageState, ImageUsage, MemoryClass,
    QueuePath, QueueRole, Rect, Rule, Sharing, StateOn, TimelinePoint,
};
use lactose_task::JobContext;

use crate::extracted::Extracted;
use crate::shaders::CookedShaders;

/// The offscreen targets' format: the renderer's pipelines are made for it.
pub const TARGET_FORMAT: Format = Format::Rgba8Unorm;

/// The instance slots the renderer keeps: the scene's seven rows and room to spare.
pub const INSTANCES: u32 = 64;

/// The camera's projection.
pub const PROJECTION: Projection = Projection { fov_y: 1.0, near: 0.1, far: 1000.0 };

/// The camera's distance from its pivot, in metres, while the presentation schedule names
/// none: the scene has no camera rig, so its pivot is the origin and its distance this.
pub const BOOM: f32 = 14.0;

/// Each pixel of a readback, `TARGET_FORMAT`'s four bytes.
pub const PIXEL_BYTES: u32 = 4;

const GRAPHICS: QueueRole = QueueRole::Graphics;

/// The bound of the render side's wait for each of its last points when it is dropped.
const SETTLE: std::time::Duration = std::time::Duration::from_secs(1);

fn on<S>(state: S, queue: QueueRole) -> StateOn<S> {
    StateOn { state, queue }
}

fn reached(device: &dyn Device, point: Option<TimelinePoint>) -> Result<bool> {
    match point {
        None => Ok(true),
        Some(point) => Ok(device.timeline_value(point.timeline)? >= point.value),
    }
}

/// One offscreen target.
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

/// The fixed-frame mode's copies of a frame: its target's pixels and the renderer's instance
/// records, each frame at the readback's extent into the buffers of the target it drew.
struct Readback {
    extent: Extent,
    pixels: [BufferHandle; TARGETS],
    records: [BufferHandle; TARGETS],
    /// What each target's buffers were last copied from, `None` where the last frame that
    /// drew the target was at another extent and copied nothing.
    drawn: [Option<Copied>; TARGETS],
    /// The placements extract handed the renderer.
    extracted: Extracted,
}

/// What a frame copied into its target's buffers was drawn with.
#[derive(Clone)]
struct Copied {
    camera: CameraData,
    fraction: f32,
    /// The records the scatter should have written, or why they are not known.
    expected: core::result::Result<Vec<InstanceRecord>, String>,
}

/// The frame recorded and not yet submitted.
struct Recorded {
    frame: RecordedFrame,
    target: TargetIndex,
    extent: Extent,
}

/// What a fixed frame drew with and what it drew: decision 5.1's dump and the image read.
#[derive(Clone, Debug)]
pub struct LastFrame {
    /// The target's extent.
    pub extent: Extent,
    /// The target's pixels, row by row from the top left.
    pub pixels: Vec<[u8; 4]>,
    /// The renderer's instance records the frame drew from, slot by slot below the slot
    /// table's extent, as the device's scatter wrote them. The records above it are
    /// undefined and not read.
    pub records: Vec<InstanceRecord>,
    /// The records the placements extract handed the renderer give, slot by slot over the
    /// same slots: what the scatter should have written, and what the reference is
    /// rasterised from (decision 5.1).
    pub expected: Vec<InstanceRecord>,
    /// The camera the frame latched.
    pub camera: CameraData,
    /// The interpolation fraction.
    pub fraction: f32,
}

/// What the presentation schedule's events reach: nothing at milestone 2, which has no audio
/// and no effects.
struct NoEvents;

impl EventSink for NoEvents {
    fn emitted(&mut self, _: &PresentEmission) {}

    fn cancelled(&mut self, _: PresentEventKey) {}
}

/// The render side `lactose-app-client` hands `build_shown`.
pub struct ClientRender {
    graph: RenderGraph,
    renderer: Renderer,
    /// The frame recorded and not yet submitted.
    recorded: Option<Recorded>,
    /// What the binary publishes and the latch reads.
    input: Arc<LatestSample>,
    schedule: Presentation,
    pivot: RenderPosition,
    boom: f32,
    fraction: f32,
    /// The target the last submitted frame drew.
    last: Option<TargetIndex>,
    /// With a readback, the records the frame recorded and not yet submitted should leave,
    /// or why they are not known.
    expected: Option<core::result::Result<Vec<InstanceRecord>, String>>,
    /// Last, so that the graph that submitted the renderer's frames and then the renderer
    /// are dropped before the images and buffers are destroyed (renderer, "Errors and
    /// drops").
    images: Images,
}

/// The images and buffers the render side made, destroyed when it is dropped: off the
/// frame-critical workers, after the present thread has stopped, when nothing pending names
/// them.
struct Images {
    device: Arc<dyn Device>,
    targets: [Option<Target>; TARGETS],
    retired: Vec<Retired>,
    readback: Option<Readback>,
}

impl Drop for Images {
    fn drop(&mut self) {
        // "Shutdown", step 4: the last points, each within a bound. Past it, or on a lost
        // device, the device refuses a destroy as pending and the object is freed with the
        // device.
        let drawn = self.targets.iter().flatten().map(|target| target.drawn);
        for point in drawn.chain(self.retired.iter().map(|retired| retired.drawn)).flatten() {
            let _ = self.device.wait_timeline(point, SETTLE);
        }
        for image in self.targets.iter().flatten().map(|target| target.image) {
            let _ = self.device.destroy_image(image);
        }
        for retired in &self.retired {
            let _ = self.device.destroy_image(retired.image);
        }
        if let Some(readback) = &self.readback {
            for buffer in readback.pixels.iter().chain(&readback.records) {
                let _ = self.device.destroy_buffer(*buffer);
            }
        }
    }
}

impl ClientRender {
    /// The render side over `device`, with `shaders`' renderer stages, latching `input`.
    /// With `readback`, each frame at that extent also copies its target and the instance
    /// records into buffers the fixed-frame mode reads after the run ([`Self::last_frame`]).
    ///
    /// # Errors
    ///
    /// The device's, the renderer's and the graph's.
    pub fn new(
        device: &Arc<dyn Device>,
        shaders: &CookedShaders,
        input: Arc<LatestSample>,
        readback: Option<Extent>,
    ) -> Result<Self> {
        let desc = RendererDesc {
            instances: INSTANCES,
            rows: INSTANCES,
            regions: 2,
            shaders: Some(Shaders {
                scatter: &shaders.scatter,
                vertex: &shaders.vertex,
                fragment: &shaders.fragment,
                target: TARGET_FORMAT,
            }),
        };
        let renderer = Renderer::new(Arc::clone(device), &desc)?;
        let graph = RenderGraph::new(Arc::clone(device), &GraphDesc { jobs: 2, plans: 2 })?;
        let readback = match readback {
            Some(extent) => Some(Readback::new(&**device, extent)?),
            None => None,
        };
        Ok(Self {
            graph,
            renderer,
            recorded: None,
            input,
            schedule: Presentation::new(),
            pivot: RenderPosition::from_sim(Vec3::ZERO),
            boom: BOOM,
            fraction: 0.0,
            last: None,
            expected: None,
            images: Images {
                device: Arc::clone(device),
                targets: [None; TARGETS],
                retired: Vec::new(),
                readback,
            },
        })
    }

    /// The device the render side draws on.
    #[must_use]
    pub fn device(&self) -> &Arc<dyn Device> {
        self.graph.device()
    }

    /// What the last submitted frame drew with and what it drew, once its point is reached:
    /// `None` without a readback, before any frame was submitted, or where the last frame's
    /// extent was not the readback's. Called after `Shown::stop`, off the workers.
    ///
    /// # Errors
    ///
    /// The device's; `Cancelled` where the frame's point is not reached within a second; and
    /// `InvalidData` where the placements extract handed the renderer are not known at the
    /// last frame, naming why.
    pub fn last_frame(&self) -> Result<Option<LastFrame>> {
        let (Some(readback), Some(target)) = (&self.images.readback, self.last) else {
            return Ok(None);
        };
        let at = target.index();
        let (Some(drawn), Some(copied)) =
            (self.images.targets[at].and_then(|target| target.drawn), &readback.drawn[at])
        else {
            return Ok(None);
        };
        let expected = copied.expected.clone().map_err(|why| {
            Error::new(ErrorKind::InvalidData, format!("the extracted placements: {why}"))
        })?;
        let device = self.device();
        if !device.wait_timeline(drawn, std::time::Duration::from_secs(1))? {
            return Err(Error::new(ErrorKind::Cancelled, "the last frame's point was not reached"));
        }
        let mut pixels = Vec::new();
        read(&**device, readback.pixels[at], readback.pixel_bytes(), |bytes| {
            pixels.extend(bytes.as_chunks::<4>().0.iter().copied());
        })?;
        let mut records = Vec::new();
        let size = Readback::record_bytes(expected.len());
        if size > 0 {
            read(&**device, readback.records[at], size, |bytes| {
                records.extend(bytes.as_chunks::<128>().0.iter().map(InstanceRecord::from_bytes));
            })?;
        }
        let (camera, fraction) = (copied.camera, copied.fraction);
        Ok(Some(LastFrame { extent: readback.extent, pixels, records, expected, camera, fraction }))
    }

    /// The image of `draw.target` at `draw.extent`, and whether it is new. A retired image is
    /// destroyed once the frame that last drew it and the present thread's last read of it
    /// have both completed, which `timeline_value` says without a wait; the device refuses
    /// the destroy while a heap slot holds the image, and the present side's slot lets go at
    /// the first call that names another image for the index, or at `close`.
    ///
    /// A retired image whose points cannot be read or whose destroy fails with an error other
    /// than a refusal as pending stays retired, and the first such error is returned once
    /// every retired image was tried. The old image of `draw.target` is retired before the
    /// new one is made, so a failed make leaves the slot empty, and the next frame that
    /// draws the target makes it as new.
    fn target(&mut self, draw: Draw) -> Result<(ImageHandle, bool)> {
        let device = Arc::clone(self.graph.device());
        let mut kept = Vec::new();
        let mut failed = None;
        for retired in self.images.retired.drain(..) {
            let done = reached(&*device, retired.drawn)
                .and_then(|drawn| Ok(drawn && reached(&*device, retired.read)?))
                .and_then(|done| {
                    if done {
                        device.destroy_image(retired.image).map(|()| true)
                    } else {
                        Ok(false)
                    }
                });
            match done {
                Ok(true) => {}
                Ok(false) => kept.push(retired),
                Err(error) => {
                    if Rule::of(&error) != Some(Rule::Pending) {
                        failed.get_or_insert(error);
                    }
                    kept.push(retired);
                }
            }
        }
        self.images.retired = kept;
        if let Some(error) = failed {
            return Err(error);
        }
        let slot = &mut self.images.targets[draw.target.index()];
        if let Some(target) = *slot
            && target.extent == draw.extent
        {
            return Ok((target.image, false));
        }
        if let Some(old) = slot.take() {
            // `draw.ready` is the present thread's last read of the old image.
            self.images.retired.push(Retired {
                image: old.image,
                drawn: old.drawn,
                read: draw.ready,
            });
        }
        let image = device.create_image(&ImageDesc {
            label: "target",
            format: TARGET_FORMAT,
            extent: draw.extent,
            mips: 1,
            layers: 1,
            usage: ImageUsage::COLOUR.union(ImageUsage::SAMPLED).union(ImageUsage::COPY_SOURCE),
            memory: ImageMemory::Own,
            sharing: Sharing::Exclusive,
        })?;
        *slot = Some(Target { image, extent: draw.extent, drawn: None });
        Ok((image, true))
    }

    /// Ends the prepared frame as torn. The placements extract handed the renderer are
    /// not known from then on: the renderer forgets them, and extract writes them again.
    fn torn(&mut self) {
        self.renderer.torn();
        self.expected = None;
        if let Some(readback) = &mut self.images.readback {
            readback.extracted.break_for("a torn frame");
        }
    }

    /// Declares, compiles and records the frame against a shared borrow of the renderer,
    /// which ends when the recorded frame is returned.
    fn recorded_frame(
        renderer: &Renderer,
        graph: &mut RenderGraph,
        ctx: &mut JobContext<'_>,
        target: &ImageImport<'_>,
        view: (Extent, f32),
        readback: Option<(&Readback, usize)>,
    ) -> Result<RecordedFrame> {
        let (extent, fraction) = view;
        let mut decl = FrameDecl::new(1);
        let image = decl.import_image(target);
        let declared =
            renderer.declare(&mut decl, &FrameView { target: image, extent, fraction })?;
        if let Some((readback, at)) = readback.filter(|(readback, _)| readback.extent == extent) {
            // After the triangles and before the hand-off: the image read is the one the
            // graph rendered, never one taken back from the present role (rhi-api,
            // "States"). Each buffer's last use completed before this frame began, since
            // the fixed-frame mode waits for every frame's point.
            let host = |buffer, label| BufferImport {
                label,
                buffer,
                sharing: Sharing::Exclusive,
                initial: on(BufferState::Undefined, GRAPHICS),
                last: on(BufferState::HostRead, GRAPHICS),
                ready: None,
            };
            let pixels = decl.import_buffer(&host(readback.pixels[at], "pixels"));
            let records = decl.import_buffer(&host(readback.records[at], "records"));
            // The records below the slot table's extent, every one written by a scatter;
            // those above it are undefined, and none is copied where the extent is zero.
            let instances = declared.instances;
            let record_bytes = Readback::record_bytes(renderer.slots().extent() as usize);
            let (sources, destinations) = if record_bytes == 0 {
                (&[][..], &[pixels][..])
            } else {
                (&[instances][..], &[pixels, records][..])
            };
            decl.copy_pass(
                &CopyPassDecl {
                    label: "read back",
                    image_sources: &[image],
                    image_destinations: &[],
                    buffer_sources: sources,
                    buffer_destinations: destinations,
                },
                move |pass| {
                    let rect = Rect { x: 0, y: 0, extent };
                    pass.copy_image_to_buffer(
                        image,
                        pixels,
                        &[BufferImageCopy { buffer_offset: 0, mip: 0, layer: 0, rect }],
                    );
                    if record_bytes > 0 {
                        pass.copy_buffer(
                            instances,
                            records,
                            &[BufferCopy {
                                source_offset: 0,
                                destination_offset: 0,
                                size: record_bytes,
                            }],
                        );
                    }
                },
            );
        }
        let compiled = graph.compile(decl)?;
        graph.record(compiled, ctx)
    }
}

impl Readback {
    fn new(device: &dyn Device, extent: Extent) -> Result<Self> {
        let buffer = |label, size| {
            device.create_buffer(&BufferDesc {
                label,
                size,
                usage: BufferUsage::COPY_DESTINATION,
                memory: MemoryClass::Readback,
                sharing: Sharing::Exclusive,
            })
        };
        let pixel_bytes =
            u64::from(extent.width) * u64::from(extent.height) * u64::from(PIXEL_BYTES);
        Ok(Self {
            extent,
            pixels: [buffer("pixels", pixel_bytes)?, buffer("pixels", pixel_bytes)?],
            records: [
                buffer("records", Self::record_bytes(INSTANCES as usize))?,
                buffer("records", Self::record_bytes(INSTANCES as usize))?,
            ],
            drawn: core::array::from_fn(|_| None),
            extracted: Extracted::new(),
        })
    }

    fn pixel_bytes(&self) -> u64 {
        u64::from(self.extent.width) * u64::from(self.extent.height) * u64::from(PIXEL_BYTES)
    }

    /// The bytes of `records` instance records.
    const fn record_bytes(records: usize) -> u64 {
        records as u64 * InstanceRecord::BYTES as u64
    }
}

fn read(
    device: &dyn Device,
    buffer: BufferHandle,
    len: u64,
    mut take: impl FnMut(&[u8]),
) -> Result<()> {
    device.invalidate_buffer(buffer, 0, len)?;
    device.read_buffer(buffer, 0, len, &mut |bytes| take(bytes))
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
                // After torn frames alone: a frame with no pass still signals a point. This
                // is the one submit outside `submit`, which is why the frame calls `open`
                // with no present call in flight where the roles share a queue.
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
        let sink = self.renderer.sink();
        match &mut self.images.readback {
            Some(readback) => Box::new(readback.extracted.recording(sink)),
            None => Box::new(sink),
        }
    }

    fn present(
        &mut self,
        _ctx: &mut JobContext<'_>,
        frame: &PresentFrame<'_>,
        time: FrameTime,
    ) -> Result<()> {
        let view = PresentView { fraction: time.fraction, focus: None, remote: None };
        let output = self.schedule.frame(frame, &view, &mut NoEvents);
        // With no rig in the scene the schedule names no camera, and the frame keeps its own.
        if let Some(pose) = output.camera {
            self.pivot = pose.pivot;
            self.boom = pose.boom;
        }
        self.fraction = time.fraction;
        Ok(())
    }

    fn draw(&mut self, ctx: &mut JobContext<'_>, draw: Draw) -> Result<FrameCounts> {
        // Nothing is prepared yet: an error here ends no frame.
        let (image, new) = self.target(draw)?;
        // The `Client` world's pass ended inside `Simulation::run_passes`, before this job
        // began, so the batch begins after it (renderer, "Writers").
        {
            let writer = self.renderer.remote();
            lactose_presentation::upload(ctx, self.schedule.remote(), |rows| writer.write(rows));
        }
        if let Some(readback) = &mut self.images.readback {
            readback.extracted.before_prepare();
        }
        let stats = match self.renderer.prepare() {
            Ok(stats) => stats,
            Err(error) => {
                // A device's error left the frame prepared, to be ended here; after a
                // refusal `torn` does nothing (renderer, "Errors and drops").
                self.torn();
                return Err(error);
            }
        };
        let expected = self.images.readback.as_mut().map(|readback| {
            let extracted = &mut readback.extracted;
            if stats.any_lost() || stats.dropped > 0 || stats.unplaced > 0 || stats.duplicates > 0 {
                extracted
                    .break_for(format!("a frame lost, dropped or did not place rows: {stats:?}"));
            }
            extracted
                .records(self.renderer.slots())
                .ok_or_else(|| extracted.broken().unwrap_or_default().to_owned())
        });
        // Taken from `Undefined` in a submit that waits for the present thread's last read
        // of this image, and handed on to the present role (rhi-api, "The present thread";
        // graph, "Imports"). A new image has no read to wait for.
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
        let at = draw.target.index();
        let readback = self.images.readback.as_ref().map(|readback| (readback, at));
        match Self::recorded_frame(
            &self.renderer,
            &mut self.graph,
            ctx,
            &import,
            (draw.extent, self.fraction),
            readback,
        ) {
            Ok(frame) => {
                self.recorded = Some(Recorded { frame, target: draw.target, extent: draw.extent });
                self.expected = expected;
                Ok(counts_of(&stats))
            }
            Err(error) => {
                // The frame was prepared and will not be submitted.
                self.torn();
                Err(error)
            }
        }
    }

    fn submit(&mut self) -> Result<Submitted> {
        let Recorded { frame, target, extent } = self
            .recorded
            .take()
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "no frame is recorded"))?;
        let expected = self.expected.take();
        let latch = CameraLatch {
            pivot: self.pivot,
            input: &self.input,
            boom: self.boom,
            projection: PROJECTION,
        };
        let latched = match self.renderer.latch(&latch) {
            Ok(latched) => latched,
            Err(error) => {
                self.torn();
                return Err(error);
            }
        };
        let point = match self.graph.submit(frame) {
            Ok(point) => point,
            Err(error) => {
                self.torn();
                return Err(error);
            }
        };
        self.renderer.submitted(point);
        let at = target.index();
        let slot = self.images.targets[at].as_mut().expect("`draw` made the target");
        slot.drawn = Some(point);
        let image = slot.image;
        if let Some(readback) = &mut self.images.readback {
            // A frame at another extent copied nothing into the target's buffers.
            readback.drawn[at] = expected
                .filter(|_| extent == readback.extent)
                .map(|expected| Copied { camera: latched.data, fraction: self.fraction, expected });
        }
        self.last = Some(target);
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
