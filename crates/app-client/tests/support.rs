//! What the null runs share: a null device with the window's stand-in, the shaders null
//! takes, a swapchain and a device that watch what is asked of them, and the bounded waits.
//!
//! Every wait of a test is bounded at [`WAIT_BOUND`] by a clock that is no worker's, a thread
//! of the test's own, and fails past it with a message naming what never arrived.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, mpsc};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use lactose_app_client::CookedShaders;
use lactose_core::{Error, ErrorKind, Result};
use lactose_rhi_api::{
    Acquired, BufferDesc, BufferHandle, Capabilities, ComputePipelineDesc, ComputePipelineHandle,
    ConstantSlot, ConstantType, CookedShader, Device, DeviceInfo, DeviceRequest, DisplayTime,
    Encoder, Extent, Format, GraphicsPipelineDesc, GraphicsPipelineHandle, HEAP_SET,
    HEAP_SLOTS_DEFAULT, HeapArray, HeapArrayKind, HeapForm, HeapSlot, HeapUse, ImageDesc,
    ImageHandle, MemoryBlockDesc, MemoryBlockHandle, Message, MessageSink, PipelineStats,
    Placement, PresentId, Presented, QueueRole, Rule, SAMPLERS_DEFAULT, SamplerDesc, SamplerIndex,
    ShaderStage, ShaderTarget, Submit, Swapchain, SwapchainDesc, TimelineHandle, TimelinePoint,
};
use lactose_rhi_null::{Control, NullDevice, NullSurface};
use lactose_task::Executor;

/// The bound of every wait.
pub const WAIT_BOUND: Duration = Duration::from_secs(10);

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Counts what the device says. Null sends a message for nothing it accepts.
#[derive(Default)]
pub struct Said {
    pub count: AtomicUsize,
    pub texts: Mutex<Vec<String>>,
}

impl MessageSink for Said {
    fn message(&self, message: &Message<'_>) {
        self.count.fetch_add(1, Ordering::Relaxed);
        lock(&self.texts).push(format!("{message:?}"));
    }
}

/// A null device of `capabilities` on the window's stand-in, with the heap and sampler
/// table the cooked shaders are compiled for, as the binary asks for them.
pub fn null_device(capabilities: Capabilities) -> (Arc<dyn Device>, Control, Arc<Said>) {
    let said = Arc::new(Said::default());
    let request = DeviceRequest {
        adapter: None,
        surface: Some(Arc::new(NullSurface)),
        heap_slots: HEAP_SLOTS_DEFAULT,
        samplers: SAMPLERS_DEFAULT,
        pipeline_cache: Vec::new(),
        messages: Arc::clone(&said) as Arc<dyn MessageSink>,
    };
    let device = NullDevice::new(request, capabilities).expect("a null device");
    let control = device.control();
    (Arc::new(device), control, said)
}

/// The smallest SPIR-V module null reads, with the reflection `stage` asks: the renderer's
/// three stages as `lactose-renderer`'s tests give them, and the present pass's two.
fn null_shader(
    stage: ShaderStage,
    push_bytes: u32,
    constants: Vec<ConstantSlot>,
    outputs: u32,
    heap: Option<HeapUse>,
) -> CookedShader {
    CookedShader {
        stage,
        target: ShaderTarget::SpirV,
        entry: "main".to_owned(),
        code: [0x0723_0203_u32, 0x0001_0000, 0, 1, 0]
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect(),
        heap,
        constants,
        push_bytes,
        inputs: 0,
        outputs,
        workgroup: (stage == ShaderStage::Compute).then_some([64, 1, 1]),
        varyings: Vec::new(),
    }
}

