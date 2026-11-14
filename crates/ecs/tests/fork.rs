//! A schedule run forked over workers (D137 part B): each unit of a per-entity system runs
//! exactly once whatever the steals and splits, and events and commands reach the world in
//! the order D117 fixes, by emitting system in plan order then by source, whatever order
//! the units and the systems finished in. Each case runs on one worker and on four under
//! chaos seeds with every split forced, over the production layout and a perturbed one.

#![forbid(unsafe_code)]

mod common;

use std::sync::Mutex;

use common::*;
use lactose_core::StableKey;
use lactose_ecs::{
    ChunkOrder, Commands, Ctx, EventWriter, Events, LayoutConfig, Mut, Res, Schedule,
    StableKeyOf, WorldKind, finalize, system,
};
use lactose_task::{ChaosConfig, Executor};

/// Rows per world; few under Miri, which runs every interleaving slowly.
const ROWS: u64 = if cfg!(miri) { 24 } else { 600 };
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
            record: false,
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

/// What each tick's reduction counted.
type Totals = Mutex<Vec<i128>>;

#[system(name = "test::count_rows", worlds = [server])]
fn count_rows(counter: Mut<Counter>) {
    counter.0 += 1;
}

#[system(name = "test::rows_seen", worlds = [server], after = count_rows,
    reduce = lactose_math::IntSum<u32>)]
fn rows_seen(_counter: &Counter) -> u32 {
    1
}

#[finalize(rows_seen)]
fn store_rows_seen(sum: i128, totals: Res<Totals>) {
    totals.lock().unwrap().push(sum);
}

#[test]
fn each_unit_runs_once_under_forced_splits() {
    for layout in fork_layouts() {
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
            assert_eq!(totals, vec![i128::from(ROWS); TICKS as usize], "{name}, {layout:?}");
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
