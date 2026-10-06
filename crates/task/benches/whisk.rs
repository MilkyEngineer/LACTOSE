//! WHISK benchmarks: the milestone 0 proof measurements of `crates/task/README.md`.
//!
//! ```text
//! cargo bench -p lactose-task --bench whisk
//! ```
//!
//! That one command measures everything and writes `docs/results/whisk-<machine>-<date>.json`
//! and a `.md` summary next to it. Run it on a quiet machine; the numbers are wall-clock.
//!
//! Environment:
//!
//! | Variable | Meaning | Default |
//! |---|---|---|
//! | `WHISK_BENCH_RUNS` | Graph runs per shape and condition | 2000 |
//! | `WHISK_BENCH_MACHINE` | Machine name used in the file name | derived from the CPU name |
//! | `WHISK_BENCH_OUT` | Output directory | `docs/results` |
//!
//! On macOS the benchmark puts its own process in the foreground-application task role,
//! which a game on screen has, so user-interactive threads run at the priority they would
//! there (47 rather than 31). Each phase prints how long it took against what its samples
//! add up to; a phase that takes more than twice that is flagged.
//!
//! What is measured:
//!
//! - **Makespan ratio.** Three graph shapes of single-job nodes whose durations are drawn
//!   log-uniformly from 10 µs to 1 ms for every run. A job burns a calibrated number of
//!   iterations of an arithmetic kernel; it never sleeps and never looks at a clock. The
//!   makespan runs from the `run_graph` call to the moment the awaiting frame-critical task
//!   resumes, and is divided by the run's lower bound: the larger of the heaviest
//!   dependency chain and the total drawn work over the frame-critical worker count. Runs
//!   are a millisecond apart, so every run starts with the pool parked, as a frame does.
//!   Each shape is measured alone and with two background tasks compressing with zstd.
//!
//!   Beside each measured ratio stand two simulated ones on the same draws, for schedulers
//!   with no overhead that never idle a worker while a node is ready: `greedy` takes ready
//!   nodes first come, first served, `longest-first` takes the longest ready node. Neither
//!   is a bound. They show how much of a ratio is the lower bound being out of reach.
//!
//!   The process's CPU time per run is reported next to the work a run draws. The
//!   difference is what the executor burns around the jobs, mostly idle workers scanning
//!   before they park.
//! - **Timer wake.** From a tick deadline until the frame-critical task that slept until it
//!   resumes, the way the timer root ticks: `sleep_until` one deadline after another, with
//!   the pool otherwise idle, at a 2 ms period and at 60 Hz. The 60 Hz P99 is a pass
//!   criterion of its own.
//! - **Wake latency.** From just before a one-node graph is pushed to a parked pool until
//!   its job starts. The push comes from a dedicated thread at frame-critical priority. The
//!   CPU the process uses during this phase, in which it does nothing but wake up five
//!   hundred times a second, is the cost of whatever keeps the pool responsive.
//! - **Steal.** The distribution of single steals timed inside the executor
//!   (`lactose_task::bench_support::time_steals`): a thief thread takes range entries from a
//!   deque another thread filled, through the function a worker steals with, which halves
//!   the range, pushes the upper half and checks for sleepers. A second figure,
//!   `forced split`, is not a steal time: it is worker time per entry when a node of empty
//!   units is run with every range forced down to single units, which bounds the whole
//!   per-entry path (split, push, pop or steal, dispatch) from above.

use core::cmp::Reverse;
use core::hint::black_box;
use core::ops::Range;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::collections::BinaryHeap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lactose_core::ThreadClass;
use lactose_task::{
    ChaosConfig, Executor, Graph, GraphBuilder, GraphNode, JobContext, NodeId, Platform,
    StdPlatform, ThreadSpec, WorkerClass, yield_now,
};

const MIN_JOB_NANOS: f64 = 10_000.0;
const MAX_JOB_NANOS: f64 = 1_000_000.0;
const WARM_UP_RUNS: usize = 20;
const RUN_GAP: Duration = Duration::from_millis(1);
const WAKE_SAMPLES: usize = 5_000;
const WAKE_GAP: Duration = Duration::from_millis(2);
const STEAL_SAMPLES: usize = 400_000;

/// Thresholds from `crates/task/README.md`.
const RATIO_P50_LIMIT: f64 = 1.25;
const RATIO_P99_LIMIT: f64 = 1.5;
const WAKE_P99_LIMIT_NANOS: f64 = 20_000.0;
const TIMER_P99_LIMIT_NANOS: f64 = 20_000.0;
const TICK: Duration = Duration::from_micros(16_667);
const STEAL_LIMIT_NANOS: f64 = 1_000.0;

/// splitmix64: reproducible draws without a dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A job duration in nanoseconds, log-uniform between the two limits.
    fn job_nanos(&mut self) -> f64 {
        let unit = (self.next() >> 11) as f64 / (1u64 << 53) as f64;
        (MIN_JOB_NANOS.ln() + unit * (MAX_JOB_NANOS.ln() - MIN_JOB_NANOS.ln())).exp()
    }
}

/// The CPU-burning kernel: a dependent chain of shifts and xors, about two nanoseconds a
/// turn.
#[inline(never)]
fn burn(iterations: u64) -> u64 {
    let mut x = 0x2545_F491_4F6C_DD1Du64;
    for _ in 0..iterations {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
    }
    black_box(x)
}

/// A node that is one job burning the iterations stored for it.
struct BurnNode {
    iterations: Arc<[AtomicU64]>,
    index: usize,
}

impl GraphNode for BurnNode {
    fn name(&self) -> &'static str {
        "bench::burn"
    }

    fn units(&self) -> u32 {
        1
    }

    fn run(&self, _units: Range<u32>, _ctx: &mut JobContext<'_>) {
        burn(self.iterations[self.index].load(Ordering::Relaxed));
    }
}

/// A graph shape: node count and `(before, after)` edges with `before < after`.
struct Shape {
    name: &'static str,
    description: String,
    nodes: usize,
    edges: Vec<(u32, u32)>,
}

