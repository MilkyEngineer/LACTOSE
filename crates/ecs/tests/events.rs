//! Events and channels: a reader ordered after the emitters sees every event in (emitting
//! system, source, sequence) order; events clear at the next input sync point; channel
//! entries reach later stages' readers and takers with their keys resolved, and the closing
//! sync point of every run clears every channel (D126), so a reaction after the physics
//! stage reaches the next tick's through a carrier row.

#![forbid(unsafe_code)]

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use lactose_core::{EntityRef, StableKey};
use lactose_ecs::{
    Channel, ChannelTake, ChannelWriter, ClaimedBy, Commands, Ctx, Event, EventWriter, Events,
    LayoutConfig, Mut, Res, Schedule, ScheduleError, StableKeyOf, World, WorldKind, finalize,
    system,
};
use lactose_math::{Scalar, Vec2};
use lactose_reflect::component;

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
fn channel_entries_reach_later_stages_with_keys_resolved_and_do_not_outlive_the_tick() {
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
    assert!(take_seen(&mut world).is_empty(), "tick 2 wrote nothing and tick 1's entry is gone");
}

#[system(name = "test::read_impulses_later", worlds = [server])]
fn read_impulses_later(impulses: Channel<Physics>, seen: Res<Seen>) {
    let mut seen = seen.lock().unwrap();
    for impulse in impulses.iter() {
        seen.push((impulse.entry.body, impulse.source.map_or(0, |_| 1)));
    }
}

/// Writes one entry per row at tick 1 alone.
#[system(name = "test::push_at_one", worlds = [server])]
fn push_at_one(ctx: Ctx, key: StableKeyOf, physics: ChannelWriter<Physics>, _counter: &Counter) {
    if ctx.tick().0 == 1 {
        physics.apply_impulse(*key, Vec2::ZERO);
    }
}

/// A reader at or before the writer's stage: whatever it sees outlived a run.
#[system(name = "test::observe_early", worlds = [server])]
fn observe_early(impulses: Channel<Physics>, seen: Res<Seen>) {
    let mut seen = seen.lock().unwrap();
    for impulse in impulses.iter() {
        seen.push((impulse.entry.body, 10));
    }
}

fn take_seen(world: &mut World) -> Vec<(StableKey, i32)> {
    std::mem::take(&mut *world.service_mut::<Seen>().unwrap().get_mut().unwrap())
}

#[test]
fn the_closing_sync_point_of_every_run_clears_every_channel() {
    let executor = executor();
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    world.spawn(net(5), REGION, (Counter(0),)).unwrap();
    world.insert_service::<Seen>(Box::new(Mutex::new(Vec::new())));
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("early", &[]).add(&OBSERVE_EARLY);
    builder.stage("write", &[]).add(&PUSH_AT_ONE).add(&COUNT_IMPULSES);
    builder.stage("read", &[]).add(&READ_IMPULSES_LATER);
    let mut schedule = builder.build().unwrap();
    // A pass's first sync point clears nothing, so an entry that outlived the tick would
    // reach this observer.
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("observe", &[]).add(&OBSERVE_EARLY);
    let mut pass = builder.build().unwrap();

    // The reader after the writer's stage sees the entry; the writer's stage and the one
    // before it do not.
    world = tick(&mut schedule, world, &executor);
    assert_eq!(take_seen(&mut world), [(net(5), 1)]);

    // Between the runs: a snapshot point.
    let snapshot = world.snapshot();
    world = executor.block_on(pass.run_pass(world, &executor)).unwrap();
    assert!(take_seen(&mut world).is_empty(), "the tick's closing sync point cleared it");
    world = tick(&mut schedule, world, &executor);
    assert!(take_seen(&mut world).is_empty(), "nothing reaches the next tick");

    // The snapshot carries no channel: the restored world's next runs see nothing either.
    world.restore(&snapshot).unwrap();
    assert_eq!(world.tick().0, 1);
    world = executor.block_on(pass.run_pass(world, &executor)).unwrap();
    world = tick(&mut schedule, world, &executor);
    assert!(take_seen(&mut world).is_empty(), "not in the snapshot");
}

/// A reaction carried by a row: the kind and the other body's key, persistent and
/// replicated, so a snapshot, save and replication carry it as any other column.
#[component(key = "test::carrier", persistent, replicated)]
#[derive(PartialEq, Debug)]
struct Carrier {
    #[field(id = 1)]
    kind: u32,
    #[field(id = 2)]
    other: EntityRef,
}

/// What the relay hands the other body.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Push {
    target: StableKey,
    strength: u32,
}

impl Event for Push {
    fn target(&self) -> StableKey {
        self.target
    }
}

/// The post-stage reaction: arms the carrier through `Commands::set` on its own row, since
/// a carrier holding a key is not written through `Mut`.
#[system(name = "test::react", worlds = [server, prediction])]
fn react(key: StableKeyOf, bomb: Mut<Bomb>, cmd: Commands) {
    if bomb.fuse == 180 {
        bomb.fuse = 0;
        cmd.set(*key, Carrier { kind: 7, other: EntityRef::new(net(2)) });
    }
}

