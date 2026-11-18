//! What the contract cases share: the context a body runs in, which knows its case and the
//! capability set it runs under and panics with the case's name; the resources most cases
//! start from; a gate that holds a submit pending by a wait on a point the host signals
//! later; and the minimal SPIR-V modules the pipelines are made from. A module here is
//! well formed and does nothing: null runs no shader, and a device runs these only in
//! contract cases, which draw no pixel anyone reads.

use std::fmt::{Debug, Display};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use lactose_core::{ErrorKind, Result};

use super::{Case, Harness};
use crate::{
    BufferDesc, BufferHandle, BufferImageCopy, BufferState, BufferTransition, BufferUsage,
    Capabilities, ColourAttachment, ColourTargetDesc, Compare, ComputePass, ComputePipelineDesc,
    ComputePipelineHandle, CookedShader, Cull, DepthDesc, Device, Encoder, Extent, Format,
    GraphicsPipelineDesc, GraphicsPipelineHandle, HEAP_SET, HeapArray, HeapArrayKind, HeapForm,
    HeapUse, ImageDesc, ImageHandle, ImageMemory, ImageRange, ImageState, ImageTransition,
    ImageUsage, Load, MemoryClass, Message, MessageSink, PassReach, QueueRole, Rect, RenderPass,
    Rule, ShaderStage, ShaderTarget, Sharing, StageDesc, StateOn, Store, Submit, Target,
    TimelineHandle, TimelinePoint, WriteMask,
};

/// A case's body under one capability set: `Err` names why the case does not apply to
/// the set, and a failure panics.
pub(super) type Body = fn(&Ctx<'_>) -> core::result::Result<(), &'static str>;

/// How long a case waits for a point it expects to be reached. Long beside any backend's
/// work, so a loaded machine does not fail a case; a point that is never reached fails it.
pub(super) const PATIENCE: Duration = Duration::from_secs(20);

/// The timeout of a wait a case expects another call to wake: long, so that a wait that
/// misses its wake shows as a wait to its end.
pub(super) const LONG: Duration = Duration::from_secs(60);

/// How long a woken wait may take to return once the call that wakes it has been made: far
/// under [`LONG`] however loaded the machine, so a missed wake fails the case with a message
/// rather than passing late.
pub(super) const WOKEN: Duration = Duration::from_secs(10);

/// The head start a waiter is given before the call that should wake it.
pub(super) const HEAD: Duration = Duration::from_millis(100);

/// Runs `wait` on a thread, gives it [`HEAD`], runs `wake`, and returns what `wait` returned
/// and how long after `wake` began it returned.
pub(super) fn woken<T: Send>(
    wait: impl FnOnce() -> T + Send,
    wake: impl FnOnce(),
) -> (T, Duration) {
    std::thread::scope(|scope| {
        let waiter = scope.spawn(move || {
            let out = wait();
            (out, Instant::now())
        });
        std::thread::sleep(HEAD);
        let woke = Instant::now();
        wake();
        let (out, at) = waiter.join().unwrap_or_else(|_| panic!("a waiter panicked"));
        (out, at.saturating_duration_since(woke))
    })
}

/// The extent of the images the cases render into.
pub(super) const EXTENT: Extent = Extent { width: 64, height: 64 };

/// The sink of every device a case makes: it counts what it is sent, and what of it the
/// case provoked.
#[derive(Default)]
pub(super) struct Counter {
    sent: AtomicUsize,
    provoked: AtomicUsize,
}

impl MessageSink for Counter {
    fn message(&self, _: &Message<'_>) {
        self.sent.fetch_add(1, Ordering::Relaxed);
    }
}

/// What a body runs in.
pub(super) struct Ctx<'a> {
    harness: &'a dyn Harness,
    case: &'a Case,
    /// The capability set the body runs under.
    pub(super) set: Capabilities,
    sink: Arc<Counter>,
}

impl<'a> Ctx<'a> {
    pub(super) fn new(harness: &'a dyn Harness, case: &'a Case, set: Capabilities) -> Self {
        Self { harness, case, set, sink: Arc::default() }
    }

