//! What the tests on null share: a null device under either queue layout, wrapped so that
//! every command an encoder records is kept and every submit's commands are read back in
//! submission order; a job to call the graph from; and the resources and pipelines a frame
//! names.

#![allow(dead_code, reason = "each test target uses part of the shared module")]

use std::any::Any;
use std::ops::Range;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use lactose_core::Result;
use lactose_render_graph::{
    BufferAccess, BufferId, BufferImport, BufferUse, ColourUse, FrameDecl, GraphDesc, ImageId,
    ImageImport, RenderGraph, RenderPassDecl, TransientImage,
};
use lactose_rhi_api::{
    BufferCopy, BufferDesc, BufferHandle, BufferImageCopy, BufferTransition, BufferUsage,
    Capabilities, ComputePass, ComputePipelineDesc, ComputePipelineHandle, CookedShader, Device,
    DeviceInfo, DeviceRequest, Encoder, Extent, Format, GraphicsPipelineDesc,
    GraphicsPipelineHandle, HeapSlot, ImageDesc, ImageHandle, ImageMemory, ImageTransition,
    ImageUsage, MemoryBlockDesc, MemoryBlockHandle, MemoryClass, Message, MessageSink,
    PipelineStats, Placement, QueueLayout, QueueRole, Rect, RenderPass, SamplerDesc, SamplerIndex,
    ShaderStage, ShaderTarget, Sharing, StageDesc, StateOn, Submit, Swapchain, SwapchainDesc,
    TimelineHandle, TimelinePoint, Viewport,
};
use lactose_rhi_api::{BufferState, ImageState, Load};
use lactose_rhi_null::NullDevice;
use lactose_task::{ChaosConfig, Executor, GraphBuilder, GraphNode, JobContext};

/// The two capability sets every row with a queue in it runs under.
pub const LAYOUTS: [QueueLayout; 2] = [QueueLayout::ONE, QueueLayout::SEVERAL];

/// The longest a test waits for a point or a job.
pub const PATIENCE: Duration = Duration::from_secs(20);

pub const G: QueueRole = QueueRole::Graphics;
pub const C: QueueRole = QueueRole::Compute;

pub const fn on<S>(state: S, queue: QueueRole) -> StateOn<S> {
    StateOn { state, queue }
}

pub fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Silent;

