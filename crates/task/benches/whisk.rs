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
//! | `WHISK_BENCH_RUNS` | Graph runs per shape and condition | 300 |
//! | `WHISK_BENCH_MACHINE` | Machine name used in the file name | derived from the CPU name |
//! | `WHISK_BENCH_OUT` | Output directory | `docs/results` |
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
//! - **Wake latency.** From just before a one-node graph is pushed to a parked pool until
//!   its job starts.
//! - **Steal.** Two numbers. `executor` runs a node of empty units with every range forced
//!   down to single units, so each unit is one entry created by one split and taken by one
//!   steal or pop; wall time times workers over entries bounds a steal with its split from
//!   above, on the executor's own code path. `deque` times the same sequence in isolation
//!   (steal, halve, clone the run handle, push, check for sleepers) on a replica of the
//!   entry, because the executor's internals are private to it.

use core::cmp::Reverse;
use core::hint::black_box;
use core::ops::Range;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering, fence};
use std::collections::{BinaryHeap, VecDeque};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_deque::{Steal, Worker};
use lactose_task::{
    ChaosConfig, Executor, Graph, GraphBuilder, GraphNode, JobContext, NodeId, Platform,
    StdPlatform, WorkerClass, yield_now,
};

const MIN_JOB_NANOS: f64 = 10_000.0;
const MAX_JOB_NANOS: f64 = 1_000_000.0;
const WARM_UP_RUNS: usize = 20;
const RUN_GAP: Duration = Duration::from_millis(1);

/// Thresholds from `crates/task/README.md`.
const RATIO_P50_LIMIT: f64 = 1.25;
const RATIO_P99_LIMIT: f64 = 1.5;
const WAKE_P99_LIMIT_NANOS: f64 = 20_000.0;
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

/// The CPU-burning kernel: a dependent chain of shifts and xors, about a nanosecond a turn.
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
    description: &'static str,
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
        description: "192 independent nodes, 64 of them with one dependent",
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
        description: "a 128-node chain; every fourth link forks a side node that rejoins two links on",
        nodes: next as usize,
        edges,
    }
}