    /// The messages the sink of every device of this context has been sent that the case
    /// did not provoke ([`provoking`](Self::provoking)).
    pub(super) fn messages(&self) -> usize {
        self.sink.sent.load(Ordering::Relaxed) - self.sink.provoked.load(Ordering::Relaxed)
    }

    /// Runs `misuse`, a call sequence that provokes messages and has returned once they are
    /// sent, and returns how many it provoked, which [`messages`](Self::messages) then
    /// leaves out.
    pub(super) fn provoking(&self, misuse: impl FnOnce()) -> usize {
        let before = self.sink.sent.load(Ordering::Relaxed);
        misuse();
        let provoked = self.sink.sent.load(Ordering::Relaxed) - before;
        self.sink.provoked.fetch_add(provoked, Ordering::Relaxed);
        provoked
    }

    /// The suite's cooked shader of `name`, `<source>.<entry>` below `shaders/conformance/`,
    /// for `stage`.
    pub(super) fn shader(&self, name: &str, stage: ShaderStage) -> CookedShader {
        let shader = self.ok(self.harness.shader(name, stage), "the harness's cooked shader");
        self.check(shader.stage == stage, "a cooked shader of the stage asked for");
        shader
    }

    /// Fails the case.
    #[track_caller]
    pub(super) fn fail(&self, what: impl Display) -> ! {
        panic!("{} under {:?}: {what}", self.case.name, self.set)
    }

    #[track_caller]
    pub(super) fn check(&self, holds: bool, what: &str) {
        if !holds {
            self.fail(format_args!("did not hold: {what}"));
        }
    }

    /// The value of a call the case expects to succeed.
    #[track_caller]
    pub(super) fn ok<T>(&self, result: Result<T>, what: &str) -> T {
        result.unwrap_or_else(|error| self.fail(format_args!("{what} failed: {error}")))
    }

    /// Asserts that a call is refused by the rule the case exists to see refused.
    #[track_caller]
    pub(super) fn refused<T: Debug>(&self, result: Result<T>, what: &str) {
        let Some(rule) = self.case.refuses else {
            self.fail("a case with no rule asserted a refusal by it")
        };
        self.refused_by(result, rule, what);
    }

    /// Asserts that a call is refused by `rule`.
    #[track_caller]
    pub(super) fn refused_by<T: Debug>(&self, result: Result<T>, rule: Rule, what: &str) {
        match result {
            Ok(value) => self
                .fail(format_args!("{what} was accepted ({value:?}); `{}` expected", rule.name())),
            Err(error) if Rule::of(&error) == Some(rule) => {}
            Err(error) => self.fail(format_args!(
                "{what} was refused otherwise than by `{}`: {error}",
                rule.name()
            )),
        }
    }

    /// Asserts that a call fails with an error of `kind` that is no rule's refusal.
    #[track_caller]
    pub(super) fn fails<T: Debug>(&self, result: Result<T>, kind: ErrorKind, what: &str) {
        match result {
            Err(error) if error.kind() == kind && Rule::of(&error).is_none() => {}
            other => self.fail(format_args!("{what}: {kind:?} expected, got {other:?}")),
        }
    }

    /// A device that reports the set.
    pub(super) fn device(&self) -> Box<dyn Device> {
        let sink: Arc<dyn MessageSink> = self.sink.clone();
        let device = self.ok(self.harness.device(&self.set, sink, false), "the harness's device");
        self.check(
            *device.capabilities() == self.set,
            "the device reports the set it was made for",
        );
        device
    }

    /// A device with no surface, started from `cache`.
    pub(super) fn cached(&self, cache: Vec<u8>, what: &str) -> Box<dyn Device> {
        let sink: Arc<dyn MessageSink> = self.sink.clone();
        self.ok(self.harness.device_with_cache(&self.set, sink, cache), what)
    }

