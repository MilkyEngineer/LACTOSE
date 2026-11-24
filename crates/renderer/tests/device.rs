//! The renderer on a Vulkan device (`README.md`, "Proof"): the rows that need a shader to
//! run, each an ignored test that `cargo xtask gpu` runs in a process of its own under the
//! Khronos layer and, under MoltenVK, Metal's API and shader validation, and judges by
//! decision 5.10's rule: any message fails it. The shaders are `shaders/renderer/`'s, which
//! the run cooks into the directory `LACTOSE_GPU_SHADERS` names.
//!
//! Under rhi-api the layer's synchronisation validation tracks no shader access, and every
//! dependency between the scatter and the draw passes through device addresses, so a clean
//! run is no evidence for the scatter's barriers. What the scatter wrote is read back
//! instead, through a copy of the instance buffer, which is in device-only memory, into a
//! `Readback` buffer invalidated before the host reads it. A frame held in flight is held
//! in two ways: behind a `ready` point of its target's import, which the host signals once
//! the later frame's writes are made, so the GPU starts the held frame after them; and
//! behind a dispatch at the head of the frame that runs for about a second (the conformance
//! suite's `store.slang` with a lag), so the GPU runs the frame while the host writes. The
//! barrier mutations, which need the GPU made to lag inside a frame, are the desktop's.
//!
//! Each image is compared with a reference the test computes on the CPU, never with an image
//! checked in.

use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, PoisonError};

use lactose_core::{StableKey, Tick};
use lactose_ecs::{World, WorldKind};
use lactose_input_host::LatestSample;
use lactose_math::render::RenderPosition;
use lactose_math::{Quat, Scalar};
use lactose_physics_api::Transform;
use lactose_present_api::{ExtractPass, InstanceRow, InstanceSink, RenderTransform};
use lactose_render_graph::{
    BufferAccess, BufferImport, BufferUse, ComputePassDecl, CopyPassDecl, FrameDecl, GraphDesc,
    ImageImport, RenderGraph,
};
use lactose_renderer::{
    CameraData, CameraLatch, FrameStats, FrameView, InstanceRecord, LatchedCamera, Opened,
    Projection, Renderer, RendererDesc, ScatterEntry, Shaders, shader,
};
use lactose_rhi_api::{
    BufferCopy, BufferDesc, BufferHandle, BufferImageCopy, BufferState, BufferUsage,
    ComputePipelineDesc, ComputePipelineHandle, CookedShader, Device, DeviceRequest, Extent,
    HEAP_SLOTS_DEFAULT, ImageDesc, ImageHandle, ImageMemory, ImageState, ImageUsage, MemoryClass,
    Message, MessageSink, QueueRole, Rect, SAMPLERS_DEFAULT, Sharing, StageDesc, StateOn,
    TimelinePoint,
};
use lactose_rhi_vulkan::VulkanDevice;
use lactose_task::{Executor, JobContext};

use crate::common::{
    PATIENCE, RegionLog, Rig, TARGET, executor, in_job, key, placed, row, scatter, simulation,
    spawn, world_pass,
};

/// The target's extent: square, so that the camera's aspect ratio is one.
const EXTENT: Extent = Extent { width: 128, height: 128 };

const PROJECTION: Projection = Projection { fov_y: 1.0, near: 0.1, far: 1000.0 };

/// The boom: at yaw and pitch zero the eye is four metres from the pivot along `+Z`, and
/// looks along `-Z`.
const BOOM: f32 = 4.0;

/// The bytes of one `EXTENT` image of `Rgba8Unorm`.
const PIXEL_BYTES: u64 = EXTENT.width as u64 * EXTENT.height as u64 * 4;

/// `shader::CLEAR` as `Rgba8Unorm` reads it back.
const CLEAR: [u8; 4] = [0, 0, 0, 255];

/// A sink that keeps what it is sent and prints each message in the form the run's judge
/// counts.
#[derive(Default)]
struct Kept(Mutex<Vec<String>>);

impl MessageSink for Kept {
    fn message(&self, message: &Message<'_>) {
        eprintln!("message: {message:?}");
        self.0.lock().unwrap_or_else(PoisonError::into_inner).push(format!("{message:?}"));
    }
}

/// A device on the adapter `LACTOSE_GPU_ADAPTER` names, or the loader's first, with the
/// default heap and sampler table and no surface.
fn vulkan(sink: &Arc<Kept>) -> Arc<dyn Device> {
    let request = DeviceRequest {
        adapter: std::env::var("LACTOSE_GPU_ADAPTER").ok(),
        surface: None,
        heap_slots: HEAP_SLOTS_DEFAULT,
        samplers: SAMPLERS_DEFAULT,
        pipeline_cache: Vec::new(),
        messages: Arc::clone(sink) as _,
    };
    Arc::new(VulkanDevice::new(request).expect("a Vulkan device"))
}

/// The cooked shader `<dir>/<source>.<entry>`, which `cargo xtask gpu` cooks from
/// `shaders/` into the directory `LACTOSE_GPU_SHADERS` names.
fn cooked(name: &str) -> CookedShader {
    let dir = std::env::var_os("LACTOSE_GPU_SHADERS").expect("LACTOSE_GPU_SHADERS is set");
    let path = std::path::Path::new(&dir).join(format!("{name}.spirv"));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    CookedShader::decode(&bytes).expect("a cooked shader")
}

/// Runs `body` with a renderer on the test's device, then drops the last holder of the
/// device, so that every message of its destroy is sent, fails the test on any message,
/// and says it passed.
fn run(test: &str, desc: Desc, body: impl FnOnce(&mut Gpu)) {
    session(desc, body);
    eprintln!("{test}: passed");
}

/// [`run`]'s device and renderer, without its last word, for a test of several sessions.
fn session(desc: Desc, body: impl FnOnce(&mut Gpu)) {
    let sink = Arc::new(Kept::default());
    let device = vulkan(&sink);
    let mut gpu = Gpu::new(&device, desc);
    body(&mut gpu);
    gpu.finish();
    assert_eq!(Arc::strong_count(&device), 1, "the test holds the device's last reference");
    drop(device);
    let texts = sink.0.lock().unwrap_or_else(PoisonError::into_inner);
    assert!(texts.is_empty(), "the layer or the driver spoke: {texts:?}");
}

// ---- A renderer with its graph, and the targets and readbacks of its frames ----

/// What a test's renderer is made with: `RendererDesc`'s counts, and the sets of a target
/// and its readbacks, one for each frame the test reads back at once.
#[derive(Clone, Copy)]
struct Desc {
    instances: u32,
    rows: u32,
    regions: u32,
    backs: usize,
}

