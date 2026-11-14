//! What one tick costs at the milestone 1 scale (architecture section 22: "100k entities
//! simulate a tick in at most 4 ms on 8 performance cores"): a server world of 100,000 rows
//! in four archetypes over sixteen regions, stepped through `Simulation::step` with a
//! representative load (systems that read and write a few columns, a transient column
//! written by an input applier and by a kernel, an event channel indexed by target, region
//! resources read by kernels and written at a sync point, a reduction, command traffic that
//! despawns and spawns rows every tick) and hashed after every step as SOUR does. The time
//! of each phase (sync points, each system, the hash) is collected from the `casein` spans
//! and printed; it is measured by an ignored test, run by hand, and reported, never
//! asserted. What the scene computes is asserted in a test that runs everywhere.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use lactose_core::{NetId, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{
    ClaimedBy, Commands, Ctx, Event, EventWriter, Events, Executor, LayoutConfig, Mut,
    RegionConfig, RegionId, RegionMode, RegionRes, Res, ResMut, StableKeyOf, SystemSet, Without,
    World, WorldKind, finalize, system,
};
use lactose_math::{ExactSum, Scalar};
use lactose_reflect::{component, resource};
use lactose_sim::harness::{ChunkCapacity, layout_mode};
use lactose_sim::{
    InputApplier, PlayerSample, Registration, Simulation, SimulationBuilder, Stage, TickDelta,
    TickInput, TypeTable, sets,
};

// ---- The scene ----

include!("cost/scene.rs");

/// The scene at a small scale runs, every system does its work, and the result does not
/// depend on the worker count. Until the run forks over chunks (D137's part B), each system
/// runs as one job, so the equality of the two worker counts holds by construction; the
/// scene builds its own executor, so SOUR's modes give it no coverage, and the next test
/// runs it under every layout instead.
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

// ---- The measurement ----

/// Times the `casein` spans: each sync point, stage and system, summed by name until taken.
mod timing {
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::subscriber::Interest;
    use tracing::{Event, Metadata, Subscriber};

    /// The time since the first call: the one place the subscriber reads the clock.
    #[expect(clippy::disallowed_types, reason = "test code timing the step from outside")]
    fn clock() -> Duration {
        static START: OnceLock<std::time::Instant> = OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed()
    }

    struct Open {
        label: String,
        entered: Option<Duration>,
    }

    #[derive(Default)]
    pub struct Spans {
        next: AtomicU64,
        open: Mutex<BTreeMap<u64, Open>>,
        totals: Mutex<BTreeMap<String, Duration>>,
    }

    impl Spans {
        /// The time spent in each span since the last take, by label.
        pub fn take(&self) -> BTreeMap<String, Duration> {
            core::mem::take(&mut *self.totals.lock().unwrap())
        }
    }

    struct Name(String);

    impl Visit for Name {
        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "name" || field.name() == "stage" {
                self.0 = value.to_owned();
            }
        }

        fn record_debug(&mut self, field: &Field, value: &dyn core::fmt::Debug) {
            if field.name() == "name" || field.name() == "stage" {
                self.0 = format!("{value:?}");
            }
        }
    }

    impl Subscriber for &'static Spans {
        fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
            if self.enabled(metadata) { Interest::always() } else { Interest::never() }
        }

        fn enabled(&self, metadata: &Metadata<'_>) -> bool {
            metadata.is_span() && metadata.target() == "casein"
        }

        fn new_span(&self, span: &Attributes<'_>) -> Id {
            let mut name = Name(String::new());
            span.record(&mut name);
            let label = format!("{} {}", span.metadata().name(), name.0);
            let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
            self.open.lock().unwrap().insert(id, Open { label, entered: None });
            Id::from_u64(id)
        }

        fn record(&self, _: &Id, _: &Record<'_>) {}

        fn record_follows_from(&self, _: &Id, _: &Id) {}

        fn event(&self, _: &Event<'_>) {}

        fn enter(&self, span: &Id) {
            if let Some(open) = self.open.lock().unwrap().get_mut(&span.into_u64()) {
                open.entered = Some(clock());
            }
        }

        fn exit(&self, span: &Id) {
            let now = clock();
            let Some(open) = self.open.lock().unwrap().remove(&span.into_u64()) else { return };
            if let Some(entered) = open.entered {
                *self.totals.lock().unwrap().entry(open.label).or_default() += now - entered;
            }
        }
    }
}

/// The median and the 90th percentile of `samples`, in microseconds.
fn quantiles(samples: &mut [core::time::Duration]) -> (f64, f64) {
    samples.sort();
    let at = |q: f64| samples[((samples.len() - 1) as f64 * q).round() as usize];
    (at(0.5).as_secs_f64() * 1e6, at(0.9).as_secs_f64() * 1e6)
}

/// The time of a step at 100,000 rows, phase by phase, and of the hash after it, printed
/// for the report. Run with `cargo nextest run -p lactose-sim --run-ignored only
/// --cargo-profile release --no-capture -E 'test(cost_)'`.
#[test]
#[ignore = "a measurement, run by hand: timings are reported, not asserted"]
#[expect(clippy::disallowed_types, reason = "test code measuring the step, outside it")]
fn cost_of_a_tick_at_100k_rows() {
    use std::time::{Duration, Instant};
    const WARM: u32 = 10;
    const TICKS: u32 = 60;
    let spans: &'static timing::Spans = Box::leak(Box::default());
    tracing::subscriber::set_global_default(spans).unwrap();
    for workers in [1, 4] {
        let executor =
            Executor::builder().frame_workers(workers).background_workers(1).build().unwrap();
        let mut simulation = scene(ROWS);
        let mut phases: BTreeMap<String, Vec<Duration>> = BTreeMap::new();
        let mut steps = Vec::new();
        let mut hashes = Vec::new();
        for tick in 0..WARM + TICKS {
            spans.take();
            let start = Instant::now();
            simulation = step(&executor, simulation);
            let stepped = start.elapsed();
            let start = Instant::now();
            let hash = simulation.hash().unwrap();
            let hashed = start.elapsed();
            assert_ne!(hash, 0);
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
        println!("{workers} frame worker(s), {rows} rows, {TICKS} ticks: median / p90 in µs");
        let (median, p90) = quantiles(&mut steps);
        println!("  {:<40} {median:>9.1} {p90:>9.1}", "step (whole)");
        for (label, samples) in &mut phases {
            let (median, p90) = quantiles(samples);
            println!("  {label:<40} {median:>9.1} {p90:>9.1}");
        }
        let (median, p90) = quantiles(&mut hashes);
        println!("  {:<40} {median:>9.1} {p90:>9.1}", "hash (all regions)");
        let start = Instant::now();
        let one = simulation.world_hash(RegionId(1)).unwrap();
        println!("  {:<40} {:>9.1}", "hash (one region)", start.elapsed().as_secs_f64() * 1e6);
        assert_ne!(one, 0);
    }
}
