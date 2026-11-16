//! What the per-chunk kernel path visits, counted rather than timed: a per-entity run is
//! one unit per chunk, or under `split_batches` one per random range of a chunk's rows;
//! a `Changed` filter skips every chunk whose column tick is not after its window, and
//! `World::changed_since` every chunk whose column tick is not after the mark, without
//! reading a row of it. The counts are the `casein` trace events the run and
//! `changed_since` emit, captured by the process's subscriber; each test reads only the
//! events of its own system or component.

#![forbid(unsafe_code)]

use crate::common;

use std::collections::BTreeMap;
use std::sync::{Mutex, Once};

use common::*;
use lactose_core::StableKey;
use lactose_ecs::{
    Changed, Ctx, LayoutConfig, RegionConfig, RegionId, RegionMode, Res, ResMut, Schedule,
    StableKeyOf, World, WorldKind, finalize, system,
};
use lactose_math::{ExactSum, Scalar};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Metadata, Subscriber, span};

/// The fields of every `casein` trace event, by name.
static EVENTS: Mutex<Vec<BTreeMap<String, String>>> = Mutex::new(Vec::new());

struct Capture;

struct Fields(BTreeMap<String, String>);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn core::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }
}

impl Subscriber for Capture {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.is_event() && *metadata.level() == Level::TRACE && metadata.target() == "casein"
    }
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }
    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
    fn event(&self, event: &Event<'_>) {
        let mut fields = Fields(BTreeMap::new());
        event.record(&mut fields);
        EVENTS.lock().expect("the events").push(fields.0);
    }
    fn enter(&self, _: &span::Id) {}
    fn exit(&self, _: &span::Id) {}
}

fn capture() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| tracing::subscriber::set_global_default(Capture).expect("one subscriber"));
}

/// The counts of the events whose `field` is `value`, oldest first, taken out of the log.
fn take(field: &str, value: &str, counts: &[&str]) -> Vec<Vec<u64>> {
    let mut events = EVENTS.lock().expect("the events");
    let (mine, rest): (Vec<_>, Vec<_>) =
        events.drain(..).partition(|fields| fields.get(field).is_some_and(|v| v == value));
    *events = rest;
    mine.iter()
        .map(|fields| counts.iter().map(|count| fields[*count].parse().unwrap()).collect())
        .collect()
}

/// The counts of the per-entity runs of `system` since the last call.
fn runs(system: &str) -> Vec<[u64; 4]> {
    take("system", system, &["units", "chunks_skipped", "rows", "invocations"])
        .into_iter()
        .map(|counts| counts.try_into().unwrap())
        .collect()
}

const CHUNK_ROWS: u32 = 8;
const ROWS: u64 = 80;

/// `ROWS` rows with `Health` and `Mass`, filling `ROWS / CHUNK_ROWS` chunks in key order.
fn populated(layout: LayoutConfig) -> World {
    let mut world = world(WorldKind::Server, layout);
    for id in 1..=ROWS {
        let mass = Mass(Scalar::from_i32(id as i32));
        world.spawn(net(id), REGION, (Health(100), mass)).unwrap();
    }
    world
}

fn rows_of(chunk_rows: u32) -> LayoutConfig {
    LayoutConfig { chunk_rows: Some(chunk_rows), ..LayoutConfig::default() }
}

#[system(name = "units::wounded", worlds = [server])]
fn wounded(_hp: &Health, _changed: Changed<Health>) {}

#[test]
fn a_changed_filter_skips_every_chunk_whose_column_tick_is_old() {
    capture();
    let executor = executor();
    let mut world = populated(rows_of(CHUNK_ROWS));
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("simulation", &[]).add(&WOUNDED);
    let mut schedule = builder.build().unwrap();
    let chunks = ROWS / u64::from(CHUNK_ROWS);

    // Every row was spawned since the system last ran; then nothing changed.
    world = tick(&mut schedule, world, &executor);
    world = tick(&mut schedule, world, &executor);
    // Three rows in two chunks: the first and the third.
    for id in [1, 2, 20] {
        world.insert(net(id), Health(50)).unwrap();
    }
    world = tick(&mut schedule, world, &executor);
    drop(world);

    // [units, chunks skipped, rows visited, invocations]
    assert_eq!(
        runs("units::wounded"),
        [[chunks, 0, ROWS, ROWS], [0, chunks, 0, 0], [2, chunks - 2, 2 * u64::from(CHUNK_ROWS), 3],]
    );
}

/// What `units::wounded_rows` saw of each row it ran on: the key, the region and the mass.
type Seen = Mutex<Vec<(StableKey, RegionId, Scalar)>>;

#[system(name = "units::wounded_rows", worlds = [server])]
fn wounded_rows(
    mass: &Mass,
    _changed: Changed<Health>,
    key: StableKeyOf,
    ctx: Ctx,
    seen: Res<Seen>,
) {
    seen.lock().expect("the rows").push((*key, ctx.region(), mass.0));
}