/// One frame's target, and where its copies land: the target's pixels and the instance
/// buffer's records. A target of its own, so that a frame held in flight shares no image
/// with the frame after it.
struct Back {
    target: ImageHandle,
    pixels: BufferHandle,
    records: BufferHandle,
}

/// What a frame is drawn with.
#[derive(Clone, Copy)]
struct Shot {
    /// The camera's pivot.
    pivot: RenderPosition,
    /// The point the target's import waits for, if the frame is held.
    held: Option<TimelinePoint>,
    /// The set the frame draws into and copies into.
    back: usize,
    /// The presentation fraction.
    fraction: f32,
    /// The rounds of [`Lag`]'s dispatch the frame begins with, if any.
    lag: Option<u32>,
}

impl Shot {
    /// A frame that looks at `pivot`, at fraction one, into set 0, with no lag.
    fn at(pivot: RenderPosition) -> Self {
        Self { pivot, held: None, back: 0, fraction: 1.0, lag: None }
    }
}

/// A dispatch that holds the GPU for about a second on the M1: one thread of the
/// conformance suite's `store.slang`, which writes a word through its block's address after
/// `LAG_ROUNDS` rounds of arithmetic or more, into `scratch`, which the frame's copy pass then
/// copies into `back`. The copy keeps the dispatch on the graphics queue, ahead of the
/// frame's passes.
struct Lag {
    pipeline: ComputePipelineHandle,
    scratch: BufferHandle,
    back: BufferHandle,
}

/// `store.slang`'s rounds, as rhi-vulkan's lagging submits give it: [`Lag`]'s first.
const LAG_ROUNDS: u32 = 1 << 25;

/// The times [`holding`] runs a lag again, with twice the rounds, when it did not hold.
const LAG_RETRIES: u32 = 3;

/// The word `store.slang` writes after its rounds.
const LAG_WORD: u32 = 7;

/// What a frame drew.
#[derive(Clone, Copy)]
struct Drawn {
    stats: FrameStats,
    camera: LatchedCamera,
    point: TimelinePoint,
}

/// A renderer with the shaders of `shaders/renderer/` and its graph, and the sets a frame
/// draws and copies into.
struct Parts {
    renderer: Renderer,
    graph: RenderGraph,
    input: LatestSample,
    backs: Vec<Back>,
    lag: Lag,
    instances: u32,
    last: Option<TimelinePoint>,
}

/// The parts, and the executor a frame is recorded in a job of. The parts move into the job
/// and back, so that the test thread steps a world between frames: `Executor::block_on` is
/// not for a worker.
struct Gpu {
    parts: Option<Parts>,
    executor: Executor,
}

impl Deref for Gpu {
    type Target = Parts;

    fn deref(&self) -> &Parts {
        self.parts.as_ref().expect("between frames")
    }
}

impl DerefMut for Gpu {
    fn deref_mut(&mut self) -> &mut Parts {
        self.parts.as_mut().expect("between frames")
    }
}

impl Gpu {
    fn new(device: &Arc<dyn Device>, desc: Desc) -> Self {
        let scatter = cooked("renderer/scatter.cs_main");
        let vertex = cooked("renderer/triangle.vs_main");
        let fragment = cooked("renderer/triangle.fs_main");
        let shaders =
            Shaders { scatter: &scatter, vertex: &vertex, fragment: &fragment, target: TARGET };
        let renderer_desc = RendererDesc {
            instances: desc.instances,
            rows: desc.rows,
            regions: desc.regions,
            shaders: Some(shaders),
        };
        let renderer = Renderer::new(Arc::clone(device), &renderer_desc).expect("a renderer");
        let graph = RenderGraph::new(Arc::clone(device), &GraphDesc { jobs: 2, plans: 4 })
            .expect("a graph");
        let readback = |label, size| {
            let desc = BufferDesc {
                label,
                size,
                usage: BufferUsage::COPY_DESTINATION,
                memory: MemoryClass::Readback,
                sharing: Sharing::Exclusive,
            };
            device.create_buffer(&desc).expect("a readback")
        };
        let target = || {
            let desc = ImageDesc {
                label: "target",
                format: TARGET,
                extent: EXTENT,
                mips: 1,
                layers: 1,
                usage: ImageUsage::COLOUR.union(ImageUsage::COPY_SOURCE),
                memory: ImageMemory::Own,
                sharing: Sharing::Exclusive,
            };
            device.create_image(&desc).expect("a target")
        };
        let store = cooked("conformance/store.cs_main");
        let stage = StageDesc { shader: &store, constants: &[] };
        let pipeline = device
            .create_compute_pipeline(&ComputePipelineDesc { label: "lag", stage })
            .expect("the lag's pipeline");
        let scratch = BufferDesc {
            label: "lag",
            size: 4,
            usage: BufferUsage::DEVICE_ADDRESS.union(BufferUsage::COPY_SOURCE),
            memory: MemoryClass::Device,
            sharing: Sharing::Exclusive,
        };
        let scratch = device.create_buffer(&scratch).expect("the lag's buffer");
        let lag = Lag { pipeline, scratch, back: readback("lag back", 4) };
        let records = u64::from(desc.instances) * u64::from(InstanceRecord::BYTES);
        let backs = (0..desc.backs)
            .map(|_| Back {
                target: target(),
                pixels: readback("pixels back", PIXEL_BYTES),
                records: readback("records back", records),
            })
            .collect();
        let parts = Parts {
            renderer,
            graph,
            input: LatestSample::new(),
            backs,
            lag,
            instances: desc.instances,
            last: None,
        };
        Self { parts: Some(parts), executor: executor(2) }
    }

    /// A frame of [`Parts::frame`], recorded in a job.
    fn frame(&mut self, shot: Shot) -> Drawn {
        let mut parts = self.parts.take().expect("between frames");
        let (parts, drawn) = in_job(&self.executor, move |ctx| {
            let drawn = parts.frame(ctx, shot);
            (parts, drawn)
        });
        self.parts = Some(parts);
        drawn
    }

    /// Waits for the last frame, then destroys the renderer, the graph and what the test
    /// made beside them.
    fn finish(self) {
        let Self { parts, executor } = self;
        executor.shutdown();
        let Parts { renderer, graph, backs, lag, last, .. } = parts.expect("between frames");
        let device = Arc::clone(renderer.device());
        if let Some(point) = last {
            assert!(device.wait_timeline(point, PATIENCE).expect("a wait"), "the last frame");
        }
        drop(graph);
        drop(renderer);
        device.destroy_compute_pipeline(lag.pipeline).expect("a destroy");
        device.destroy_buffer(lag.scratch).expect("a destroy");
        device.destroy_buffer(lag.back).expect("a destroy");
        for back in backs {
            device.destroy_buffer(back.pixels).expect("a destroy");
            device.destroy_buffer(back.records).expect("a destroy");
            device.destroy_image(back.target).expect("a destroy");
        }
    }
}

