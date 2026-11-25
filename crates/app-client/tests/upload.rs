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
//!
//! Where macOS runs the paced job moves the figures more than the upload does: a light job
//! that sleeps most of each period is placed on an efficiency core at a low clock as often as
//! not, frame-critical though its worker is, and at a clock less than half the fastest
//! (`measure-m2`'s review, which read the cores the probe's jobs ran on). So the test runs in
//! the foreground task role, which the shared support takes (`task_role`), and each job reads
//! the class of the core it began on and the one it ended on through
//! `lactose_task::current_core_class`, and times a fixed chain of dependent multiply-adds,
//! [`CHAIN`] long, whose time is the clock the job meets. Criterion 5 reads the frames whose
//! job began and ended on a performance core, at whatever clock (D164). The file gives the
//! median, P99 and worst frame over every frame (`frame.*`), over those frames
//! (`performance.*`) and over the rest (`rest.*`), each with its share of the frames and its
//! split by clock band, the band read against the fastest chain of the run; how many frames
//! ran in each placement (`placement.*`); and which figure the criterion reads and whether
//! the run judges it (`criterion.*`). The same frames are then run back to back with no
//! timer, a control whose job stays busy and whose clock stays up, and given the same way
//! under `unpaced.`. The fastest chain on each class of core
//! (`calibration.fastest_us.performance` and `.efficiency`), over the frames and the chains of
//! a numbering probe that runs a `Background` thread on every logical CPU after them
//! (`calibration.probe.*`), shows the CPU numbering the class call rests on: the efficiency
//! cores' is the slower (`calibration.numbering`).
//!
//! The bands are relative to the run's fastest chain, not to a core's rated clock. The file
//! flags a run whose control has under half its frames `full` (`calibration.flag`): a fastest
//! chain the busy control cannot hold, an outlier or a performance core's chain among
//! efficiency cores' frames. A run slowed throughout, under Low Power Mode or a thermal limit,
//! has a slow fastest chain and a control nearly all `full` against it: no band read against
//! the run's own chains can tell it, and the flag reads `no`.

use std::hint::black_box;
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
use lactose_task::{CoreClass, current_core_class};

use crate::support::{bounded, lock};
use crate::timing::{
    Ender, Waits, enter, figure, measured_workers, median, micros, short, spread, vulkan,
};

const REMOTE: u32 = 5_000;
const RATE_HZ: u32 = 144;
const SEND_HZ: f64 = 30.0;
const RING_REGIONS: u32 = 3;
/// The calibration chain's steps: about 10 µs on a performance core at its full clock.
const CHAIN: u32 = 5_000;
/// The clock bands, by a chain's time against the fastest of the run: under the first bound
/// the core is at the run's fastest clock or near it; at the second or over, it runs at half
/// that clock or less.
const BANDS: [(&str, f64); 3] = [("full", 1.25), ("reduced", 2.0), ("half", f64::INFINITY)];

/// The time of [`CHAIN`] dependent multiply-adds, each through `black_box`, so that nothing
/// folds, reorders or vectorises them: a figure of the clock the calling thread meets.
fn calibration() -> Duration {
    let start = Instant::now();
    let mut x = 1_u64;
    for step in 0..CHAIN {
        x = black_box(x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(u64::from(step)));
    }
    black_box(x);
    start.elapsed()
}

/// The chains each thread of [`numbering_probe`] times.
const PROBE_CHAINS: usize = 400;