    /// A device with a surface, or why the run has none.
    pub(super) fn presenting(&self) -> core::result::Result<Box<dyn Device>, &'static str> {
        let sink: Arc<dyn MessageSink> = self.sink.clone();
        match self.harness.device(&self.set, sink, true) {
            Ok(device) => Ok(device),
            Err(error) if error.kind() == ErrorKind::Unsupported => {
                Err("the run has no window for a swapchain")
            }
            Err(error) => {
                self.fail(format_args!("the harness's presenting device failed: {error}"))
            }
        }
    }

    /// Makes `device` lost, or says the harness cannot.
    pub(super) fn lose(&self, device: &dyn Device) -> core::result::Result<(), &'static str> {
        if self.harness.lose(device) { Ok(()) } else { Err("the harness cannot lose a device") }
    }

    /// Stalls or unstalls `device`'s presents, and says whether the harness can.
    pub(super) fn stall(&self, device: &dyn Device, on: bool) -> bool {
        self.harness.stall(device, on)
    }

    pub(super) fn buffer(
        &self,
        device: &dyn Device,
        size: u64,
        usage: BufferUsage,
        memory: MemoryClass,
    ) -> BufferHandle {
        self.buffer_shared(device, size, usage, memory, Sharing::Exclusive)
    }

    pub(super) fn buffer_shared(
        &self,
        device: &dyn Device,
        size: u64,
        usage: BufferUsage,
        memory: MemoryClass,
        sharing: Sharing,
    ) -> BufferHandle {
        let desc = BufferDesc { label: "conformance", size, usage, memory, sharing };
        self.ok(device.create_buffer(&desc), "a buffer")
    }

    /// An `Upload` buffer that is a copy's source, with `usage` besides, holding `bytes`,
    /// written and flushed.
    pub(super) fn upload(
        &self,
        device: &dyn Device,
        usage: BufferUsage,
        bytes: &[u8],
    ) -> BufferHandle {
        let size = bytes.len() as u64;
        let buffer =
            self.buffer(device, size, BufferUsage::COPY_SOURCE.union(usage), MemoryClass::Upload);
        self.write(device, buffer, 0, bytes);
        buffer
    }

    /// Writes `bytes` at `offset` and flushes them.
    pub(super) fn write(
        &self,
        device: &dyn Device,
        buffer: BufferHandle,
        offset: u64,
        bytes: &[u8],
    ) {
        let len = bytes.len() as u64;
        let fill = &mut |lent: &mut [u8]| lent.copy_from_slice(bytes);
        self.ok(device.write_buffer(buffer, offset, len, fill), "a host write");
        self.ok(device.flush_buffer(buffer, offset, len), "a flush");
    }

    /// Invalidates and reads `len` bytes from `offset`.
    pub(super) fn read(
        &self,
        device: &dyn Device,
        buffer: BufferHandle,
        offset: u64,
        len: u64,
    ) -> Vec<u8> {
        self.ok(device.invalidate_buffer(buffer, offset, len), "an invalidate");
        let mut out = Vec::new();
        self.ok(
            device.read_buffer(buffer, offset, len, &mut |lent| out.extend_from_slice(lent)),
            "a host read",
        );
        out
    }

    pub(super) fn image(&self, device: &dyn Device, desc: &ImageDesc<'_>) -> ImageHandle {
        self.ok(device.create_image(desc), "an image")
    }

    /// A colour target of [`EXTENT`] that a pass may render into and a copy read.
    pub(super) fn target(&self, device: &dyn Device) -> ImageHandle {
        self.image(
            device,
            &image_desc(
                Format::Rgba8Unorm,
                EXTENT,
                ImageUsage::COLOUR.union(ImageUsage::COPY_SOURCE),
            ),
        )
    }

    /// An image whose contents a copy wrote, in `SampledRead` on the graphics queue, with
    /// the submit that left it there seen complete.
    pub(super) fn sampled(&self, device: &dyn Device) -> ImageHandle {
        let usage = ImageUsage::SAMPLED.union(ImageUsage::COPY_DESTINATION);
        let image = self.image(device, &image_desc(Format::Rgba8Unorm, EXTENT, usage));
        let bytes = vec![7; (EXTENT.width * EXTENT.height * 4) as usize];
        let source = self.upload(device, BufferUsage::EMPTY, &bytes);
        let mut encoder = self.encoder(device, QueueRole::Graphics);
        let queue = QueueRole::Graphics;
        encoder.transition_buffers(&[buffer_to(
            source,
            BufferState::Undefined,
            BufferState::CopySource,
            queue,
        )]);
        encoder.transition_images(&[image_to(
            image,
            ImageState::Undefined,
            ImageState::CopyDestination,
            queue,
        )]);
        encoder.copy_buffer_to_image(source, image, &[whole_image()]);
        encoder.transition_images(&[image_to(
            image,
            ImageState::CopyDestination,
            ImageState::SampledRead,
            queue,
        )]);
        let done = self.point(device, 1);
        self.submit(device, queue, &mut [encoder], &[], &[done]);
        self.reached(device, done);
        image
    }

    pub(super) fn encoder(&self, device: &dyn Device, role: QueueRole) -> Box<dyn Encoder> {
        self.ok(device.create_encoder(role, "conformance"), "an encoder")
    }

    pub(super) fn timeline(&self, device: &dyn Device, initial: u64) -> TimelineHandle {
        self.ok(device.create_timeline(initial), "a timeline")
    }

    /// A point `value` on a new timeline made at zero.
    pub(super) fn point(&self, device: &dyn Device, value: u64) -> TimelinePoint {
        TimelinePoint { timeline: self.timeline(device, 0), value }
    }

    /// A submit the case expects accepted.
    pub(super) fn submit(
        &self,
        device: &dyn Device,
        role: QueueRole,
        encoders: &mut [Box<dyn Encoder>],
        waits: &[TimelinePoint],
        signals: &[TimelinePoint],
    ) {
        self.ok(device.submit(role, Submit { encoders, waits, signals }), "a submit");
    }

    /// Waits until `point` is reached, which the case expects.
    pub(super) fn reached(&self, device: &dyn Device, point: TimelinePoint) {
        let reached = self.ok(device.wait_timeline(point, PATIENCE), "a wait");
        self.check(reached, "a point the case expects reached is reached");
    }

    /// Submits `encoders`, held pending by a wait on a point the host has not signalled,
    /// and returns the gate that releases them.
    pub(super) fn held(
        &self,
        device: &dyn Device,
        role: QueueRole,
        encoders: &mut [Box<dyn Encoder>],
    ) -> Gate {
        let gate = self.gate(device);
        self.submit(device, role, encoders, &[gate.wait], &[gate.done]);
        gate
    }

    /// A gate whose submit the caller makes: wait for [`Gate::wait`], signal [`Gate::done`].
    pub(super) fn gate(&self, device: &dyn Device) -> Gate {
        Gate { wait: self.point(device, 1), done: self.point(device, 1) }
    }

    /// A graphics pipeline that writes every colour target in `colour` whose mask is not
    /// empty, reads the inputs of `inputs`, and writes depth where `depth` is a format.
    pub(super) fn graphics(
        &self,
        device: &dyn Device,
        colour: &[(Format, WriteMask)],
        inputs: u32,
        depth: Option<Format>,
    ) -> GraphicsPipelineHandle {
        let targets: Vec<ColourTargetDesc> = colour
            .iter()
            .map(|&(format, write_mask)| ColourTargetDesc { format, write_mask })
            .collect();
        let outputs = colour
            .iter()
            .enumerate()
            .filter(|(_, (_, mask))| *mask != WriteMask::NONE)
            .fold(0, |bits, (at, _)| bits | (1 << at));
        let vertex = cooked(ShaderStage::Vertex, 0, 0);
        let fragment = cooked(ShaderStage::Fragment, inputs, outputs);
        let desc = GraphicsPipelineDesc {
            label: "conformance",
            vertex: StageDesc { shader: &vertex, constants: &[] },
            fragment: Some(StageDesc { shader: &fragment, constants: &[] }),
            colour: &targets,
            depth: depth.map(|format| DepthDesc { format, compare: Compare::Less, write: true }),
            cull: Cull::None,
        };
        self.ok(device.create_graphics_pipeline(&desc), "a graphics pipeline")
    }

    /// The pipeline of a pass with one `Rgba8Unorm` target, written whole.
    pub(super) fn plain(&self, device: &dyn Device) -> GraphicsPipelineHandle {
        self.graphics(device, &[(Format::Rgba8Unorm, WriteMask::ALL)], 0, None)
    }

    pub(super) fn compute(&self, device: &dyn Device) -> ComputePipelineHandle {
        let shader = cooked(ShaderStage::Compute, 0, 0);
        let desc = ComputePipelineDesc {
            label: "conformance",
            stage: StageDesc { shader: &shader, constants: &[] },
        };
        self.ok(device.create_compute_pipeline(&desc), "a compute pipeline")
    }
}