impl Parts {
    fn device(&self) -> &Arc<dyn Device> {
        self.renderer.device()
    }

    /// Prepares, declares, records, latches and submits a frame, which draws into
    /// `shot.back`'s target and copies the target and the instance buffer into its
    /// readbacks. Any refusal fails the test.
    fn frame(&mut self, ctx: &mut JobContext<'_>, shot: Shot) -> Drawn {
        let stats = self.renderer.prepare().expect("the frame prepares");
        let back = &self.backs[shot.back];
        let size = u64::from(self.instances) * u64::from(InstanceRecord::BYTES);
        let recorded = {
            let mut decl = FrameDecl::new(1);
            let graphics = |state| StateOn { state, queue: QueueRole::Graphics };
            let target = decl.import_image(&ImageImport {
                label: "target",
                image: back.target,
                mips: 1,
                layers: 1,
                sharing: Sharing::Exclusive,
                initial: graphics(ImageState::Undefined),
                last: graphics(ImageState::CopySource),
                handoff: None,
                ready: shot.held,
            });
            let mut readback = |label, buffer| {
                decl.import_buffer(&BufferImport {
                    label,
                    buffer,
                    sharing: Sharing::Exclusive,
                    initial: StateOn { state: BufferState::Undefined, queue: QueueRole::Graphics },
                    last: StateOn { state: BufferState::HostRead, queue: QueueRole::Graphics },
                    ready: None,
                })
            };
            let pixels = readback("pixels back", back.pixels);
            let records = readback("records back", back.records);
            let lag_back = shot.lag.map(|rounds| (rounds, readback("lag back", self.lag.back)));
            let (pipeline, scratch) = (self.lag.pipeline, self.lag.scratch);
            let lag = lag_back.map(|(rounds, back)| {
                let scratch = decl.import_buffer(&BufferImport {
                    label: "lag",
                    buffer: scratch,
                    sharing: Sharing::Exclusive,
                    initial: StateOn { state: BufferState::Undefined, queue: QueueRole::Graphics },
                    last: StateOn { state: BufferState::CopySource, queue: QueueRole::Graphics },
                    ready: None,
                });
                let buffers = [BufferUse { buffer: scratch, access: BufferAccess::Write }];
                let pass =
                    ComputePassDecl { label: "lag", sampled: &[], buffers: &buffers, slots: &[] };
                decl.compute_pass(&pass, move |recorder| {
                    recorder.bind_pipeline(pipeline);
                    recorder.push_address(0, scratch, 0);
                    recorder.push_bytes(8, &1u32.to_le_bytes());
                    recorder.push_bytes(12, &rounds.to_le_bytes());
                    recorder.push_bytes(16, &LAG_WORD.to_le_bytes());
                    recorder.dispatch([1, 1, 1]);
                });
                (scratch, back)
            });
            let view = FrameView { target, extent: EXTENT, fraction: shot.fraction };
            let declared = self.renderer.declare(&mut decl, &view).expect("the frame declares");
            let instances = declared.instances;
            let mut sources = vec![instances];
            let mut destinations = vec![pixels, records];
            if let Some((scratch, back)) = lag {
                sources.push(scratch);
                destinations.push(back);
            }
            decl.copy_pass(
                &CopyPassDecl {
                    label: "read back",
                    image_sources: &[target],
                    image_destinations: &[],
                    buffer_sources: &sources,
                    buffer_destinations: &destinations,
                },
                move |pass| {
                    let rect = Rect { x: 0, y: 0, extent: EXTENT };
                    let whole = BufferImageCopy { buffer_offset: 0, mip: 0, layer: 0, rect };
                    pass.copy_image_to_buffer(target, pixels, &[whole]);
                    let all = BufferCopy { source_offset: 0, destination_offset: 0, size };
                    pass.copy_buffer(instances, records, &[all]);
                    if let Some((scratch, back)) = lag {
                        let word = BufferCopy { source_offset: 0, destination_offset: 0, size: 4 };
                        pass.copy_buffer(scratch, back, &[word]);
                    }
                },
            );
            let compiled = self.graph.compile(decl).expect("the frame compiles");
            self.graph.record(compiled, ctx).expect("the frame records")
        };
        let latch = CameraLatch {
            pivot: shot.pivot,
            input: &self.input,
            boom: BOOM,
            projection: PROJECTION,
        };
        let camera = self.renderer.latch(&latch).expect("the latch");
        let point = self.graph.submit(recorded).expect("the frame submits");
        self.renderer.submitted(point);
        self.last = Some(point);
        Drawn { stats, camera, point }
    }

    /// Opens a region to the writers, waiting for the point that frees one.
    fn open(&mut self) {
        loop {
            match self.renderer.open().expect("the timeline's value") {
                Opened::Ready => return,
                Opened::Wait(point) => self.reached(point),
                Opened::Unsubmitted => panic!("no frame of a device test is torn"),
            }
        }
    }

    fn reached(&self, point: TimelinePoint) {
        let reached = self.device().wait_timeline(point, PATIENCE).expect("a wait");
        assert!(reached, "{point:?} is reached");
    }

    /// A point the host signals, which a held frame's target waits for.
    fn gate(&self) -> TimelinePoint {
        let timeline = self.device().create_timeline(0).expect("a timeline");
        TimelinePoint { timeline, value: 1 }
    }

    /// The pixels set `back`'s readback holds, row by row from the top. Its frame is
    /// reached.
    fn pixels(&self, back: usize) -> Vec<[u8; 4]> {
        let mut out = Vec::new();
        self.read(self.backs[back].pixels, PIXEL_BYTES, |bytes| {
            out.extend(bytes.as_chunks::<4>().0.iter().copied());
        });
        out
    }

    /// The first `count` records set `back`'s readback holds. Its frame is reached.
    fn records(&self, back: usize, count: u32) -> Vec<InstanceRecord> {
        let mut out = Vec::new();
        let len = u64::from(count) * u64::from(InstanceRecord::BYTES);
        self.read(self.backs[back].records, len, |bytes| {
            out.extend(bytes.as_chunks::<128>().0.iter().map(InstanceRecord::from_bytes));
        });
        out
    }

    fn read(&self, buffer: BufferHandle, len: u64, mut take: impl FnMut(&[u8])) {
        let device = self.device();
        device.invalidate_buffer(buffer, 0, len).expect("the invalidate");
        device.read_buffer(buffer, 0, len, &mut |bytes| take(bytes)).expect("the read");
    }
}

// ---- Placements, passes and colours ----

