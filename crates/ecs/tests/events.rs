//! Events and channels: a reader ordered after the emitters sees every event in (emitting
//! system, source, sequence) order; events clear at the next input sync point; channel
//! entries reach later stages' readers and takers with their keys resolved.

#![forbid(unsafe_code)]

mod common;

use std::sync::Mutex;

use common::*;
use lactose_core::StableKey;
use lactose_ecs::{
    Channel, ChannelTake, ChannelWriter, CommandChannel, Commands, Ctx, EventWriter, Events,
    LayoutConfig, Res, Schedule, ScheduleError, StableKeyOf, World, WorldKind, system,
};
use lactose_math::{Scalar, Vec2};

/// What the readers saw, in order. A service resource, so the test can read it back.
type Seen = Mutex<Vec<(StableKey, i32)>>;

const TARGET: u64 = 1;

fn emit(damage: &mut EventWriter<Damage>, key: StableKey, base: i32) {
    for seq in 0..2 {
        damage.emit(Damage { target: net(TARGET), source: key, amount: base + seq });
    }
}

#[system(name = "test::emit_a", worlds = [server])]
fn emit_a(ctx: Ctx, damage: EventWriter<Damage>, key: StableKeyOf, _counter: &Counter) {
    if ctx.tick().0 == 1 {
        emit(&mut damage, *key, 0);
    }
}

#[system(name = "test::emit_b", worlds = [server], after = emit_a)]
fn emit_b(ctx: Ctx, damage: EventWriter<Damage>, key: StableKeyOf, _counter: &Counter) {
    if ctx.tick().0 == 1 {
        emit(&mut damage, *key, 100);
    }
}

#[system(name = "test::read_damage", worlds = [server], after = [emit_a, emit_b])]
fn read_damage(events: Events<Damage>, seen: Res<Seen>) {
    let mut seen = seen.lock().unwrap();
    for event in events.iter() {
        seen.push((event.source, event.amount));
    }
}

/// The same reader in a later stage, which needs no edge.
#[system(name = "test::read_damage_later", worlds = [server])]
fn read_damage_later(events: Events<Damage>, seen: Res<Seen>) {
    let mut seen = seen.lock().unwrap();
    for event in events.for_target(net(TARGET)) {
        seen.push((event.source, event.amount));
    }
}

fn run_events(layout: LayoutConfig) -> Vec<Vec<(StableKey, i32)>> {
    let executor = executor();
    let mut world = world(WorldKind::Server, layout);
    for id in [9, 4, 7, 2, 8] {
        world.spawn(net(id), REGION, (Counter(0),)).unwrap();
    }
    world.insert_service::<Seen>(Box::new(Mutex::new(Vec::new())));
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("emit", &[]).add(&EMIT_B).add(&READ_DAMAGE).add(&EMIT_A);
    builder.stage("later", &[]).add(&READ_DAMAGE_LATER);
    let mut schedule = builder.build().unwrap();
    let mut ticks = Vec::new();
    for _ in 0..2 {
        world = tick(&mut schedule, world, &executor);
        ticks.push(std::mem::take(&mut *world.service_mut::<Seen>().unwrap().get_mut().unwrap()));
    }
    ticks
}

#[test]
fn readers_see_every_event_in_emitter_then_source_then_sequence_order() {
    let ticks = run_events(LayoutConfig::default());
    let sources = [2, 4, 7, 8, 9];
    let mut expected = Vec::new();
    for base in [0, 100] {
        for source in sources {
            for seq in 0..2 {
                expected.push((net(source), base + seq));
            }
        }
    }
    let both_readers = [expected.clone(), expected].concat();
    assert_eq!(ticks[0], both_readers);
    assert!(ticks[1].is_empty(), "events clear at the next input sync point");
    for layout in layouts() {
        assert_eq!(run_events(layout), ticks, "{layout:?}");
    }
}

#[test]
fn a_reader_not_ordered_after_an_emitter_is_rejected() {
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("emit", &[]).add(&EMIT_A).add(&READ_DAMAGE_LATER);
    let error = builder.build().map(|_| ()).unwrap_err();
    assert_eq!(
        error,
        ScheduleError::EventsBeforeEmitter {
            reader: "test::read_damage_later",
            emitter: "test::emit_a",
            event: core::any::type_name::<Damage>(),
        }
    );
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("read", &[]).add(&READ_DAMAGE_LATER);
    builder.stage("emit", &[]).add(&EMIT_A);
    assert!(matches!(builder.build(), Err(ScheduleError::EventsBeforeEmitter { .. })));
}

/// Writes a physics command naming an entity spawned in the same invocation.
#[system(name = "test::push_spawned", worlds = [server])]
fn push_spawned(ctx: Ctx, cmd: Commands, physics: ChannelWriter<Physics>, _counter: &Counter) {
    if ctx.tick().0 == 1 {
        let spawned = cmd.spawn((Bomb { fuse: 1 },));
        physics.apply_impulse(spawned, Vec2::new(Scalar::ONE, Scalar::ZERO));
    }
}