/// A submit held pending: it waits for `wait`, which the host signals, and signals `done`.
#[derive(Clone, Copy, Debug)]
pub(super) struct Gate {
    pub(super) wait: TimelinePoint,
    pub(super) done: TimelinePoint,
}

impl Gate {
    /// Signals the wait from the host and waits for the held submit's point.
    pub(super) fn open(&self, ctx: &Ctx<'_>, device: &dyn Device) {
        ctx.ok(device.signal_timeline(self.wait), "the gate's host signal");
        ctx.reached(device, self.done);
    }
}

pub(super) const fn image_desc(
    format: Format,
    extent: Extent,
    usage: ImageUsage,
) -> ImageDesc<'static> {
    ImageDesc {
        label: "conformance",
        format,
        extent,
        mips: 1,
        layers: 1,
        usage,
        memory: ImageMemory::Own,
        sharing: Sharing::Exclusive,
    }
}

/// A transition of an image's first subresource on one queue.
pub(super) const fn image_to(
    image: ImageHandle,
    from: ImageState,
    to: ImageState,
    queue: QueueRole,
) -> ImageTransition {
    image_across(image, ImageRange::FIRST, (from, queue), (to, queue))
}

pub(super) const fn image_across(
    image: ImageHandle,
    range: ImageRange,
    from: (ImageState, QueueRole),
    to: (ImageState, QueueRole),
) -> ImageTransition {
    ImageTransition {
        image,
        range,
        from: StateOn { state: from.0, queue: from.1 },
        to: StateOn { state: to.0, queue: to.1 },
    }
}