impl MessageSink for Silent {
    fn message(&self, _: &Message<'_>) {}
}

/// A null device of `set`, wrapped to keep what its encoders record.
pub fn device_of(set: Capabilities) -> Arc<Logged> {
    let request = DeviceRequest {
        adapter: None,
        surface: None,
        heap_slots: set.heap_slots,
        samplers: set.samplers,
        pipeline_cache: Vec::new(),
        messages: Arc::new(Silent),
    };
    let inner = NullDevice::new(request, set).expect("a null device");
    Arc::new(Logged { inner, submitted: Mutex::new(Vec::new()), destroyed: Mutex::new(Vec::new()) })
}

/// A null device of the least capabilities under `queues`.
pub fn device(queues: QueueLayout) -> Arc<Logged> {
    device_of(Capabilities { queues, ..Capabilities::LEAST })
}

/// Whether `device`'s compute role has a queue of its own.
pub fn several(device: &dyn Device) -> bool {
    device.capabilities().queues == QueueLayout::SEVERAL
}

// ---- A job ----

type Body = Box<dyn FnOnce(&mut JobContext<'_>) + Send>;

/// One unit that runs a closure once.
struct Job(Mutex<Option<Body>>);

impl GraphNode for Job {
    fn name(&self) -> &'static str {
        "test::frame"
    }

    fn units(&self) -> u32 {
        1
    }

    fn run(&self, _units: Range<u32>, ctx: &mut JobContext<'_>) {
        let body = lock(&self.0).take().expect("one unit, run once");
        body(ctx);
    }
}

/// An executor of `workers` frame-critical workers.
pub fn executor(workers: u32, chaos: Option<ChaosConfig>) -> Executor {
    let builder = Executor::builder().frame_workers(workers).background_workers(1);
    let builder = match chaos {
        Some(chaos) => builder.chaos(chaos),
        None => builder,
    };
    builder.build().expect("an executor")
}

/// Runs `body` as a job of `executor`'s and returns what it returned.
pub fn in_job_of<R: Send + 'static>(
    executor: &Executor,
    body: impl FnOnce(&mut JobContext<'_>) -> R + Send + 'static,
) -> R {
    let out = Arc::new(Mutex::new(None));
    let kept = Arc::clone(&out);
    let body: Body = Box::new(move |ctx| *lock(&kept) = Some(body(ctx)));
    let mut builder = GraphBuilder::new();
    builder.add_node(Arc::new(Job(Mutex::new(Some(body)))));
    let graph = builder.build().expect("one node");
    executor.block_on(executor.run_graph(graph)).expect("the job runs");
    lock(&out).take().expect("the job ran")
}

/// Runs `body` as a job of a fresh executor of two frame-critical workers.
pub fn in_job<R: Send + 'static>(
    body: impl FnOnce(&mut JobContext<'_>) -> R + Send + 'static,
) -> R {
    let executor = executor(2, None);
    let out = in_job_of(&executor, body);
    executor.shutdown();
    out
}

// ---- Resources ----

pub fn buffer(device: &dyn Device, label: &str, sharing: Sharing) -> BufferHandle {
    let usage = BufferUsage::DEVICE_ADDRESS
        .union(BufferUsage::INDEX)
        .union(BufferUsage::INDIRECT)
        .union(BufferUsage::COPY_SOURCE)
        .union(BufferUsage::COPY_DESTINATION);
    let desc = BufferDesc { label, size: 1 << 16, usage, memory: MemoryClass::Readback, sharing };
    device.create_buffer(&desc).expect("a buffer")
}

pub const EXTENT: Extent = Extent { width: 64, height: 64 };

pub fn image(device: &dyn Device, label: &str, sharing: Sharing) -> ImageHandle {
    image_with(device, label, sharing, 1, 1)
}

pub fn image_with(
    device: &dyn Device,
    label: &str,
    sharing: Sharing,
    mips: u32,
    layers: u32,
) -> ImageHandle {
    let usage = ImageUsage::SAMPLED
        .union(ImageUsage::COLOUR)
        .union(ImageUsage::COPY_SOURCE)
        .union(ImageUsage::COPY_DESTINATION);
    let desc = ImageDesc {
        label,
        format: Format::Rgba8Unorm,
        extent: EXTENT,
        mips,
        layers,
        usage,
        memory: ImageMemory::Own,
        sharing,
    };
    device.create_image(&desc).expect("an image")
}

/// A compute pipeline of one invocation a workgroup, which reads and writes nothing it
/// declares: null judges the pass's declaration, not the shader.
pub fn compute_pipeline(device: &dyn Device) -> ComputePipelineHandle {
    let shader = CookedShader {
        stage: ShaderStage::Compute,
        target: ShaderTarget::SpirV,
        entry: "main".to_owned(),
        code: [0x0723_0203_u32, 0x0001_0000, 0, 1, 0]
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect(),
        heap: None,
        constants: Vec::new(),
        push_bytes: 16,
        inputs: 0,
        outputs: 0,
        workgroup: Some([1, 1, 1]),
        varyings: Vec::new(),
    };
    let desc =
        ComputePipelineDesc { label: "test", stage: StageDesc { shader: &shader, constants: &[] } };
    device.create_compute_pipeline(&desc).expect("a compute pipeline")
}

/// Takes `buffer` from `Undefined` to `state` on `queue`, outside any graph, and waits.
pub fn bring_buffer(
    device: &dyn Device,
    buffer: BufferHandle,
    to: StateOn<lactose_rhi_api::BufferState>,
) {
    let mut encoder = device.create_encoder(to.queue, "test").expect("an encoder");
    encoder.transition_buffers(&[BufferTransition {
        buffer,
        from: on(lactose_rhi_api::BufferState::Undefined, to.queue),
        to,
    }]);
    submit_alone(device, to.queue, encoder, &[]);
}

/// Submits `encoder` alone to `queue` after `waits`, and waits for it.
pub fn submit_alone(
    device: &dyn Device,
    queue: QueueRole,
    encoder: Box<dyn Encoder>,
    waits: &[TimelinePoint],
) {
    let timeline = device.create_timeline(0).expect("a timeline");
    let done = TimelinePoint { timeline, value: 1 };
    let mut encoders = [encoder];
    device
        .submit(queue, Submit { encoders: &mut encoders, waits, signals: &[done] })
        .expect("the test's own submit");
    assert!(device.wait_timeline(done, PATIENCE).expect("a wait"), "the test's submit ran");
}

// ---- The logging device ----

/// What one encoder recorded, a line a command, and the role it was made for.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Recorded {
    pub queue: QueueRole,
    pub commands: Vec<String>,
}