/// The client's five stages as null takes them. The present pass's fragment stage reads
/// the heap at the default capacities, as `shaders/client/present.slang` is compiled.
pub fn null_shaders() -> CookedShaders {
    let float = vec![ConstantSlot { id: 0, ty: ConstantType::Float }];
    let heap = HeapUse {
        form: HeapForm::FixedArray,
        set: HEAP_SET,
        arrays: vec![
            HeapArray {
                kind: HeapArrayKind::SampledImage2d,
                binding: 0,
                capacity: HEAP_SLOTS_DEFAULT,
            },
            HeapArray { kind: HeapArrayKind::Sampler, binding: 1, capacity: SAMPLERS_DEFAULT },
        ],
    };
    CookedShaders {
        scatter: null_shader(ShaderStage::Compute, 32, Vec::new(), 0, None),
        vertex: null_shader(ShaderStage::Vertex, 32, float, 0, None),
        fragment: null_shader(ShaderStage::Fragment, 0, Vec::new(), 1, None),
        present_vertex: null_shader(ShaderStage::Vertex, 16, Vec::new(), 0, None),
        present_fragment: null_shader(ShaderStage::Fragment, 16, Vec::new(), 1, Some(heap)),
    }
}

/// What a [`Watched`] swapchain and a [`WatchedDevice`] were asked, and how they answer.
#[derive(Default)]
pub struct Watch {
    /// Each resize, in order.
    pub resizes: Mutex<Vec<Extent>>,
    /// The acquires asked of the swapchain.
    pub acquires: AtomicUsize,
    /// Those it answered with no image itself, without asking null.
    pub skipped: AtomicUsize,
    /// The images presents were taken for, in order.
    pub presented: Mutex<Vec<ImageHandle>>,
    /// The rule the next present is refused by, without reaching null.
    pub refuse_present: Mutex<Option<Rule>>,
    /// The present calls begun: each begins with `display_times`.
    pub begun: AtomicUsize,
    /// Those whose last swapchain call returned: an acquire that gave no image, a present,
    /// or a resize that failed. A call that makes none of them, such as one that returns at
    /// a refused heap slot's write, is not counted.
    pub ended: AtomicUsize,
    /// Whether a [`WatchedDevice`] answers a read of a timeline a present signals with the
    /// value it last saw reached, without asking null: null keeps a submit pending until a
    /// point it signals is seen, so every present's read stays pending until a wait sees
    /// it.
    pub lag: AtomicBool,
    /// The timelines presents signal.
    reads: Mutex<HashSet<TimelineHandle>>,
    /// Their values last seen reached through a wait.
    seen: Mutex<HashMap<TimelineHandle, u64>>,
    /// Every refusal of a submit, a present, an acquire, a resize or a heap slot's write or
    /// clear, by rule and text.
    pub refusals: Mutex<Vec<String>>,
    /// Whether a [`WatchedDevice`] refuses the next image's make, without asking null.
    pub fail_make: AtomicBool,
    /// The images a [`WatchedDevice`] made, and those it destroyed.
    pub made: Mutex<Vec<ImageHandle>>,
    pub destroyed: Mutex<Vec<ImageHandle>>,
    /// The samplers a [`WatchedDevice`] made and has not destroyed.
    pub samplers: Mutex<Vec<SamplerIndex>>,
}

impl Watch {
    fn noted<T>(&self, what: &str, result: Result<T>) -> Result<T> {
        if let Err(error) = &result
            && let Some(rule) = Rule::of(error)
        {
            lock(&self.refusals).push(format!("{what}: {rule:?}: {error}"));
        }
        result
    }

    /// The presents taken so far.
    pub fn presents(&self) -> usize {
        lock(&self.presented).len()
    }

