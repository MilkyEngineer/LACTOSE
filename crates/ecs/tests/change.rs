//! Change tracking: `Mut` stamps on a mutable dereference only, `set_if_neq` stamps only a
//! different value, `changed_since` with fresh and stale marks, `Changed<T>` sees each
//! change once per reader, and `ColumnMut::write` stamps exactly the rows it records.

#![forbid(unsafe_code)]

mod common;

use std::sync::Mutex;

use common::*;
use lactose_core::StableKey;
use lactose_ecs::{
    Changed, Commands, LayoutConfig, Mut, Res, Schedule, StableKeyOf, World, WorldKind, system,
};
use lactose_math::Scalar;

/// What the `Changed` readers saw, in order of the readers' runs.
type Seen = Mutex<Vec<(&'static str, StableKey)>>;

/// Writes `Position` the way its `Counter` says: 1 a mutable dereference, 2 an equal value
/// through `set_if_neq`, 3 a different one through it, anything else a read.
#[system(name = "test::nudge", worlds = [server])]
fn nudge(how: &Counter, position: Mut<Position>) {
    match how.0 {
        1 => position.x += Scalar::ONE,
        2 => {
            let same = *position;
            assert!(!position.set_if_neq(same));
        }
        3 => {
            let next = Position { x: position.x + Scalar::ONE, y: position.y };
            assert!(position.set_if_neq(next));
        }
        _ => {
            let _ = position.x;
        }
    }
}

#[system(name = "test::watch_after", worlds = [server], after = nudge)]
fn watch_after(_changed: Changed<Position>, _position: &Position, key: StableKeyOf, seen: Res<Seen>) {
    seen.lock().unwrap().push(("after", *key));
}

/// Moves `net(5)` by command, so the write happens at the stage's closing sync point.
#[system(name = "test::by_command", worlds = [server])]
fn by_command(cmd: Commands, tick: &Health) {
    if tick.0 == 7 {
        cmd.set(net(5), pos(50, 50));
        cmd.set(net(5), Health(8));
    }
}

fn seen(world: &mut World) -> Vec<(&'static str, StableKey)> {
    let mut seen = std::mem::take(&mut *world.service_mut::<Seen>().unwrap().lock().unwrap());
    seen.sort();
    seen
}

#[test]
fn changed_sees_each_change_once_per_reader() {
    let executor = executor();
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    for (id, how) in [(1, 1), (2, 2), (3, 3), (4, 4)] {
        world.spawn(net(id), REGION, (pos(0, 0), Counter(how))).unwrap();
    }
    world.spawn(net(5), REGION, (pos(0, 0), Health(0))).unwrap();
    world.insert_service::<Seen>(Box::new(Mutex::new(Vec::new())));
    // `watch_before` runs before `nudge`: it is in an earlier stage.
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("before", &[]).add(&WATCH_BEFORE_ALONE).add(&BY_COMMAND);
    builder.stage("main", &[]).add(&NUDGE).add(&WATCH_AFTER);
    let mut schedule = builder.build().unwrap();

    let all = |label| (1..=5).map(|id| (label, net(id))).collect::<Vec<_>>();
    // Tick 1: the spawns happened between ticks, so both readers see every row once.
    world = tick(&mut schedule, world, &executor);
    let mut expected = [all("after"), all("before")].concat();
    expected.sort();
    assert_eq!(seen(&mut world), expected);

    // Every tick `nudge` writes rows 1 and 3. The reader after it sees a write in the tick
    // it happens; the reader before it sees it in the next tick. Row 2's equal value and
    // row 4's read are not changes.
    let nudged = [("after", net(1)), ("after", net(3)), ("before", net(1)), ("before", net(3))];
    world = tick(&mut schedule, world, &executor);
    assert_eq!(seen(&mut world), nudged);

    // A command applied at the sync point between the readers (tick 3 sets `net(5)`'s
    // position) is after one reader and before the other.
    world.get_mut::<Health>(net(5)).unwrap().0 = 7;
    world = tick(&mut schedule, world, &executor);
    let mut expected = [nudged.to_vec(), vec![("after", net(5))]].concat();
    expected.sort();
    assert_eq!(seen(&mut world), expected);

    // A direct write between ticks is before both readers.
    world.get_mut::<Position>(net(4)).unwrap().y = Scalar::TWO;
    for row in [1, 3] {
        world.insert(net(row), Counter(0)).unwrap();
    }
    world = tick(&mut schedule, world, &executor);
    assert_eq!(
        seen(&mut world),
        [
            ("after", net(4)),
            ("before", net(1)),
            ("before", net(3)),
            ("before", net(4)),
            ("before", net(5)),
        ]
    );
    world = tick(&mut schedule, world, &executor);
    assert_eq!(seen(&mut world), [], "each change was seen once by each reader");
}

#[system(name = "test::watch_before_alone", worlds = [server])]
fn watch_before_alone(_changed: Changed<Position>, key: StableKeyOf, seen: Res<Seen>) {
    seen.lock().unwrap().push(("before", *key));
}

#[test]
fn mut_marks_on_mutable_dereference_only() {
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    world.spawn(net(1), REGION, (pos(0, 0), Counter(0))).unwrap();
    let mark = world.mark();

    // A read through `Mut` is not a change.
    let position = world.get_mut::<Position>(net(1)).unwrap();
    assert_eq!(position.x, Scalar::ZERO);
    drop(position);
    assert_eq!(world.changed_since::<Position>(mark).unwrap().count(), 0);

    // An equal value through `set_if_neq` is not a change; a different one is.
    assert!(!world.get_mut::<Position>(net(1)).unwrap().set_if_neq(pos(0, 0)));
    assert_eq!(world.changed_since::<Position>(mark).unwrap().count(), 0);
    assert!(world.get_mut::<Position>(net(1)).unwrap().set_if_neq(pos(1, 0)));
    let changed: Vec<_> = world.changed_since::<Position>(mark).unwrap().collect();
    assert_eq!(changed, [(net(1), &pos(1, 0))]);

    // A fresh mark is after the write; a write after it is newer than it.
    let fresh = world.mark();
    assert_eq!(world.changed_since::<Position>(fresh).unwrap().count(), 0);
    world.get_mut::<Position>(net(1)).unwrap().y = Scalar::ONE;
    assert_eq!(world.changed_since::<Position>(fresh).unwrap().count(), 1);

    // A new epoch makes every older mark stale.
    assert!(world.is_current(mark));
    world.set_tick(t(100)).unwrap();
    assert!(!world.is_current(mark) && !world.is_current(fresh));
    assert!(world.changed_since::<Position>(mark).is_none());
    assert!(world.is_current(world.mark()));
}

#[test]
#[should_panic(expected = "not tracked")]
fn changed_since_on_an_untracked_column_panics() {
    let world = world(WorldKind::Server, LayoutConfig::default());
    let _ = world.changed_since::<Velocity>(world.mark());
}

#[test]
fn column_writes_stamp_exactly_the_rows_they_record() {
    for layout in [LayoutConfig::default(), LayoutConfig { chunk_rows: Some(1), ..LayoutConfig::default() }] {
        let mut world = world(WorldKind::Server, layout);
        for id in 1..=4 {
            world.spawn(net(id), REGION, (pos(0, 0), vel(0, 0))).unwrap();
        }
        let mark = world.mark();
        let ticks_before: Vec<_> = world.columns::<Position>().map(|c| c.column_tick()).collect();

        // A correction without recording leaves every stamp alone.
        for column in world.columns_mut::<Position>() {
            column.write(|_, values, _| values.iter_mut().for_each(|p| p.x = Scalar::TWO));
        }
        assert_eq!(world.changed_since::<Position>(mark).unwrap().count(), 0);
        assert_eq!(world.get::<Position>(net(3)), Some(&pos(2, 0)), "the write itself happened");

        // A write that records one row stamps that row only.
        for column in world.columns_mut::<Position>() {
            column.write(|keys, values, dirty| {
                for (row, key) in keys.iter().enumerate() {
                    values[row].y = Scalar::ONE;
                    if *key == net(3) {
                        dirty.mark(row);
                    }
                }
            });
        }
        let changed: Vec<StableKey> =
            world.changed_since::<Position>(mark).unwrap().map(|(key, _)| key).collect();
        assert_eq!(changed, [net(3)]);
        let raised = world
            .columns::<Position>()
            .filter(|column| column.keys().contains(&net(3)))
            .all(|column| column.column_tick() >= ticks_before[0]);
        assert!(raised);

        // `mark_all` stamps every row of the slice; untracked columns carry no row ticks.
        let mark = world.mark();
        for column in world.columns_mut::<Velocity>() {
            assert_eq!(column.keys().len(), column.regions().len());
            column.write(|_, _, dirty| dirty.mark_all());
        }
        assert!(world.columns::<Velocity>().all(|column| column.row_ticks().is_none()));
        assert!(world.columns::<Position>().all(|column| column.row_ticks().is_some()));
        for column in world.columns_mut::<Position>() {
            column.write(|_, _, dirty| dirty.mark_all());
        }
        assert_eq!(world.changed_since::<Position>(mark).unwrap().count(), 4);
    }
}

#[system(name = "test::bump", worlds = [server])]
fn bump(counter: Mut<Counter>) {
    counter.0 += 1;
}

#[test]
fn a_writing_system_raises_the_column_tick_of_every_chunk_it_runs_over() {
    let executor = executor();
    let mut world = world(WorldKind::Server, LayoutConfig { chunk_rows: Some(2), ..LayoutConfig::default() });
    for id in 1..=5 {
        world.spawn(net(id), REGION, (Counter(0),)).unwrap();
    }
    world.set_tick(t(40)).unwrap();
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("bump", &[]).add(&BUMP);
    let mut schedule = builder.build().unwrap();
    world = tick(&mut schedule, world, &executor);
    let columns: Vec<_> = world.columns::<Counter>().collect();
    assert_eq!(columns.len(), 3, "five rows in chunks of two");
    assert!(columns.iter().all(|column| column.column_tick() == t(41)));
    assert!(columns.iter().all(|column| column.row_ticks().unwrap().iter().all(|&tick| tick == t(41))));
    assert_eq!(world.get::<Counter>(net(5)), Some(&Counter(1)));
}