/// Many independent nodes; a quarter of them have one dependent, so the critical path is one
/// or two nodes.
fn wide_shallow() -> Shape {
    let roots = 192u32;
    let edges = (0..64).map(|second| (second * 3, roots + second)).collect();
    Shape {
        name: "wide-shallow",
        description: "192 independent nodes, 64 of them with one dependent".to_owned(),
        nodes: 256,
        edges,
    }
}

/// One chain of 128 nodes. Every fourth link also feeds a side node that rejoins the chain
/// two links later, which is all the parallel slack there is.
fn deep_chain() -> Shape {
    let links = 128;
    let mut edges = Vec::new();
    let mut next = 1u32;
    let mut previous = 0u32;
    // A side node waiting to rejoin: `(node, links until it does)`.
    let mut side = None;
    for link in 1..links {
        if link % 4 == 1 && link + 1 < links {
            edges.push((previous, next));
            side = Some((next, 2));
            next += 1;
        }
        edges.push((previous, next));
        if let Some((node, wait)) = side {
            if wait == 1 {
                edges.push((node, next));
                side = None;
            } else {
                side = Some((node, wait - 1));
            }
        }
        previous = next;
        next += 1;
    }
    Shape {
        name: "deep-chain",
        description: "a 128-node chain; every fourth link forks a side node that rejoins two \
                      links on"
            .to_owned(),
        nodes: next as usize,
        edges,
    }
}

/// Sixteen rounds of one node spreading to many and joining to one. "Many" is twice the
/// frame-critical worker count, so the spread is wider than the pool on every machine and
/// the shape asks the same of a four-core laptop and a sixteen-core desktop.
fn fan_in_fan_out(workers: u32) -> Shape {
    let (rounds, width) = (16u32, 2 * workers);
    let mut edges = Vec::new();
    for round in 0..rounds {
        let hub = round * (width + 1);
        let next_hub = hub + width + 1;
        for spoke in 1..=width {
            edges.push((hub, hub + spoke));
            edges.push((hub + spoke, next_hub));
        }
    }
    Shape {
        name: "fan-in-fan-out",
        description: format!(
            "16 rounds of 1 node spreading to {width} (twice the {workers} frame-critical \
             workers) and joining to 1"
        ),
        nodes: (rounds * (width + 1) + 1) as usize,
        edges,
    }
}

/// How a simulated scheduler picks among ready nodes.
#[derive(Clone, Copy)]
enum Pick {
    /// The node that became ready first.
    FirstReady,
    /// The node with the longest job.
    Longest,
}

impl Shape {
    fn build(&self, iterations: &Arc<[AtomicU64]>) -> Graph {
        let mut builder = GraphBuilder::new();
        for index in 0..self.nodes {
            builder.add_node(Arc::new(BurnNode { iterations: iterations.clone(), index }));
        }
        for &(before, after) in &self.edges {
            assert!(before < after, "shapes list nodes in dependency order");
            builder.add_dependency(NodeId(before), NodeId(after));
        }
        builder.build().expect("benchmark shapes are acyclic")
    }

    /// The lower bound of a run with these job durations on `workers` workers: the larger
    /// of the heaviest dependency chain and the total work divided by the worker count.
    fn lower_bound(&self, nanos: &[f64], workers: u32) -> f64 {
        let mut heaviest = nanos.to_vec();
        // Edges sorted by `after` visit every node's dependencies before the node is used
        // as a dependency itself, because `before < after`.
        let mut edges = self.edges.clone();
        edges.sort_unstable_by_key(|&(_, after)| after);
        for (before, after) in edges {
            let through = heaviest[before as usize] + nanos[after as usize];
            if through > heaviest[after as usize] {
                heaviest[after as usize] = through;
            }
        }
        let critical_path = heaviest.iter().copied().fold(0.0, f64::max);
        let total: f64 = nanos.iter().sum();
        critical_path.max(total / f64::from(workers))
    }

    /// The makespan of a scheduler with no overhead that never leaves a worker idle while
    /// a node is ready and picks among ready nodes by `pick`. Not a bound: it shows how
    /// much of a measured ratio is the lower bound being unreachable for the shape.
    fn simulated_makespan(&self, nanos: &[f64], workers: u32, pick: Pick) -> f64 {
        let mut pending = vec![0u32; self.nodes];
        let mut dependents = vec![Vec::new(); self.nodes];
        for &(before, after) in &self.edges {
            pending[after as usize] += 1;
            dependents[before as usize].push(after);
        }
        // Ready nodes in the order they became ready.
        let mut ready: Vec<u32> =
            (0..self.nodes as u32).filter(|&node| pending[node as usize] == 0).collect();
        // Running nodes as `(finish time in picoseconds, node)`, earliest first.
        let mut running = BinaryHeap::new();
        let mut now = 0u64;
        loop {
            while running.len() < workers as usize && !ready.is_empty() {
                let at = match pick {
                    Pick::FirstReady => 0,
                    Pick::Longest => (0..ready.len())
                        .max_by(|&a, &b| {
                            nanos[ready[a] as usize].total_cmp(&nanos[ready[b] as usize])
                        })
                        .expect("ready is not empty"),
                };
                let node = ready.remove(at);
                running.push(Reverse((now + (nanos[node as usize] * 1e3) as u64, node)));
            }
            let Some(Reverse((finished, node))) = running.pop() else { break };
            now = finished;
            for &after in &dependents[node as usize] {
                pending[after as usize] -= 1;
                if pending[after as usize] == 0 {
                    ready.push(after);
                }
            }
        }
        now as f64 / 1e3
    }
}

struct Percentiles {
    p50: f64,
    p99: f64,
    max: f64,
    mean: f64,
    samples: usize,
    /// Samples over a millisecond: for a wake or a timer, a machine that was away rather
    /// than a scheduler that was slow. Counted so a bad maximum can be read.
    over_a_millisecond: usize,
}