/// A placement at `offset` metres from the centre of `cell`, turned by `angle` radians
/// about `+Z`.
fn placement(cell: [i32; 3], offset: [f32; 3], angle: f32) -> RenderTransform {
    let (sin, cos) = (angle / 2.0).sin_cos();
    RenderTransform { position: RenderPosition { cell, offset }, rotation: [0.0, 0.0, sin, cos] }
}

/// The row of instance `n` at `transform`.
fn moved(n: u64, transform: RenderTransform) -> InstanceRow {
    InstanceRow { transform, ..row(n, 0.0) }
}

/// A pass of a world of `kind` at `tick` that removes `removed` and writes nothing.
fn remove(renderer: &Renderer, kind: WorldKind, tick: Tick, removed: &[StableKey]) {
    let mut sink = renderer.sink();
    sink.begin(ExtractPass { kind, tick, full: false });
    sink.removed(removed);
    sink.end();
}

/// The colour slot `slot` is drawn in, as `Rgba8Unorm` reads it back.
fn colour(slot: usize) -> [u8; 4] {
    shader::PALETTE[slot % shader::PALETTE.len()]
}

/// A pivot in the origin cell at `offset`.
fn pivot(offset: [f32; 3]) -> RenderPosition {
    RenderPosition { cell: [0; 3], offset }
}

// ---- The CPU reference ----

/// How far a pixel's centre must lie inside or outside every edge the reference computes for
/// the device's pixel to be held to it: a quarter of a pixel, decision 5.1's margin.
///
/// Vulkan's least `subPixelPrecisionBits` is 4, so a device snaps each vertex to a sixteenth
/// of a pixel, a thirty-second at most on each axis, which moves an edge by at most
/// `sqrt(2) / 32` of a pixel, under 0.05. The vertex stage computes what the reference
/// computes in 32-bit floats, and differs only in the order and the fusion of its
/// operations: each clip coordinate of these scenes is a sum of four products of at most
/// about a hundred, so the two differ by a few times 1e-5 at most, which the division by a
/// `w` of at least three and the 64 pixels of half the extent leave under 1e-3 of a pixel. A
/// quarter of a pixel holds both five times over.
const MARGIN: f64 = 0.25;

/// What the reference holds a pixel to.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Expected {
    /// Inside one triangle by the margin and outside every other by it: its colour.
    Drawn([u8; 4]),
    /// Outside every triangle by the margin: the clear colour.
    Clear,
    /// Within the margin of an edge: the clear colour or the colour of a triangle whose
    /// edge it is near or that it is inside.
    Either(Vec<[u8; 4]>),
}

/// Where a pixel's centre lies against a triangle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Inside,
    Outside,
    Margin,
}

/// The reference image of `records` drawn at `fraction` with `camera`: each slot's triangle
/// from `InstanceRecord::drawn`, `CellTransform::apply` and `CameraData::clip` in 32-bit
/// floats, as the vertex stage computes it, then each pixel's centre against its edges in
/// `f64`. Clip space is `y` up and the framebuffer's origin its top left.
fn reference(records: &[InstanceRecord], camera: &CameraData, fraction: f32) -> Vec<Expected> {
    let (width, height) = (f64::from(EXTENT.width), f64::from(EXTENT.height));
    let triangles: Vec<([[f64; 2]; 3], [u8; 4])> = records
        .iter()
        .enumerate()
        .filter_map(|(slot, record)| {
            let drawn = record.drawn(fraction, camera.cell)?;
            let window = shader::TRIANGLE.map(|vertex| {
                let [x, y, z, w] = camera.clip(drawn.apply(vertex)).map(f64::from);
                assert!(w > 0.0 && z > 0.0 && z < w, "slot {slot} lies inside the depth range");
                [(x / w + 1.0) / 2.0 * width, (1.0 - y / w) / 2.0 * height]
            });
            Some((window, colour(slot)))
        })
        .collect();
    let mut out = Vec::new();
    for py in 0..EXTENT.height {
        for px in 0..EXTENT.width {
            let centre = [f64::from(px) + 0.5, f64::from(py) + 0.5];
            let mut inside = Vec::new();
            let mut near = Vec::new();
            for &(window, colour) in &triangles {
                match side(&window, centre) {
                    Side::Inside => inside.push(colour),
                    Side::Margin => near.push(colour),
                    Side::Outside => {}
                }
            }
            assert!(inside.len() < 2, "pixel ({px}, {py}) lies inside two triangles");
            out.push(match (inside.as_slice(), near.is_empty()) {
                (&[colour], true) => Expected::Drawn(colour),
                ([], true) => Expected::Clear,
                _ => Expected::Either([inside, near].concat()),
            });
        }
    }
    out
}

/// Where `point` lies against the triangle `window`, by its distance in pixels from each
/// edge.
fn side(window: &[[f64; 2]; 3], point: [f64; 2]) -> Side {
    let [a, b, c] = *window;
    let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    assert!(area.abs() > 1.0, "a triangle of more than a pixel");
    let mut inside = true;
    for (from, to) in [(a, b), (b, c), (c, a)] {
        let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
        let cross = dx * (point[1] - from[1]) - dy * (point[0] - from[0]);
        let distance = cross * area.signum() / dx.hypot(dy);
        if distance <= -MARGIN {
            return Side::Outside;
        }
        inside &= distance >= MARGIN;
    }
    if inside { Side::Inside } else { Side::Margin }
}

/// Holds `pixels` to `reference`, failing at the first wrong pixel, and prints and returns
/// the three counts: surely drawn, surely clear and in the margin.
fn compare(what: &str, pixels: &[[u8; 4]], reference: &[Expected]) -> [usize; 3] {
    assert_eq!(pixels.len(), reference.len());
    let mut counts = [0; 3];
    for (at, (pixel, expected)) in pixels.iter().zip(reference).enumerate() {
        let (x, y) = (at % EXTENT.width as usize, at / EXTENT.width as usize);
        match expected {
            Expected::Drawn(colour) => {
                assert_eq!(pixel, colour, "{what}: pixel ({x}, {y}) is surely drawn");
                counts[0] += 1;
            }
            Expected::Clear => {
                assert_eq!(*pixel, CLEAR, "{what}: pixel ({x}, {y}) is surely clear");
                counts[1] += 1;
            }
            Expected::Either(near) => {
                assert!(
                    *pixel == CLEAR || near.contains(pixel),
                    "{what}: pixel ({x}, {y}) is {pixel:?}, neither clear nor of {near:?}"
                );
                counts[2] += 1;
            }
        }
    }
    eprintln!(
        "{what}: surely drawn {}, surely clear {}, in the margin {}, wrong 0",
        counts[0], counts[1], counts[2]
    );
    counts
}