/// A null device whose encoders keep a line for each command they record, and which keeps,
/// for each submit it accepted, its role and its encoders' lines in slice order.
pub struct Logged {
    pub inner: NullDevice,
    submitted: Mutex<Vec<(QueueRole, Vec<Recorded>)>>,
    destroyed: Mutex<Vec<(&'static str, bool)>>,
}

impl Logged {
    /// The submits accepted so far, in the order they were made, and forgets them.
    pub fn take_submitted(&self) -> Vec<(QueueRole, Vec<Recorded>)> {
        core::mem::take(&mut *lock(&self.submitted))
    }

    /// Each destroy of an image, a memory block or a timeline so far, and whether the device
    /// accepted it.
    pub fn destroyed(&self) -> Vec<(&'static str, bool)> {
        lock(&self.destroyed).clone()
    }

    fn destroy(&self, kind: &'static str, result: Result<()>) -> Result<()> {
        lock(&self.destroyed).push((kind, result.is_ok()));
        result
    }
}

struct LoggedEncoder {
    inner: Option<Box<dyn Encoder>>,
    queue: QueueRole,
    commands: Vec<String>,
}

impl LoggedEncoder {
    fn inner(&mut self) -> &mut dyn Encoder {
        &mut **self.inner.as_mut().expect("the encoder is held between submits")
    }

    fn log(&mut self, line: String) {
        self.commands.push(line);
    }
}

impl Encoder for LoggedEncoder {
    fn queue(&self) -> QueueRole {
        self.queue
    }

    fn reset(&mut self) -> Result<()> {
        self.inner().reset()?;
        self.commands.clear();
        Ok(())
    }

    fn transition_images(&mut self, transitions: &[ImageTransition]) {
        self.log(format!("transition_images {transitions:?}"));
        self.inner().transition_images(transitions);
    }

    fn transition_buffers(&mut self, transitions: &[BufferTransition]) {
        self.log(format!("transition_buffers {transitions:?}"));
        self.inner().transition_buffers(transitions);
    }

    fn alias(&mut self, image: ImageHandle) {
        self.log(format!("alias {image:?}"));
        self.inner().alias(image);
    }

    fn copy_buffer(
        &mut self,
        source: BufferHandle,
        destination: BufferHandle,
        regions: &[BufferCopy],
    ) {
        self.log(format!("copy_buffer {source:?} {destination:?} {regions:?}"));
        self.inner().copy_buffer(source, destination, regions);
    }

    fn copy_buffer_to_image(
        &mut self,
        source: BufferHandle,
        destination: ImageHandle,
        regions: &[BufferImageCopy],
    ) {
        self.log(format!("copy_buffer_to_image {source:?} {destination:?} {regions:?}"));
        self.inner().copy_buffer_to_image(source, destination, regions);
    }

    fn copy_image_to_buffer(
        &mut self,
        source: ImageHandle,
        destination: BufferHandle,
        regions: &[BufferImageCopy],
    ) {
        self.log(format!("copy_image_to_buffer {source:?} {destination:?} {regions:?}"));
        self.inner().copy_image_to_buffer(source, destination, regions);
    }

    fn begin_render_pass(&mut self, pass: &RenderPass<'_>) {
        self.log(format!("begin_render_pass {pass:?}"));
        self.inner().begin_render_pass(pass);
    }

    fn begin_compute_pass(&mut self, pass: &ComputePass<'_>) {
        self.log(format!("begin_compute_pass {pass:?}"));
        self.inner().begin_compute_pass(pass);
    }

    fn end_pass(&mut self) {
        self.log("end_pass".to_owned());
        self.inner().end_pass();
    }

    fn set_viewport(&mut self, viewport: Viewport) {
        self.log(format!("set_viewport {viewport:?}"));
        self.inner().set_viewport(viewport);
    }

    fn set_scissor(&mut self, scissor: Rect) {
        self.log(format!("set_scissor {scissor:?}"));
        self.inner().set_scissor(scissor);
    }

    fn bind_graphics_pipeline(&mut self, pipeline: GraphicsPipelineHandle) {
        self.log(format!("bind_graphics_pipeline {pipeline:?}"));
        self.inner().bind_graphics_pipeline(pipeline);
    }