fn percentiles(mut samples: Vec<f64>) -> Percentiles {
    samples.sort_unstable_by(f64::total_cmp);
    let at = |fraction: f64| {
        let rank = (fraction * samples.len() as f64).ceil() as usize;
        samples[rank.clamp(1, samples.len()) - 1]
    };
    Percentiles {
        p50: at(0.50),
        p99: at(0.99),
        max: at(1.0),
        mean: samples.iter().sum::<f64>() / samples.len() as f64,
        samples: samples.len(),
        over_a_millisecond: samples.iter().filter(|&&sample| sample > 1e6).count(),
    }
}

/// The CPU time this process has used, user and system, in seconds.
#[cfg(unix)]
fn process_cpu_seconds() -> Option<f64> {
    let mut usage = core::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `usage` points to writable memory of the size and alignment of `rusage`,
    // which `getrusage` fills in when it returns zero.
    let usage = unsafe {
        if libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) != 0 {
            return None;
        }
        usage.assume_init()
    };
    let seconds = |time: libc::timeval| time.tv_sec as f64 + time.tv_usec as f64 / 1e6;
    Some(seconds(usage.ru_utime) + seconds(usage.ru_stime))
}

#[cfg(not(unix))]
fn process_cpu_seconds() -> Option<f64> {
    None
}

/// Kernel iterations per nanosecond on a frame-critical worker, from the fastest of several
/// timed burns. Taking the fastest means anything that slows a job later makes it run
/// longer than its drawn duration, never shorter, so the ratio errs upwards.
fn calibrate(executor: &Executor) -> f64 {
    executor
        .block_on(executor.spawn(WorkerClass::FrameCritical, async {
            let iterations = 20_000_000u64;
            burn(iterations);
            let mut fastest = f64::INFINITY;
            for _ in 0..9 {
                let start = Instant::now();
                burn(iterations);
                fastest = fastest.min(start.elapsed().as_nanos() as f64);
                yield_now().await;
            }
            iterations as f64 / fastest
        }))
        .expect("the executor is running")
}

/// Burns a set of drawn durations serially on a frame-critical worker and returns the time
/// it took over the time drawn. Close to 1.0 when the calibration holds.
fn check_calibration(executor: &Executor, rate: f64) -> f64 {
    executor
        .block_on(executor.spawn(WorkerClass::FrameCritical, async move {
            let mut rng = Rng(7);
            let jobs: Vec<f64> = (0..400).map(|_| rng.job_nanos()).collect();
            let start = Instant::now();
            for nanos in &jobs {
                burn((nanos * rate) as u64);
            }
            start.elapsed().as_nanos() as f64 / jobs.iter().sum::<f64>()
        }))
        .expect("the executor is running")
}

struct MakespanResult {
    shape: &'static str,
    background_load: bool,
    ratio: Percentiles,
    /// The same statistic for the simulated schedulers on the same draws.
    greedy: Percentiles,
    longest_first: Percentiles,
    mean_makespan_micros: f64,
    mean_lower_bound_micros: f64,
    /// The work one run draws, summed over its jobs.
    mean_work_micros: f64,
    /// The CPU time the process used per run, gaps between runs included.
    mean_cpu_micros: Option<f64>,
}

fn measure_makespan(
    executor: &Executor,
    shape: &Shape,
    rate: f64,
    runs: usize,
    background_load: bool,
) -> MakespanResult {
    let iterations: Arc<[AtomicU64]> = (0..shape.nodes).map(|_| AtomicU64::new(0)).collect();
    let mut graph = shape.build(&iterations);
    let workers = executor.frame_worker_count();
    let draws: Vec<(Vec<f64>, f64)> = {
        let mut rng = Rng(0xC0FFEE ^ shape.nodes as u64);
        (0..runs + WARM_UP_RUNS)
            .map(|_| {
                let nanos: Vec<f64> = (0..shape.nodes).map(|_| rng.job_nanos()).collect();
                let bound = shape.lower_bound(&nanos, workers);
                (nanos, bound)
            })
            .collect()
    };

    let stop = Arc::new(AtomicBool::new(false));
    let load: Vec<_> = (0..if background_load { 2 } else { 0 })
        .map(|seed| {
            let stop = stop.clone();
            executor.spawn(WorkerClass::Background, async move {
                // Compressible but not trivial input, compressed in pieces small enough to
                // yield about every half millisecond, as background CPU work must.
                let mut rng = Rng(seed);
                let input: Vec<u8> =
                    (0..32 * 1024).map(|_| (rng.next() % 23) as u8 + b'a').collect();
                let mut compressed = 0usize;
                while !stop.load(Ordering::Relaxed) {
                    compressed += zstd::bulk::compress(&input, 3).expect("zstd compresses").len();
                    yield_now().await;
                }
                compressed
            })
        })
        .collect();

    let runner = executor.clone();
    let driver = executor.spawn(WorkerClass::FrameCritical, async move {
        let mut makespans = Vec::with_capacity(draws.len());
        let mut cpu_start = None;
        for (run, (nanos, _)) in draws.iter().enumerate() {
            if run == WARM_UP_RUNS {
                cpu_start = process_cpu_seconds();
            }
            for (slot, nanos) in iterations.iter().zip(nanos) {
                slot.store((nanos * rate) as u64, Ordering::Relaxed);
            }
            let start = Instant::now();
            graph = runner.run_graph(graph).await.expect("the executor is running");
            makespans.push(start.elapsed().as_nanos() as f64);
            // Let the pool park, as it does between frames.
            runner.sleep_until(Instant::now() + RUN_GAP).await.expect("the executor is running");
        }
        let cpu = cpu_start.zip(process_cpu_seconds()).map(|(start, end)| end - start);
        (makespans, draws, cpu)
    });
    let (makespans, draws, cpu) = executor.block_on(driver).expect("the executor is running");
    stop.store(true, Ordering::Relaxed);
    for task in load {
        assert!(
            executor.block_on(task).is_ok_and(|compressed| compressed > 0),
            "the background load never ran"
        );
    }

    let measured = makespans.iter().zip(&draws).skip(WARM_UP_RUNS);
    let simulated = |pick| {
        let ratios = draws
            .iter()
            .skip(WARM_UP_RUNS)
            .map(|(nanos, bound)| shape.simulated_makespan(nanos, workers, pick) / bound);
        percentiles(ratios.collect())
    };
    let count = runs as f64;
    let mean_micros = |total: f64| total / count / 1e3;
    MakespanResult {
        shape: shape.name,
        background_load,
        ratio: percentiles(
            measured.clone().map(|(makespan, (_, bound))| makespan / bound).collect(),
        ),
        greedy: simulated(Pick::FirstReady),
        longest_first: simulated(Pick::Longest),
        mean_makespan_micros: mean_micros(measured.clone().map(|(makespan, _)| makespan).sum()),
        mean_lower_bound_micros: mean_micros(measured.clone().map(|(_, (_, bound))| bound).sum()),
        mean_work_micros: mean_micros(
            measured.map(|(_, (nanos, _))| nanos.iter().sum::<f64>()).sum(),
        ),
        mean_cpu_micros: cpu.map(|seconds| seconds * 1e6 / count),
    }
}