/// The rows `units::wounded_rows` ran on in the third tick, after rows 2 and 6 of the first
/// chunk and row 3 of the third changed, sorted; and the units of that run. Even keys live in
/// `OTHER_REGION`, so a chunk holds rows of both regions.
fn changed_rows_after_three_ticks(
    layout: LayoutConfig,
) -> (Vec<(StableKey, RegionId, Scalar)>, u64) {
    let executor = executor();
    let mut world = world(WorldKind::Server, layout);
    world
        .create_region(RegionConfig { id: OTHER_REGION, seed: 7, mode: RegionMode::Simulated })
        .unwrap();
    for id in 1..=ROWS {
        let region = if id % 2 == 0 { OTHER_REGION } else { REGION };
        world.spawn(net(id), region, (Health(100), Mass(Scalar::from_i32(id as i32)))).unwrap();
    }
    world.insert_service::<Seen>(Box::new(Mutex::new(Vec::new())));
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("simulation", &[]).add(&WOUNDED_ROWS);
    let mut schedule = builder.build().unwrap();
    world = tick(&mut schedule, world, &executor);
    world = tick(&mut schedule, world, &executor);
    world.service_mut::<Seen>().unwrap().get_mut().unwrap().clear();
    for id in [3, 7, 20] {
        world.insert(net(id), Health(50)).unwrap();
    }
    world = tick(&mut schedule, world, &executor);
    let mut rows = std::mem::take(world.service_mut::<Seen>().unwrap().get_mut().unwrap());
    rows.sort_by_key(|&(key, ..)| key);
    let [.., [units, ..]] = runs("units::wounded_rows")[..] else { panic!("three runs") };
    (rows, units)
}

/// A unit after a cut reads the `Changed` selection, the keys, the regions and the read
/// columns from its own first row in the chunk, so the rows a filter passes, and what the
/// kernel sees of them, are the same however the chunk is cut. Seeds 1 to 8 each cut the
/// two written chunks into more than two units; the rows are compared, not counted, since a
/// unit that read the wrong bits could still run three rows.
#[test]
fn a_changed_filter_over_split_units_runs_exactly_the_rows_it_runs_unsplit() {
    capture();
    let (whole, units) = changed_rows_after_three_ticks(rows_of(CHUNK_ROWS));
    let row = |id: u64, region| (net(id), region, Scalar::from_i32(id as i32));
    assert_eq!(whole, [row(3, REGION), row(7, REGION), row(20, OTHER_REGION)]);
    assert_eq!(units, 2);
    for seed in 1..=8 {
        let layout = LayoutConfig { split_batches: true, seed, ..rows_of(CHUNK_ROWS) };
        let (split, units) = changed_rows_after_three_ticks(layout);
        assert_eq!(split, whole, "seed {seed}: the rows the filter passed");
        assert!(units > 2, "seed {seed}: {units} units over the two written chunks");
    }
}

#[system(name = "units::weigh", worlds = [server], reduce = ExactSum<Scalar>)]
fn weigh(mass: &Mass, key: StableKeyOf) -> Scalar {
    // The key decides nothing; it makes the kernel take a per-row parameter beside the column.
    let _ = key;
    mass.0
}

#[finalize(weigh)]
fn store_weight(sum: Scalar, total: ResMut<TotalMass>) {
    total.0 = sum;
}

#[test]
fn a_unit_is_a_chunk_or_a_random_range_of_one_and_the_sum_is_the_same() {
    capture();
    let executor = executor();
    let expected = Scalar::from_i32((ROWS * (ROWS + 1) / 2) as i32);
    let chunks = ROWS / u64::from(CHUNK_ROWS);
    let mut totals = Vec::new();
    for split_batches in [false, true] {
        let layout = LayoutConfig { split_batches, seed: 0x5EED, ..rows_of(CHUNK_ROWS) };
        let mut world = populated(layout);
        let mut builder = Schedule::builder(WorldKind::Server);
        builder.stage("simulation", &[]).add(&WEIGH);
        let mut schedule = builder.build().unwrap();
        world = tick(&mut schedule, world, &executor);
        totals.push(world.resource::<TotalMass>().unwrap().0);

        let [[units, skipped, rows, invocations]] = runs("units::weigh")[..] else {
            panic!("one run")
        };
        assert_eq!((skipped, rows, invocations), (0, ROWS, ROWS));
        if split_batches {
            // A split after a row with probability one in three: about 33 more units.
            assert!(units > chunks && units <= ROWS, "{units} units over {chunks} chunks");
        } else {
            assert_eq!(units, chunks);
        }
    }
    assert_eq!(totals, [expected, expected]);
}

#[test]
fn changed_since_skips_every_chunk_whose_column_tick_predates_the_mark() {
    capture();
    let mut world = populated(rows_of(CHUNK_ROWS));
    let chunks = ROWS / u64::from(CHUNK_ROWS);
    let mark = world.mark();
    for id in [3, 4, 41] {
        world.insert(net(id), Health(7)).unwrap();
    }
    let changed: Vec<_> =
        world.changed_since::<Health>(mark).unwrap().map(|(key, hp)| (key, hp.0)).collect();
    assert_eq!(changed, [(net(3), 7), (net(4), 7), (net(41), 7)]);
    let counts = take("component", "test::health", &["chunks_skipped", "rows_visited"]);
    assert_eq!(counts, [[chunks - 2, 2 * u64::from(CHUNK_ROWS)]]);
}
