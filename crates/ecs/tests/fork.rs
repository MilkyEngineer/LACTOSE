//! A schedule run forked over workers (D137 part B): each unit of a per-entity system runs
//! exactly once whatever the steals and splits, and events and commands reach the world in
//! the order D117 fixes, by emitting system in plan order then by source, whatever order
//! the units and the systems finished in. Each case runs on one worker and on four under
//! chaos seeds with every split forced, over the production layout and a perturbed one.

#![forbid(unsafe_code)]

use crate::common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use common::*;
use lactose_core::StableKey;
use lactose_ecs::{
    ChunkOrder, Commands, Ctx, EventWriter, Events, LayoutConfig, Mut, Res, Schedule, StableKeyOf,
    WorldKind, finalize, system,
};
use lactose_task::{ChaosConfig, Executor};

/// Rows per world; few under Miri, which runs every interleaving slowly. Off Miri, more
/// than two chunks of the production layout, so its nodes have units to split.
const ROWS: u64 = if cfg!(miri) { 24 } else { 3_000 };
const TICKS: u32 = 3;
const TARGET: u64 = 1;

/// One worker, then four under each chaos seed.
fn executors() -> Vec<(String, Executor)> {
    let seeds: &[u64] = if cfg!(miri) { &[1] } else { &[1, 2, 3] };
    let one = Executor::builder().frame_workers(1).background_workers(1).build().unwrap();
    let mut executors = vec![("one worker".to_owned(), one)];
    for &seed in seeds {
        let chaos = ChaosConfig {
            seed,
            shuffle_steals: true,
            inject_yields: true,
            force_split: true,
            record: true,
        };
        let four = Executor::builder()
            .frame_workers(4)
            .background_workers(1)
            .chaos(chaos)
            .build()
            .unwrap();
        executors.push((format!("four workers, chaos seed {seed}"), four));
    }
    executors
}

/// The production layout, and seven-row chunks shuffled and split into random batches.
fn fork_layouts() -> [LayoutConfig; 2] {
    let split = LayoutConfig {
        chunk_rows: Some(7),
        seed: 0x5eed,
        permute_rows: true,
        chunk_order: ChunkOrder::Shuffled,
        split_batches: true,
    };
    [LayoutConfig::default(), split]
}

// ---- Exactly once ----

/// What each tick's reduction summed.
type Totals = Mutex<Vec<i128>>;

#[system(name = "test::count_rows", worlds = [server])]
fn count_rows(counter: Mut<Counter>) {
    counter.0 += 1;
}

/// Sums the counters after this tick's count: a row read before its count, or counted
/// twice, changes the sum.
#[system(name = "test::rows_seen", worlds = [server], after = count_rows,
    reduce = lactose_math::IntSum<u32>)]
fn rows_seen(counter: &Counter) -> u32 {
    u32::try_from(counter.0).unwrap()
}

#[finalize(rows_seen)]
fn store_rows_seen(sum: i128, totals: Res<Totals>) {
    totals.lock().unwrap().push(sum);
}

/// How many nodes of a recorded log had their units run on more than one worker. The log
/// is opaque to this crate; this reads the executor's version 1 encoding: a 16-byte header
/// (magic, version, workers), then 28-byte records of worker, kind (0 for units), graph run
/// ordinal, node index and unit range.
fn nodes_on_several_workers(log: &[u8]) -> usize {
    assert_eq!(&log[..12], b"WHISKLOG\x01\0\0\0", "the log encoding changed");
    let mut workers: BTreeMap<(u64, u32), BTreeSet<u32>> = BTreeMap::new();
    for record in log[16..].chunks(28) {
        let u32_at = |at: usize| u32::from_le_bytes(record[at..at + 4].try_into().unwrap());
        if u32_at(4) == 0 {
            let run = u64::from_le_bytes(record[8..16].try_into().unwrap());
            workers.entry((run, u32_at(16))).or_default().insert(u32_at(0));
        }
    }
    workers.values().filter(|set| set.len() > 1).count()
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "test code outside the step: the log shows the run was split, never what it computed"
)]
fn each_unit_runs_once_under_forced_splits() {
    for layout in fork_layouts() {
        let mut split_nodes = 0;
        for (name, executor) in executors() {
            let mut world = world(WorldKind::Server, layout);
            for id in 1..=ROWS {
                world.spawn(net(id), REGION, (Counter(0),)).unwrap();
            }
            world.insert_service::<Totals>(Box::new(Mutex::new(Vec::new())));
            let mut builder = Schedule::builder(WorldKind::Server);
            builder.stage("count", &[]).add(&ROWS_SEEN).add(&COUNT_ROWS);
            let mut schedule = builder.build().unwrap();
            for _ in 0..TICKS {
                world = tick(&mut schedule, world, &executor);
            }
            for id in 1..=ROWS {
                let counter = world.get::<Counter>(net(id));
                assert_eq!(counter, Some(&Counter(TICKS.into())), "row {id}, {name}, {layout:?}");
            }
            let totals = world.service::<Totals>().unwrap().lock().unwrap().clone();
            let expected: Vec<i128> =
                (1..=TICKS).map(|t| i128::from(ROWS * u64::from(t))).collect();
            assert_eq!(totals, expected, "{name}, {layout:?}");
            if let Some(log) = executor.execution_log() {
                split_nodes += nodes_on_several_workers(log.as_bytes());
            }
        }
        // Summed over the chaos seeds, so a loaded machine that leaves one run on one
        // worker does not fail it. Under Miri the production layout holds every row in one
        // chunk, so only the split layout can split.
        if !cfg!(miri) || layout.chunk_rows.is_some() {
            assert!(split_nodes > 0, "no node's units ran on two workers: {layout:?}");
        }
    }
}