    fn bind_compute_pipeline(&mut self, pipeline: ComputePipelineHandle) {
        self.log(format!("bind_compute_pipeline {pipeline:?}"));
        self.inner().bind_compute_pipeline(pipeline);
    }

    fn bind_index_buffer(&mut self, buffer: BufferHandle, offset: u64) {
        self.log(format!("bind_index_buffer {buffer:?} {offset}"));
        self.inner().bind_index_buffer(buffer, offset);
    }

    fn push_bytes(&mut self, offset: u32, bytes: &[u8]) {
        self.log(format!("push_bytes {offset} {bytes:?}"));
        self.inner().push_bytes(offset, bytes);
    }

    fn push_address(&mut self, offset: u32, buffer: BufferHandle, byte_offset: u64) {
        self.log(format!("push_address {offset} {buffer:?} {byte_offset}"));
        self.inner().push_address(offset, buffer, byte_offset);
    }

    fn push_slot(&mut self, offset: u32, slot: HeapSlot) {
        self.log(format!("push_slot {offset} {slot:?}"));
        self.inner().push_slot(offset, slot);
    }

    fn local_read(&mut self) {
        self.log("local_read".to_owned());
        self.inner().local_read();
    }

    fn draw(&mut self, vertices: Range<u32>, instances: Range<u32>) {
        self.log(format!("draw {vertices:?} {instances:?}"));
        self.inner().draw(vertices, instances);
    }

    fn draw_indexed(&mut self, indices: Range<u32>, base_vertex: i32, instances: Range<u32>) {
        self.log(format!("draw_indexed {indices:?} {base_vertex} {instances:?}"));
        self.inner().draw_indexed(indices, base_vertex, instances);
    }

    fn draw_indexed_indirect(
        &mut self,
        arguments: BufferHandle,
        offset: u64,
        count: u32,
        stride: u32,
    ) {
        self.log(format!("draw_indexed_indirect {arguments:?} {offset} {count} {stride}"));
        self.inner().draw_indexed_indirect(arguments, offset, count, stride);
    }

    fn draw_indexed_indirect_count(
        &mut self,
        arguments: BufferHandle,
        offset: u64,
        count: BufferHandle,
        count_offset: u64,
        max_count: u32,
        stride: u32,
    ) {
        self.log(format!(
            "draw_indexed_indirect_count {arguments:?} {offset} {count:?} {count_offset} \
             {max_count} {stride}"
        ));
        self.inner().draw_indexed_indirect_count(
            arguments,
            offset,
            count,
            count_offset,
            max_count,
            stride,
        );
    }

    fn dispatch(&mut self, groups: [u32; 3]) {
        self.log(format!("dispatch {groups:?}"));
        self.inner().dispatch(groups);
    }

    fn dispatch_indirect(&mut self, arguments: BufferHandle, offset: u64) {
        self.log(format!("dispatch_indirect {arguments:?} {offset}"));
        self.inner().dispatch_indirect(arguments, offset);
    }
}

impl Device for Logged {
    fn info(&self) -> &DeviceInfo {
        self.inner.info()
    }

    fn capabilities(&self) -> &Capabilities {
        self.inner.capabilities()
    }

    fn is_lost(&self) -> bool {
        self.inner.is_lost()
    }

    fn create_buffer(&self, desc: &BufferDesc<'_>) -> Result<BufferHandle> {
        self.inner.create_buffer(desc)
    }

    fn destroy_buffer(&self, buffer: BufferHandle) -> Result<()> {
        self.inner.destroy_buffer(buffer)
    }

    fn buffer_address(&self, buffer: BufferHandle) -> Result<u64> {
        self.inner.buffer_address(buffer)
    }

    fn write_buffer(
        &self,
        buffer: BufferHandle,
        offset: u64,
        len: u64,
        fill: &mut dyn FnMut(&mut [u8]),
    ) -> Result<()> {
        self.inner.write_buffer(buffer, offset, len, fill)
    }

    fn read_buffer(
        &self,
        buffer: BufferHandle,
        offset: u64,
        len: u64,
        read: &mut dyn FnMut(&[u8]),
    ) -> Result<()> {
        self.inner.read_buffer(buffer, offset, len, read)
    }

    fn flush_buffer(&self, buffer: BufferHandle, offset: u64, len: u64) -> Result<()> {
        self.inner.flush_buffer(buffer, offset, len)
    }

    fn invalidate_buffer(&self, buffer: BufferHandle, offset: u64, len: u64) -> Result<()> {
        self.inner.invalidate_buffer(buffer, offset, len)
    }