/// A one-job node that notes when its job started, in nanoseconds since `epoch`.
struct StampNode {
    epoch: Instant,
    started: Arc<AtomicU64>,
}

impl GraphNode for StampNode {
    fn name(&self) -> &'static str {
        "bench::stamp"
    }

    fn units(&self) -> u32 {
        1
    }

    fn run(&self, _units: Range<u32>, _ctx: &mut JobContext<'_>) {
        self.started.store(self.epoch.elapsed().as_nanos() as u64, Ordering::Release);
    }
}

struct WakeResult {
    latency: Percentiles,
    /// CPU seconds the whole process used per second of this phase.
    cpu_per_second: Option<f64>,
}

fn measure_wake_latency(executor: &Executor, samples: usize) -> WakeResult {
    // The push comes from a dedicated thread at frame-critical priority, created the way
    // the engine creates its own (network, present): a push from a thread the system has
    // demoted to an efficiency core would measure that thread, not the pool.
    let spec = ThreadSpec {
        name: "whisk-bench-pusher".to_owned(),
        class: ThreadClass::Network,
        index_in_class: 0,
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    let pusher = executor.clone();
    let body = Box::new(move || {
        let epoch = Instant::now();
        let started = Arc::new(AtomicU64::new(0));
        let mut builder = GraphBuilder::new();
        builder.add_node(Arc::new(StampNode { epoch, started: started.clone() }));
        let mut graph = builder.build().expect("one node");
        let mut latencies = Vec::with_capacity(samples);
        let mut measured_from = None;
        for sample in 0..samples + WARM_UP_RUNS {
            if sample == WARM_UP_RUNS {
                measured_from = Some((Instant::now(), process_cpu_seconds()));
            }
            // Long enough for every worker to give up scanning and park.
            std::thread::sleep(WAKE_GAP);
            let pushed = epoch.elapsed().as_nanos() as u64;
            graph = pusher.block_on(pusher.run_graph(graph)).expect("the executor is running");
            if sample >= WARM_UP_RUNS {
                latencies.push(started.load(Ordering::Acquire).saturating_sub(pushed) as f64);
            }
        }
        let (wall, cpu) = measured_from.expect("the loop ran");
        let cpu = cpu.zip(process_cpu_seconds()).map(|(start, end)| end - start);
        let cpu_per_second = cpu.map(|cpu| cpu / wall.elapsed().as_secs_f64());
        sender.send((latencies, cpu_per_second)).expect("the benchmark waits for the result");
    });
    let thread = executor.platform().spawn_thread(spec, body).expect("the pusher thread starts");
    let (latencies, cpu_per_second) = receiver.recv().expect("the pusher thread reports");
    thread.join();
    WakeResult { latency: percentiles(latencies), cpu_per_second }
}

/// From a tick deadline until the frame-critical task that slept until it resumes, the way
/// the timer root ticks: `sleep_until` one deadline after another, `period` apart, with the
/// pool otherwise idle. A deadline that has already passed when the task resumes (the
/// machine was away) is replaced by one half a period off, so the phase cannot fall behind.
fn measure_timer_latency(executor: &Executor, samples: usize, period: Duration) -> WakeResult {
    let ticker = executor.clone();
    let task = executor.spawn(WorkerClass::FrameCritical, async move {
        let mut latencies = Vec::with_capacity(samples);
        let mut measured_from = None;
        let mut deadline = Instant::now() + period;
        for sample in 0..samples + WARM_UP_RUNS {
            if sample == WARM_UP_RUNS {
                measured_from = Some((Instant::now(), process_cpu_seconds()));
            }
            ticker.sleep_until(deadline).await.expect("the executor is running");
            let late = deadline.elapsed();
            if sample >= WARM_UP_RUNS {
                latencies.push(late.as_nanos() as f64);
            }
            deadline = (deadline + period).max(Instant::now() + period / 2);
        }
        let (wall, cpu) = measured_from.expect("the loop ran");
        let cpu = cpu.zip(process_cpu_seconds()).map(|(start, end)| end - start);
        (latencies, cpu.map(|cpu| cpu / wall.elapsed().as_secs_f64()))
    });
    let (latencies, cpu_per_second) = executor.block_on(task).expect("the executor is running");
    WakeResult { latency: percentiles(latencies), cpu_per_second }
}

/// Runs one phase and prints how long it took. `expected`, given the result, is what the
/// phase's samples add up to; a phase that took more than twice that is flagged, since it
/// means the executor spent most of the time doing nothing, as a lost wake-up would.
fn phase<R>(
    name: &str,
    run: impl FnOnce() -> R,
    expected: impl FnOnce(&R) -> Option<Duration>,
) -> R {
    let start = Instant::now();
    let result = run();
    let took = start.elapsed();
    match expected(&result) {
        Some(expected) => {
            let ratio = took.as_secs_f64() / expected.as_secs_f64().max(1e-9);
            let flag = if ratio > 2.0 { "  <-- more than twice the expected time" } else { "" };
            eprintln!(
                "phase {name}: {took:.2?} (samples add up to {expected:.2?}, {ratio:.2}x){flag}"
            );
        }
        None => eprintln!("phase {name}: {took:.2?}"),
    }
    result
}

/// Puts the process in the task role of a frontmost application on macOS and returns the
/// role it ends up in, as the system reports it, for the record.
#[cfg(target_os = "macos")]
#[allow(
    deprecated,
    reason = "libc deprecates mach_task_self in favour of a crate this benchmark does not need"
)]
fn foreground_task_role() -> String {
    // <mach/task_policy.h>: `task_category_policy` is one `integer_t`, the role.
    const TASK_CATEGORY_POLICY: u32 = 1;
    const TASK_FOREGROUND_APPLICATION: i32 = 1;
    unsafe extern "C" {
        fn task_policy_set(
            task: libc::mach_port_t,
            flavor: u32,
            policy: *const i32,
            count: u32,
        ) -> libc::kern_return_t;
        fn task_policy_get(
            task: libc::mach_port_t,
            flavor: u32,
            policy: *mut i32,
            count: *mut u32,
            get_default: *mut u32,
        ) -> libc::kern_return_t;
    }
    let role = TASK_FOREGROUND_APPLICATION;
    // SAFETY: the calls take the process's own task port, a pointer to one live `i32`
    // with the count that says so, and write only through the out-pointers given.
    unsafe {
        let task = libc::mach_task_self();
        let set = task_policy_set(task, TASK_CATEGORY_POLICY, &raw const role, 1);
        let mut current = -1i32;
        let mut count = 1u32;
        let mut get_default = 0u32;
        let got = task_policy_get(
            task,
            TASK_CATEGORY_POLICY,
            &raw mut current,
            &raw mut count,
            &raw mut get_default,
        );
        let name = match current {
            0 => "TASK_UNSPECIFIED",
            1 => "TASK_FOREGROUND_APPLICATION",
            2 => "TASK_BACKGROUND_APPLICATION",
            3 => "TASK_CONTROL_APPLICATION",
            4 => "TASK_GRAPHICS_SERVER",
            5 => "TASK_THROTTLE_APPLICATION",
            6 => "TASK_NONUI_APPLICATION",
            7 => "TASK_DEFAULT_APPLICATION",
            8 => "TASK_DARWINBG_APPLICATION",
            _ => "unknown",
        };
        format!("{name} (set returned {set}, get returned {got})")
    }
}

