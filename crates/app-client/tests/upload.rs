//! `remote-upload-5k` (the milestone 2 plan's criterion 5 and verification rows 19 and 102,
//! section 4's table): the presentation schedule's remote upload of 5,000 remote entities
//! into the renderer's ring on a Vulkan device, paced at 144 Hz for ten seconds (one in a
//! short run), with the network thread writing every slot at 30 Hz beside it. `cargo xtask
//! measure remote-upload-5k` runs it under the driver it names with every layer off, and
//! writes the device's identity and the ring's memory type into the file's header; `cargo
//! xtask gpu` runs it short under the validators, whose figures it discards.
//!
//! The network thread runs under `ThreadClass::Network`, as the client's does. Each frame,
//! paced by the test thread's timer, opens a region (a wait for a free one is outside the
//! time and reported), then runs one job on a frame-critical worker, as the frame runs the
//! schedule's upload: one `sample_batch` over the 5,000 slots at the presentation time two
//! send intervals behind the newest state, the conversion of each sample to render space
//! (`RenderTransform::from_sim`, through `RenderPosition::from_sim`), and the write of the
//! rows to the ring through `lactose-presentation`'s `upload` and the renderer's
//! `RemoteWriter`, each part timed inside the job. `Renderer::prepare` and an empty submit
//! that signals the frame's timeline value end it, so the ring's regions come free as they
//! do in a frame. The steps are `lactose-presentation`'s `RemoteRows::sample` and `upload`
//! as the frame runs them, but for the merge of the snapshot's `RemoteSlot` and mesh rows,
//! which is made once: no remote entity comes or goes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lactose_app::{Workers, start_executor};
use lactose_asset_api::{AssetId, AssetRef};
use lactose_core::{NetId, StableKey, ThreadClass, Tick};
use lactose_math::{Quat, Scalar, Vec3};
use lactose_net_api::InterpolationSlot;
use lactose_net_host::InterpolationBuffer;
use lactose_net_host::interpolation::{
    InterpolationReader, PresentationTime, ReceivedState, RemoteSample, SampleStatus,
};
use lactose_physics_api::Transform;
use lactose_present_api::{InstanceRow, RenderTransform};
use lactose_renderer::{Renderer, RendererDesc};

use crate::support::{bounded, lock};
use crate::timing::{Ender, Waits, enter, figure, measured_workers, short, spread, vulkan};

const REMOTE: u32 = 5_000;
const RATE_HZ: u32 = 144;
const SEND_HZ: f64 = 30.0;
const RING_REGIONS: u32 = 3;

/// The state the network thread pushes for slot `n` at server tick `tick`.
fn state(tick: u32, n: u32) -> ReceivedState {
    let x = Scalar::from_f64(f64::from(n) + f64::from(tick) * 0.1);
    let position = Vec3::new(x, Scalar::ONE, Scalar::from_u32(n % 64));
    ReceivedState { tick: Tick(tick), teleported: Tick(0), position, rotation: Quat::IDENTITY }
}

/// What the frame's job owns between frames, so that a steady frame allocates nothing.
struct Frame {
    renderer: Renderer,
    reader: InterpolationReader,
    slots: Vec<InterpolationSlot>,
    samples: Vec<RemoteSample>,
    /// The rows, ascending by key, each placed where its last sample put it.
    rows: Vec<InstanceRow>,
}

/// What one frame's job took, by part.
#[derive(Clone, Copy)]
struct Parts {
    batch: Duration,
    convert: Duration,
    write: Duration,
}

impl Frame {
    /// The schedule's remote upload at `at`, from a frame's job.
    fn upload(&mut self, ctx: &mut lactose_task::JobContext<'_>, at: PresentationTime) -> Parts {
        let start = Instant::now();
        self.reader.sample_batch(&self.slots, at, &mut self.samples);
        let sampled = Instant::now();
        for (row, sample) in self.rows.iter_mut().zip(&self.samples) {
            // A slot with nothing in it keeps its row where it was shown last.
            if sample.status != SampleStatus::Missing {
                let placed = Transform::new(sample.position, sample.rotation);
                row.transform = RenderTransform::from_sim(&placed);
            }
        }
        let converted = Instant::now();
        {
            let writer = self.renderer.remote();
            lactose_presentation::upload(ctx, &self.rows, |rows| writer.write(rows));
        }
        let written = Instant::now();
        Parts { batch: sampled - start, convert: converted - sampled, write: written - converted }
    }
}

