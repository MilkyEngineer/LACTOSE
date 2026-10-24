//! Bulk copy (architecture section 15.2): a cooked baseline and an overlay with every
//! record kind load into a region; the export diffs back to the same overlay through a
//! field-level diff, which the bundle format accepts (`lactose-persist` writes whole
//! components, D129); region resources travel beside the rows.

#![forbid(unsafe_code)]

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::*;
use lactose_asset_api::AssetSet;
use lactose_bundle::{Baseline, BaselineRow, ComponentData, Overlay, Record};
use lactose_core::{ComponentKey, EntityRef, ErrorKind, NetId, PredictionKey, StableKey};
use lactose_ecs::{
    LayoutConfig, OwnedBy, PredictedOrigin, RegionConfig, RegionMode, RegionRows, StructuralChange,
    World, WorldBuilder, WorldKind,
};
use lactose_reflect::{Reflect, Registry};

fn cooked(hash: u64) -> NetId {
    NetId::cooked(hash)
}

fn runtime(id: u64) -> NetId {
    NetId::from_bits(id).unwrap()
}

fn data<T: Reflect>(value: T) -> ComponentData {
    ComponentData { component: T::KEY, bytes: bytemuck::bytes_of(&value).to_vec() }
}

const C1: u64 = 1;
const C2: u64 = 2;
const C3: u64 = 3;
const C4: u64 = 4;
const MOVED_IN: u64 = 9;
const R1: u64 = 5;
const R2: u64 = 6;

fn predicted_r2() -> PredictionKey {
    PredictionKey { player: PLAYER, input_tick: t(12), ordinal: 0xABCD }
}

fn baseline(registry: &Registry) -> Baseline {
    let owned_by_c1 = OwnedBy { owner: EntityRef::new(StableKey::Net(cooked(C1))) };
    let thrown_by_c1 = ThrownBy { thrower: EntityRef::new(StableKey::Net(cooked(C1))) };
    let rows = [
        BaselineRow { id: cooked(C1), components: vec![data(pos(1, 1)), data(Health(10))] },
        BaselineRow {
            id: cooked(C2),
            components: vec![data(pos(2, 2)), data(Mass(lactose_math::Scalar::ONE))],
        },
        BaselineRow { id: cooked(C3), components: vec![data(pos(3, 3)), data(owned_by_c1)] },
        BaselineRow { id: cooked(C4), components: vec![data(pos(4, 4)), data(thrown_by_c1)] },
    ];
    Baseline::from_rows(AssetSet::new(), schemas(registry), rows).unwrap()
}

/// An overlay with every record kind.
fn overlay(registry: &Registry, baseline: &Baseline) -> Overlay {
    let predicted = predicted_r2();
    let origin = PredictedOrigin {
        player: predicted.player,
        input_tick: predicted.input_tick,
        ordinal: predicted.ordinal,
    };
    let x = lactose_math::Scalar::from_i32(42);
    let records = [
        Record::Spawn {
            id: runtime(R1),
            components: vec![
                data(pos(10, 10)),
                data(ThrownBy { thrower: EntityRef::new(StableKey::Predicted(predicted)) }),
            ],
        },
        Record::Spawn { id: runtime(R2), components: vec![data(pos(11, 11)), data(origin)] },
        Record::OwnershipChange { id: runtime(R2), owner: Some(runtime(R1)) },
        Record::SpawnFromBaseline { id: cooked(MOVED_IN), components: vec![data(pos(9, 9))] },
        Record::Destroy { id: cooked(C2) },
        Record::AddComponent { id: cooked(C1), data: data(Counter(5)) },
        Record::RemoveComponent { id: cooked(C1), component: Health::KEY },
        Record::SetField {
            id: cooked(C1),
            component: Position::KEY,
            field: lactose_core::FieldId(1),
            bytes: bytemuck::bytes_of(&x).to_vec(),
        },
        Record::OwnershipChange { id: cooked(C3), owner: Some(runtime(R1)) },
    ];
    Overlay::new(baseline.content_hash(), schemas(registry), records).unwrap()
}