/// How many pixels of `reference` are surely drawn in slot `slot`'s colour.
fn surely(reference: &[Expected], slot: usize) -> usize {
    reference.iter().filter(|&pixel| *pixel == Expected::Drawn(colour(slot))).count()
}

// ---- The rows ----

/// Many duplicates in different workgroups of a large batch: `N` slots made `LIVE` at `c0`
/// by a first frame, then a world pass at a new tick that writes each instance twice, row
/// `i` and row `N + i`, 2^15 rows apart. The records read back: each `base` and `previous`
/// `c0`, and each `current` the kept row. Then the batch on the host: one entry for each
/// slot, each `TAKE_BASE | HAS_BASE`, and `FrameStats::duplicates` `N`.
///
/// The builder sorts a pass's rows by key to resolve them, so a build that keeps both rows
/// of an instance and that sort puts them in adjacent entries, one workgroup; a build
/// without the resolution has no use for the sort and keeps region order, which runs them
/// 512 workgroups apart. Without the resolution and its assertion, the second entry of a
/// slot is `HAS_BASE` alone, and run in order its records match, so the host's check of the
/// flags is what fails it. One that marks both entries `TAKE_BASE | HAS_BASE` makes the
/// later entry's base the earlier's row, which the records show.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn many_duplicates_across_workgroups_scatter_the_kept_row() {
    const N: u32 = 1 << 15;
    let desc = Desc { instances: N, rows: 2 * N + 8, regions: 2, backs: 1 };
    run("many_duplicates_across_workgroups_scatter_the_kept_row", desc, |gpu| {
        let c0 = placed(0.5);
        let first: Vec<InstanceRow> = (1..=u64::from(N)).map(|n| moved(n, c0)).collect();
        world_pass(&gpu.renderer, WorldKind::Server, Tick(1), true, &first);
        let drawn = gpu.frame(Shot::at(pivot([0.0; 3])));
        assert_eq!(drawn.stats.live, N);
        gpu.open();

        let kept = |n: u64| placed(n as f32 / 1024.0);
        let rows: Vec<InstanceRow> = (1..=u64::from(N))
            .map(|n| moved(n, placed(-(n as f32))))
            .chain((1..=u64::from(N)).map(|n| moved(n, kept(n))))
            .collect();
        world_pass(&gpu.renderer, WorldKind::Server, Tick(2), false, &rows);
        let drawn = gpu.frame(Shot::at(pivot([0.0; 3])));
        gpu.reached(drawn.point);
        let records = gpu.records(0, N);
        for n in 1..=u64::from(N) {
            let slot = gpu.renderer.slots().get(WorldKind::Server, key(n));
            let record = records[slot.expect("every instance holds a slot").index() as usize];
            assert_eq!(record.flags, InstanceRecord::LIVE, "instance {n}");
            assert_eq!((record.base, record.previous), (c0, c0), "instance {n}: the base");
            assert_eq!(record.current, kept(n), "instance {n}: the kept row");
        }
        let entries = gpu.renderer.batch().entries();
        let both = ScatterEntry::TAKE_BASE | ScatterEntry::HAS_BASE;
        assert!(entries.iter().all(|entry| entry.flags == both), "every entry's flags");
        let mut named: Vec<u32> = entries.iter().map(|entry| entry.slot.index()).collect();
        named.sort_unstable();
        named.dedup();
        assert_eq!(named.len(), N as usize, "each slot named once");
        assert_eq!((drawn.stats.duplicates, drawn.stats.entries), (N, N));
    });
}

/// How [`held`] holds frame 1 in flight.
#[derive(Clone, Copy, Debug)]
enum Hold {
    /// Behind a gate on its target's import, which the host signals after its writes: the
    /// GPU starts frame 1 after them.
    Gate,
    /// Behind [`Lag`]'s dispatch at its head: the GPU runs frame 1 while the host writes.
    Lag,
}

/// Runs `check` over [`held`]'s frames, each try in a session of its own. Behind a gate
/// frame 1 is held by construction; behind a lag only while the lag outlasts the host's
/// work, which a fast device or a slow host can undo, so a lag that did not hold is run
/// again with twice the rounds, [`LAG_RETRIES`] times at most, and the test fails if none
/// held. Each retry is printed.
fn holding(test: &str, desc: Desc, hold: Hold, check: impl Fn(&mut Gpu, Held)) {
    let mut rounds = LAG_ROUNDS;
    for retry in 0..=LAG_RETRIES {
        let mut was_held = false;
        session(desc, |gpu| {
            if let Some(held) = held(gpu, hold, rounds) {
                was_held = true;
                check(gpu, held);
            }
        });
        if was_held {
            if retry > 0 {
                eprintln!(
                    "{test}: {hold:?} held frame 1 at {rounds} rounds, after {retry} retries"
                );
            }
            return;
        }
        eprintln!("{test}: {hold:?} at {rounds} rounds was reached before the release: retrying");
        rounds *= 2;
    }
    panic!("{test}: {hold:?} never held frame 1 in flight through the host's work");
}

/// What two frames held in flight show: frame 1, held, and frame 2, submitted after it;
/// then `open`'s answer with both regions held, and a pass written whatever it answered;
/// then frame 1 let go. Frame 1's scatter runs after that pass, and its triangles after
/// frame 2's latch.
struct Held {
    first: Drawn,
    second: Drawn,
    opened: Opened,
    row: RenderTransform,
}

/// Instance 1 at `x = 0` in frame 1, which looks at it from the origin's cell, and nothing
/// new in frame 2, which looks from 50 metres along `+X`, where nothing is. `None` where a
/// lag of `rounds` let frame 1's point be reached before the host's work for frame 2 was
/// done, so that the frames show nothing of a frame in flight.
fn held(gpu: &mut Gpu, hold: Hold, rounds: u32) -> Option<Held> {
    let row = placement([0; 3], [0.0; 3], 0.3);
    world_pass(&gpu.renderer, WorldKind::Server, Tick(1), true, &[moved(1, row)]);
    let gate = matches!(hold, Hold::Gate).then(|| gpu.gate());
    let lag = matches!(hold, Hold::Lag).then_some(rounds);
    let first = gpu.frame(Shot { held: gate, lag, ..Shot::at(pivot([0.0; 3])) });
    gpu.open();
    let second = gpu.frame(Shot { back: 1, ..Shot::at(pivot([50.0, 0.0, 0.0])) });
    let opened = gpu.renderer.open().expect("the timeline's value");
    // Into frame 1's region, were it reopened: the same indices, another placement.
    world_pass(&gpu.renderer, WorldKind::Server, Tick(2), true, &[moved(1, placed(-1.0))]);
    let value = gpu.device().timeline_value(first.point.timeline).expect("the value");
    let in_flight = value < first.point.value;
    assert!(in_flight || lag.is_some(), "the gate held frame 1 until the writes were made");
    if let Some(gate) = gate {
        gpu.device().signal_timeline(gate).expect("the gate's signal");
    }
    gpu.reached(first.point);
    gpu.reached(second.point);
    if lag.is_some() {
        let mut word = [0; 4];
        gpu.read(gpu.lag.back, 4, |bytes| word.copy_from_slice(bytes));
        assert_eq!(u32::from_le_bytes(word), LAG_WORD, "the lag ran");
    }
    in_flight.then_some(Held { first, second, opened, row })
}