#[cfg(not(target_os = "macos"))]
fn foreground_task_role() -> String {
    "not applicable".to_owned()
}

/// A node of empty units, for timing the scheduling around them.
struct EmptyNode {
    units: u32,
}

impl GraphNode for EmptyNode {
    fn name(&self) -> &'static str {
        "bench::empty"
    }

    fn units(&self) -> u32 {
        self.units
    }

    fn run(&self, units: Range<u32>, _ctx: &mut JobContext<'_>) {
        black_box(units);
    }
}

/// Worker-nanoseconds per entry when every range is forced down to single units. Not a
/// steal time: an upper bound on the whole per-entry path.
fn measure_forced_split(topology_workers: u32) -> f64 {
    let workers = topology_workers.clamp(2, 4);
    let executor = Executor::builder()
        .frame_workers(workers)
        .background_workers(1)
        .chaos(ChaosConfig { force_split: true, ..ChaosConfig::default() })
        .build()
        .expect("executor starts");
    let units = 1u32 << 16;
    let mut builder = GraphBuilder::new();
    builder.add_node(Arc::new(EmptyNode { units }));
    let mut graph = builder.build().expect("one node");
    let mut fastest = f64::INFINITY;
    for _ in 0..40 {
        let start = Instant::now();
        graph = executor.block_on(executor.run_graph(graph)).expect("the executor is running");
        fastest = fastest.min(start.elapsed().as_nanos() as f64);
    }
    executor.shutdown();
    fastest * f64::from(workers) / f64::from(units)
}

/// What one pair of clock readings costs, which every steal sample includes.
fn clock_pair_nanos() -> f64 {
    let mut fastest = f64::INFINITY;
    for _ in 0..20 {
        let start = Instant::now();
        for _ in 0..1000 {
            black_box(Instant::now().elapsed());
        }
        fastest = fastest.min(start.elapsed().as_nanos() as f64 / 1000.0);
    }
    fastest
}

fn command_output(program: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new(program).args(arguments).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn cpu_name() -> String {
    let name = if cfg!(target_os = "macos") {
        command_output("sysctl", &["-n", "machdep.cpu.brand_string"])
    } else if cfg!(target_os = "linux") {
        std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|info| {
            let line = info.lines().find(|line| line.starts_with("model name"))?;
            Some(line.split_once(':')?.1.trim().to_owned())
        })
    } else {
        std::env::var("PROCESSOR_IDENTIFIER").ok()
    };
    name.filter(|name| !name.is_empty()).unwrap_or_else(|| "unknown CPU".to_owned())
}

/// The one-minute load average, where the system has one.
fn load_average() -> Option<String> {
    let output = command_output("uptime", &[])?;
    let averages = output.rsplit_once("load average")?.1;
    Some(averages.trim_start_matches(['s', ':', ' ']).to_owned())
}

fn slug(text: &str) -> String {
    let mut slug = String::new();
    for character in text.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_owned()
}

