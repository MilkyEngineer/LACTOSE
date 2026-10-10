//! Passes between ticks (extract, the replication gather, the save copy): an exclusive
//! system that forks over the world's columns through `Job`, a tick that does not advance,
//! nothing cleared, and `Changed` in a pass seeing what changed since that pass last ran.

#![forbid(unsafe_code)]

mod common;

use std::sync::Mutex;

use common::*;
use lactose_core::StableKey;
use lactose_ecs::{
    Changed, EventWriter, Events, Job, LayoutConfig, Res, Schedule, StableKeyOf, World, WorldKind,
    system,
};

/// What a pass gathered: per row, its key and its x position.
type Gathered = Mutex<Vec<(StableKey, i64)>>;

/// Extract in miniature: a fork-join over every chunk of `Position`, keyed by row key.
#[system(name = "test::extract", worlds = [server], exclusive)]
fn extract(world: &mut World, job: Job, gathered: Res<Gathered>) {
    let world: &World = world;
    let gathered: &Gathered = &gathered;
    job.context().scope(|scope| {
        for column in world.columns::<Position>() {
            scope.fork(move |_| {
                let rows = column.keys().iter().zip(column.values());
                let mut out = gathered.lock().unwrap();
                out.extend(rows.map(|(&key, value)| (key, value.x.to_i64())));
            });
        }
    });
}

/// A per-entity system in a pass: sees the changes since the pass last ran.
#[system(name = "test::gather_changed", worlds = [server])]
fn gather_changed(
    _changed: Changed<Position>,
    position: &Position,
    key: StableKeyOf,
    gathered: Res<Gathered>,
) {
    gathered.lock().unwrap().push((*key, -position.x.to_i64()));
}

#[system(name = "test::shout", worlds = [server])]
fn shout(damage: EventWriter<Damage>, key: StableKeyOf, _counter: &Counter) {
    damage.emit(Damage { target: *key, source: *key, amount: 1 });
}

/// Reads the last tick's events in a pass: nothing was cleared.
#[system(name = "test::count_events", worlds = [server])]
fn count_events(events: Events<Damage>, gathered: Res<Gathered>) {
    gathered.lock().unwrap().push((net(1000), events.len() as i64));
}

fn gathered(world: &mut World) -> Vec<(StableKey, i64)> {
    let mut gathered = std::mem::take(world.service_mut::<Gathered>().unwrap().get_mut().unwrap());
    gathered.sort();
    gathered
}

#[test]
fn a_pass_forks_over_columns_and_advances_nothing() {
    let executor = executor();
    let mut world =
        world(WorldKind::Server, LayoutConfig { chunk_rows: Some(2), ..LayoutConfig::default() });
    for id in 1..=5 {
        world.spawn(net(id), REGION, (pos(id as i32, 0), Counter(0))).unwrap();
    }
    world.insert_service::<Gathered>(Box::new(Mutex::new(Vec::new())));
    let mut tick_schedule = Schedule::builder(WorldKind::Server);
    tick_schedule.stage("shout", &[]).add(&SHOUT);
    let mut tick_schedule = tick_schedule.build().unwrap();
    let mut pass = Schedule::builder(WorldKind::Server);
    pass.stage("extract", &[]).add(&EXTRACT);
    pass.stage("gather", &[]).add(&GATHER_CHANGED).add(&COUNT_EVENTS);
    let mut pass = pass.build().unwrap();

    world = tick(&mut tick_schedule, world, &executor);
    world = executor.block_on(pass.run_pass(world, &executor)).unwrap();
    assert_eq!(world.tick(), t(1), "a pass does not advance the tick");
    let all: Vec<(StableKey, i64)> = (1..=5).map(|id| (net(id), id as i64)).collect();
    let changed: Vec<(StableKey, i64)> = (1..=5).map(|id| (net(id), -(id as i64))).collect();
    let mut expected = [all.clone(), changed, vec![(net(1000), 5)]].concat();
    expected.sort();
    assert_eq!(gathered(&mut world), expected, "every row once, every change, every event");

    // Only what changed since the last pass is seen by the next one.
    world.get_mut::<Position>(net(3)).unwrap().x = lactose_math::Scalar::from_i32(30);
    world = executor.block_on(pass.run_pass(world, &executor)).unwrap();
    let mut expected = all.clone();
    expected[2].1 = 30;
    expected.extend([(net(1000), 5), (net(3), -30)]);
    expected.sort();
    assert_eq!(gathered(&mut world), expected);

    // The pass's own state is per schedule and the next tick clears the events.
    world = tick(&mut tick_schedule, world, &executor);
    world = executor.block_on(pass.run_pass(world, &executor)).unwrap();
    let mut expected = all;
    expected[2].1 = 30;
    expected.push((net(1000), 5));
    expected.sort();
    assert_eq!(gathered(&mut world), expected);
}

#[system(name = "test::beep", worlds = [server])]
fn beep(present: lactose_ecs::PresentEvents, key: StableKeyOf, _counter: &Counter) {
    present.emit(*key, AudioRequest { sound: 7 });
}

/// A pass is not a re-simulation: it cancels nothing the tick delivered, and what it emits
/// continues the tick's sequence.
#[test]
fn a_pass_neither_cancels_nor_repeats_the_ticks_presentation_events() {
    let executor = executor();
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    world.spawn(net(1), REGION, (Counter(0),)).unwrap();
    let mut ticks = Schedule::builder(WorldKind::Server);
    ticks.stage("beep", &[]).add(&BEEP);
    let mut ticks = ticks.build().unwrap();
    let mut quiet = Schedule::builder(WorldKind::Server);
    quiet.stage("nothing", &[]);
    let mut quiet = quiet.build().unwrap();

    world = tick(&mut ticks, world, &executor);
    let first = world.take_present_events();
    assert_eq!(first.emitted.len(), 1);
    world = executor.block_on(quiet.run_pass(world, &executor)).unwrap();
    assert!(world.take_present_events().cancelled.is_empty());
    world = executor.block_on(ticks.run_pass(world, &executor)).unwrap();
    let pass = world.take_present_events();
    assert_eq!(pass.emitted.len(), 1);
    assert_eq!((pass.emitted[0].key.tick, pass.emitted[0].key.seq), (t(1), 1 << 31));
    assert!(pass.cancelled.is_empty());
}

/// Records whether the world still held the resource the system took.
#[system(name = "test::holds_gravity", worlds = [server], exclusive)]
fn holds_gravity(world: &mut World, gravity: Res<Gravity>, seen: Res<Gathered>) {
    let held = world.resource::<Gravity>().is_some();
    seen.lock().unwrap().push((net(1000), i64::from(held) * 10 + gravity.0.to_i64()));
}

#[test]
fn an_exclusive_systems_resources_leave_the_world_for_its_call() {
    let executor = executor();
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    world.insert_service::<Gathered>(Box::new(Mutex::new(Vec::new())));
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("only", &[]).add(&HOLDS_GRAVITY);
    let mut schedule = builder.build().unwrap();
    world = tick(&mut schedule, world, &executor);
    assert_eq!(gathered(&mut world), [(net(1000), 2)], "not held inside; gravity is two");
    assert!(world.resource::<Gravity>().is_some(), "and back afterwards");
}