/// With the GPU made to lag, the ring never overwrites a region a frame in flight reads:
/// frame 1's record read back holds its own row, though a pass was written after frame 2,
/// while both regions were held and before frame 1's scatter ran; and `open` said to wait
/// for frame 1's point. The readback is checked first, so that a build whose `open` ignores
/// the point fails by what the GPU read, not by the answer alone. Once with frame 1 held
/// behind a gate, once behind a lag.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn a_region_a_held_frame_reads_is_not_reopened() {
    let desc = Desc { instances: 4, rows: 16, regions: 2, backs: 2 };
    for hold in [Hold::Gate, Hold::Lag] {
        holding("a_region_a_held_frame_reads_is_not_reopened", desc, hold, |gpu, held| {
            let record = gpu.records(0, 1)[0];
            let scattered = (record.previous, record.current, record.flags);
            let own = (held.row, held.row, InstanceRecord::LIVE);
            assert_eq!(scattered, own, "{hold:?}: frame 1's own row");
            let wait = Opened::Wait(held.first.point);
            assert_eq!(held.opened, wait, "{hold:?}: open waits for frame 1");
            assert_eq!(held.second.stats.entries, 0);
        });
    }
    eprintln!("a_region_a_held_frame_reads_is_not_reopened: passed");
}

/// Frame N's readback shows N's camera after N + 1's has been written: frame 1, held, draws
/// its instance as its own latch's camera shows it, though frame 2's latch, which looks
/// where nothing is, was written before frame 1's triangles ran; and frame 2 draws nothing.
/// A build with one camera buffer for every region draws frame 1 from frame 2's camera.
/// Once with frame 1 held behind a gate, once behind a lag.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn a_held_frame_draws_with_its_own_camera() {
    let desc = Desc { instances: 4, rows: 16, regions: 2, backs: 2 };
    for hold in [Hold::Gate, Hold::Lag] {
        holding("a_held_frame_draws_with_its_own_camera", desc, hold, |gpu, held| {
            let records = gpu.records(0, 1);
            let first = reference(&records, &held.first.camera.data, 1.0);
            assert!(surely(&first, 0) > 100, "frame 1's camera sees the instance");
            compare(&format!("{hold:?}: frame 1"), &gpu.pixels(0), &first);
            let second = reference(&records, &held.second.camera.data, 1.0);
            let nothing = second.iter().all(|pixel| *pixel == Expected::Clear);
            assert!(nothing, "frame 2's camera sees nothing");
            compare(&format!("{hold:?}: frame 2"), &gpu.pixels(1), &second);
        });
    }
    eprintln!("a_held_frame_draws_with_its_own_camera: passed");
}

/// A held frame draws its own records though the next frame's scatter moves them: frame 1,
/// held behind a gate, draws instance 1 at `x = -0.8`; frame 2, submitted after it, moves
/// the instance to `x = 0.8`; then the gate opens. Frame 1's records and image are its own,
/// and frame 2's are the move, from frame 1's placement as its base. Frame 2's scatter is
/// held behind frame 1's draw by the instance buffer's import from `ShaderRead`, and, as
/// frame 2 is declared, also by the entries' import from `Undefined`, a full execution
/// dependency: a build that drops the first alone is correct by the specification, and no
/// device fails it.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn a_held_frame_draws_its_own_records_though_the_next_frame_moves_them() {
    let desc = Desc { instances: 4, rows: 16, regions: 2, backs: 2 };
    run("a_held_frame_draws_its_own_records_though_the_next_frame_moves_them", desc, |gpu| {
        let (from, to) = (placed(-0.8), placed(0.8));
        world_pass(&gpu.renderer, WorldKind::Server, Tick(1), true, &[moved(1, from)]);
        let gate = gpu.gate();
        let first = gpu.frame(Shot { held: Some(gate), ..Shot::at(pivot([0.0; 3])) });
        gpu.open();
        world_pass(&gpu.renderer, WorldKind::Server, Tick(2), false, &[moved(1, to)]);
        let second = gpu.frame(Shot { back: 1, ..Shot::at(pivot([0.0; 3])) });
        assert_eq!(second.stats.entries, 1, "frame 2 scatters the move");
        let value = gpu.device().timeline_value(first.point.timeline).expect("the value");
        assert!(value < first.point.value, "frame 1 was held until frame 2 was submitted");
        gpu.device().signal_timeline(gate).expect("the gate's signal");
        gpu.reached(first.point);
        gpu.reached(second.point);
        let records = gpu.records(0, 1);
        assert_eq!((records[0].previous, records[0].current), (from, from), "frame 1's own");
        let expected = reference(&records, &first.camera.data, 1.0);
        assert!(surely(&expected, 0) > 100, "frame 1 shows the instance");
        compare("frame 1", &gpu.pixels(0), &expected);
        let records = gpu.records(1, 1);
        assert_eq!((records[0].base, records[0].current), (from, to), "frame 2's move");
        compare("frame 2", &gpu.pixels(1), &reference(&records, &second.camera.data, 1.0));
    });
}

/// The cells of the far scene along `x`: 2^25 + 1, where a 32-bit float's spacing is four,
/// so that a cell converted to a float before the camera's is subtracted loses the
/// neighbour's difference of one.
const FAR: i32 = (1 << 25) + 1;

/// Instance 1 in cell `cell` and instance 2 in the next along `+X`, both in view of a
/// camera at `(31, 0, 0)` in cell `cell`: 30.2 metres from the cell's centre, and 32.2
/// metres back from the next cell's, 31.8 from this one's. Neither is interpolated, so that
/// each record's previous placement is its current one, and what the shader draws is the
/// row's placement.
fn scene(cell: i32) -> [InstanceRow; 2] {
    let near = placement([cell, 0, 0], [30.2, 0.0, 0.0], 0.0);
    let next = placement([cell + 1, 0, 0], [-32.2, 0.0, 0.0], 0.4);
    [moved(1, near), moved(2, next)].map(|row| InstanceRow { no_interpolate: true, ..row })
}