    fn create_image(&self, desc: &ImageDesc<'_>) -> Result<ImageHandle> {
        self.inner.create_image(desc)
    }

    fn destroy_image(&self, image: ImageHandle) -> Result<()> {
        self.destroy("image", self.inner.destroy_image(image))
    }

    fn image_memoryless(&self, image: ImageHandle) -> Result<bool> {
        self.inner.image_memoryless(image)
    }

    fn image_placement(&self, desc: &ImageDesc<'_>) -> Result<Placement> {
        self.inner.image_placement(desc)
    }

    fn create_memory_block(&self, desc: &MemoryBlockDesc<'_>) -> Result<MemoryBlockHandle> {
        self.inner.create_memory_block(desc)
    }

    fn destroy_memory_block(&self, block: MemoryBlockHandle) -> Result<()> {
        self.destroy("block", self.inner.destroy_memory_block(block))
    }

    fn write_heap_slot(&self, slot: HeapSlot, image: ImageHandle) -> Result<()> {
        self.inner.write_heap_slot(slot, image)
    }

    fn clear_heap_slot(&self, slot: HeapSlot) -> Result<()> {
        self.inner.clear_heap_slot(slot)
    }

    fn create_sampler(&self, desc: &SamplerDesc) -> Result<SamplerIndex> {
        self.inner.create_sampler(desc)
    }

    fn destroy_sampler(&self, sampler: SamplerIndex) -> Result<()> {
        self.inner.destroy_sampler(sampler)
    }

    fn create_graphics_pipeline(
        &self,
        desc: &GraphicsPipelineDesc<'_>,
    ) -> Result<GraphicsPipelineHandle> {
        self.inner.create_graphics_pipeline(desc)
    }

    fn create_compute_pipeline(
        &self,
        desc: &ComputePipelineDesc<'_>,
    ) -> Result<ComputePipelineHandle> {
        self.inner.create_compute_pipeline(desc)
    }

    fn destroy_graphics_pipeline(&self, pipeline: GraphicsPipelineHandle) -> Result<()> {
        self.inner.destroy_graphics_pipeline(pipeline)
    }

    fn destroy_compute_pipeline(&self, pipeline: ComputePipelineHandle) -> Result<()> {
        self.inner.destroy_compute_pipeline(pipeline)
    }

    fn pipeline_cache(&self) -> Result<Vec<u8>> {
        self.inner.pipeline_cache()
    }

    fn pipeline_stats(&self) -> PipelineStats {
        self.inner.pipeline_stats()
    }

    fn create_timeline(&self, initial: u64) -> Result<TimelineHandle> {
        self.inner.create_timeline(initial)
    }

    fn destroy_timeline(&self, timeline: TimelineHandle) -> Result<()> {
        self.destroy("timeline", self.inner.destroy_timeline(timeline))
    }

    fn timeline_value(&self, timeline: TimelineHandle) -> Result<u64> {
        self.inner.timeline_value(timeline)
    }

    fn signal_timeline(&self, point: TimelinePoint) -> Result<()> {
        self.inner.signal_timeline(point)
    }

    fn wait_timeline(&self, point: TimelinePoint, timeout: Duration) -> Result<bool> {
        self.inner.wait_timeline(point, timeout)
    }

    fn create_encoder(&self, queue: QueueRole, label: &str) -> Result<Box<dyn Encoder>> {
        let inner = self.inner.create_encoder(queue, label)?;
        Ok(Box::new(LoggedEncoder { inner: Some(inner), queue, commands: Vec::new() }))
    }

    fn submit(&self, queue: QueueRole, submit: Submit<'_>) -> Result<()> {
        let Submit { encoders, waits, signals } = submit;
        let mut inner = Vec::with_capacity(encoders.len());
        let mut recorded = Vec::with_capacity(encoders.len());
        for encoder in encoders.iter_mut() {
            let logged = (&mut **encoder as &mut dyn Any)
                .downcast_mut::<LoggedEncoder>()
                .expect("an encoder of the logging device");
            inner.push(logged.inner.take().expect("held"));
            recorded.push(Recorded { queue: logged.queue, commands: logged.commands.clone() });
        }
        let result = self.inner.submit(queue, Submit { encoders: &mut inner, waits, signals });
        for (encoder, inner) in encoders.iter_mut().zip(inner) {
            let logged = (&mut **encoder as &mut dyn Any)
                .downcast_mut::<LoggedEncoder>()
                .expect("an encoder of the logging device");
            logged.inner = Some(inner);
        }
        if result.is_ok() {
            lock(&self.submitted).push((queue, recorded));
        }
        result
    }

