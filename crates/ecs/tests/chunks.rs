//! `World::chunks` is the join of every column of a chunk: each `ChunkRef` holds the rows of
//! one chunk, its columns are those `columns_erased` and `columns` yield for that chunk, row
//! for row, and its `changed_since` selects what theirs does, under every layout.

#![forbid(unsafe_code)]

use crate::common;

use common::*;
use lactose_core::{ComponentKey, StableKey};
use lactose_ecs::{ChangeMark, InRegion, World, WorldKind};
use lactose_math::Scalar;
use lactose_reflect::Reflect;

/// Every (key, component, bytes) the world holds, through `chunks`, sorted.
fn joined(world: &World) -> Vec<(StableKey, ComponentKey, Vec<u8>)> {
    let mut rows = Vec::new();
    for chunk in world.chunks() {
        let components: Vec<ComponentKey> = chunk.components().collect();
        assert!(components.is_sorted(), "a chunk's components ascend");
        assert!(components.contains(&InRegion::KEY), "every row has a region");
        assert_eq!(chunk.regions().len(), chunk.keys().len());
        for component in components {
            let column = chunk.column(component).expect("a component the chunk lists");
            assert_eq!(column.component(), component);
            assert_eq!(column.keys(), chunk.keys());
            assert_eq!(column.bytes().len(), column.element_size() * chunk.keys().len());
            for (row, &key) in chunk.keys().iter().enumerate() {
                let size = column.element_size();
                rows.push((key, component, column.bytes()[row * size..(row + 1) * size].to_vec()));
            }
        }
    }
    rows.sort();
    rows
}

/// The same, through `columns_erased` for every registered component.
fn erased(world: &World) -> Vec<(StableKey, ComponentKey, Vec<u8>)> {
    let mut rows = Vec::new();
    for info in world.registry().iter() {
        for column in world.columns_erased(info.key()) {
            let size = column.element_size();
            for (row, &key) in column.keys().iter().enumerate() {
                rows.push((key, info.key(), column.bytes()[row * size..(row + 1) * size].to_vec()));
            }
        }
    }
    rows.sort();
    rows
}

/// The keys whose `Position` changed since `mark`, through `chunks`, sorted.
fn changed(world: &World, mark: ChangeMark) -> Option<Vec<StableKey>> {
    let mut keys = Vec::new();
    for chunk in world.chunks() {
        let Some(column) = chunk.column(Position::KEY) else { continue };
        keys.extend(column.changed_since(mark)?.map(|row| chunk.keys()[row]));
    }
    keys.sort();
    Some(keys)
}

#[test]
fn a_chunk_joins_its_columns_row_for_row_under_every_layout() {
    let mut reference = None;
    for layout in layouts() {
        let mut world = world(WorldKind::Server, layout);
        for id in 1..=12 {
            world.spawn(net(id), REGION, (pos(i32::try_from(id).unwrap(), 0),)).unwrap();
            if id % 2 == 0 {
                world.insert(net(id), vel(0, i32::try_from(id).unwrap())).unwrap();
            }
            if id % 3 == 0 {
                world.insert(net(id), Counter(id)).unwrap();
            }
        }
        world.spawn(net(20), REGION, (Counter(20),)).unwrap();

        // The typed view of a chunk is its erased one, row for row, and a chunk without the
        // component has neither.
        for chunk in world.chunks() {
            match (chunk.values::<Position>(), chunk.column(Position::KEY)) {
                (Some(values), Some(column)) => {
                    assert_eq!(values.len(), chunk.keys().len());
                    for (row, value) in values.iter().enumerate() {
                        assert_eq!(world.get::<Position>(chunk.keys()[row]), Some(value));
                        let bytes = lactose_reflect::bytemuck::bytes_of(value);
                        assert_eq!(&column.bytes()[row * bytes.len()..][..bytes.len()], bytes);
                    }
                }
                (None, None) => {
                    assert!(chunk.keys().iter().all(|&key| world.get::<Position>(key).is_none()))
                }
                _ => panic!(
                    "the typed and erased views disagree on whether the chunk has the column"
                ),
            }
        }

        let rows = joined(&world);
        assert_eq!(rows, erased(&world));
        assert_eq!(
            *reference.get_or_insert_with(|| rows.clone()),
            rows,
            "the same under every layout"
        );

        // A chunk's column selects by stamp exactly as the world's does.
        let mark = world.mark();
        assert_eq!(changed(&world, mark), Some(Vec::new()));
        world.get_mut::<Position>(net(4)).unwrap().y = Scalar::ONE;
        world.get_mut::<Position>(net(9)).unwrap().y = Scalar::ONE;
        let mut typed: Vec<StableKey> =
            world.changed_since::<Position>(mark).unwrap().map(|(key, _)| key).collect();
        typed.sort();
        assert_eq!(changed(&world, mark), Some(typed));
        assert_eq!(changed(&world, mark), Some(vec![net(4), net(9)]));
        world.set_tick(t(50)).unwrap();
        assert_eq!(changed(&world, mark), None, "a stale mark");
    }
}