/// Next tick, before the physics stage: emits to the kept key and disarms the carrier.
#[system(name = "test::relay", worlds = [server, prediction])]
fn relay(key: StableKeyOf, carrier: &Carrier, push: EventWriter<Push>, cmd: Commands) {
    if let Some(other) = carrier.other.get() {
        push.emit(Push { target: other, strength: carrier.kind });
        cmd.set(*key, Carrier { kind: 0, other: EntityRef::NONE });
    }
}

/// The other body's kernel records the command on itself.
#[system(name = "test::receive", worlds = [server, prediction], after = relay)]
fn receive(
    key: StableKeyOf,
    pos: &Position,
    pushes: Events<Push>,
    physics: ChannelWriter<Physics>,
) {
    for push in pushes.for_target(*key) {
        let strength = Scalar::from_i32(i32::try_from(push.strength).unwrap());
        physics.apply_impulse(*key, Vec2::new(pos.x + strength, pos.y));
    }
}

/// The carrier's row and the other body, both claimed so the prediction world runs their
/// kernels.
fn carrier_world(kind: WorldKind) -> World {
    let mut registry = base_registry();
    registry.register::<Carrier>().unwrap();
    let mut world = world_with(kind, LayoutConfig::default(), Arc::new(registry));
    let disarmed = Carrier { kind: 0, other: EntityRef::NONE };
    world.spawn(net(1), REGION, (Bomb { fuse: 180 }, disarmed)).unwrap();
    world.spawn(net(2), REGION, (Position { x: Scalar::ONE, y: Scalar::ZERO },)).unwrap();
    for key in [net(1), net(2)] {
        world.insert(key, ClaimedBy { player: PLAYER }).unwrap();
    }
    world
}

#[test]
fn a_keyed_carrier_set_after_the_step_reaches_the_next_ticks_physics_stage() {
    let executor = executor();
    for kind in [WorldKind::Server, WorldKind::Prediction] {
        let mut world = carrier_world(kind);
        let mut builder = Schedule::builder(kind);
        builder.stage("simulation", &[]).add(&RELAY).add(&RECEIVE);
        builder.stage("physics", &[]).add(&PHYSICS);
        builder.stage("post", &[]).add(&REACT);
        let mut schedule = builder.build().unwrap();
        let mut seen = Vec::new();
        for _ in 0..3 {
            world = tick(&mut schedule, world, &executor);
            let carrier = *world.get::<Carrier>(net(1)).unwrap();
            let count = world.resource::<ImpulseCount>().unwrap().0;
            seen.push((count, carrier.kind, carrier.other.get()));
        }
        // Tick 1's post stage arms the carrier; tick 2's simulation stage relays it into
        // that tick's channel, which tick 2's physics stage reads, and disarms it.
        assert_eq!(seen, [(0, 7, Some(net(2))), (1, 0, None), (1, 0, None)], "{kind:?}");
    }
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

/// A reduction whose finalize writes the channel: one entry naming the row count.
#[system(name = "test::count_rows", worlds = [server], reduce = lactose_math::IntSum<u32>)]
fn count_rows(_counter: &Counter) -> u32 {
    1
}

#[finalize(count_rows)]
fn push_count(sum: i128, physics: ChannelWriter<Physics>) {
    physics.apply_impulse(net(500 + u64::try_from(sum).unwrap()), Vec2::ZERO);
}

#[system(name = "test::push_exclusive", worlds = [server], exclusive)]
fn push_exclusive(world: &mut World, physics: ChannelWriter<Physics>) {
    let _ = world.tick();
    physics.apply_impulse(net(600), Vec2::ZERO);
}

#[system(name = "test::push_once", worlds = [server])]
fn push_once(physics: ChannelWriter<Physics>) {
    physics.apply_impulse(net(700), Vec2::ZERO);
}

#[test]
fn a_finalize_an_exclusive_and_a_once_per_tick_writer_reach_a_later_stage() {
    let executor = executor();
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    for id in [5, 6, 7] {
        world.spawn(net(id), REGION, (Counter(0),)).unwrap();
    }
    world.insert_service::<Seen>(Box::new(Mutex::new(Vec::new())));
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("write", &[]).add(&COUNT_ROWS).add(&PUSH_EXCLUSIVE).add(&PUSH_ONCE);
    builder.stage("read", &[]).add(&READ_IMPULSES_LATER);
    let mut schedule = builder.build().unwrap();
    // Each tick's reader sees each writer's entry once, with no source: none of the three
    // runs for an entity.
    for _ in 0..3 {
        world = tick(&mut schedule, world, &executor);
        assert_eq!(take_seen(&mut world), [(net(600), 0), (net(503), 0), (net(700), 0)]);
    }
}