/// Counts what earlier stages wrote to the channel.
#[system(name = "test::count_impulses", worlds = [server])]
fn count_impulses(impulses: Channel<Physics>, seen: Res<Seen>) {
    let mut seen = seen.lock().unwrap();
    for impulse in impulses.iter() {
        seen.push((impulse.entry.body, impulse.source.map_or(0, |_| 1)));
    }
}

#[test]
fn channel_entries_reach_later_stages_with_keys_resolved_and_clear_at_the_input_sync() {
    let executor = executor();
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    world.spawn(net(5), REGION, (Counter(0),)).unwrap();
    world.insert_service::<Seen>(Box::new(Mutex::new(Vec::new())));
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("write", &[]).add(&PUSH_SPAWNED).add(&COUNT_IMPULSES);
    builder.stage("read", &[]).add(&READ_IMPULSES_LATER);
    let mut schedule = builder.build().unwrap();

    world = tick(&mut schedule, world, &executor);
    let seen = std::mem::take(&mut *world.service_mut::<Seen>().unwrap().get_mut().unwrap());
    // The writing stage saw nothing; the next stage saw the entry with its key resolved.
    assert_eq!(seen, [(net(1), 1)]);
    world = tick(&mut schedule, world, &executor);
    assert!(world.take_channel::<Physics>().is_empty(), "the input sync point cleared it");
}

#[system(name = "test::read_impulses_later", worlds = [server])]
fn read_impulses_later(impulses: Channel<Physics>, seen: Res<Seen>) {
    let mut seen = seen.lock().unwrap();
    for impulse in impulses.iter() {
        seen.push((impulse.entry.body, impulse.source.map_or(0, |_| 1)));
    }
}

/// A channel that is not cleared at the input sync point: `lactose-physics-api`'s, whose
/// consumer takes it at the physics sync point, so a command recorded after the step
/// applies next tick.
pub struct Deferred;

impl CommandChannel for Deferred {
    type Entry = Impulse;

    const CLEAR_AT_INPUT: bool = false;

    fn remap(entry: &mut Impulse, f: &mut dyn FnMut(&mut StableKey)) {
        f(&mut entry.body);
    }
}

/// Writes one entry to each channel in the last stage of tick 1.
#[system(name = "test::push_late", worlds = [server])]
fn push_late(ctx: Ctx, cmd: Commands, _counter: &Counter) {
    if ctx.tick().0 == 1 {
        cmd.channel::<Deferred>().push(Impulse { body: net(5), impulse: Vec2::ZERO });
        cmd.channel::<Physics>().apply_impulse(net(5), Vec2::ZERO);
    }
}

/// Reads both channels in the first stage: 1 for the deferred one, 2 for the clearing one.
#[system(name = "test::read_both_early", worlds = [server])]
fn read_both_early(deferred: Channel<Deferred>, cleared: Channel<Physics>, seen: Res<Seen>) {
    let mut seen = seen.lock().unwrap();
    for entry in deferred.iter() {
        seen.push((entry.entry.body, 1));
    }
    for entry in cleared.iter() {
        seen.push((entry.entry.body, 2));
    }
}

fn late_writer_schedule() -> Schedule {
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("read", &[]).add(&READ_BOTH_EARLY);
    builder.stage("write", &[]).add(&PUSH_LATE);
    builder.build().unwrap()
}

#[test]
fn a_channel_that_opts_out_of_clearing_keeps_its_entries_until_taken() {
    let executor = executor();
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    world.spawn(net(5), REGION, (Counter(0),)).unwrap();
    world.insert_service::<Seen>(Box::new(Mutex::new(Vec::new())));
    let mut schedule = late_writer_schedule();

    // Tick 1 writes in the last stage, after the reader ran.
    world = tick(&mut schedule, world, &executor);
    assert!(world.service_mut::<Seen>().unwrap().get_mut().unwrap().is_empty());
    // Between ticks both channels hold the entry, readable without taking.
    assert_eq!(world.channel::<Deferred>().len(), 1);
    assert_eq!(world.channel::<Deferred>()[0].entry.body, net(5));
    assert_eq!(world.channel::<Physics>().len(), 1);

    // Tick 2's input sync point clears the clearing channel alone.
    world = tick(&mut schedule, world, &executor);
    let seen = std::mem::take(&mut *world.service_mut::<Seen>().unwrap().get_mut().unwrap());
    assert_eq!(seen, [(net(5), 1)]);
    assert_eq!(world.channel::<Deferred>().len(), 1, "nothing took it");
    assert!(world.channel::<Physics>().is_empty());
}

