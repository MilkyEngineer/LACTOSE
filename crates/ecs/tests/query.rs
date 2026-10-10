//! Whole-world queries for exclusive code: ascending key order under every layout, the
//! filters, `Mut` items that stamp, and the alias rule.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::StableKey;
use lactose_ecs::{Changed, LayoutConfig, Mut, With, Without, WorldKind};
use lactose_math::Scalar;

#[test]
fn queries_iterate_in_key_order_under_every_layout() {
    for layout in [LayoutConfig::default()].into_iter().chain(layouts()) {
        let mut world = world(WorldKind::Server, layout);
        for id in [9, 2, 7, 4, 1, 8] {
            if id % 2 == 0 {
                world.spawn(net(id), REGION, (pos(id as i32, 0), Counter(id))).unwrap();
            } else {
                world.spawn(net(id), REGION, (pos(id as i32, 0),)).unwrap();
            }
        }
        let mut seen = Vec::new();
        world.query::<(&Position, Option<&Counter>)>().for_each(|key, (position, counter)| {
            seen.push((key, position.x, counter.map(|c| c.0)));
        });
        let keys: Vec<StableKey> = seen.iter().map(|&(key, ..)| key).collect();
        assert_eq!(keys, [1, 2, 4, 7, 8, 9].map(net), "{layout:?}");
        assert_eq!(seen[1], (net(2), Scalar::TWO, Some(2)));
        assert_eq!(seen[0].2, None);

        assert_eq!(world.query::<(&Position, With<Counter>)>().count(), 3);
        assert_eq!(world.query::<(&Position, Without<Counter>)>().count(), 3);
        assert!(world.query::<&Counter>().get(net(1)).is_none());
        assert_eq!(world.query::<&Counter>().get(net(4)).map(|c| c.0), Some(4));
    }
}

#[test]
fn mut_items_stamp_on_write_and_changed_items_filter() {
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    for id in 1..=3 {
        world.spawn(net(id), REGION, (pos(0, 0), vel(0, 0))).unwrap();
    }
    let mark = world.mark();
    world.query::<(Mut<Position>, &Velocity)>().for_each(|key, (mut position, _)| {
        if key == net(2) {
            position.y = Scalar::ONE;
        }
    });
    let changed: Vec<StableKey> =
        world.changed_since::<Position>(mark).unwrap().map(|(key, _)| key).collect();
    assert_eq!(changed, [net(2)]);
    // `Changed` in a query passes rows changed since this point of the previous tick.
    assert_eq!(world.query::<(Changed<Position>, &Position)>().count(), 3);
}

/// A tag: a component with no fields, so its column has no bytes.
#[lactose_reflect::component(key = "test::tag", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct Tag {}

#[test]
fn a_zero_sized_component_is_stored_queried_tracked_and_exported() {
    let mut registry = base_registry();
    registry.register::<Tag>().unwrap();
    let mut world =
        world_with(WorldKind::Server, LayoutConfig::default(), std::sync::Arc::new(registry));
    world.spawn(net(1), REGION, (Tag {}, Counter(1))).unwrap();
    world.spawn(net(2), REGION, (Counter(2),)).unwrap();
    let mark = world.mark();
    world.insert(net(2), Tag {}).unwrap();
    assert_eq!(world.get::<Tag>(net(1)), Some(&Tag {}));
    assert_eq!(world.query::<(&Tag, &Counter)>().count(), 2);
    let changed: Vec<StableKey> =
        world.changed_since::<Tag>(mark).unwrap().map(|(key, _)| key).collect();
    assert_eq!(changed, [net(2)]);
    let column_rows: usize = world.columns::<Tag>().map(|column| column.values().len()).sum();
    assert_eq!(column_rows, 2);
    let exported = world.export_region(REGION).unwrap();
    let tag = <Tag as lactose_reflect::Reflect>::KEY;
    assert!(
        exported
            .rows
            .iter()
            .all(|row| row.components.iter().any(|c| c.component == tag && c.bytes.is_empty()))
    );
    assert_eq!(world.remove::<Tag>(net(1)).unwrap(), Some(Tag {}));
}

#[test]
#[should_panic(expected = "aliases")]
fn two_data_items_on_one_component_panic() {
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    let _ = world.query::<(&Position, Mut<Position>)>();
}

#[test]
#[should_panic(expected = "maintains")]
fn a_mut_item_on_a_maintained_component_panics() {
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    let _ = world.query::<Mut<lactose_ecs::OwnedBy>>();
}