/// The chain on every class of core, apart from the frames: a `Background` thread for each
/// logical CPU, so that some run on the efficiency cores wherever the frames' job ran, each
/// timing [`PROBE_CHAINS`] chains between two reads of its core's class. Each chain's
/// [`placement`] and time.
fn numbering_probe() -> Vec<(&'static str, Duration)> {
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let handles: Vec<_> = (0..threads)
        .map(|_| {
            std::thread::spawn(|| {
                enter(ThreadClass::Background);
                (0..PROBE_CHAINS)
                    .map(|_| {
                        let began = current_core_class();
                        let chain = calibration();
                        (placement(began, current_core_class()), chain)
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    handles.into_iter().flat_map(|handle| handle.join().expect("a probe thread")).collect()
}

/// Where a job or a chain ran, from the class of the core it began on and the one it ended
/// on: `performance` or `efficiency` where both are that class, `moved` where they differ,
/// `unknown` where the platform could not tell.
fn placement(began: Option<CoreClass>, ended: Option<CoreClass>) -> &'static str {
    match (began, ended) {
        (Some(CoreClass::Performance), Some(CoreClass::Performance)) => "performance",
        (Some(CoreClass::Efficiency), Some(CoreClass::Efficiency)) => "efficiency",
        (Some(_), Some(_)) => "moved",
        _ => "unknown",
    }
}

/// Every value of [`placement`].
const PLACEMENTS: [&str; 4] = ["performance", "efficiency", "moved", "unknown"];

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

/// What one frame's job took, by part, its calibration chain's time, and the class of the
/// core it began on and the one it ended on.
#[derive(Clone, Copy)]
struct Parts {
    began: Option<CoreClass>,
    ended: Option<CoreClass>,
    calibration: Duration,
    batch: Duration,
    convert: Duration,
    write: Duration,
}

impl Parts {
    fn frame(&self) -> Duration {
        self.batch + self.convert + self.write
    }

    /// Where the job ran.
    fn placement(&self) -> &'static str {
        placement(self.began, self.ended)
    }
}

impl Frame {
    /// The class of the core, the calibration chain, then the schedule's remote upload at
    /// `at`, from a frame's job, and the class of the core again.
    fn upload(&mut self, ctx: &mut lactose_task::JobContext<'_>, at: PresentationTime) -> Parts {
        let began = current_core_class();
        let calibration = calibration();
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
        Parts {
            began,
            ended: current_core_class(),
            calibration,
            batch: sampled - start,
            convert: converted - sampled,
            write: written - converted,
        }
    }
}

/// What the frames share: the executor, the frame's state and the renderer's waits and end.
struct Rig {
    executor: lactose_task::Executor,
    frame: Arc<Mutex<Frame>>,
    ender: Ender,
    waits: Waits,
    begun: Instant,
}

impl Rig {
    /// `frames` frames, each at its deadline `period` apart from now where `period` is given,
    /// else back to back; returns their parts and how many started a period or more late.
    fn run(&mut self, frames: usize, period: Option<Duration>) -> (Vec<Parts>, u32) {
        let mut parts = Vec::with_capacity(frames);
        let mut late = 0_u32;
        let first = Instant::now();
        for index in 0..frames {
            if let Some(period) = period {
                let due = first + period * index as u32;
                let now = Instant::now();
                if now > due + period {
                    late += 1;
                }
                std::thread::sleep(due.saturating_duration_since(now));
            }
            // Two send intervals behind the newest state, as the clock estimate would place it.
            let server = (self.begun.elapsed().as_secs_f64() * SEND_HZ - 2.0).max(1.0);
            let at = PresentationTime {
                tick: Tick(server as u32),
                fraction: (server.fract() * 65536.0) as u16,
            };
            self.waits.open(&mut lock(&self.frame).renderer);
            let job = {
                let frame = Arc::clone(&self.frame);
                self.executor.run_job(move |ctx| lock(&frame).upload(ctx, at))
            };
            parts.push(bounded(&self.executor, job, "a frame's upload job").expect("the job runs"));
            let mut held = lock(&self.frame);
            let stats = held.renderer.prepare().expect("the frame prepares");
            assert_eq!(stats.remote_rows, REMOTE, "the batch wrote every remote row: {stats:?}");
            assert_eq!(stats.dropped, 0, "every row fits the region: {stats:?}");
            self.ender.end(&mut held.renderer);
        }
        (parts, late)
    }
}

/// The band of a chain that took `calibration`, against the run's fastest.
fn band(calibration: Duration, fastest: Duration) -> &'static str {
    let ratio = calibration.as_secs_f64() / fastest.as_secs_f64();
    BANDS.iter().find(|&&(_, bound)| ratio < bound).map_or("half", |&(name, _)| name)
}

/// `part` of `whole` as a percentage, to a tenth.
fn share(part: usize, whole: usize) -> String {
    format!("{:.1}", part as f64 * 100.0 / whole as f64)
}

/// One run's figures under `prefix`: each part's spread over every frame, the calibration's,
/// and each band's frames, share and part medians; the same over the frames whose job began
/// and ended on a performance core and over the rest, each with its share of the run; and
/// how many frames ran in each placement.
fn report(prefix: &str, parts: &[Parts], fastest: Duration) {
    let all = |part: fn(&Parts) -> Duration| parts.iter().map(part).collect::<Vec<_>>();
    spread(&format!("{prefix}frame"), all(Parts::frame));
    spread(&format!("{prefix}batch"), all(|p| p.batch));
    spread(&format!("{prefix}convert"), all(|p| p.convert));
    spread(&format!("{prefix}write"), all(|p| p.write));
    spread(&format!("{prefix}calibration"), all(|p| p.calibration));
    bands(prefix, parts, fastest);
    let (performance, rest): (Vec<Parts>, Vec<Parts>) =
        parts.iter().partition(|p| p.placement() == "performance");
    for (name, group) in [("performance", performance), ("rest", rest)] {
        let at = format!("{prefix}{name}.");
        figure(&format!("{at}frames"), group.len());
        figure(&format!("{at}share_percent"), share(group.len(), parts.len()));
        if group.is_empty() {
            continue;
        }
        spread(&format!("{at}frame"), group.iter().map(Parts::frame).collect());
        bands(&at, &group, fastest);
    }
    for name in PLACEMENTS {
        let count = parts.iter().filter(|p| p.placement() == name).count();
        figure(&format!("{prefix}placement.{name}.frames"), count);
    }
}

/// Each band's frames under `prefix`, their share of `parts` and their part medians.
fn bands(prefix: &str, parts: &[Parts], fastest: Duration) {
    for (name, _) in BANDS {
        let banded: Vec<&Parts> =
            parts.iter().filter(|p| band(p.calibration, fastest) == name).collect();
        let at = format!("{prefix}band.{name}");
        figure(&format!("{at}.frames"), banded.len());
        figure(&format!("{at}.share_percent"), share(banded.len(), parts.len()));
        if banded.is_empty() {
            continue;
        }
        let of = |part: fn(&Parts) -> Duration| median(banded.iter().map(|p| part(p)).collect());
        figure(&format!("{at}.frame.p50_us"), micros(of(Parts::frame)));
        figure(&format!("{at}.batch.p50_us"), micros(of(|p| p.batch)));
        figure(&format!("{at}.convert.p50_us"), micros(of(|p| p.convert)));
        figure(&format!("{at}.write.p50_us"), micros(of(|p| p.write)));
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
    // The machine's classes are read at the first call: here, outside every frame.
    let _ = current_core_class();
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
    let ender = Ender::new(&*device);

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

    let mut rig = Rig { executor, frame, ender, waits: Waits::default(), begun };
    let (paced, late) = rig.run(frames, Some(period));
    let paced_waits = std::mem::take(&mut rig.waits);
    let paced_retried = reader.retried_reads();
    let (unpaced, _) = rig.run(frames, None);
    let unpaced_retried = reader.retried_reads() - paced_retried;
    stop.store(true, Ordering::Relaxed);
    let sent = network.join().expect("the network thread");
    let Rig { executor, frame, waits: unpaced_waits, .. } = rig;
    executor.shutdown();
    drop(frame);
    drop(device);
    messages.assert_none();
    let probed = numbering_probe();

    // One reference for both runs, so that a band means the same clock in each.
    let fastest = paced.iter().chain(&unpaced).map(|p| p.calibration).min().expect("frames");
    figure("calibration.chain_steps", CHAIN);
    figure("calibration.fastest_us", micros(fastest));
    figure(
        "calibration.bands",
        "full under 1.25x the fastest chain, reduced under 2x, half at 2x or over",
    );
    // The numbering the class call rests on, shown on the machine: each class's fastest chain,
    // over the frames of both runs and the probe's chains.
    let fastest_on = |class| {
        let frames = paced.iter().chain(&unpaced).map(|p| (p.placement(), p.calibration));
        frames.chain(probed.iter().copied()).filter(|&(at, _)| at == class).map(|(_, c)| c).min()
    };
    figure("calibration.probe.chains", probed.len());
    for name in PLACEMENTS {
        let count = probed.iter().filter(|&&(at, _)| at == name).count();
        figure(&format!("calibration.probe.placement.{name}.chains"), count);
    }
    let (on_performance, on_efficiency) = (fastest_on("performance"), fastest_on("efficiency"));
    for (class, chain) in [("performance", on_performance), ("efficiency", on_efficiency)] {
        let value = chain
            .map_or_else(|| "none: no frame or probe chain ran on the class".to_owned(), micros);
        figure(&format!("calibration.fastest_us.{class}"), value);
    }
    figure(
        "calibration.numbering",
        match (on_performance, on_efficiency) {
            (Some(performance), Some(efficiency)) if efficiency > performance => {
                "the efficiency cores' fastest chain is the slower, as the class call's \
                 numbering assumes"
            }
            (Some(_), Some(_)) => {
                "FLAGGED: the efficiency cores' fastest chain is not the slower: the class \
                 call's numbering is in doubt"
            }
            _ => "not shown: no frame or probe chain ran on one of the classes",
        },
    );
    report("", &paced, fastest);
    report("unpaced.", &unpaced, fastest);
    // Criterion 5 (D164): the paced frames whose job began and ended on a performance core,
    // every core where the system reports one class; under a tenth of the frames, no verdict.
    figure(
        "criterion.reads",
        "performance.frame.p50_us: the paced frames that began and ended on a performance core",
    );
    let counted = paced.iter().filter(|p| p.placement() == "performance").count();
    figure(
        "criterion.judges",
        if counted * 10 < paced.len() {
            "no: under a tenth of the paced frames ran on a performance core"
        } else {
            "yes"
        },
    );
    let full = unpaced.iter().filter(|p| band(p.calibration, fastest) == "full").count();
    figure(
        "calibration.flag",
        if full * 2 < unpaced.len() {
            "FLAGGED: under half the control's frames are `full`: the busy control cannot hold \
             the run's fastest chain; a run slowed throughout is not flagged"
        } else {
            "no"
        },
    );
    let reads = frames as u64 * u64::from(REMOTE);
    for (prefix, retried) in [("", paced_retried), ("unpaced.", unpaced_retried)] {
        figure(&format!("{prefix}reader.reads"), reads);
        figure(&format!("{prefix}reader.retried_reads"), retried);
        let rate = retried as f64 / reads as f64;
        figure(&format!("{prefix}reader.retry_rate"), format!("{rate:.2e}"));
    }
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
    paced_waits.report("ring");
    unpaced_waits.report("unpaced.ring");
}
