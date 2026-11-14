//! What one tick costs at the milestone 1 scale (architecture section 22: "100k entities
//! simulate a tick in at most 4 ms on 8 performance cores"): a server world of 100,000 rows
//! in four archetypes over sixteen regions, stepped through `Simulation::step` with a
//! representative load (systems that read and write a few columns, a transient column
//! written by an input applier and by a kernel, an event channel indexed by target, region
//! resources read by kernels and written at a sync point, a reduction, command traffic that
//! despawns and spawns rows every tick) and hashed after every step as SOUR does. The time
//! of each phase (sync points, each system, the hash) is collected from the `casein` spans
//! and the executor's per-range spans and printed; it is measured by an ignored test, run
//! by hand, and reported, never asserted. What the scene computes is asserted in a test
//! that runs everywhere, and the scene at a small scale records its hash under the
//! runner's session.

#![forbid(unsafe_code)]

mod glue;

use std::collections::BTreeMap;

use glue::{Harness, protocol_error};
use lactose_core::{NetId, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{
    ClaimedBy, Commands, Ctx, Event, EventWriter, Events, Executor, LayoutConfig, Mut,
    RegionConfig, RegionId, RegionMode, RegionRes, Res, ResMut, StableKeyOf, SystemSet, Without,
    World, WorldKind, finalize, system,
};
use lactose_math::{ExactSum, Scalar};
use lactose_reflect::{component, resource};
use lactose_sim::harness::{ChunkCapacity, layout_mode, run_recorded};
use lactose_sim::{
    InputApplier, PlayerSample, Registration, Simulation, SimulationBuilder, Stage, TickDelta,
    TickInput, TypeTable, sets,
};

// ---- The scene ----

include!("cost/scene.rs");

/// The scene at a small scale runs, every system does its work, and the result does not
/// depend on the worker count: each per-entity system's units fork over the workers
/// (D137 part B), so one worker and four run the same units in different orders and on
/// different threads.
#[test]
fn the_scene_runs_and_its_hash_is_the_same_on_one_and_four_workers() {
    let mut hashes = Vec::new();
    for workers in [1, 4] {
        let executor =
            Executor::builder().frame_workers(workers).background_workers(1).build().unwrap();
        let mut simulation = scene(2_000);
        let mut trace = Vec::new();
        for _ in 0..30 {
            simulation = step(&executor, simulation);
            trace.push(simulation.hash().unwrap());
        }
        let world = simulation.world_mut();
        assert_eq!(world.query::<&Position>().count(), 2_000 + PLAYERS as usize);
        assert!(world.query::<&Timer>().count() > 0);
        let mut hurt = 0;
        world.query::<&Health>().for_each(|_, hp| hurt += usize::from(hp.0 < 100));
        assert!(hurt > 0, "hits landed");
        assert_ne!(world.resource::<Energy>().unwrap().0, Scalar::ZERO);
        let wind = world.region_resource::<Wind>(RegionId(1)).unwrap().0;
        assert_eq!(wind, Scalar::from_i32(-8), "turned an even number of times");
        hashes.push(trace);
    }
    assert_eq!(hashes[0], hashes[1]);
}

/// Every kernel of the scene runs per chunk, or under `split_batches` per random range of a
/// chunk's rows, so the scene under SOUR's layout mode at every chunk capacity is the same
/// computation cut into different units: one hash every tick, and one merged output of the
/// reduction, which is derived and so outside the hash.
#[test]
fn the_scene_gives_one_hash_and_one_reduction_every_tick_under_every_layout() {
    let executor = Executor::builder().frame_workers(1).background_workers(1).build().unwrap();
    let trace = |layout: LayoutConfig| {
        let mut simulation = scene_in(2_000, layout);
        let mut trace = Vec::new();
        for _ in 0..20 {
            simulation = step(&executor, simulation);
            let energy = simulation.world_mut().resource::<Energy>().unwrap().0;
            trace.push((simulation.hash().unwrap(), energy));
        }
        trace
    };
    let reference = trace(LayoutConfig::default());
    assert!(reference.windows(2).all(|pair| pair[0].0 != pair[1].0), "every tick changes");
    for capacity in [
        ChunkCapacity::Rows(1),
        ChunkCapacity::Rows(7),
        ChunkCapacity::Rows(64),
        ChunkCapacity::Bytes(16384),
    ] {
        for seed in [1, 0x5eed_1a70] {
            let layout = layout_mode(capacity, seed).unwrap();
            let first = trace(layout).iter().zip(&reference).position(|(a, b)| a != b);
            assert_eq!(first, None, "{capacity:?} seed {seed}: first differing tick");
        }
    }
}

/// The scene at a small scale, under the runner's session: its worker count, its layout
/// and, in chaos mode, every perturbation with each split forced, recording the hash after
/// every step, so every SOUR mode compares the scene's ticks with the baseline's.
#[test]
fn the_scene_records_its_hash_every_tick_under_the_session() {
    const TEST: &str = "lactose_sim::cost::scene";
    let harness = Harness::from_env().unwrap();
    let mut simulation = scene_in(2_000, harness.layout);
    let mut recorder = harness.recorder(TEST, &simulation).unwrap();
    let mut record = |tick: Tick, hash: u64| recorder.record(tick.0, hash).map_err(protocol_error);
    let frames = (1..=30).map(frame);
    simulation = run_recorded(&harness.executor, simulation, frames, &mut record).unwrap();
    harness.finish(TEST).unwrap();
    let mut hurt = 0;
    simulation.world_mut().query::<&Health>().for_each(|_, hp| hurt += usize::from(hp.0 < 100));
    assert!(hurt > 0, "hits landed");
}

// ---- The measurement ----

#[path = "cost/timing.rs"]
mod timing;

/// The median and the 90th percentile of `samples`, in microseconds.
fn quantiles(samples: &mut [core::time::Duration]) -> (f64, f64) {
    samples.sort();
    let at = |q: f64| samples[((samples.len() - 1) as f64 * q).round() as usize];
    (at(0.5).as_secs_f64() * 1e6, at(0.9).as_secs_f64() * 1e6)
}

/// Whether `cargo xtask measure --short` runs the test: fewer ticks, the same figures.
#[expect(clippy::disallowed_methods, reason = "the measurement's repetitions, outside the step")]
fn short() -> bool {
    std::env::var_os("LACTOSE_MEASURE_SHORT").is_some()
}

/// One figure for `cargo xtask measure`, which reads the lines that start `measure: `.
fn figure(name: &str, value: impl core::fmt::Display) {
    println!("measure: {name} {value}");
}

/// The variable that sets N in place of the topology's, as `cargo xtask measure` passes it.
const WORKERS_VAR: &str = "LACTOSE_MEASURE_WORKERS";

/// N of plan decision 5.1, by the rule of the plan's section 4: the performance cores the
/// platform reports or, on a part that reports no core types (efficiency zero), its
/// physical cores, which `CoreTopology::homogeneous` stores as one more than its
/// `performance`; at most eight. [`WORKERS_VAR`] overrides it, a whole number above zero;
/// empty, it is unset, as `cargo xtask measure` reads it. `lactose-app`'s cost test has a copy
/// of this rule, and the two must match. Returns N and the topology, printed beside it.
#[expect(
    clippy::disallowed_methods,
    reason = "the worker count the measurement runs on, read outside the step"
)]
fn measured_workers() -> (u32, lactose_task::CoreTopology) {
    use lactose_task::{Platform, StdPlatform};
    let topology = StdPlatform.topology();
    if let Some(value) = std::env::var_os(WORKERS_VAR).filter(|value| !value.is_empty()) {
        let value = value.to_string_lossy();
        let n = value
            .parse::<u32>()
            .ok()
            .filter(|&n| n > 0)
            .unwrap_or_else(|| panic!("{WORKERS_VAR} is `{value}`, not a count above zero"));
        return (n, topology);
    }
    let cores =
        if topology.efficiency == 0 { topology.performance + 1 } else { topology.performance };
    (cores.min(8), topology)
}