    /// Waits, within [`WAIT_BOUND`], until `calls` present calls have begun and every call
    /// begun has made its last swapchain call. What the call does after it, which is to
    /// note the present it made and return through the lane, is not waited for.
    pub fn settle(&self, calls: usize) {
        let deadline = Instant::now() + WAIT_BOUND;
        loop {
            let begun = self.begun.load(Ordering::Acquire);
            if begun >= calls && self.ended.load(Ordering::Acquire) == begun {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{calls} present calls did not end within the bound"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn end<T>(&self, result: Result<T>) -> Result<T> {
        self.ended.fetch_add(1, Ordering::Release);
        result
    }
}

/// Null's swapchain, watched: each resize, acquire and present written down, every refusal
/// noted, a present refused on request and a call held on request; with `skip`, every
/// second acquire answered `Acquired::None`, so that every second present call makes no
/// submit and its target's release toward the present role meets no acquire.
pub struct Watched {
    pub inner: Box<dyn Swapchain>,
    pub watch: Arc<Watch>,
    pub skip: bool,
}

impl Swapchain for Watched {
    fn format(&self) -> Format {
        self.inner.format()
    }

    fn extent(&self) -> Extent {
        self.inner.extent()
    }

    fn image_count(&self) -> u32 {
        self.inner.image_count()
    }

    fn acquire(&mut self) -> Result<Acquired> {
        if self.watch.acquires.fetch_add(1, Ordering::Relaxed) % 2 == 1 && self.skip {
            self.watch.skipped.fetch_add(1, Ordering::Relaxed);
            return self.watch.end(Ok(Acquired::None));
        }
        let acquired = self.watch.noted("an acquire", self.inner.acquire());
        match acquired {
            Ok(Acquired::Image(image)) => Ok(Acquired::Image(image)),
            other => self.watch.end(other),
        }
    }

    fn present(&mut self, image: ImageHandle, submit: Submit<'_>) -> Result<Presented> {
        if let Some(rule) = lock(&self.watch.refuse_present).take() {
            return self.watch.end(Err(rule.refuse("a present refused by the test")));
        }
        lock(&self.watch.reads).extend(submit.signals.iter().map(|point| point.timeline));
        let presented = self.watch.noted("a present", self.inner.present(image, submit));
        if presented.is_ok() {
            lock(&self.watch.presented).push(image);
        }
        self.watch.end(presented)
    }

    fn resize(&mut self, extent: Extent) -> Result<()> {
        lock(&self.watch.resizes).push(extent);
        let resized = self.watch.noted("a resize", self.inner.resize(extent));
        if resized.is_ok() { resized } else { self.watch.end(resized) }
    }

    fn last_present(&self) -> Option<PresentId> {
        self.inner.last_present()
    }

    fn display_times(&mut self, into: &mut Vec<DisplayTime>) -> bool {
        self.watch.begun.fetch_add(1, Ordering::Release);
        self.inner.display_times(into)
    }
}

/// A device that passes every call to `inner`, noting each refusal of a submit or of a heap
/// slot's write or clear and every image made and destroyed, that refuses an image's make
/// on request, and that answers the reads of the presents' timeline as [`Watch::lag`] says.
pub struct WatchedDevice {
    pub inner: Arc<dyn Device>,
    pub watch: Arc<Watch>,
}

impl Device for WatchedDevice {
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
        if self.watch.fail_make.swap(false, Ordering::AcqRel) {
            return Err(Error::new(ErrorKind::ResourceExhausted, "a make refused by the test"));
        }
        let image = self.inner.create_image(desc)?;
        lock(&self.watch.made).push(image);
        Ok(image)
    }

    fn destroy_image(&self, image: ImageHandle) -> Result<()> {
        self.inner.destroy_image(image)?;
        lock(&self.watch.destroyed).push(image);
        Ok(())
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
        self.inner.destroy_memory_block(block)
    }

    fn write_heap_slot(&self, slot: HeapSlot, image: ImageHandle) -> Result<()> {
        self.watch.noted("a heap slot's write", self.inner.write_heap_slot(slot, image))
    }

    fn clear_heap_slot(&self, slot: HeapSlot) -> Result<()> {
        self.watch.noted("a heap slot's clear", self.inner.clear_heap_slot(slot))
    }

    fn create_sampler(&self, desc: &SamplerDesc) -> Result<SamplerIndex> {
        let sampler = self.inner.create_sampler(desc)?;
        lock(&self.watch.samplers).push(sampler);
        Ok(sampler)
    }

    fn destroy_sampler(&self, sampler: SamplerIndex) -> Result<()> {
        self.watch.noted("a sampler's destroy", self.inner.destroy_sampler(sampler))?;
        lock(&self.watch.samplers).retain(|&live| live != sampler);
        Ok(())
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
        self.inner.destroy_timeline(timeline)
    }

    fn timeline_value(&self, timeline: TimelineHandle) -> Result<u64> {
        if self.watch.lag.load(Ordering::Acquire) && lock(&self.watch.reads).contains(&timeline) {
            return Ok(lock(&self.watch.seen).get(&timeline).copied().unwrap_or(0));
        }
        self.inner.timeline_value(timeline)
    }

    fn signal_timeline(&self, point: TimelinePoint) -> Result<()> {
        self.inner.signal_timeline(point)
    }

    fn wait_timeline(&self, point: TimelinePoint, timeout: Duration) -> Result<bool> {
        let reached = self.inner.wait_timeline(point, timeout)?;
        if reached {
            let mut seen = lock(&self.watch.seen);
            let value = seen.entry(point.timeline).or_default();
            *value = (*value).max(point.value);
        }
        Ok(reached)
    }

    fn create_encoder(&self, queue: QueueRole, label: &str) -> Result<Box<dyn Encoder>> {
        self.inner.create_encoder(queue, label)
    }

    fn submit(&self, queue: QueueRole, submit: Submit<'_>) -> Result<()> {
        self.watch.noted("a submit", self.inner.submit(queue, submit))
    }

    fn create_swapchain(&self, desc: &SwapchainDesc) -> Result<Box<dyn Swapchain>> {
        self.inner.create_swapchain(desc)
    }
}

// ---- Warnings ----

/// What a [`warnings`] capture writes into.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        lock(&self.0).extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// `body`'s value, and the events at `warn` or above that any thread logged while it ran,
/// each with its thread's name: the capture is the process's subscriber, set at the first
/// call, which nextest's process for each test makes the test's alone. Calls run one at a
/// time.
pub fn warnings<T>(body: impl FnOnce() -> T) -> (T, String) {
    static CAPTURED: OnceLock<Captured> = OnceLock::new();
    let captured = CAPTURED.get_or_init(|| {
        let captured = Captured::default();
        let writer = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .with_thread_names(true)
            .finish();
        tracing::subscriber::set_global_default(subscriber).expect("the process's subscriber");
        captured
    });
    lock(&captured.0).clear();
    let value = body();
    let text = String::from_utf8_lossy(&lock(&captured.0)).into_owned();
    (value, text)
}

// ---- Bounded waits ----

/// What a bounded wait shares with the thread that keeps its bound.
struct Expiry {
    passed: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

/// A wait raced against a clock that is no worker's: `None` once the bound passed first.
struct Bounded<W> {
    wait: Pin<Box<W>>,
    expiry: Arc<Expiry>,
}

impl<W: Future> Future for Bounded<W> {
    type Output = Option<W::Output>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Read before the wait is polled: a wait that is ready only when the keeper's wake
        // polls it, its own waker having woken nothing, fails at the bound.
        if self.expiry.passed.load(Ordering::Acquire) {
            return Poll::Ready(None);
        }
        if let Poll::Ready(value) = self.wait.as_mut().poll(cx) {
            return Poll::Ready(Some(value));
        }
        // The waker is left before the flag is read, and the keeping thread raises the
        // flag before it takes the waker: a bound that passes between the two is seen here
        // or woken there.
        *lock(&self.expiry.waker) = Some(cx.waker().clone());
        if self.expiry.passed.load(Ordering::Acquire) { Poll::Ready(None) } else { Poll::Pending }
    }
}

/// `wait`, driven by `Executor::block_on` on the calling thread, within [`WAIT_BOUND`],
/// kept by a thread of the test's own; past the bound the test fails naming `what`.
pub fn bounded<W: Future>(executor: &Executor, wait: W, what: &str) -> W::Output {
    let expiry = Arc::new(Expiry { passed: AtomicBool::new(false), waker: Mutex::new(None) });
    let (resolved, ended) = mpsc::channel::<()>();
    let keeper = {
        let expiry = Arc::clone(&expiry);
        std::thread::Builder::new()
            .name("wait-bound".to_owned())
            .spawn(move || {
                if ended.recv_timeout(WAIT_BOUND) == Err(mpsc::RecvTimeoutError::Timeout) {
                    expiry.passed.store(true, Ordering::Release);
                    if let Some(waker) = lock(&expiry.waker).take() {
                        waker.wake();
                    }
                }
            })
            .expect("the thread that keeps the bound")
    };
    let value = executor.block_on(Bounded { wait: Box::pin(wait), expiry });
    drop(resolved);
    keeper.join().expect("the thread that keeps the bound");
    value.unwrap_or_else(|| panic!("{what} did not resolve within its bound"))
}
