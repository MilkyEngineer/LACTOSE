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
    Submit, TimelinePoint,
};

use crate::VulkanDevice;

/// The time between the unlocked wait's start and the submit: long enough on a loaded
/// machine for the wait to have begun, and well inside the lagging dispatch's run.
const PAUSE: Duration = Duration::from_millis(300);

/// The lagging dispatch's rounds at its first try: about two seconds on the M1.
const LAG: u32 = 1 << 26;

/// How many times the lag is doubled where it ended before the pause, as it may on a CPU
/// driver.
const DOUBLINGS: u32 = 4;

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
/// that serialises its queue holds the submit until then. The submit is made only while the
/// dispatch is still pending: where it ended before the pause, the lag is doubled and the
/// race tried again, up to [`DOUBLINGS`] times, so that a fast driver fails the canary as
/// "the lag ended before the pause", not as a validator unheard.
///
/// The misuse is undefined behaviour for the driver, which may instead deadlock: after
/// [`WATCHDOG`] a watchdog thread says so and aborts the process, which `cargo xtask gpu`
/// records as the canary failed, rather than waiting on it for ever. An abort, not an exit:
/// an exit runs the layer's teardown of the live device, which takes locks a hung thread
/// may hold.
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
            std::process::abort();
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
    let timeline = device.create_timeline(0).expect("a timeline");
    let on = |state| StateOn { state, queue: QueueRole::Graphics };
    // Each submit's encoder is kept until its try ends: a drop waits for its submit.
    let submit = |encoder, signals: &[TimelinePoint]| {
        let mut encoders = [encoder];
        device
            .submit(QueueRole::Graphics, Submit { encoders: &mut encoders, waits: &[], signals })
            .expect("a submit");
        encoders
    };
    let shared = device.shared();
    let queue = shared.queue(shared.queue_of(QueueRole::Graphics)).queue;
    let mut raced = false;
    for doubling in 0..=DOUBLINGS {
        let lag = LAG << doubling;
        let scratch = device
            .create_buffer(&BufferDesc {
                label: "canary",
                size: 256,
                usage: BufferUsage::DEVICE_ADDRESS,
                memory: MemoryClass::Device,
                sharing: Sharing::Exclusive,
            })
            .expect("a buffer");
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
        // `store.slang`'s block: one word of 7, after `lag` rounds.
        lagging.push_address(0, scratch, 0);
        lagging.push_bytes(8, &1u32.to_le_bytes());
        lagging.push_bytes(12, &lag.to_le_bytes());
        lagging.push_bytes(16, &7u32.to_le_bytes());
        lagging.dispatch([1, 1, 1]);
        lagging.end_pass();
        let lagged = TimelinePoint { timeline, value: u64::from(doubling) + 1 };
        let held = submit(lagging, &[lagged]);
        raced = thread::scope(|scope| {
            let waiting = scope.spawn(|| {
                // SAFETY: the queue is the device's, which outlives the scope. The call takes
                // no queue lock on purpose: the submit below races it, the canary's misuse.
                unsafe { shared.raw.device.queue_wait_idle(queue) }
            });
            thread::sleep(PAUSE);
            // The wait spans the submit only while the lagging dispatch runs.
            let pending = !device.wait_timeline(lagged, Duration::ZERO).expect("a wait of no time");
            let _empty = pending.then(|| {
                let empty = device.create_encoder(QueueRole::Graphics, "canary");
                submit(empty.expect("an encoder"), &[])
            });
            waiting.join().expect("the waiting thread").expect("the wait for idle");
            pending
        });
        drop(held);
        device.destroy_buffer(scratch).expect("the buffer destroyed");
        if raced {
            break;
        }
        eprintln!("the canary's lag of {lag} rounds ended before the pause: doubled");
    }
    drop(device);
    // A send fails only where the watchdog has already ended the process.
    let _ = done.send(());
    assert!(raced, "the lag ended before the pause, at {} rounds", LAG << DOUBLINGS);
    let texts = heard.0.lock().unwrap_or_else(PoisonError::into_inner).clone();
    eprintln!("the canary heard: {texts:?}");
    assert!(!texts.is_empty(), "the canary {THREAD_SAFETY_CANARY} was not heard");
    assert!(texts.iter().all(|text| text.contains(THREAD_SAFETY_CANARY)), "{texts:?}");
}