/// A field-level diff of a region, which the bundle format accepts: the records that turn
/// `baseline` into the exported rows, one `SetField` per changed field so the test covers
/// that record kind. `lactose-persist` writes a changed component whole (D129).
fn diff(registry: &Registry, baseline: &Baseline, exported: &RegionRows) -> Overlay {
    let schemas = schemas(registry);
    let schema = |key: ComponentKey| schemas.iter().find(|s| s.component() == key).unwrap();
    let exported_rows: BTreeMap<NetId, _> = exported.rows.iter().map(|row| (row.id, row)).collect();
    let mut records = Vec::new();
    for archetype in baseline.archetypes() {
        for (index, &id) in archetype.ids().iter().enumerate() {
            let Some(row) = exported_rows.get(&id) else {
                records.push(Record::Destroy { id });
                continue;
            };
            let mut base: BTreeMap<ComponentKey, Vec<u8>> = archetype
                .columns()
                .map(|column| {
                    let size = column.element_size as usize;
                    (column.component, column.bytes[index * size..(index + 1) * size].to_vec())
                })
                .collect();
            let base_owner = base.remove(&OwnedBy::KEY).map(|bytes| {
                let owned: OwnedBy = bytemuck::checked::pod_read_unaligned(&bytes);
                match owned.owner.get() {
                    Some(StableKey::Net(id)) => id,
                    other => panic!("a cooked owner is a cooked key: {other:?}"),
                }
            });
            for component in &row.components {
                match base.remove(&component.component) {
                    Some(old) if old == component.bytes => {}
                    Some(old) => {
                        for field in schema(component.component).fields() {
                            let range = field.offset as usize..(field.offset + field.size) as usize;
                            if old[range.clone()] != component.bytes[range.clone()] {
                                records.push(Record::SetField {
                                    id,
                                    component: component.component,
                                    field: field.id,
                                    bytes: component.bytes[range].to_vec(),
                                });
                            }
                        }
                    }
                    None => records.push(Record::AddComponent { id, data: component.clone() }),
                }
            }
            for component in base.into_keys() {
                records.push(Record::RemoveComponent { id, component });
            }
            if row.owner != base_owner {
                records.push(Record::OwnershipChange { id, owner: row.owner });
            }
        }
    }
    for row in &exported.rows {
        if baseline.contains(row.id) {
            continue;
        }
        let components = row.components.clone();
        records.push(if row.id.is_cooked() {
            Record::SpawnFromBaseline { id: row.id, components }
        } else {
            Record::Spawn { id: row.id, components }
        });
        if row.owner.is_some() {
            records.push(Record::OwnershipChange { id: row.id, owner: row.owner });
        }
    }
    Overlay::new(baseline.content_hash(), schemas, records).unwrap()
}

fn fresh(registry: std::sync::Arc<Registry>, layout: LayoutConfig, seed: u64) -> World {
    let mut world = WorldBuilder::new(WorldKind::Server, registry).layout(layout).build().unwrap();
    world.create_region(RegionConfig { id: REGION, seed, mode: RegionMode::Simulated }).unwrap();
    world
}

#[test]
fn an_overlay_with_every_record_loads_and_diffs_back_to_itself() {
    let registry = registry();
    let baseline = baseline(&registry);
    let overlay = overlay(&registry, &baseline);
    for layout in [LayoutConfig::default()].into_iter().chain(layouts()) {
        let mut world = fresh(registry.clone(), layout, 77);
        let mut cursor = world.structural_log().cursor();
        let mark = world.mark();
        world.load_region(REGION, &baseline, Some(&overlay)).unwrap();

        let c1 = StableKey::Net(cooked(C1));
        let r2 = StableKey::Predicted(predicted_r2());
        assert!(!world.contains(StableKey::Net(cooked(C2))), "destroyed");
        assert_eq!(world.get::<Health>(c1), None, "removed");
        assert_eq!(world.get::<Counter>(c1), Some(&Counter(5)), "added");
        assert_eq!(
            world.get::<Position>(c1),
            Some(&Position { x: lactose_math::Scalar::from_i32(42), ..pos(1, 1) })
        );
        assert_eq!(
            world.owned_by(StableKey::Net(runtime(R1))).collect::<Vec<_>>(),
            [StableKey::Net(cooked(C3)), r2]
        );
        assert_eq!(world.origin(StableKey::Net(runtime(R2))), Some(r2));
        assert_eq!(world.referrers(c1).count(), 1, "the baseline's reference is indexed");
        assert_eq!(world.region(REGION).unwrap().entity_count, 6);

        // A region load starts no epoch: the rows are changes and spawns since the mark.
        assert!(world.is_current(mark));
        assert_eq!(world.changed_since::<Position>(mark).unwrap().count(), 6);
        let spawned: BTreeSet<StableKey> = world
            .structural_log()
            .since(&mut cursor)
            .unwrap()
            .filter_map(|entry| match entry.change {
                StructuralChange::Spawned { key, .. } => Some(key),
                _ => None,
            })
            .collect();
        assert_eq!(spawned.len(), 6);

        let exported = world.export_region(REGION).unwrap();
        assert_eq!(exported.seed, 77);
        let ids: Vec<NetId> = exported.rows.iter().map(|row| row.id).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted, "rows in ascending NetId order");
        assert_eq!(diff(&registry, &baseline, &exported), overlay, "{layout:?}");
    }
}

