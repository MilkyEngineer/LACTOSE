//! What the null runs share: a null device with the window's stand-in, the shaders null
//! takes, and the bounded waits.
//!
//! Every wait of a test is bounded at [`WAIT_BOUND`] by a clock that is no worker's, a thread
//! of the test's own, and fails past it with a message naming what never arrived.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use lactose_app_client::CookedShaders;
use lactose_core::Result;
use lactose_rhi_api::{
    Acquired, Capabilities, ConstantSlot, ConstantType, CookedShader, Device, DeviceRequest,
    DisplayTime, Extent, Format, HEAP_SET, HEAP_SLOTS_DEFAULT, HeapArray, HeapArrayKind, HeapForm,
    HeapUse, ImageHandle, Message, MessageSink, PresentId, Presented, SAMPLERS_DEFAULT,
    ShaderStage, ShaderTarget, Submit, Swapchain,
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

/// What a [`Watched`] swapchain was asked.
#[derive(Default)]
pub struct Watch {
    /// Each resize, in order.
    pub resizes: Mutex<Vec<Extent>>,
    /// The acquires asked of it.
    pub acquires: AtomicUsize,
    /// Those it answered with no image itself, without asking null.
    pub skipped: AtomicUsize,
}

/// Null's swapchain, watched: each resize written down and, with `skip`, every second
/// acquire answered `Acquired::None`, so that every second present call makes no submit and
/// its target's release toward the present role meets no acquire.
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
            return Ok(Acquired::None);
        }
        self.inner.acquire()
    }

    fn present(&mut self, image: ImageHandle, submit: Submit<'_>) -> Result<Presented> {
        self.inner.present(image, submit)
    }

    fn resize(&mut self, extent: Extent) -> Result<()> {
        lock(&self.watch.resizes).push(extent);
        self.inner.resize(extent)
    }

    fn last_present(&self) -> Option<PresentId> {
        self.inner.last_present()
    }

    fn display_times(&mut self, into: &mut Vec<DisplayTime>) -> bool {
        self.inner.display_times(into)
    }
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