/// Today's UTC date as `YYYY-MM-DD` (days-to-civil, Howard Hinnant's algorithm).
fn today() -> String {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
    let days = (seconds / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn commit() -> String {
    let hash = command_output("git", &["rev-parse", "--short=12", "HEAD"]);
    let dirty = command_output("git", &["status", "--porcelain", "--untracked-files=no"])
        .is_some_and(|status| !status.is_empty());
    match hash {
        Some(hash) if dirty => format!("{hash}-dirty"),
        Some(hash) => hash,
        None => "unknown".to_owned(),
    }
}

fn verdict(pass: bool) -> &'static str {
    if pass { "pass" } else { "FAIL" }
}

fn optional(value: Option<f64>, digits: usize) -> String {
    value.map_or_else(|| "null".to_owned(), |value| format!("{value:.digits$}"))
}

fn main() {
    let runs = std::env::var("WHISK_BENCH_RUNS")
        .ok()
        .map_or(2000, |runs| runs.parse().expect("WHISK_BENCH_RUNS must be a number"));
    let cpu = cpu_name();
    let machine = std::env::var("WHISK_BENCH_MACHINE").unwrap_or_else(|_| slug(&cpu));
    let out_dir = std::env::var_os("WHISK_BENCH_OUT").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/results"),
        PathBuf::from,
    );
    let date = today();
    let commit = commit();
    let topology = StdPlatform.topology();
    let load_before = load_average().unwrap_or_else(|| "unknown".to_owned());
    let task_role = foreground_task_role();

    let executor = Executor::builder().build().expect("executor starts");
    let frame_workers = executor.frame_worker_count();
    let background_workers = lactose_task::bench_support::background_workers(&executor);
    eprintln!(
        "whisk bench: {cpu}, {} performance + {} efficiency cores, {frame_workers} frame-critical \
         workers, commit {commit}, load average {load_before}, task role {task_role}",
        topology.performance, topology.efficiency
    );

    let rate = calibrate(&executor);
    let calibration_check = check_calibration(&executor, rate);
    eprintln!("calibration: {rate:.4} iterations/ns, serial check {calibration_check:.3}");

    let shapes = [wide_shallow(), deep_chain(), fan_in_fan_out(frame_workers)];
    let mut makespans = Vec::new();
    for shape in &shapes {
        for background_load in [false, true] {
            let name = format!("{} {}", shape.name, if background_load { "zstd" } else { "quiet" });
            let result = phase(
                &name,
                || measure_makespan(&executor, shape, rate, runs, background_load),
                |result| {
                    let per_run =
                        Duration::from_micros(result.mean_makespan_micros as u64) + RUN_GAP;
                    Some(per_run * (runs + WARM_UP_RUNS) as u32)
                },
            );
            eprintln!(
                "{:<15} {:<8} p50 {:.3} p99 {:.3} max {:.3} | greedy {:.3}/{:.3} longest-first \
                 {:.3}/{:.3} | cpu {} us, work {:.0} us per run",
                result.shape,
                if background_load { "zstd x2" } else { "quiet" },
                result.ratio.p50,
                result.ratio.p99,
                result.ratio.max,
                result.greedy.p50,
                result.greedy.p99,
                result.longest_first.p50,
                result.longest_first.p99,
                optional(result.mean_cpu_micros, 0),
                result.mean_work_micros,
            );
            makespans.push(result);
        }
    }
    let wake = phase(
        "wake latency",
        || measure_wake_latency(&executor, WAKE_SAMPLES),
        |_| Some(WAKE_GAP * (WAKE_SAMPLES + WARM_UP_RUNS) as u32),
    );
    eprintln!(
        "wake latency: p50 {:.0} ns, p99 {:.0} ns, max {:.0} ns over {} pushes; process CPU {} \
         per second while doing so",
        wake.latency.p50,
        wake.latency.p99,
        wake.latency.max,
        wake.latency.samples,
        optional(wake.cpu_per_second, 4),
    );
    let timer_periods = [WAKE_GAP, TICK];
    let timers: Vec<WakeResult> = timer_periods
        .iter()
        .map(|&period| {
            let name = format!("timer wake every {period:.3?}");
            let timer = phase(
                &name,
                || measure_timer_latency(&executor, WAKE_SAMPLES, period),
                |_| Some(period * (WAKE_SAMPLES + WARM_UP_RUNS) as u32),
            );
            eprintln!(
                "timer wake every {period:.3?}: p50 {:.0} ns, p99 {:.0} ns, max {:.0} ns over {} \
                 deadlines, {} of them over a millisecond; process CPU {} per second",
                timer.latency.p50,
                timer.latency.p99,
                timer.latency.max,
                timer.latency.samples,
                timer.latency.over_a_millisecond,
                optional(timer.cpu_per_second, 4),
            );
            timer
        })
        .collect();
    let timer_at_tick = &timers[1];
    executor.shutdown();

    let steal_samples = phase(
        "steal",
        || {
            lactose_task::bench_support::time_steals(&StdPlatform, STEAL_SAMPLES)
                .expect("the thief thread starts")
        },
        |_| None,
    );
    let steal = percentiles(steal_samples.into_iter().map(|nanos| nanos as f64).collect());
    let clock_pair = clock_pair_nanos();
    let forced_split =
        phase("forced split", || measure_forced_split(topology.performance), |_| None);
    eprintln!(
        "steal: p50 {:.0} ns, p99 {:.0} ns, max {:.0} ns over {} steals (each includes one \
         clock pair, {clock_pair:.0} ns); forced split {forced_split:.0} worker-ns per entry",
        steal.p50, steal.p99, steal.max, steal.samples
    );
    let load_after = load_average().unwrap_or_else(|| "unknown".to_owned());

    let ratio_pass = makespans
        .iter()
        .all(|result| result.ratio.p50 <= RATIO_P50_LIMIT && result.ratio.p99 <= RATIO_P99_LIMIT);
    let wake_pass = wake.latency.p99 < WAKE_P99_LIMIT_NANOS;
    let timer_pass = timer_at_tick.latency.p99 < TIMER_P99_LIMIT_NANOS;
    let steal_pass = steal.p99 < STEAL_LIMIT_NANOS;

    let mut json = String::new();
    let _ = writeln!(json, "{{");
    let _ = writeln!(json, "  \"benchmark\": \"whisk\",");
    let _ = writeln!(json, "  \"machine\": \"{machine}\",");
    let _ = writeln!(json, "  \"cpu\": \"{}\",", cpu.replace('"', "'"));
    let _ = writeln!(json, "  \"os\": \"{}\",", std::env::consts::OS);
    let _ = writeln!(json, "  \"arch\": \"{}\",", std::env::consts::ARCH);
    let _ = writeln!(json, "  \"date\": \"{date}\",");
    let _ = writeln!(json, "  \"commit\": \"{commit}\",");
    let _ = writeln!(json, "  \"task_role\": \"{task_role}\",");
    let _ = writeln!(
        json,
        "  \"load_average\": {{ \"before\": \"{load_before}\", \"after\": \"{load_after}\" }},"
    );
    let _ = writeln!(
        json,
        "  \"topology\": {{ \"performance\": {}, \"efficiency\": {} }},",
        topology.performance, topology.efficiency
    );
    let _ = writeln!(json, "  \"frame_workers\": {frame_workers},");
    let _ = writeln!(json, "  \"background_workers\": {background_workers},");
    let _ = writeln!(
        json,
        "  \"calibration\": {{ \"iterations_per_nanosecond\": {rate:.6}, \"serial_check\": \
         {calibration_check:.4} }},"
    );
    let _ = writeln!(json, "  \"runs_per_case\": {runs},");
    let _ = writeln!(json, "  \"makespan_ratio\": [");
    for (index, result) in makespans.iter().enumerate() {
        let _ = writeln!(
            json,
            "    {{ \"shape\": \"{}\", \"background_zstd_tasks\": {}, \"samples\": {}, \"p50\": \
             {:.4}, \"p99\": {:.4}, \"max\": {:.4}, \"mean\": {:.4}, \"greedy_p50\": {:.4}, \
             \"greedy_p99\": {:.4}, \"longest_first_p50\": {:.4}, \"longest_first_p99\": {:.4}, \
             \"mean_makespan_us\": {:.1}, \"mean_lower_bound_us\": {:.1}, \"mean_work_us\": \
             {:.1}, \"mean_cpu_us\": {} }}{}",
            result.shape,
            if result.background_load { 2 } else { 0 },
            result.ratio.samples,
            result.ratio.p50,
            result.ratio.p99,
            result.ratio.max,
            result.ratio.mean,
            result.greedy.p50,
            result.greedy.p99,
            result.longest_first.p50,
            result.longest_first.p99,
            result.mean_makespan_micros,
            result.mean_lower_bound_micros,
            result.mean_work_micros,
            optional(result.mean_cpu_micros, 1),
            if index + 1 == makespans.len() { "" } else { "," }
        );
    }
    let _ = writeln!(json, "  ],");
    let _ = writeln!(
        json,
        "  \"wake_latency_ns\": {{ \"samples\": {}, \"p50\": {:.0}, \"p99\": {:.0}, \"max\": \
         {:.0}, \"mean\": {:.0}, \"process_cpu_per_second\": {} }},",
        wake.latency.samples,
        wake.latency.p50,
        wake.latency.p99,
        wake.latency.max,
        wake.latency.mean,
        optional(wake.cpu_per_second, 5),
    );
    let _ = writeln!(
        json,
        "  \"steal_ns\": {{ \"samples\": {}, \"p50\": {:.0}, \"p99\": {:.0}, \"max\": {:.0}, \
         \"mean\": {:.1}, \"clock_pair\": {clock_pair:.1} }},",
        steal.samples, steal.p50, steal.p99, steal.max, steal.mean
    );
    let _ = writeln!(json, "  \"timer_wake_ns\": [");
    for (index, (period, timer)) in timer_periods.iter().zip(&timers).enumerate() {
        let _ = writeln!(
            json,
            "    {{ \"period_us\": {:.1}, \"samples\": {}, \"p50\": {:.0}, \"p99\": {:.0}, \
             \"max\": {:.0}, \"mean\": {:.0}, \"over_a_millisecond\": {}, \
             \"process_cpu_per_second\": {} }}{}",
            period.as_secs_f64() * 1e6,
            timer.latency.samples,
            timer.latency.p50,
            timer.latency.p99,
            timer.latency.max,
            timer.latency.mean,
            timer.latency.over_a_millisecond,
            optional(timer.cpu_per_second, 5),
            if index + 1 == timers.len() { "" } else { "," }
        );
    }
    let _ = writeln!(json, "  ],");
    let _ = writeln!(json, "  \"forced_split_worker_ns_per_entry\": {forced_split:.1},");
    let _ = writeln!(
        json,
        "  \"thresholds\": {{ \"ratio_p50\": {RATIO_P50_LIMIT}, \"ratio_p99\": {RATIO_P99_LIMIT}, \
         \"wake_p99_ns\": {WAKE_P99_LIMIT_NANOS}, \"timer_wake_at_tick_p99_ns\": \
         {TIMER_P99_LIMIT_NANOS}, \"steal_p99_ns\": {STEAL_LIMIT_NANOS} }},"
    );
    let _ = writeln!(
        json,
        "  \"pass\": {{ \"makespan_ratio\": {ratio_pass}, \"wake_latency\": {wake_pass}, \
         \"timer_wake\": {timer_pass}, \"steal\": {steal_pass} }}"
    );
    let _ = writeln!(json, "}}");

    let mut summary = String::new();
    let _ = writeln!(summary, "# WHISK benchmark, {machine}, {date}\n");
    let _ = writeln!(
        summary,
        "Written by `cargo bench -p lactose-task --bench whisk`. Thresholds are those of\n\
         `crates/task/README.md`.\n"
    );
    let _ = writeln!(summary, "| | |");
    let _ = writeln!(summary, "|---|---|");
    let _ =
        writeln!(summary, "| CPU | {cpu} ({}, {}) |", std::env::consts::OS, std::env::consts::ARCH);
    let _ = writeln!(
        summary,
        "| Detected topology | {} performance, {} efficiency cores |",
        topology.performance, topology.efficiency
    );
    let _ = writeln!(
        summary,
        "| Workers | {frame_workers} frame-critical, {background_workers} background |"
    );
    let _ = writeln!(summary, "| Commit | `{commit}` |");
    let _ = writeln!(summary, "| Load average | {load_before} before, {load_after} after |");
    let _ = writeln!(summary, "| Task role | {task_role} |");
    let _ = writeln!(
        summary,
        "| Calibration | {rate:.4} kernel iterations per ns; serial check {calibration_check:.3} \
         (1.0 is exact) |"
    );
    let _ = writeln!(summary, "| Runs per case | {runs} |\n");
    let _ = writeln!(summary, "## Makespan divided by its lower bound ({})\n", verdict(ratio_pass));
    let _ = writeln!(summary, "At most {RATIO_P50_LIMIT} at P50 and {RATIO_P99_LIMIT} at P99.\n");
    let _ = writeln!(
        summary,
        "| Shape | Background load | P50 | P99 | Max | Greedy P50 / P99 | Longest-first P50 / \
         P99 | Makespan | Lower bound | CPU per run | Work per run |"
    );
    let _ = writeln!(summary, "|---|---|---|---|---|---|---|---|---|---|---|");
    for result in &makespans {
        let _ = writeln!(
            summary,
            "| {} | {} | {:.3} | {:.3} | {:.3} | {:.3} / {:.3} | {:.3} / {:.3} | {:.0} µs | \
             {:.0} µs | {} µs | {:.0} µs |",
            result.shape,
            if result.background_load { "two zstd tasks" } else { "none" },
            result.ratio.p50,
            result.ratio.p99,
            result.ratio.max,
            result.greedy.p50,
            result.greedy.p99,
            result.longest_first.p50,
            result.longest_first.p99,
            result.mean_makespan_micros,
            result.mean_lower_bound_micros,
            optional(result.mean_cpu_micros, 0),
            result.mean_work_micros,
        );
    }
    let _ = writeln!(
        summary,
        "\nGreedy and longest-first are schedulers with no overhead that never idle a worker \
         while a node is ready, simulated on the same draws; they differ in which ready node \
         they take. Neither is a bound. CPU per run is the whole process, gaps between runs \
         and any background load included; with no background load, what it exceeds the work \
         by is what the executor spends around the jobs.\n"
    );
    for shape in &shapes {
        let _ =
            writeln!(summary, "- {}: {} ({} nodes).", shape.name, shape.description, shape.nodes);
    }
    let _ = writeln!(summary, "\n## Wake latency ({})\n", verdict(wake_pass));
    let _ = writeln!(
        summary,
        "Push to a parked pool until the job starts; under {:.0} µs at P99. {} pushes, {} ms \
         apart.\n",
        WAKE_P99_LIMIT_NANOS / 1e3,
        wake.latency.samples,
        WAKE_GAP.as_millis()
    );
    let _ = writeln!(summary, "| P50 | P99 | Max | Process CPU per second of this phase |");
    let _ = writeln!(summary, "|---|---|---|---|");
    let _ = writeln!(
        summary,
        "| {:.1} µs | {:.1} µs | {:.1} µs | {} s |",
        wake.latency.p50 / 1e3,
        wake.latency.p99 / 1e3,
        wake.latency.max / 1e3,
        optional(wake.cpu_per_second, 4),
    );
    let _ = writeln!(summary, "\n## Timer wake ({})\n", verdict(timer_pass));
    let _ = writeln!(
        summary,
        "From a deadline until the frame-critical task sleeping until it resumes, as the timer \
         root ticks, with the pool otherwise idle. The 60 Hz P99 must be under {:.0} µs.\n",
        TIMER_P99_LIMIT_NANOS / 1e3
    );
    let _ = writeln!(
        summary,
        "| Period | Deadlines | P50 | P99 | Max | Over 1 ms | Process CPU per second |"
    );
    let _ = writeln!(summary, "|---|---|---|---|---|---|---|");
    for (period, timer) in timer_periods.iter().zip(&timers) {
        let _ = writeln!(
            summary,
            "| {:.3} ms | {} | {:.1} µs | {:.1} µs | {:.1} µs | {} | {} s |",
            period.as_secs_f64() * 1e3,
            timer.latency.samples,
            timer.latency.p50 / 1e3,
            timer.latency.p99 / 1e3,
            timer.latency.max / 1e3,
            timer.latency.over_a_millisecond,
            optional(timer.cpu_per_second, 4),
        );
    }
    let _ = writeln!(summary, "\n## Steal ({})\n", verdict(steal_pass));
    let _ = writeln!(
        summary,
        "One successful steal including the split, timed inside the executor on a thief thread \
         taking from a deque another thread filled; under {:.0} µs at P99. {} steals. Each \
         figure includes one pair of clock readings, {clock_pair:.0} ns here.\n",
        STEAL_LIMIT_NANOS / 1e3,
        steal.samples
    );
    let _ = writeln!(summary, "| P50 | P99 | Max | Mean |");
    let _ = writeln!(summary, "|---|---|---|---|");
    let _ = writeln!(
        summary,
        "| {:.0} ns | {:.0} ns | {:.0} ns | {:.0} ns |",
        steal.p50, steal.p99, steal.max, steal.mean
    );
    let _ = writeln!(
        summary,
        "\nNot a steal time, for reference: with every range forced down to single units, a \
         node of empty units costs {forced_split:.0} worker-ns per entry (split, push, pop or \
         steal, dispatch)."
    );

    std::fs::create_dir_all(&out_dir).expect("the results directory can be created");
    let stem = out_dir.join(format!("whisk-{machine}-{date}"));
    std::fs::write(stem.with_extension("json"), json).expect("the result file can be written");
    std::fs::write(stem.with_extension("md"), &summary).expect("the summary can be written");
    eprintln!("wrote {}.json and .md", stem.display());
    println!("{summary}");
}