/// A transition of a buffer on one queue.
pub(super) const fn buffer_to(
    buffer: BufferHandle,
    from: BufferState,
    to: BufferState,
    queue: QueueRole,
) -> BufferTransition {
    buffer_across(buffer, (from, queue), (to, queue))
}

pub(super) const fn buffer_across(
    buffer: BufferHandle,
    from: (BufferState, QueueRole),
    to: (BufferState, QueueRole),
) -> BufferTransition {
    BufferTransition {
        buffer,
        from: StateOn { state: from.0, queue: from.1 },
        to: StateOn { state: to.0, queue: to.1 },
    }
}

/// A colour attachment of an image's first subresource.
pub(super) const fn attachment(
    image: ImageHandle,
    load: Load<[f32; 4]>,
    store: Store,
    local_read: bool,
) -> ColourAttachment {
    ColourAttachment { target: Target { image, mip: 0, layer: 0 }, load, store, local_read }
}

/// The region of a copy that covers an [`EXTENT`] image's first subresource.
pub(super) const fn whole_image() -> BufferImageCopy {
    BufferImageCopy {
        buffer_offset: 0,
        mip: 0,
        layer: 0,
        rect: Rect { x: 0, y: 0, extent: EXTENT },
    }
}

/// Records a render pass over `image`, in `Undefined` before it: the transition, a
/// cleared and stored attachment, and, where `pipeline` is one, a draw.
pub(super) fn clear(
    encoder: &mut dyn Encoder,
    image: ImageHandle,
    pipeline: Option<GraphicsPipelineHandle>,
) {
    let queue = encoder.queue();
    encoder.transition_images(&[image_to(
        image,
        ImageState::Undefined,
        ImageState::ColourAttachment,
        queue,
    )]);
    begin(
        encoder,
        &[attachment(image, Load::Clear([0.0; 4]), Store::Store, false)],
        PassReach::NOTHING,
    );
    if let Some(pipeline) = pipeline {
        encoder.bind_graphics_pipeline(pipeline);
        encoder.draw(0..3, 0..1);
    }
    encoder.end_pass();
}