    fn create_swapchain(&self, desc: &SwapchainDesc) -> Result<Box<dyn Swapchain>> {
        self.inner.create_swapchain(desc)
    }
}

/// The lines of the commands submitted so far, by submit, each encoder's in order.
pub fn lines(submitted: &[(QueueRole, Vec<Recorded>)]) -> Vec<String> {
    submitted
        .iter()
        .flat_map(|(_, encoders)| {
            encoders.iter().flat_map(|encoder| encoder.commands.iter().cloned())
        })
        .collect()
}

/// The lines that start with `what`.
pub fn count(lines: &[String], what: &str) -> usize {
    lines.iter().filter(|line| line.starts_with(what)).count()
}

// ---- Frames ----

/// Compiles, records and submits `decl`.
pub fn frame(
    graph: &mut RenderGraph,
    ctx: &mut JobContext<'_>,
    decl: FrameDecl<'_>,
) -> Result<TimelinePoint> {
    let compiled = graph.compile(decl)?;
    let recorded = graph.record(compiled, ctx)?;
    graph.submit(recorded)
}

/// Waits for `point`, which the test expects reached within [`PATIENCE`].
pub fn reached(device: &dyn Device, point: TimelinePoint) {
    assert!(device.wait_timeline(point, PATIENCE).expect("a wait"), "{point:?} is reached");
}

/// A point of a fresh timeline of the test's, which the host owes: a frame that waits for
/// it stays pending until the test signals it.
pub fn held(device: &dyn Device) -> TimelinePoint {
    TimelinePoint { timeline: device.create_timeline(0).expect("a timeline"), value: 1 }
}

/// A graph on `device` of `jobs` jobs and `plans` plans.
pub fn graph(device: &Arc<Logged>, jobs: u32, plans: u32) -> RenderGraph {
    let device: Arc<dyn Device> = Arc::clone(device) as _;
    RenderGraph::new(device, &GraphDesc { jobs, plans }).expect("a graph")
}

/// The role a compute pass with no graphics-queue dependency runs on.
pub fn home(device: &dyn Device) -> QueueRole {
    if several(device) { C } else { G }
}

pub fn colour(image: ImageId, load: Load<[f32; 4]>) -> ColourUse {
    ColourUse { image, mip: 0, layer: 0, load, local_read: false }
}

/// A render pass that draws into colour attachments and uses nothing else.
pub fn draws<'a>(label: &'a str, colour: &'a [ColourUse]) -> RenderPassDecl<'a> {
    RenderPassDecl { label, colour, depth: None, sampled: &[], buffers: &[], slots: &[] }
}

pub fn writes(buffer: BufferId) -> [BufferUse; 1] {
    [BufferUse { buffer, access: BufferAccess::Write }]
}

pub fn reads(buffer: BufferId) -> [BufferUse; 1] {
    [BufferUse { buffer, access: BufferAccess::Read }]
}

/// A buffer imported from `Undefined` on `queue` and left in `last` there.
pub fn fresh_buffer<'a>(
    label: &'a str,
    buffer: BufferHandle,
    queue: QueueRole,
    last: BufferState,
) -> BufferImport<'a> {
    BufferImport {
        label,
        buffer,
        sharing: Sharing::Exclusive,
        initial: on(BufferState::Undefined, queue),
        last: on(last, queue),
        ready: None,
    }
}

/// An image of one mip and layer imported from `initial` and left in `last`.
pub fn image_import<'a>(
    label: &'a str,
    image: ImageHandle,
    initial: StateOn<ImageState>,
    last: StateOn<ImageState>,
) -> ImageImport<'a> {
    ImageImport {
        label,
        image,
        mips: 1,
        layers: 1,
        sharing: Sharing::Exclusive,
        initial,
        last,
        handoff: None,
        ready: None,
    }
}

pub const SCRATCH: TransientImage<'static> = TransientImage {
    label: "scratch",
    format: Format::Rgba8Unorm,
    extent: EXTENT,
    mips: 1,
    layers: 1,
};

pub const WHOLE: BufferImageCopy = BufferImageCopy {
    buffer_offset: 0,
    mip: 0,
    layer: 0,
    rect: Rect { x: 0, y: 0, extent: EXTENT },
};
