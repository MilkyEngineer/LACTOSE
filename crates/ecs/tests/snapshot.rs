//! Snapshot and restore of the prediction world (architecture section 10): ten ticks,
//! a snapshot at three, a restore and a re-run that reproduces every column, every stamp
//! and every `Changed` set; presentation events deduplicated and cancelled across the
//! rollback; stale cursors; binds that survive the restore.

#![forbid(unsafe_code)]

mod common;

use std::sync::Mutex;

use common::*;
use lactose_core::{PredictionKey, StableKey, Tick};
use lactose_ecs::{
    Changed, Ctx, LayoutConfig, PresentEventKey, PresentEvents, Res, Schedule, StableKeyOf, World,
    WorldKind, system,
};

/// The rows each tick's `Changed<Position>` reader saw.
type Seen = Mutex<Vec<(Tick, StableKey)>>;

#[system(name = "test::record_changed", worlds = [prediction], after = integrate)]
fn record_changed(ctx: Ctx, _changed: Changed<Position>, key: StableKeyOf, seen: Res<Seen>) {
    seen.lock().unwrap().push((ctx.tick(), *key));
}

/// Two presentation events on every other tick, by the animation phase.
#[system(name = "test::chirp", worlds = [prediction], after = animate)]
fn chirp(ctx: Ctx, present: PresentEvents, key: StableKeyOf, anim: &AnimState) {
    if anim.phase.is_multiple_of(2) {
        present.emit(*key, AudioRequest { sound: ctx.tick().0 });
        present.emit(*key, AudioRequest { sound: 1000 + ctx.tick().0 });
    }
}

fn schedule() -> Schedule {
    let mut builder = Schedule::builder(WorldKind::Prediction);
    builder.stage("input", &[]).add(&READ_INPUT);
    builder
        .stage("simulation", &[&sets::MOVEMENT])
        .add(&APPLY_GRAVITY)
        .add(&INTEGRATE)
        .add(&THROW_BOMB)
        .add(&ANIMATE)
        .add(&CHIRP)
        .add(&RECORD_CHANGED);
    builder.build().unwrap()
}

/// Every column of every registered component, with each row's tick, by row key.
fn columns(world: &World) -> Vec<(u64, StableKey, Vec<u8>, Option<Tick>)> {
    let mut rows = Vec::new();
    for info in world.registry().iter() {
        for column in world.columns_erased(info.key()) {
            for (row, &key) in column.keys().iter().enumerate() {
                let size = column.element_size();
                rows.push((
                    info.key().0,
                    key,
                    column.bytes()[row * size..(row + 1) * size].to_vec(),
                    column.row_ticks().map(|ticks| ticks[row]),
                ));
            }
        }
    }
    rows.sort();
    rows
}

/// One run of ticks: the `Changed` sets, and what presentation was handed after each tick.
struct Run {
    changed: Vec<(Tick, StableKey)>,
    emitted: Vec<(PresentEventKey, u32)>,
    cancelled: Vec<PresentEventKey>,
}

fn run(world: World, schedule: &mut Schedule, ticks: u32) -> (World, Run) {
    let executor = executor();
    let mut world = world;
    let mut run = Run { changed: Vec::new(), emitted: Vec::new(), cancelled: Vec::new() };
    for _ in 0..ticks {
        world = tick(schedule, world, &executor);
        let batch = world.take_present_events();
        run.emitted.extend(batch.emitted.iter().map(|e| (e.key, e.get::<AudioRequest>().unwrap().sound)));
        run.cancelled.extend(batch.cancelled);
    }
    let mut changed = std::mem::take(&mut *world.service_mut::<Seen>().unwrap().lock().unwrap());
    changed.sort();
    run.changed = changed;
    (world, run)
}