/// Begins a render pass with no depth attachment.
pub(super) fn begin(encoder: &mut dyn Encoder, colour: &[ColourAttachment], reach: PassReach<'_>) {
    encoder.begin_render_pass(&RenderPass { label: "conformance", colour, depth: None, reach });
}

/// Begins a compute pass and binds `pipeline`.
pub(super) fn compute_pass(
    encoder: &mut dyn Encoder,
    pipeline: ComputePipelineHandle,
    reach: PassReach<'_>,
) {
    encoder.begin_compute_pass(&ComputePass { label: "conformance", reach });
    encoder.bind_compute_pipeline(pipeline);
}

/// `OpCapability` and the capabilities the required set enables that the modules declare:
/// `Shader` alone.
const OP_CAPABILITY: u32 = 17;
const SHADER: u32 = 1;

/// `Int64Atomics`, which no device enables without `buffer_atomics_64`.
pub(super) const INT64_ATOMICS: u32 = 12;

/// A SPIR-V 1.0 module of one empty entry point `main` for `stage`, declaring `Shader` and
/// `capabilities`.
pub(super) fn spirv(stage: ShaderStage, capabilities: &[u32]) -> Vec<u8> {
    fn op(words: &mut Vec<u32>, code: u32, operands: &[u32]) {
        words.push(((operands.len() as u32 + 1) << 16) | code);
        words.extend_from_slice(operands);
    }
    // Ids: 1 void, 2 the function type, 3 `main`, 4 its label; the bound is one past.
    let mut words = vec![0x0723_0203, 0x0001_0000, 0, 5, 0];
    for &capability in [SHADER].iter().chain(capabilities) {
        op(&mut words, OP_CAPABILITY, &[capability]);
    }
    op(&mut words, 14, &[0, 1]); // OpMemoryModel Logical GLSL450
    let model = match stage {
        ShaderStage::Vertex => 0,
        ShaderStage::Fragment => 4,
        ShaderStage::Compute => 5,
    };
    let main = u32::from_le_bytes(*b"main");
    op(&mut words, 15, &[model, 3, main, 0]); // OpEntryPoint
    match stage {
        ShaderStage::Fragment => op(&mut words, 16, &[3, 7]), // OriginUpperLeft
        ShaderStage::Compute => op(&mut words, 16, &[3, 17, 1, 1, 1]), // LocalSize
        ShaderStage::Vertex => {}
    }
    op(&mut words, 19, &[1]); // OpTypeVoid
    op(&mut words, 33, &[2, 1]); // OpTypeFunction
    op(&mut words, 54, &[1, 3, 0, 2]); // OpFunction
    op(&mut words, 248, &[4]); // OpLabel
    op(&mut words, 253, &[]); // OpReturn
    op(&mut words, 56, &[]); // OpFunctionEnd
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

/// A cooked SPIR-V stage with no heap and no constants.
pub(super) fn cooked(stage: ShaderStage, inputs: u32, outputs: u32) -> CookedShader {
    CookedShader {
        stage,
        target: ShaderTarget::SpirV,
        entry: "main".to_owned(),
        code: spirv(stage, &[]),
        heap: None,
        constants: Vec::new(),
        push_bytes: 16,
        inputs,
        outputs,
    }
}

/// The heap's block for a device of `set`.
pub(super) fn heap_of(set: &Capabilities) -> HeapUse {
    HeapUse {
        form: HeapForm::FixedArray,
        set: HEAP_SET,
        arrays: vec![
            HeapArray { kind: HeapArrayKind::SampledImage2d, binding: 0, capacity: set.heap_slots },
            HeapArray { kind: HeapArrayKind::Sampler, binding: 1, capacity: set.samplers },
        ],
    }
}