/// How many sets of the scene's components its rows hold: the four kinds of rows, the
/// players' and any a despawn and spawn left.
fn archetypes(world: &mut World) -> usize {
    let mut keys = Vec::new();
    world.query::<&Position>().for_each(|key, _| keys.push(key));
    let shapes: std::collections::BTreeSet<[bool; 7]> = keys
        .iter()
        .map(|&key| {
            [
                world.get::<Velocity>(key).is_some(),
                world.get::<Health>(key).is_some(),
                world.get::<Timer>(key).is_some(),
                world.get::<Marker>(key).is_some(),
                world.get::<Contacts>(key).is_some(),
                world.get::<Steer>(key).is_some(),
                world.get::<ClaimedBy>(key).is_some(),
            ]
        })
        .collect();
    shapes.len()
}

/// Criterion 1 (architecture section 22, plan decision 5.1): the median step at 100,064 rows
/// on one frame worker (T₁) and on N ([`measured_workers`]: the performance cores, or the
/// physical cores of a part that reports no core types, up to eight) (T_N), in one sitting;
/// then S = (N·T_N − T₁)/(N − 1), the time that did not shrink with workers, and the
/// projected eight-core tick S + (T₁ − S)/8. Each phase of the step, and the
/// hash after it, which is outside the step, are printed beside them; the world hash after
/// the last tick is the same on both worker counts. `cargo xtask measure tick-100k` runs it
/// and writes the result file; by hand, `cargo nextest run -p lactose-sim --run-ignored only
/// --cargo-profile release --no-capture -E 'test(=cost_of_a_tick_at_100k_rows)'`.
#[test]
#[ignore = "a measurement, run by `cargo xtask measure`: timings are reported, not asserted"]
#[expect(clippy::disallowed_types, reason = "test code measuring the step, outside it")]
fn cost_of_a_tick_at_100k_rows() {
    use std::time::{Duration, Instant};
    const WARM: u32 = 10;
    let ticks: u32 = if short() { 6 } else { 60 };
    let spans: &'static timing::Spans = Box::leak(Box::default());
    tracing::subscriber::set_global_default(spans).unwrap();
    let (n, topology) = measured_workers();
    let counts = if n > 1 { vec![1, n] } else { vec![1] };
    let mut medians = Vec::new();
    let mut last_hashes = Vec::new();
    let mut systems = std::collections::BTreeSet::new();
    let mut shape = (0, 0);
    for &workers in &counts {
        let executor =
            Executor::builder().frame_workers(workers).background_workers(1).build().unwrap();
        let mut simulation = scene(ROWS);
        let world = simulation.world_mut();
        shape = (world.query::<&Position>().count(), archetypes(world));
        let mut phases: BTreeMap<String, Vec<Duration>> = BTreeMap::new();
        let mut steps = Vec::new();
        let mut hashes = Vec::new();
        let mut last = 0;
        for tick in 0..WARM + ticks {
            spans.take();
            let start = Instant::now();
            simulation = step(&executor, simulation);
            let stepped = start.elapsed();
            let start = Instant::now();
            last = simulation.hash().unwrap();
            let hashed = start.elapsed();
            assert_ne!(last, 0);
            if tick < WARM {
                continue;
            }
            steps.push(stepped);
            hashes.push(hashed);
            for (label, time) in spans.take() {
                phases.entry(label).or_default().push(time);
            }
        }
        let rows = simulation.world_mut().query::<&Position>().count();
        println!("{workers} frame worker(s), {rows} rows, {ticks} ticks: median / p90 in µs");
        let (median, p90) = quantiles(&mut steps);
        println!("  {:<40} {median:>9.1} {p90:>9.1}", "step (whole)");
        for (label, samples) in &mut phases {
            let (median, p90) = quantiles(samples);
            println!("  {label:<40} {median:>9.1} {p90:>9.1}");
            if let Some(name) = label.strip_prefix("system ") {
                systems.insert(name.to_owned());
            }
        }
        let (hash_median, hash_p90) = quantiles(&mut hashes);
        println!("  {:<40} {hash_median:>9.1} {hash_p90:>9.1}", "hash (all regions)");
        let start = Instant::now();
        let one = simulation.world_hash(RegionId(1)).unwrap();
        println!("  {:<40} {:>9.1}", "hash (one region)", start.elapsed().as_secs_f64() * 1e6);
        assert_ne!(one, 0);
        medians.push(median);
        last_hashes.push(last);
    }
    assert!(
        last_hashes.windows(2).all(|pair| pair[0] == pair[1]),
        "the world hash after the last tick depends on the worker count: {last_hashes:x?}"
    );

    let t1 = medians[0];
    figure("rows", shape.0);
    figure("archetypes", shape.1);
    figure("regions", REGIONS);
    figure("systems", systems.len());
    figure("ticks", ticks);
    figure("warm_up_ticks", WARM);
    figure("workers", counts.iter().map(u32::to_string).collect::<Vec<_>>().join(" and "));
    figure("hash", format!("{:#018x}", last_hashes[0]));
    figure("topology.performance", topology.performance);
    figure("topology.efficiency", topology.efficiency);
    figure("n", n);
    figure("t1_us", format!("{t1:.1}"));
    if let [_, tn] = medians[..] {
        let nf = f64::from(n);
        let serial = (nf * tn - t1) / (nf - 1.0);
        let projected = serial + (t1 - serial) / 8.0;
        figure("tn_us", format!("{tn:.1}"));
        figure("s_us", format!("{serial:.1}"));
        figure("projection_us", format!("{projected:.1}"));
        println!(
            "T1 {t1:.1} µs, T{n} {tn:.1} µs, S {serial:.1} µs, projected eight-core tick \
             {projected:.1} µs (at most 4,000)"
        );
    } else {
        println!("T1 {t1:.1} µs; one performance core, so no T_N and no projection");
    }
}