// ---- The order D117 fixes ----

/// What the reader saw, in order: each event's source and amount.
type Seen = Mutex<Vec<(StableKey, i32)>>;

fn strike(damage: &mut EventWriter<Damage>, source: StableKey, base: i32) {
    for sequence in 0..2 {
        damage.emit(Damage { target: net(TARGET), source, amount: base + sequence });
    }
}

/// Two emitters of one type with no order between them, so their nodes may finish in
/// either order; D117 hands their events on by plan position.
#[system(name = "test::strike_low", worlds = [server])]
fn strike_low(ctx: Ctx, damage: EventWriter<Damage>, key: StableKeyOf, _counter: &Counter) {
    if ctx.tick().0 == 1 {
        strike(&mut damage, *key, 0);
    }
}

#[system(name = "test::strike_high", worlds = [server])]
fn strike_high(ctx: Ctx, damage: EventWriter<Damage>, key: StableKeyOf, _counter: &Counter) {
    if ctx.tick().0 == 1 {
        strike(&mut damage, *key, 1_000);
    }
}

#[system(name = "test::strike_region", worlds = [server])]
fn strike_region(ctx: Ctx, damage: EventWriter<Damage>) {
    if ctx.tick().0 == 1 {
        strike(&mut damage, net(100_000 + ctx.region().0), 2_000);
    }
}

#[system(name = "test::read_strikes", worlds = [server],
    after = [strike_low, strike_high, strike_region])]
fn read_strikes(events: Events<Damage>, seen: Res<Seen>) {
    let mut seen = seen.lock().unwrap();
    seen.extend(events.iter().map(|event| (event.source, event.amount)));
}

/// Two spawners with no order between them: the keys the spawns get are allocated in the
/// order the commands apply, so the key each counter lands on shows that order.
#[system(name = "test::spawn_low", worlds = [server])]
fn spawn_low(ctx: Ctx, cmd: Commands, key: StableKeyOf, counter: &Counter) {
    if ctx.tick().0 == 1 && *key != net(TARGET) {
        cmd.spawn((Counter(counter.0 + 1_000_000),));
    }
}

#[system(name = "test::spawn_high", worlds = [server])]
fn spawn_high(ctx: Ctx, cmd: Commands, key: StableKeyOf, counter: &Counter) {
    if ctx.tick().0 == 1 && *key != net(TARGET) {
        cmd.spawn((Counter(counter.0 + 2_000_000),));
    }
}

/// What a run leaves: the events the reader saw, and each row's key and counter in key
/// order.
type Outcome = (Vec<(StableKey, i32)>, Vec<(StableKey, u64)>);

fn run_outputs(layout: LayoutConfig, executor: &Executor) -> Outcome {
    let mut world = world(WorldKind::Server, layout);
    world
        .create_region(lactose_ecs::RegionConfig {
            id: OTHER_REGION,
            seed: 7,
            mode: lactose_ecs::RegionMode::Simulated,
        })
        .unwrap();
    for id in 1..=ROWS {
        let region = if id % 3 == 0 { OTHER_REGION } else { REGION };
        world.spawn(net(id), region, (Counter(id),)).unwrap();
    }
    world.insert_service::<Seen>(Box::new(Mutex::new(Vec::new())));
    let mut builder = Schedule::builder(WorldKind::Server);
    builder
        .stage("strike", &[])
        .add(&READ_STRIKES)
        .add(&STRIKE_HIGH)
        .add(&STRIKE_REGION)
        .add(&STRIKE_LOW)
        .add(&SPAWN_HIGH)
        .add(&SPAWN_LOW);
    let mut schedule = builder.build().unwrap();
    for _ in 0..2 {
        world = tick(&mut schedule, world, executor);
    }
    let seen = std::mem::take(&mut *world.service_mut::<Seen>().unwrap().get_mut().unwrap());
    let mut rows: Vec<(StableKey, u64)> = [REGION, OTHER_REGION]
        .into_iter()
        .flat_map(|region| world.entities_in(region).collect::<Vec<_>>())
        .map(|key| (key, world.get::<Counter>(key).unwrap().0))
        .collect();
    rows.sort();
    (seen, rows)
}

#[test]
fn events_and_commands_reach_the_world_in_plan_then_source_order() {
    let reference = run_outputs(LayoutConfig::default(), &executors().remove(0).1);
    let (seen, rows) = &reference;
    // Each emitter's events are contiguous, in source then sequence order.
    let per_row = 2 * ROWS as usize;
    assert_eq!(seen.len(), 2 * per_row + 2 * 2, "every row of both emitters, two regions");
    let mut blocks: Vec<i32> = seen.iter().map(|(_, amount)| amount / 1_000).collect();
    blocks.dedup();
    assert_eq!(blocks.len(), 3, "one block per emitting system: {blocks:?}");
    for block in seen.chunk_by(|a, b| a.1 / 1_000 == b.1 / 1_000) {
        let mut sorted = block.to_vec();
        sorted.sort();
        assert_eq!(block, sorted, "a system's events in source, then sequence order");
    }
    assert_eq!(rows.len(), 3 * ROWS as usize - 2, "both spawners' rows landed");
    for layout in fork_layouts() {
        for (name, executor) in executors() {
            let outcome = run_outputs(layout, &executor);
            assert_eq!(outcome.0, reference.0, "events, {name}, {layout:?}");
            assert_eq!(outcome.1, reference.1, "spawned keys, {name}, {layout:?}");
        }
    }
}