/// An instance in the camera's cell and one in the next cell along `+X` render the pixels
/// they render at the origin when the scene is moved 2^25 + 1 cells along `+X`: equal
/// images, each the CPU reference of the scene at the origin, both instances drawn. A shader
/// that converts the cells to floats before it subtracts them puts the neighbour 64 metres
/// back, out of view.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn a_scene_far_from_the_origin_renders_as_at_the_origin() {
    let desc = Desc { instances: 4, rows: 16, regions: 2, backs: 2 };
    run("a_scene_far_from_the_origin_renders_as_at_the_origin", desc, |gpu| {
        let camera = |cell| RenderPosition { cell: [cell, 0, 0], offset: [31.0, 0.0, 0.0] };
        world_pass(&gpu.renderer, WorldKind::Server, Tick(1), true, &scene(0));
        let origin = gpu.frame(Shot::at(camera(0)));
        gpu.open();
        world_pass(&gpu.renderer, WorldKind::Server, Tick(2), true, &scene(FAR));
        let far = gpu.frame(Shot { back: 1, ..Shot::at(camera(FAR)) });
        gpu.reached(far.point);
        assert_eq!(origin.camera.data.clip_from_cell, far.camera.data.clip_from_cell);
        let expected = reference(&gpu.records(0, 2), &origin.camera.data, 1.0);
        for slot in 0..2 {
            assert!(surely(&expected, slot) > 100, "slot {slot} is in view");
        }
        let at_origin = gpu.pixels(0);
        compare("at the origin", &at_origin, &expected);
        assert!(at_origin == gpu.pixels(1), "the far scene's image is the origin's");
    });
}

/// A record without `LIVE` produces no fragment: slots 0 and 1 drawn in frame 1, then slot 0
/// freed in frame 2, which leaves `SlotTable::extent` at two, so that frame 2 still draws an
/// instance of slot 0. Its triangle's pixels of frame 1 are `CLEAR` in frame 2, and frame 2
/// is the reference of its records, slot 1 alone. A build that ignores `LIVE` draws slot 0
/// where frame 1 drew it; one that draws `SlotTable::live` instances draws slot 0 alone.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn a_freed_slot_produces_no_fragment() {
    let desc = Desc { instances: 4, rows: 16, regions: 2, backs: 2 };
    run("a_freed_slot_produces_no_fragment", desc, |gpu| {
        let rows = [moved(1, placed(-0.8)), moved(2, placed(0.8))];
        world_pass(&gpu.renderer, WorldKind::Server, Tick(1), true, &rows);
        let first = gpu.frame(Shot::at(pivot([0.0; 3])));
        gpu.open();
        remove(&gpu.renderer, WorldKind::Server, Tick(2), &[key(1)]);
        let second = gpu.frame(Shot { back: 1, ..Shot::at(pivot([0.0; 3])) });
        gpu.reached(second.point);
        assert_eq!((second.stats.freed, gpu.renderer.slots().extent()), (1, 2));
        let before = reference(&gpu.records(0, 2), &first.camera.data, 1.0);
        compare("frame 1", &gpu.pixels(0), &before);
        let records = gpu.records(1, 2);
        assert_eq!((records[0].flags, records[1].flags), (0, InstanceRecord::LIVE));
        let after = reference(&records, &second.camera.data, 1.0);
        assert!(surely(&after, 1) > 100, "slot 1, the highest held, is drawn");
        let pixels = gpu.pixels(1);
        compare("frame 2", &pixels, &after);
        let freed: Vec<[u8; 4]> = before
            .iter()
            .zip(&pixels)
            .filter(|&(expected, _)| *expected == Expected::Drawn(colour(0)))
            .map(|(_, &pixel)| pixel)
            .collect();
        assert!(freed.len() > 100, "slot 0 covered pixels in frame 1");
        assert!(freed.iter().all(|&pixel| pixel == CLEAR), "slot 0's pixels are clear");
    });
}

/// Slots freed by one run of the batch and given by a later one are written by their
/// dispatches in recording order: frame 2's `Server` pass removes its `N` instances, and its
/// `Prediction` pass, applied after it, places `N` new ones, which take the same slots. The
/// batch names every slot `FREE` in one dispatch and the new instance's in the next, and
/// each record read back is its new instance's, `LIVE`. The barrier between the two
/// dispatches is what orders them, and nothing else does; Metal's serial compute encoder
/// and lavapipe order them anyway, so a build that drops it is the desktop's, with the
/// `FREE` dispatch made to lag (`README.md`, "Where the device tests run").
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn slots_freed_by_one_run_and_given_by_the_next_hold_the_new_instances() {
    const N: u32 = 1 << 16;
    let desc = Desc { instances: N, rows: 2 * N + 8, regions: 2, backs: 1 };
    run("slots_freed_by_one_run_and_given_by_the_next_hold_the_new_instances", desc, |gpu| {
        let keys = 1..=u64::from(N);
        let rows: Vec<InstanceRow> = keys.clone().map(|n| row(n, n as f32 / 1024.0)).collect();
        world_pass(&gpu.renderer, WorldKind::Server, Tick(1), true, &rows);
        gpu.frame(Shot::at(pivot([0.0; 3])));
        gpu.open();
        let gone: Vec<StableKey> = keys.clone().map(key).collect();
        remove(&gpu.renderer, WorldKind::Server, Tick(2), &gone);
        let given = |n: u64| placement([0; 3], [-(n as f32) / 1024.0, 0.5, 0.0], 0.7);
        let new = |n: u64| u64::from(N) + n;
        let rows: Vec<InstanceRow> = keys.clone().map(|n| moved(new(n), given(n))).collect();
        world_pass(&gpu.renderer, WorldKind::Prediction, Tick(2), true, &rows);
        let drawn = gpu.frame(Shot::at(pivot([0.0; 3])));
        let batch = gpu.renderer.batch();
        let &[freeing, giving] = batch.dispatches() else { panic!("two dispatches") };
        assert_eq!((freeing.count, giving.count), (N, N));
        assert!(batch.run(freeing).iter().all(|entry| entry.flags == ScatterEntry::FREE));
        assert!(batch.run(giving).iter().all(|entry| entry.flags != ScatterEntry::FREE));
        gpu.reached(drawn.point);
        let records = gpu.records(0, N);
        for n in keys {
            let slot = gpu.renderer.slots().get(WorldKind::Prediction, key(new(n)));
            let slot = slot.expect("every new instance holds a slot").index();
            assert!(slot < N, "instance {n} takes a freed slot");
            let record = records[slot as usize];
            let held = (record.flags, record.current);
            assert_eq!(held, (InstanceRecord::LIVE, given(n)), "slot {slot}: the new instance");
        }
    });
}