#[test]
fn region_resources_travel_beside_the_rows() {
    let registry = registry();
    let baseline = baseline(&registry);
    let mut world = fresh(registry.clone(), LayoutConfig::default(), 3);
    world.load_region(REGION, &baseline, None).unwrap();
    world.insert_region_resource(REGION, Weather(9)).unwrap();
    world.insert_region_resource(REGION, RegionScratch(1)).unwrap();
    let exported = world.export_region(REGION).unwrap();
    assert_eq!(exported.resources, [data(Weather(9))], "persistent resources only");

    let mut loaded = fresh(registry.clone(), LayoutConfig::default(), exported.seed);
    loaded.load_region(REGION, &baseline, None).unwrap();
    for resource in exported.resources.clone() {
        loaded.insert_region_resource_erased(REGION, resource).unwrap();
    }
    assert_eq!(loaded.region_resource::<Weather>(REGION), Some(&Weather(9)));
    assert_eq!(loaded.export_region(REGION).unwrap(), exported);
    let erased: Vec<ComponentKey> =
        loaded.region_resources_erased(REGION).map(|(key, _)| key).collect();
    assert_eq!(erased, [Weather::KEY]);
    let invalid = ComponentData { component: Weather::KEY, bytes: vec![1, 2] };
    assert_eq!(
        loaded.insert_region_resource_erased(REGION, invalid).unwrap_err().kind(),
        ErrorKind::InvalidData
    );
}

#[test]
fn a_load_checks_the_region_and_the_schemas() {
    let registry = registry();
    let baseline = baseline(&registry);
    let mut world = fresh(registry.clone(), LayoutConfig::default(), 1);
    world.load_region(REGION, &baseline, None).unwrap();
    assert_eq!(
        world.load_region(REGION, &baseline, None).unwrap_err().kind(),
        ErrorKind::InvalidData
    );
    assert_eq!(
        world.load_region(OTHER_REGION, &baseline, None).unwrap_err().kind(),
        ErrorKind::NotFound
    );

    // A document written under another schema version must be migrated first.
    let old = schemas(&registry)
        .into_iter()
        .map(|schema| {
            if schema.component() != Position::KEY {
                return schema;
            }
            lactose_bundle::ComponentSchema::new(
                schema.component(),
                schema.schema_version() + 1,
                schema.element_size(),
                schema.fields().iter().copied(),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let rows = [BaselineRow { id: cooked(1), components: vec![data(pos(1, 1))] }];
    let migrated_needed = Baseline::from_rows(AssetSet::new(), old, rows).unwrap();
    let mut world = fresh(registry, LayoutConfig::default(), 1);
    assert_eq!(
        world.load_region(REGION, &migrated_needed, None).unwrap_err().kind(),
        ErrorKind::VersionMismatch
    );
    assert_eq!(world.region(REGION).unwrap().entity_count, 0, "nothing was loaded");
}

#[test]
fn an_entity_without_a_net_id_cannot_be_exported() {
    let mut world = world(WorldKind::Prediction, LayoutConfig::default());
    let key = PredictionKey { player: PLAYER, input_tick: t(1), ordinal: 1 };
    world.spawn(StableKey::Predicted(key), REGION, (pos(0, 0),)).unwrap();
    assert_eq!(world.export_region(REGION).unwrap_err().kind(), ErrorKind::InvalidData);
    world.bind(key, runtime(8)).unwrap();
    let exported = world.export_region(REGION).unwrap();
    assert_eq!(exported.rows[0].id, runtime(8));
    assert!(exported.rows[0].components.iter().any(|c| c.component == PredictedOrigin::KEY));
}