/// Sixteen rounds of one node spreading to eight and joining to one.
///
/// The spread is kept at eight because the lower bound is loose for this shape: while a
/// join runs only one worker has work, which total work over worker count ignores, and a
/// spread wider than the pool is not bounded by its longest job. A scheduler with no
/// overhead at all scores 1.32 at P50 with a spread of sixteen on four workers. With eight
/// the bound stays within a few percent of what is achievable on four to sixteen workers;
/// the `ideal` column of the results shows how close for the machine at hand.
fn fan_in_fan_out() -> Shape {
    let (rounds, width) = (16u32, 8u32);
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
        description: "16 rounds of 1 node spreading to 8 and joining to 1",
        nodes: (rounds * (width + 1) + 1) as usize,
        edges,
    }
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
    /// a node is ready, taking ready nodes first come, first served. Not a bound: it shows
    /// how much of a measured ratio is the lower bound being unreachable for the shape.
    fn ideal_makespan(&self, nanos: &[f64], workers: u32) -> f64 {
        let mut pending = vec![0u32; self.nodes];
        let mut dependents = vec![Vec::new(); self.nodes];
        for &(before, after) in &self.edges {
            pending[after as usize] += 1;
            dependents[before as usize].push(after);
        }
        let mut ready: VecDeque<u32> =
            (0..self.nodes as u32).filter(|&node| pending[node as usize] == 0).collect();
        // Running nodes as `(finish time in picoseconds, node)`, earliest first.
        let mut running = BinaryHeap::new();
        let mut now = 0u64;
        loop {
            while running.len() < workers as usize
                && let Some(node) = ready.pop_front()
            {
                running.push(Reverse((now + (nanos[node as usize] * 1e3) as u64, node)));
            }
            let Some(Reverse((finished, node))) = running.pop() else { break };
            now = finished;
            for &after in &dependents[node as usize] {
                pending[after as usize] -= 1;
                if pending[after as usize] == 0 {
                    ready.push_back(after);
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
    }
}

/// Kernel iterations per nanosecond on a frame-critical worker, from the fastest of several
/// timed burns. Taking the fastest means anything that slows a job later makes it run
/// longer than its drawn duration, never shorter, so the ratio errs upwards.
fn calibrate(executor: &Executor) -> f64 {
    executor.block_on(executor.spawn(WorkerClass::FrameCritical, async {
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
}

/// Burns a set of drawn durations serially on a frame-critical worker and returns the time
/// it took over the time drawn. Close to 1.0 when the calibration holds.
fn check_calibration(executor: &Executor, rate: f64) -> f64 {
    executor.block_on(executor.spawn(WorkerClass::FrameCritical, async move {
        let mut rng = Rng(7);
        let jobs: Vec<f64> = (0..400).map(|_| rng.job_nanos()).collect();
        let start = Instant::now();
        for nanos in &jobs {
            burn((nanos * rate) as u64);
        }
        start.elapsed().as_nanos() as f64 / jobs.iter().sum::<f64>()
    }))
}

struct MakespanResult {
    shape: &'static str,
    background_load: bool,
    ratio: Percentiles,
    /// The same statistic for the no-overhead scheduler on the same draws.
    ideal_ratio: Percentiles,
    mean_makespan_micros: f64,
    mean_lower_bound_micros: f64,
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
    let lower_bounds: Vec<(Vec<f64>, f64)> = {
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
        let mut makespans = Vec::with_capacity(lower_bounds.len());
        for (nanos, _) in &lower_bounds {
            for (slot, nanos) in iterations.iter().zip(nanos) {
                slot.store((nanos * rate) as u64, Ordering::Relaxed);
            }
            let start = Instant::now();
            graph = runner.run_graph(graph).await;
            makespans.push(start.elapsed().as_nanos() as f64);
            // Let the pool park, as it does between frames.
            runner.sleep_until(Instant::now() + RUN_GAP).await;
        }
        (makespans, lower_bounds)
    });
    let (makespans, lower_bounds) = executor.block_on(driver);
    stop.store(true, Ordering::Relaxed);
    for task in load {
        assert!(executor.block_on(task) > 0, "the background load never ran");
    }

    let measured = makespans.iter().zip(&lower_bounds).skip(WARM_UP_RUNS);
    let ratios: Vec<f64> =
        measured.clone().map(|(makespan, (_, bound))| makespan / bound).collect();
    let ideal_ratios: Vec<f64> = lower_bounds
        .iter()
        .skip(WARM_UP_RUNS)
        .map(|(nanos, bound)| shape.ideal_makespan(nanos, workers) / bound)
        .collect();
    let count = ratios.len() as f64;
    MakespanResult {
        shape: shape.name,
        background_load,
        ratio: percentiles(ratios),
        ideal_ratio: percentiles(ideal_ratios),
        mean_makespan_micros: measured.clone().map(|(makespan, _)| makespan).sum::<f64>()
            / count
            / 1e3,
        mean_lower_bound_micros: measured.map(|(_, (_, bound))| bound).sum::<f64>() / count / 1e3,
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

fn measure_wake_latency(executor: &Executor, samples: usize) -> Percentiles {
    let epoch = Instant::now();
    let started = Arc::new(AtomicU64::new(0));
    let mut builder = GraphBuilder::new();
    builder.add_node(Arc::new(StampNode { epoch, started: started.clone() }));
    let mut graph = builder.build().expect("one node");
    let mut latencies = Vec::with_capacity(samples);
    for sample in 0..samples + WARM_UP_RUNS {
        // Long enough for every worker to give up scanning and park.
        std::thread::sleep(Duration::from_millis(2));
        let pushed = epoch.elapsed().as_nanos() as u64;
        graph = executor.block_on(executor.run_graph(graph));
        if sample >= WARM_UP_RUNS {
            latencies.push(started.load(Ordering::Acquire).saturating_sub(pushed) as f64);
        }
    }
    percentiles(latencies)
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

/// Worker-nanoseconds per entry when every range is forced down to single units: an upper
/// bound on one steal with its split, measured on the executor's own path.
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
        graph = executor.block_on(executor.run_graph(graph));
        fastest = fastest.min(start.elapsed().as_nanos() as f64);
    }
    executor.shutdown();
    fastest * f64::from(workers) / f64::from(units)
}

/// The shape of the executor's graph entry: a shared run handle and a unit range.
struct ReplicaEntry {
    run: Arc<AtomicU64>,
    units: Range<u32>,
}

/// One steal from another thread's deque, halving the range, cloning the run handle,
/// pushing the upper half and checking for sleepers, in isolation.
fn measure_deque_steal() -> f64 {
    let victim = Worker::<ReplicaEntry>::new_lifo();
    let stealer = victim.stealer();
    let thief = Worker::<ReplicaEntry>::new_lifo();
    let run = Arc::new(AtomicU64::new(0));
    let sleepers = AtomicU64::new(0);
    let batch = 4096u32;
    let mut fastest = f64::INFINITY;
    for _ in 0..200 {
        for start in 0..batch {
            victim.push(ReplicaEntry { run: run.clone(), units: start * 64..start * 64 + 64 });
        }
        let start = Instant::now();
        for _ in 0..batch {
            let entry = loop {
                match stealer.steal() {
                    Steal::Success(entry) => break entry,
                    Steal::Retry => {}
                    Steal::Empty => unreachable!("the victim holds a full batch"),
                }
            };
            let middle = entry.units.start + (entry.units.end - entry.units.start) / 2;
            thief.push(ReplicaEntry { run: entry.run.clone(), units: middle..entry.units.end });
            fence(Ordering::SeqCst);
            black_box(sleepers.load(Ordering::SeqCst));
            black_box(entry.units.start..middle);
        }
        fastest = fastest.min(start.elapsed().as_nanos() as f64 / f64::from(batch));
        while thief.pop().is_some() {}
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

fn main() {
    let runs = std::env::var("WHISK_BENCH_RUNS")
        .ok()
        .map_or(300, |runs| runs.parse().expect("WHISK_BENCH_RUNS must be a number"));
    let cpu = cpu_name();
    let machine = std::env::var("WHISK_BENCH_MACHINE").unwrap_or_else(|_| slug(&cpu));
    let out_dir = std::env::var_os("WHISK_BENCH_OUT").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/results"),
        PathBuf::from,
    );
    let date = today();
    let commit = commit();
    let topology = StdPlatform.topology();

    let executor = Executor::builder().build().expect("executor starts");
    let frame_workers = executor.frame_worker_count();
    let background_workers = if topology.efficiency == 0 { 2 } else { topology.efficiency };
    eprintln!(
        "whisk bench: {cpu}, {} performance + {} efficiency cores, {frame_workers} frame-critical \
         workers, commit {commit}",
        topology.performance, topology.efficiency
    );

    let rate = calibrate(&executor);
    let calibration_check = check_calibration(&executor, rate);
    eprintln!("calibration: {rate:.4} iterations/ns, serial check {calibration_check:.3}");

    let shapes = [wide_shallow(), deep_chain(), fan_in_fan_out()];
    let mut makespans = Vec::new();
    for shape in &shapes {
        for background_load in [false, true] {
            let result = measure_makespan(&executor, shape, rate, runs, background_load);
            eprintln!(
                "{:<15} {:<10} ratio p50 {:.3} p99 {:.3} max {:.3} (ideal p50 {:.3} p99 {:.3})",
                result.shape,
                if background_load { "zstd x2" } else { "quiet" },
                result.ratio.p50,
                result.ratio.p99,
                result.ratio.max,
                result.ideal_ratio.p50,
                result.ideal_ratio.p99
            );
            makespans.push(result);
        }
    }
    let wake = measure_wake_latency(&executor, 1000);
    eprintln!(
        "wake latency: p50 {:.0} ns, p99 {:.0} ns, max {:.0} ns",
        wake.p50, wake.p99, wake.max
    );
    executor.shutdown();

    let forced_split = measure_forced_split(topology.performance);
    let deque_steal = measure_deque_steal();
    eprintln!(
        "steal: executor path {forced_split:.0} ns per entry, deque replica {deque_steal:.0} ns"
    );

    let ratio_pass = makespans
        .iter()
        .all(|result| result.ratio.p50 <= RATIO_P50_LIMIT && result.ratio.p99 <= RATIO_P99_LIMIT);
    let wake_pass = wake.p99 < WAKE_P99_LIMIT_NANOS;
    let steal_pass = forced_split < STEAL_LIMIT_NANOS;

    let mut json = String::new();
    let _ = writeln!(json, "{{");
    let _ = writeln!(json, "  \"benchmark\": \"whisk\",");
    let _ = writeln!(json, "  \"machine\": \"{machine}\",");
    let _ = writeln!(json, "  \"cpu\": \"{}\",", cpu.replace('"', "'"));
    let _ = writeln!(json, "  \"os\": \"{}\",", std::env::consts::OS);
    let _ = writeln!(json, "  \"arch\": \"{}\",", std::env::consts::ARCH);
    let _ = writeln!(json, "  \"date\": \"{date}\",");
    let _ = writeln!(json, "  \"commit\": \"{commit}\",");
    let _ = writeln!(
        json,
        "  \"topology\": {{ \"performance\": {}, \"efficiency\": {} }},",
        topology.performance, topology.efficiency
    );
    let _ = writeln!(json, "  \"frame_workers\": {frame_workers},");
    let _ = writeln!(json, "  \"background_workers\": {background_workers},");
    let _ = writeln!(
        json,
        "  \"calibration\": {{ \"iterations_per_nanosecond\": {rate:.6}, \"serial_check\": {calibration_check:.4} }},"
    );
    let _ = writeln!(json, "  \"runs_per_case\": {runs},");
    let _ = writeln!(json, "  \"makespan_ratio\": [");
    for (index, result) in makespans.iter().enumerate() {
        let _ = writeln!(
            json,
            "    {{ \"shape\": \"{}\", \"background_zstd_tasks\": {}, \"p50\": {:.4}, \"p99\": {:.4}, \
             \"max\": {:.4}, \"mean\": {:.4}, \"ideal_p50\": {:.4}, \"ideal_p99\": {:.4}, \
             \"mean_makespan_us\": {:.1}, \"mean_lower_bound_us\": {:.1} }}{}",
            result.shape,
            if result.background_load { 2 } else { 0 },
            result.ratio.p50,
            result.ratio.p99,
            result.ratio.max,
            result.ratio.mean,
            result.ideal_ratio.p50,
            result.ideal_ratio.p99,
            result.mean_makespan_micros,
            result.mean_lower_bound_micros,
            if index + 1 == makespans.len() { "" } else { "," }
        );
    }
    let _ = writeln!(json, "  ],");
    let _ = writeln!(
        json,
        "  \"wake_latency_ns\": {{ \"p50\": {:.0}, \"p99\": {:.0}, \"max\": {:.0}, \"mean\": {:.0} }},",
        wake.p50, wake.p99, wake.max, wake.mean
    );
    let _ = writeln!(
        json,
        "  \"steal_ns\": {{ \"executor_forced_split_per_entry\": {forced_split:.1}, \"deque_replica\": {deque_steal:.1} }},"
    );
    let _ = writeln!(
        json,
        "  \"thresholds\": {{ \"ratio_p50\": {RATIO_P50_LIMIT}, \"ratio_p99\": {RATIO_P99_LIMIT}, \
         \"wake_p99_ns\": {WAKE_P99_LIMIT_NANOS}, \"steal_ns\": {STEAL_LIMIT_NANOS} }},"
    );
    let _ = writeln!(
        json,
        "  \"pass\": {{ \"makespan_ratio\": {ratio_pass}, \"wake_latency\": {wake_pass}, \"steal\": {steal_pass} }}"
    );
    let _ = writeln!(json, "}}");

    let mut summary = String::new();
    let _ = writeln!(summary, "# WHISK benchmark, {machine}, {date}\n");
    let _ = writeln!(
        summary,
        "Written by `cargo bench -p lactose-task --bench whisk`. Thresholds are those of"
    );
    let _ = writeln!(summary, "`crates/task/README.md`.\n");
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
    let _ = writeln!(
        summary,
        "| Calibration | {rate:.4} kernel iterations per ns; serial check {calibration_check:.3} (1.0 is exact) |"
    );
    let _ = writeln!(summary, "| Runs per case | {runs} |\n");
    let _ = writeln!(summary, "## Makespan divided by its lower bound ({})\n", verdict(ratio_pass));
    let _ = writeln!(summary, "At most {RATIO_P50_LIMIT} at P50 and {RATIO_P99_LIMIT} at P99.\n");
    let _ = writeln!(
        summary,
        "| Shape | Background load | P50 | P99 | Max | Ideal P50 | Ideal P99 | Mean makespan | Mean lower bound |"
    );
    let _ = writeln!(summary, "|---|---|---|---|---|---|---|---|---|");
    for result in &makespans {
        let _ = writeln!(
            summary,
            "| {} | {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.0} µs | {:.0} µs |",
            result.shape,
            if result.background_load { "two zstd tasks" } else { "none" },
            result.ratio.p50,
            result.ratio.p99,
            result.ratio.max,
            result.ideal_ratio.p50,
            result.ideal_ratio.p99,
            result.mean_makespan_micros,
            result.mean_lower_bound_micros
        );
    }
    let _ = writeln!(
        summary,
        "\nIdeal is a scheduler with no overhead that never idles a worker while a node is ready, \
         simulated on the same draws: the part of a ratio that no executor can remove.\n"
    );
    for shape in &shapes {
        let _ =
            writeln!(summary, "- {}: {} ({} nodes).", shape.name, shape.description, shape.nodes);
    }
    let _ = writeln!(summary, "\n## Wake latency ({})\n", verdict(wake_pass));
    let _ = writeln!(
        summary,
        "Push to a parked pool until the job starts; under {:.0} µs at P99.\n",
        WAKE_P99_LIMIT_NANOS / 1e3
    );
    let _ = writeln!(summary, "| P50 | P99 | Max |");
    let _ = writeln!(summary, "|---|---|---|");
    let _ = writeln!(
        summary,
        "| {:.1} µs | {:.1} µs | {:.1} µs |",
        wake.p50 / 1e3,
        wake.p99 / 1e3,
        wake.max / 1e3
    );
    let _ = writeln!(summary, "\n## Steal ({})\n", verdict(steal_pass));
    let _ = writeln!(
        summary,
        "One successful steal including the split; under {:.0} µs.\n",
        STEAL_LIMIT_NANOS / 1e3
    );
    let _ = writeln!(summary, "| Measure | Time |");
    let _ = writeln!(summary, "|---|---|");
    let _ = writeln!(
        summary,
        "| Executor path: worker time per entry with every range forced to single units (upper bound) | {forced_split:.0} ns |"
    );
    let _ = writeln!(
        summary,
        "| Deque replica: steal, halve, clone, push, sleeper check | {deque_steal:.0} ns |"
    );

    std::fs::create_dir_all(&out_dir).expect("the results directory can be created");
    let stem = out_dir.join(format!("whisk-{machine}-{date}"));
    std::fs::write(stem.with_extension("json"), json).expect("the result file can be written");
    std::fs::write(stem.with_extension("md"), &summary).expect("the summary can be written");
    eprintln!("wrote {}.json and .md", stem.display());
    println!("{summary}");
}