/// `cargo xtask measure remote-upload-5k` runs it.
#[test]
#[ignore = "a measurement on a Vulkan device, run by `cargo xtask measure`: times are reported"]
fn cost_of_the_remote_upload_of_5k() {
    let seconds = if short() { 1 } else { 10 };
    let frames = (RATE_HZ * seconds) as usize;
    let period = Duration::from_secs(1) / RATE_HZ;
    let n = measured_workers();
    let executor = start_executor(Workers { frame_workers: Some(n), ..Workers::default() })
        .expect("an executor");
    let (device, messages) = vulkan();
    let desc = RendererDesc {
        instances: REMOTE + 64,
        rows: REMOTE + 1_024,
        regions: RING_REGIONS,
        shaders: None,
    };
    let renderer = Renderer::new(Arc::clone(&device), &desc).expect("a renderer");
    let mut ender = Ender::new(&*device);
    let mut waits = Waits::default();

    let (mut writer, reader) = InterpolationBuffer::split(REMOTE);
    let mut slots = Vec::new();
    let mut rows = Vec::new();
    for n in 1..=REMOTE {
        let slot = writer.allocate().expect("a free slot");
        writer.push(slot, state(1, n)).expect("a live slot");
        slots.push(slot);
        rows.push(InstanceRow {
            key: StableKey::Net(NetId::cooked(u64::from(n))),
            mesh: AssetRef::new(AssetId::from_bits(u64::from(n))),
            transform: RenderTransform::from_sim(&Transform::IDENTITY),
            no_interpolate: true,
        });
    }
    let bytes = reader.bytes();
    let frame = Arc::new(Mutex::new(Frame {
        renderer,
        reader: reader.clone(),
        samples: vec![RemoteSample::MISSING; slots.len()],
        slots: slots.clone(),
        rows,
    }));

    // The network thread: a state for every slot at 30 Hz.
    let begun = Instant::now();
    let stop = Arc::new(AtomicBool::new(false));
    let network = {
        let stop = Arc::clone(&stop);
        std::thread::Builder::new()
            .name("network".to_owned())
            .spawn(move || {
                enter(ThreadClass::Network);
                let mut tick = 1;
                while !stop.load(Ordering::Relaxed) {
                    tick += 1;
                    let due = begun + Duration::from_secs_f64(f64::from(tick - 1) / SEND_HZ);
                    std::thread::sleep(due.saturating_duration_since(Instant::now()));
                    for (n, &slot) in (1..).zip(&slots) {
                        writer.push(slot, state(tick, n)).expect("a live slot");
                    }
                }
                tick
            })
            .expect("the network thread")
    };

    let mut parts = Vec::with_capacity(frames);
    let mut late = 0_u32;
    for index in 0..frames {
        let due = begun + period * index as u32;
        let now = Instant::now();
        if now > due + period {
            late += 1;
        }
        std::thread::sleep(due.saturating_duration_since(now));
        // Two send intervals behind the newest state, as the clock estimate would place it.
        let server = (begun.elapsed().as_secs_f64() * SEND_HZ - 2.0).max(1.0);
        let at = PresentationTime {
            tick: Tick(server as u32),
            fraction: (server.fract() * 65536.0) as u16,
        };
        waits.open(&mut lock(&frame).renderer);
        let job = {
            let frame = Arc::clone(&frame);
            executor.run_job(move |ctx| lock(&frame).upload(ctx, at))
        };
        parts.push(bounded(&executor, job, "a frame's upload job").expect("the job runs"));
        let mut held = lock(&frame);
        let stats = held.renderer.prepare().expect("the frame prepares");
        assert_eq!(stats.remote_rows, REMOTE, "the batch wrote every remote row: {stats:?}");
        assert_eq!(stats.dropped, 0, "every row fits the region: {stats:?}");
        ender.end(&mut held.renderer);
    }
    stop.store(true, Ordering::Relaxed);
    let sent = network.join().expect("the network thread");
    executor.shutdown();
    drop(frame);
    drop(device);
    messages.assert_none();

    spread("frame", parts.iter().map(|p| p.batch + p.convert + p.write).collect());
    spread("batch", parts.iter().map(|p| p.batch).collect());
    spread("convert", parts.iter().map(|p| p.convert).collect());
    spread("write", parts.iter().map(|p| p.write).collect());
    let reads = frames as u64 * u64::from(REMOTE);
    let retried = reader.retried_reads();
    figure("reader.reads", reads);
    figure("reader.retried_reads", retried);
    figure("reader.retry_rate", format!("{:.2e}", retried as f64 / reads as f64));
    figure("network.sends", sent - 1);
    figure("buffer.bytes", bytes);
    figure("buffer.bytes_per_slot", bytes / REMOTE as usize);
    figure("frames", frames);
    figure("frames_started_late", late);
    figure("rate_hz", RATE_HZ);
    figure("send_hz", SEND_HZ);
    figure("remote", REMOTE);
    figure("workers", n);
    figure("ring.memory_class", "Upload");
    figure("ring.regions", RING_REGIONS);
    waits.report("ring");
}