#[test]
fn a_taken_deferred_channel_is_empty_and_a_restore_drops_it() {
    let executor = executor();
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    world.spawn(net(5), REGION, (Counter(0),)).unwrap();
    world.insert_service::<Seen>(Box::new(Mutex::new(Vec::new())));
    let mut schedule = late_writer_schedule();
    world = tick(&mut schedule, world, &executor);

    // Not in the snapshot: a restore returns to the point and drops what the channel held.
    let snapshot = world.snapshot();
    let taken = world.take_channel::<Deferred>();
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].entry.body, net(5));
    assert!(world.channel::<Deferred>().is_empty());
    world = tick(&mut schedule, world, &executor);
    assert!(world.service_mut::<Seen>().unwrap().get_mut().unwrap().is_empty(), "taken");

    world.restore(&snapshot).unwrap();
    assert!(world.channel::<Deferred>().is_empty(), "a restore drops every channel");
}

/// A kernel whose only output is a channel entry: no command, event or presentation event.
#[system(name = "test::push_only", worlds = [server])]
fn push_only(key: StableKeyOf, physics: ChannelWriter<Physics>, _counter: &Counter) {
    physics.apply_impulse(*key, Vec2::ZERO);
}

/// A per-region kernel whose only output is a channel entry.
#[system(name = "test::push_per_region", worlds = [server])]
fn push_per_region(ctx: Ctx, physics: ChannelWriter<Physics>) {
    physics.apply_impulse(net(ctx.region().0 + 100), Vec2::ZERO);
}

#[test]
fn a_kernel_whose_only_output_is_a_channel_entry_keeps_it() {
    let executor = executor();
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    for id in [6, 5] {
        world.spawn(net(id), REGION, (Counter(0),)).unwrap();
    }
    world.insert_service::<Seen>(Box::new(Mutex::new(Vec::new())));
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("write", &[]).add(&PUSH_ONLY).add(&PUSH_PER_REGION);
    builder.stage("read", &[]).add(&READ_IMPULSES_LATER);
    let mut schedule = builder.build().unwrap();
    world = tick(&mut schedule, world, &executor);
    let mut seen = std::mem::take(&mut *world.service_mut::<Seen>().unwrap().get_mut().unwrap());
    seen.sort_by_key(|&(key, from_entity)| (from_entity, key));
    let region = net(REGION.0 + 100);
    assert_eq!(seen, [(region, 0), (net(5), 1), (net(6), 1)]);
}

/// Spawns, then writes through two writers: the second's entry first in the body, yet after
/// the first's in command order, which is parameter order.
#[system(name = "test::push_two_writers", worlds = [server])]
fn push_two_writers(
    ctx: Ctx,
    cmd: Commands,
    first: ChannelWriter<Physics>,
    second: ChannelWriter<Physics>,
    _counter: &Counter,
) {
    let tick = ctx.tick().0;
    if tick <= 2 {
        second.apply_impulse(net(40 + u64::from(tick)), Vec2::ZERO);
        let spawned = cmd.spawn((Bomb { fuse: tick },));
        first.apply_impulse(spawned, Vec2::ZERO);
    }
}

/// What the taker took each tick, by entry.
type Taken = Mutex<Vec<Vec<(StableKey, i32)>>>;

#[system(name = "test::take_impulses", worlds = [server], exclusive)]
fn take_impulses(world: &mut World, taken: ChannelTake<Physics>, log: Res<Taken>) {
    let _ = world.tick();
    assert_eq!(taken.len(), taken.iter().count());
    assert_eq!(taken.is_empty(), taken.iter().next().is_none());
    let entries = taken
        .into_entries()
        .into_iter()
        .map(|entry| (entry.entry.body, i32::from(entry.source.is_some())));
    log.lock().unwrap().push(entries.collect());
}

#[test]
fn a_take_yields_the_earlier_stages_entries_with_keys_resolved_once() {
    let executor = executor();
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    world.spawn(net(5), REGION, (Counter(0),)).unwrap();
    world.insert_service::<Taken>(Box::new(Mutex::new(Vec::new())));
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("simulation", &[]).add(&PUSH_TWO_WRITERS);
    builder.stage("physics", &[]).add(&TAKE_IMPULSES);
    let mut schedule = builder.build().unwrap();
    for _ in 0..3 {
        world = tick(&mut schedule, world, &executor);
    }
    let taken = std::mem::take(&mut *world.service_mut::<Taken>().unwrap().get_mut().unwrap());
    // The spawns resolve to the server's runtime keys, 1 and 2; each tick's take holds that
    // tick's entries alone, and the third tick wrote none.
    assert_eq!(
        taken,
        [vec![(net(1), 1), (net(41), 1)], vec![(net(2), 1), (net(42), 1)], Vec::new()]
    );
}
