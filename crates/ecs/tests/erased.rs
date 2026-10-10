//! `ErasedColumn::changed_since` selects exactly the rows `World::changed_since` selects:
//! by stamp, so a write in the mark's own tick before the mark is excluded and one after
//! it is included, under every layout, for writes inside a tick and between ticks.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::{StableKey, Tick};
use lactose_ecs::{ChangeMark, Mut, Schedule, World, WorldKind, system};
use lactose_math::Scalar;
use lactose_reflect::Reflect;

/// Moves the odd-counted rows.
#[system(name = "test::poke", worlds = [server])]
fn poke(how: &Counter, position: Mut<Position>) {
    if how.0 % 2 == 1 {
        position.x += Scalar::ONE;
    }
}

fn typed(world: &World, mark: ChangeMark) -> Option<Vec<StableKey>> {
    let mut keys: Vec<StableKey> =
        world.changed_since::<Position>(mark)?.map(|(key, _)| key).collect();
    keys.sort();
    Some(keys)
}

fn erased(world: &World, mark: ChangeMark) -> Option<Vec<StableKey>> {
    let mut keys = Vec::new();
    for column in world.columns_erased(Position::KEY) {
        let indices = column.changed_since(mark)?;
        let row_ticks = column.row_ticks().expect("a replicated column carries row ticks");
        for row in indices {
            assert!(row < column.keys().len());
            assert_eq!(row_ticks.len(), column.keys().len());
            keys.push(column.keys()[row]);
        }
    }
    keys.sort();
    Some(keys)
}

fn agree(world: &World, mark: ChangeMark) -> Vec<StableKey> {
    let typed = typed(world, mark);
    assert_eq!(typed, erased(world, mark));
    typed.expect("a current mark")
}

#[test]
fn erased_and_typed_selections_agree_under_every_layout() {
    for layout in layouts() {
        let executor = executor();
        let mut world = world(WorldKind::Server, layout);
        for id in 1..=9 {
            world.spawn(net(id), REGION, (pos(0, 0), Counter(id))).unwrap();
        }
        let mut builder = Schedule::builder(WorldKind::Server);
        builder.stage("main", &[]).add(&POKE);
        let mut schedule = builder.build().unwrap();

        // Writes inside a tick: the odd rows, seen from a mark taken before the tick.
        let before = world.mark();
        world = tick(&mut schedule, world, &executor);
        let odd: Vec<StableKey> = [1, 3, 5, 7, 9].map(net).to_vec();
        assert_eq!(agree(&world, before), odd);

        // A direct write between ticks, after a fresh mark taken in the same tick as the
        // writes above: only it is newer than the mark, by stamp and not by tick.
        let fresh = world.mark();
        assert_eq!(agree(&world, fresh), []);
        world.get_mut::<Position>(net(4)).unwrap().y = Scalar::ONE;
        assert_eq!(agree(&world, fresh), [net(4)]);
        assert_eq!(agree(&world, before), [1, 3, 4, 5, 7, 9].map(net).to_vec());

        // An equal value through `set_if_neq` is a change for neither.
        assert!(!world.get_mut::<Position>(net(2)).unwrap().set_if_neq(pos(0, 0)));
        assert_eq!(agree(&world, fresh), [net(4)]);

        // A new epoch makes the mark stale for both.
        world.set_tick(Tick(100)).unwrap();
        assert!(typed(&world, before).is_none() && erased(&world, before).is_none());
        assert_eq!(agree(&world, world.mark()), []);
    }
}

#[test]
#[should_panic(expected = "not tracked")]
fn erased_changed_since_on_an_untracked_column_panics() {
    let mut world = world(WorldKind::Server, Default::default());
    world.spawn(net(1), REGION, (vel(0, 0),)).unwrap();
    let mark = world.mark();
    for column in world.columns_erased(Velocity::KEY) {
        let _ = column.changed_since(mark);
    }
}
