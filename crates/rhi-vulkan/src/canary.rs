//! The thread-safety validator's canary. It is a unit test, not one of `tests/device.rs`'s,
//! because the misuse it makes is a driver call without its queue's lock, which no safe
//! call can make: the canary reaches the queue through the crate's own fields.

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use lactose_rhi_api::{
    BufferDesc, BufferReach, BufferState, BufferTransition, BufferUsage, ComputePass,
    ComputePipelineDesc, CookedShader, Device, DeviceRequest, HEAP_SLOTS_DEFAULT, MemoryClass,
    Message, MessageSink, PassReach, QueueRole, SAMPLERS_DEFAULT, Sharing, StageDesc, StateOn,
    Submit,
};

use crate::VulkanDevice;

/// The time between the unlocked wait's start and the submit: long enough on a loaded
/// machine for the wait to have begun, and well inside the lagging dispatch's run.
const PAUSE: Duration = Duration::from_millis(300);

/// What the canary must say.
const THREAD_SAFETY_CANARY: &str = "UNASSIGNED-Threading-MultipleThreads-Write";

/// How long the canary may take, its device's making and teardown included, before its
/// watchdog ends the process: about two seconds on the M1.
const WATCHDOG: Duration = Duration::from_secs(30);

/// The messages a device sends, each printed as `cargo xtask gpu` reads them.
#[derive(Default)]
struct Heard(Mutex<Vec<String>>);

impl MessageSink for Heard {
    fn message(&self, message: &Message<'_>) {
        eprintln!("message: {message:?}");
        self.0.lock().unwrap_or_else(PoisonError::into_inner).push(format!("{message:?}"));
    }
}

/// The thread-safety validator's canary: the graphics queue waited idle on one thread with
/// no lock, under a dispatch that runs for about two seconds, while another thread submits
/// to it through the safe API, a breach of the queue's external synchronisation. The wait
/// lasts until the dispatch ends, so the two calls are inside the layer at once; a driver
/// that serialises its queue holds the submit until then. The misuse is undefined behaviour
/// for the driver, which may instead deadlock: after [`WATCHDOG`] a watchdog thread says so
/// and ends the process with a failure, which `cargo xtask gpu` records as the canary
/// failed, rather than waiting on it for ever.
#[test]
#[ignore = "needs a Vulkan driver: cargo xtask gpu"]
fn canary_thread_safety_validation_reports_a_queue_used_on_two_threads() {
    let (done, watched) = mpsc::channel::<()>();
    thread::spawn(move || {
        if watched.recv_timeout(WATCHDOG) == Err(mpsc::RecvTimeoutError::Timeout) {
            eprintln!(
                "canary_thread_safety_validation_reports_a_queue_used_on_two_threads: the \
                 canary's misuse hung the driver for {WATCHDOG:?}"
            );
            std::process::exit(1);
        }
    });
    let heard = Arc::new(Heard::default());
    let device = VulkanDevice::new(DeviceRequest {
        adapter: std::env::var("LACTOSE_GPU_ADAPTER").ok(),
        surface: None,
        heap_slots: HEAP_SLOTS_DEFAULT,
        samplers: SAMPLERS_DEFAULT,
        pipeline_cache: Vec::new(),
        messages: Arc::clone(&heard) as _,
    })
    .expect("a Vulkan device");
    // `store.slang`, as `cargo xtask gpu` cooks it into the directory it names.
    let dir = std::env::var_os("LACTOSE_GPU_SHADERS").map(PathBuf::from).expect("cooked shaders");
    let bytes = std::fs::read(dir.join("conformance/store.cs_main.spirv")).expect("store.slang");
    let shader = CookedShader::decode(&bytes).expect("a cooked shader");
    let stage = StageDesc { shader: &shader, constants: &[] };
    let pipeline = device
        .create_compute_pipeline(&ComputePipelineDesc { label: "canary", stage })
        .expect("a compute pipeline");
    let scratch = device
        .create_buffer(&BufferDesc {
            label: "canary",
            size: 256,
            usage: BufferUsage::DEVICE_ADDRESS,
            memory: MemoryClass::Device,
            sharing: Sharing::Exclusive,
        })
        .expect("a buffer");
    let on = |state| StateOn { state, queue: QueueRole::Graphics };
    let mut lagging = device.create_encoder(QueueRole::Graphics, "canary").expect("an encoder");
    lagging.transition_buffers(&[BufferTransition {
        buffer: scratch,
        from: on(BufferState::Undefined),
        to: on(BufferState::ShaderWrite),
    }]);
    let declared = [BufferReach { buffer: scratch, state: BufferState::ShaderWrite }];
    lagging.begin_compute_pass(&ComputePass {
        label: "canary",
        reach: PassReach { slots: &[], buffers: &declared },
    });
    lagging.bind_compute_pipeline(pipeline);
    // `store.slang`'s block: one word of 7, after 2^26 rounds (about two seconds on the M1).
    lagging.push_address(0, scratch, 0);
    lagging.push_bytes(8, &1u32.to_le_bytes());
    lagging.push_bytes(12, &(1u32 << 26).to_le_bytes());
    lagging.push_bytes(16, &7u32.to_le_bytes());
    lagging.dispatch([1, 1, 1]);
    lagging.end_pass();
    // Each submit's encoder is kept until the end: a drop waits for its submit.
    let submit = |encoder| {
        let mut encoders = [encoder];
        device
            .submit(
                QueueRole::Graphics,
                Submit { encoders: &mut encoders, waits: &[], signals: &[] },
            )
            .expect("a submit");
        encoders
    };
    let held = submit(lagging);
    let shared = device.shared();
    let queue = shared.queue(shared.queue_of(QueueRole::Graphics)).queue;
    thread::scope(|scope| {
        let waiting = scope.spawn(|| {
            // SAFETY: the queue is the device's, which outlives the scope. The call takes
            // no queue lock on purpose: the submit below races it, the canary's misuse.
            unsafe { shared.raw.device.queue_wait_idle(queue) }
        });
        thread::sleep(PAUSE);
        let _empty =
            submit(device.create_encoder(QueueRole::Graphics, "canary").expect("an encoder"));
        waiting.join().expect("the waiting thread").expect("the wait for idle");
    });
    drop(held);
    drop(device);
    // A send fails only where the watchdog has already ended the process.
    let _ = done.send(());
    let texts = heard.0.lock().unwrap_or_else(PoisonError::into_inner).clone();
    eprintln!("the canary heard: {texts:?}");
    assert!(!texts.is_empty(), "the canary {THREAD_SAFETY_CANARY} was not heard");
    assert!(texts.iter().all(|text| text.contains(THREAD_SAFETY_CANARY)), "{texts:?}");
}
