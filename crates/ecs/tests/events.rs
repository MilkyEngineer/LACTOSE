//! Events and channels: a reader ordered after the emitters sees every event in (emitting
//! system, source, sequence) order; events clear at the next input sync point; channel
//! entries reach later stages and `take_channel` with their keys resolved.

#![forbid(unsafe_code)]

mod common;

use std::sync::Mutex;

use common::*;
use lactose_core::StableKey;
use lactose_ecs::{
    Channel, Commands, Ctx, EventWriter, Events, LayoutConfig, Res, Schedule, ScheduleError,
    StableKeyOf, WorldKind, system,
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
        ticks.push(std::mem::take(&mut *world.service_mut::<Seen>().unwrap().lock().unwrap()));
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
fn push_spawned(ctx: Ctx, cmd: Commands, _counter: &Counter) {
    if ctx.tick().0 == 1 {
        let spawned = cmd.spawn((Bomb { fuse: 1 },));
        cmd.apply_impulse(spawned, Vec2::new(Scalar::ONE, Scalar::ZERO));
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
    world.allocate_net_id().unwrap();
    world.allocate_net_id().unwrap();
    world.insert_service::<Seen>(Box::new(Mutex::new(Vec::new())));
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("write", &[]).add(&PUSH_SPAWNED).add(&COUNT_IMPULSES);
    builder.stage("read", &[]).add(&READ_IMPULSES_LATER);
    let mut schedule = builder.build().unwrap();

    world = tick(&mut schedule, world, &executor);
    let seen = std::mem::take(&mut *world.service_mut::<Seen>().unwrap().lock().unwrap());
    // The writing stage saw nothing; the next stage saw the entry with its key resolved.
    assert_eq!(seen, [(net(3), 1)]);
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