/// The dispatches of a batch of two runs each apply their own entries, all of them and no
/// more: frame 1 places 150 `Server` instances and 50 `Prediction` ones, frame 2 nothing,
/// and frame 3, in frame 1's region, moves 37 of the first and 5 of the second at a new
/// tick, so that each run's entries are `TAKE_BASE | HAS_BASE`, neither count is a multiple
/// of the workgroup, and the entries past the batch are frame 1's. Every record read back
/// is what the scatter's rule gives on the host over the rows the sinks were handed. A build
/// that reads each dispatch's entries from the batch's first, that pushes one entry too
/// many, or that rounds the groups down fails it, and so does one that pushes no rows
/// address.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn the_dispatches_of_two_runs_each_apply_their_own_entries() {
    const SERVER: u64 = 150;
    const PREDICTION: u64 = 50;
    let desc = Desc { instances: 256, rows: 512, regions: 2, backs: 1 };
    run("the_dispatches_of_two_runs_each_apply_their_own_entries", desc, |gpu| {
        let log = RegionLog::default();
        let at = |x: f32, y: f32| placement([0; 3], [x, y, 0.0], 0.0);
        let server = |n: u64, x: f32| moved(n, at(x, 0.0));
        let predicted = |n: u64, y: f32| moved(1000 + n, at(0.0, y));
        let pass = |kind, tick, full| ExtractPass { kind, tick, full };
        let rows: Vec<InstanceRow> = (1..=SERVER).map(|n| server(n, n as f32)).collect();
        log.pass(&gpu.renderer, pass(WorldKind::Server, Tick(1), true), &[], &rows);
        let rows: Vec<InstanceRow> = (1..=PREDICTION).map(|n| predicted(n, n as f32)).collect();
        log.pass(&gpu.renderer, pass(WorldKind::Prediction, Tick(1), true), &[], &rows);
        let mut host = Vec::new();
        gpu.frame(Shot::at(pivot([0.0; 3])));
        scatter(&mut host, gpu.renderer.batch(), &log.take());
        gpu.open();
        gpu.frame(Shot::at(pivot([0.0; 3])));
        scatter(&mut host, gpu.renderer.batch(), &log.take());
        gpu.open();
        let rows: Vec<InstanceRow> = (1..=37).map(|n| server(n, -(n as f32))).collect();
        log.pass(&gpu.renderer, pass(WorldKind::Server, Tick(2), false), &[], &rows);
        let rows: Vec<InstanceRow> = (1..=5).map(|n| predicted(n, -(n as f32))).collect();
        log.pass(&gpu.renderer, pass(WorldKind::Prediction, Tick(2), false), &[], &rows);
        let drawn = gpu.frame(Shot::at(pivot([0.0; 3])));
        let batch = gpu.renderer.batch();
        let counts: Vec<u32> = batch.dispatches().iter().map(|range| range.count).collect();
        assert_eq!(counts, [37, 5], "two runs, neither a multiple of the workgroup");
        let both = ScatterEntry::TAKE_BASE | ScatterEntry::HAS_BASE;
        assert!(batch.entries().iter().all(|entry| entry.flags == both), "every base taken");
        scatter(&mut host, batch, &log.take());
        gpu.reached(drawn.point);
        let given = (SERVER + PREDICTION) as u32;
        assert_eq!(host.len(), given as usize);
        let records = gpu.records(0, given);
        for (slot, (device, host)) in records.iter().zip(&host).enumerate() {
            assert_eq!(device, host, "slot {slot}: the host's rule");
        }
    });
}

/// Turns instance `n` of `world` to `angle` radians about `+Z`.
fn turn(world: &mut World, n: u64, angle: f64) {
    let mut transform = world.get_mut::<Transform>(key(n)).expect("a transform");
    transform.rotation = Quat::from_rotation_z(Scalar::from_f64(angle));
}

/// A world stepped, extracted, rendered and compared with the CPU reference: six instances
/// of a server world in a grid of two rows, each turned, then moved and turned again by a
/// second tick, and drawn halfway between the two. The records read back are those the
/// scatter's rule gives on the host over the rows the sink was handed, and the image is the
/// reference of those records at the margin, with the three counts printed. A build that
/// draws nothing fails at the first surely drawn pixel, and so does one that draws in the
/// clear colour.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn a_world_stepped_and_extracted_renders_as_the_reference() {
    const INSTANCES: u32 = 6;
    let desc = Desc { instances: 8, rows: 64, regions: 2, backs: 1 };
    run("a_world_stepped_and_extracted_renders_as_the_reference", desc, |gpu| {
        let log = RegionLog::default();
        let sink = Box::new(log.sink(gpu.renderer.sink()));
        let mut rig = Rig::new(simulation(WorldKind::Server), sink);
        for n in 1..=u64::from(INSTANCES) {
            // Columns 1.5 metres apart and rows 1.6 apart: more than the 1.42 metres two
            // triangles turned any way need, so that no pixel is inside two.
            let x = 1.5 * ((n - 1) % 3) as f64 - 1.5;
            spawn(rig.world_mut(), n, x);
            let mut transform = rig.world_mut().get_mut::<Transform>(key(n)).expect("a transform");
            transform.translation.y = Scalar::from_f64(if n <= 3 { 0.8 } else { -0.8 });
            turn(rig.world_mut(), n, 0.2 * n as f64);
        }
        let mut host = Vec::new();
        rig.step();
        rig.frame();
        gpu.frame(Shot::at(pivot([0.0; 3])));
        scatter(&mut host, gpu.renderer.batch(), &log.take());
        gpu.open();
        for n in 1..=u64::from(INSTANCES) {
            let mut transform = rig.world_mut().get_mut::<Transform>(key(n)).expect("a transform");
            transform.translation.x += Scalar::from_f64(0.1);
            turn(rig.world_mut(), n, 0.2 * n as f64 + 0.5);
        }
        rig.step();
        rig.frame();
        let drawn = gpu.frame(Shot { fraction: 0.5, ..Shot::at(pivot([0.0; 3])) });
        scatter(&mut host, gpu.renderer.batch(), &log.take());
        assert_eq!(drawn.stats.entries, INSTANCES, "every instance moved");
        gpu.reached(drawn.point);
        let records = gpu.records(0, INSTANCES);
        assert_eq!(records, host, "the scatter is the host's rule");
        assert!(records.iter().all(|r| r.previous != r.current), "each is drawn between two");
        let expected = reference(&records, &drawn.camera.data, 0.5);
        for slot in 0..INSTANCES as usize {
            assert!(surely(&expected, slot) > 100, "slot {slot} is in view");
        }
        compare("the world", &gpu.pixels(0), &expected);
    });
}
