//! What the measurements share (`extract`, `upload`, `stall`): the figure lines `cargo xtask
//! measure` reads, the short run, N, the percentiles, a Vulkan device whose messages the
//! run's judge counts, and a frame's end with an empty submit.
//!
//! A measurement prints each figure once as `measure: <name> <value>` and asserts on counts
//! alone, never on a time.

use std::fmt::Display;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lactose_core::ThreadClass;
use lactose_renderer::{Opened, Renderer};
use lactose_rhi_api::{
    Device, DeviceRequest, HEAP_SLOTS_DEFAULT, Message, MessageSink, QueueRole, SAMPLERS_DEFAULT,
    Submit, TimelineHandle, TimelinePoint,
};
use lactose_rhi_vulkan::VulkanDevice;
use lactose_task::{Platform, StdPlatform};

use crate::support::{WAIT_BOUND, lock};

/// The variable that cuts a measurement's repetitions, as `cargo xtask measure --short` and
/// `cargo xtask gpu` set it.
const SHORT_VAR: &str = "LACTOSE_MEASURE_SHORT";

/// The variable that sets N in place of the topology's, as `cargo xtask measure` passes it.
const WORKERS_VAR: &str = "LACTOSE_MEASURE_WORKERS";

/// Whether the run is short: fewer repetitions, the same figures, which mean nothing.
pub fn short() -> bool {
    std::env::var_os(SHORT_VAR).is_some()
}

/// One figure for `cargo xtask measure`.
pub fn figure(name: &str, value: impl Display) {
    println!("measure: {name} {value}");
}

/// A span in microseconds, to a tenth.
pub fn micros(span: Duration) -> String {
    format!("{:.1}", span.as_secs_f64() * 1e6)
}

/// The value at `fraction` of `sorted`, by the nearest rank.
pub fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

/// The median, the 99th percentile and the worst of `spans`, as figures `<name>.p50_us`,
/// `<name>.p99_us` and `<name>.max_us`.
pub fn spread(name: &str, mut spans: Vec<Duration>) {
    assert!(!spans.is_empty(), "{name}: nothing was timed");
    spans.sort_unstable();
    figure(&format!("{name}.p50_us"), micros(percentile(&spans, 0.5)));
    figure(&format!("{name}.p99_us"), micros(percentile(&spans, 0.99)));
    figure(&format!("{name}.max_us"), micros(spans[spans.len() - 1]));
}

/// The median of `spans`.
pub fn median(mut spans: Vec<Duration>) -> Duration {
    assert!(!spans.is_empty(), "nothing was timed");
    spans.sort_unstable();
    percentile(&spans, 0.5)
}

/// N of the milestone plans' section 4: the performance cores the platform reports or, on a
/// part that reports no core types, its physical cores, at most eight; [`WORKERS_VAR`]
/// overrides it. `lactose-app`'s and `lactose-sim`'s cost tests hold copies of this rule,
/// and the three must match.
pub fn measured_workers() -> u32 {
    if let Some(value) = std::env::var_os(WORKERS_VAR).filter(|value| !value.is_empty()) {
        let value = value.to_string_lossy();
        return value
            .parse::<u32>()
            .ok()
            .filter(|&n| n > 0)
            .unwrap_or_else(|| panic!("{WORKERS_VAR} is `{value}`, not a count above zero"));
    }
    let topology = StdPlatform.topology();
    let cores =
        if topology.efficiency == 0 { topology.performance + 1 } else { topology.performance };
    cores.min(8)
}

/// Keeps what the device says and prints each message in the form the judge of `cargo
/// xtask gpu` counts: a measurement that hears one fails, since the run it timed was not
/// the clean one.
#[derive(Default)]
pub struct Messages(Mutex<Vec<String>>);

impl MessageSink for Messages {
    fn message(&self, message: &Message<'_>) {
        eprintln!("message: {message:?}");
        lock(&self.0).push(format!("{message:?}"));
    }
}

impl Messages {
    /// Fails the test if the device said anything.
    pub fn assert_none(&self) {
        let said = lock(&self.0);
        assert!(said.is_empty(), "the device sent {} message(s): {said:?}", said.len());
    }
}

/// A Vulkan device on the adapter `LACTOSE_GPU_ADAPTER` names, or the loader's first, with
/// no surface, and its identity as figures (`device.*`), which `cargo xtask measure` joins
/// to the driver's own report in the file's header.
pub fn vulkan() -> (Arc<dyn Device>, Arc<Messages>) {
    let messages = Arc::new(Messages::default());
    let request = DeviceRequest {
        adapter: std::env::var("LACTOSE_GPU_ADAPTER").ok(),
        surface: None,
        heap_slots: HEAP_SLOTS_DEFAULT,
        samplers: SAMPLERS_DEFAULT,
        pipeline_cache: Vec::new(),
        messages: Arc::clone(&messages) as Arc<dyn MessageSink>,
    };
    let device: Arc<dyn Device> = Arc::new(VulkanDevice::new(request).expect("a Vulkan device"));
    let info = device.info();
    figure("device.backend", info.backend);
    figure("device.adapter", &info.adapter);
    figure("device.driver", if info.driver.is_empty() { "unreported" } else { &info.driver });
    figure("device.coherent_host_memory", device.capabilities().coherent_host_memory);
    (device, messages)
}

/// Ends each frame with a submit that declares nothing and signals the timeline's next
/// value, so the renderer's regions come free as they do in a frame.
pub struct Ender {
    timeline: TimelineHandle,
    value: u64,
}

impl Ender {
    pub fn new(device: &dyn Device) -> Self {
        Self { timeline: device.create_timeline(0).expect("a timeline"), value: 0 }
    }

    /// Ends the prepared frame, and returns its point.
    pub fn end(&mut self, renderer: &mut Renderer) -> TimelinePoint {
        self.value += 1;
        let point = TimelinePoint { timeline: self.timeline, value: self.value };
        let submit = Submit { encoders: &mut [], waits: &[], signals: &[point] };
        renderer.device().submit(QueueRole::Graphics, submit).expect("an empty submit");
        renderer.submitted(point);
        point
    }
}

/// What opening the next frame's region cost: how many waits for a free region, and how
/// long they took.
#[derive(Clone, Copy, Default)]
pub struct Waits {
    pub count: u32,
    pub time: Duration,
}

impl Waits {
    /// Opens a region for the next frame's writers, waiting for the point that frees one,
    /// each wait bounded.
    pub fn open(&mut self, renderer: &mut Renderer) {
        loop {
            match renderer.open().expect("a timeline value") {
                Opened::Ready => return,
                Opened::Wait(point) => {
                    let start = Instant::now();
                    let reached = renderer.device().wait_timeline(point, WAIT_BOUND);
                    assert!(reached.expect("a wait"), "a free region within {WAIT_BOUND:?}");
                    self.count += 1;
                    self.time += start.elapsed();
                }
                Opened::Unsubmitted => panic!("no frame of a measurement is left torn"),
            }
        }
    }

    pub fn report(&self, prefix: &str) {
        figure(&format!("{prefix}.region_waits"), self.count);
        figure(&format!("{prefix}.region_wait_us"), micros(self.time));
    }
}

/// Enters `class` on this thread, printing a refusal as the engine logs one: the measurement
/// runs on without its scheduling class, and says so.
pub fn enter(class: ThreadClass) {
    if let Err(error) = class.enter() {
        eprintln!("this thread runs without its scheduling class {class:?}: {error}");
    }
}