#[test]
fn a_restore_and_a_re_run_reproduce_the_first_run() {
    let mut schedule = schedule();
    let mut world = world(WorldKind::Prediction, LayoutConfig::default());
    populate(&mut world);
    world.insert_service::<Seen>(Box::new(Mutex::new(Vec::new())));
    let (world, _) = run(world, &mut schedule, 3);
    let snapshot = world.snapshot();
    assert_eq!(snapshot.tick(), t(3));
    assert!(snapshot.size_in_bytes() > 0);

    let (mut world, first) = run(world, &mut schedule, 7);
    assert_eq!(world.tick(), t(10));
    let at_ten = columns(&world);
    assert_eq!(first.emitted.len(), 6, "two events on ticks five, seven and nine");

    // A bind arrives after the snapshot; a consumer's cursor and mark predate the restore.
    let bomb = world
        .entities_in(REGION)
        .find(|&key| world.get::<Bomb>(key).is_some())
        .expect("the bomb thrown at tick 1");
    let StableKey::Predicted(bomb_key) = bomb else { panic!("predicted") };
    let bound = lactose_core::NetId::from_bits(4242).unwrap();
    world.bind(bomb_key, bound).unwrap();
    let mut cursor = world.structural_log().cursor();
    let mark = world.mark();

    world.restore(&snapshot).unwrap();
    assert_eq!(world.tick(), t(3));
    assert!(world.structural_log().since(&mut cursor).is_none(), "a stale cursor");
    assert!(world.changed_since::<Position>(mark).is_none(), "a stale mark");
    assert_eq!(world.alias(bomb_key), Some(bound), "the bind survives the restore");
    assert_eq!(world.origin(StableKey::Net(bound)), Some(bomb));

    let (mut world, second) = run(world, &mut schedule, 7);
    assert_eq!(world.tick(), t(10));
    assert_eq!(columns(&world), at_ten, "every column and every row tick");
    assert_eq!(second.changed, first.changed, "the re-run saw the first run's changes");
    assert!(second.emitted.is_empty(), "nothing presentation has seen is delivered again");
    assert!(second.cancelled.is_empty());

    // A correction written without recording changes nothing a `Changed` filter sees, and
    // here flips which ticks chirp: every delivered event is cancelled, and the others are
    // delivered once.
    world.restore(&snapshot).unwrap();
    for column in world.columns_mut::<AnimState>() {
        column.write(|_, values, _| values.iter_mut().for_each(|anim| anim.phase += 1));
    }
    let (_, third) = run(world, &mut schedule, 7);
    assert_eq!(third.changed, first.changed);
    let mut cancelled = third.cancelled.clone();
    cancelled.sort_by_key(|key| (key.tick, key.seq));
    let mut delivered: Vec<PresentEventKey> = first.emitted.iter().map(|(key, _)| *key).collect();
    delivered.sort_by_key(|key| (key.tick, key.seq));
    assert_eq!(cancelled, delivered);
    assert_eq!(third.emitted.len(), 8, "the ticks that did not chirp before: four, six, eight, ten");
    assert!(third.emitted.iter().all(|(key, _)| !delivered.contains(key)));
    // Within a tick the sequence orders an origin's events of one kind.
    let sounds: Vec<u32> = third.emitted.iter().map(|&(_, sound)| sound).collect();
    assert!(sounds.chunks(2).all(|pair| pair[1] == pair[0] + 1000));
}

#[test]
fn the_rollback_horizon_prunes_delivered_keys_and_dead_aliases() {
    let mut schedule = schedule();
    let mut world = world(WorldKind::Prediction, LayoutConfig::default());
    populate(&mut world);
    world.insert_service::<Seen>(Box::new(Mutex::new(Vec::new())));
    let snapshot = world.snapshot();
    let (mut world, first) = run(world, &mut schedule, 4);
    assert!(!first.emitted.is_empty());

    // An entity despawned at tick 4 keeps its alias until the horizon passes tick 4.
    let gone = PredictionKey { player: PLAYER, input_tick: t(4), ordinal: 9 };
    world.spawn(StableKey::Predicted(gone), REGION, (Health(1),)).unwrap();
    world.bind(gone, lactose_core::NetId::from_bits(77).unwrap()).unwrap();
    world.despawn(StableKey::Predicted(gone)).unwrap();
    world.set_rollback_horizon(t(4));
    assert!(world.alias(gone).is_some());
    world.set_rollback_horizon(t(5));
    assert!(world.alias(gone).is_none());

    // With every delivered key pruned, a re-run from the start delivers again.
    world.restore(&snapshot).unwrap();
    let (_, again) = run(world, &mut schedule, 4);
    assert_eq!(again.emitted.len(), first.emitted.len());
}

#[test]
fn a_snapshot_from_another_world_is_refused() {
    let world_a = world(WorldKind::Prediction, LayoutConfig::default());
    let mut world_b = world(WorldKind::Prediction, LayoutConfig::default());
    let error = world_b.restore(&world_a.snapshot()).unwrap_err();
    assert_eq!(error.kind(), lactose_core::ErrorKind::InvalidData);
}
