//! The diff (README, "The diff"): every change kind gives exactly its records in canonical
//! order, with transient bytes zeroed and scalars canonical; the digest is a function of
//! region state alone, whatever the layout and the worker count; the empty diff is `None`
//! and the relevance message's `Cooked`; and what the table cannot account for is refused.

#![forbid(unsafe_code)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::*;
use lactose_asset_api::AssetSet;
use lactose_bundle::{Baseline, BaselineRow, ComponentData, ComponentSchema, Record};
use lactose_core::{ComponentKey, ErrorKind};
use lactose_ecs::{
    ExportedRow, OwnedBy, RegionConfig, RegionId, RegionMode, RegionRows, WorldBuilder, WorldKind,
};
use lactose_math::Scalar;
use lactose_net_api::OverlayPayload;
use lactose_persist::{Loader, Migrations, RegionRecords, SchemaTable, diff};
use lactose_reflect::{Reflect, Registry, resource};
use lactose_sim::RegionExport;
use lactose_sim::harness::{ChunkCapacity, layout_mode};

fn zeroed<T: Reflect>() -> ComponentData {
    ComponentData { component: T::KEY, bytes: vec![0; size_of::<T>()] }
}

fn sorted(mut components: Vec<ComponentData>) -> Vec<ComponentData> {
    components.sort_by_key(|data| data.component);
    components
}

fn tag(record: &Record) -> u8 {
    match record {
        Record::Spawn { .. } => 1,
        Record::SpawnFromBaseline { .. } => 2,
        Record::Destroy { .. } => 3,
        Record::AddComponent { .. } => 4,
        Record::RemoveComponent { .. } => 5,
        Record::SetField { .. } => 6,
        Record::OwnershipChange { .. } => 7,
    }
}

fn component_of(record: &Record) -> ComponentKey {
    match record {
        Record::AddComponent { data, .. } => data.component,
        Record::RemoveComponent { component, .. } | Record::SetField { component, .. } => {
            *component
        }
        _ => ComponentKey(0),
    }
}

/// `records` in the canonical order of `lactose-bundle`: by entity, kind, component.
fn canonical(mut records: Vec<Record>) -> Vec<Record> {
    records.sort_by_key(|record| (record.id(), tag(record), component_of(record)));
    records
}

#[test]
fn every_change_kind_gives_exactly_its_records() {
    let Changed { world, table, spawned, .. } = changed();
    let rows = world.export_region(A).unwrap();

    // The NaN is exported as stored, so the canonical one below is this crate's doing.
    let canonical_nan = Scalar::from_canonical_bits(odd_nan().to_canonical_bits());
    let exported_c3 = rows.rows.iter().find(|row| row.id == c(A, 3)).unwrap();
    let exported_pos = exported_c3.components.iter().find(|d| d.component == Position::KEY);
    assert_ne!(
        exported_pos.unwrap().bytes,
        data(Position { x: canonical_nan, y: Scalar::from_i32(3) }).bytes,
        "the test writes a NaN whose bits are not canonical"
    );

    let overlay = diff(&rows, &baseline_a(), &table).unwrap();
    let expected = vec![
        Record::Spawn {
            id: spawned,
            components: sorted(vec![
                data(pos(30, 30)),
                data(Velocity { x: Scalar::ONE, y: Scalar::ONE }),
                zeroed::<Contacts>(),
            ]),
        },
        Record::OwnershipChange { id: spawned, owner: Some(c(A, 6)) },
        Record::Destroy { id: c(A, 1) },
        Record::Destroy { id: c(A, 2) },
        Record::AddComponent { id: c(A, 3), data: data(Mass(Scalar::from_i32(5))) },
        Record::AddComponent { id: c(A, 3), data: data(Health(7)) },
        Record::RemoveComponent { id: c(A, 3), component: Marker::KEY },
        Record::AddComponent {
            id: c(A, 3),
            data: data(Position { x: canonical_nan, y: Scalar::from_i32(3) }),
        },
        Record::OwnershipChange { id: c(A, 4), owner: Some(spawned) },
        Record::OwnershipChange { id: c(A, 5), owner: None },
        Record::AddComponent { id: c(A, 6), data: zeroed::<Contacts>() },
        Record::OwnershipChange { id: c(A, 6), owner: Some(c(A, 3)) },
        Record::RemoveComponent { id: c(A, 7), component: Contacts::KEY },
        Record::SpawnFromBaseline {
            id: c(B, 1),
            components: sorted(vec![data(pos(20, 20)), data(Age(4))]),
        },
    ];
    assert_eq!(overlay.records(), canonical(expected).as_slice());
    assert_eq!(overlay.baseline_hash(), baseline_a().content_hash());

    // The schemas are exactly the table's of the types carried.
    let carried: Vec<ComponentKey> = overlay.schemas().iter().map(|s| s.component()).collect();
    let mut expected_carried =
        vec![Position::KEY, Velocity::KEY, Contacts::KEY, Mass::KEY, Health::KEY, Age::KEY];
    expected_carried.sort_unstable();
    assert_eq!(carried, expected_carried);
    for schema in overlay.schemas() {
        assert_eq!(Some(schema), table.schema(schema.component()));
    }

    // B: the row that left is destroyed, the one that arrived is spawned from its baseline.
    let rows_b = world.export_region(B).unwrap();
    let overlay_b = diff(&rows_b, &baseline_b(), &table).unwrap();
    let expected_b = vec![
        Record::SpawnFromBaseline {
            id: c(A, 2),
            components: sorted(vec![data(pos(2, 2)), data(Mass(Scalar::ONE))]),
        },
        Record::Destroy { id: c(B, 1) },
    ];
    assert_eq!(overlay_b.records(), canonical(expected_b).as_slice());
}

#[test]
fn the_records_carry_the_diff_and_the_resources_canonical() {
    let Changed { world, table, gone, .. } = changed();
    let export = RegionExport { region: A, rows: world.export_region(A).unwrap() };
    assert!(export.rows.rows.iter().all(|row| row.id != gone));
    let records = RegionRecords::from_export(&export, &baseline_a(), &table).unwrap();
    let overlay = records.overlay.as_ref().unwrap();
    assert_eq!(overlay.digest(), diff(&export.rows, &baseline_a(), &table).unwrap().digest());
    assert_eq!(records.region, A);
    assert_eq!(records.seed, seed_of(A));
    assert_eq!(records.baseline_hash, baseline_a().content_hash());
    assert_eq!(records.resources, vec![data(Weather(5))]);
    assert_eq!(records.resource_schemas, vec![table.schema(Weather::KEY).unwrap().clone()]);
    assert_eq!(records.transient_resources, vec![Rain::KEY]);
    let mut layouts: Vec<ComponentKey> = overlay.schemas().iter().map(|s| s.component()).collect();
    layouts.push(Weather::KEY);
    layouts.sort_unstable();
    assert_eq!(records.layouts.keys().copied().collect::<Vec<_>>(), layouts);
    for (&key, &digest) in &records.layouts {
        assert_eq!(Some(digest), table.layout(key));
    }

    // The relevance message: the same resources, the overlay in full.
    let message = records.to_relevance(None);
    assert_eq!(message.resources, records.resources);
    assert_eq!(message.transient_resources, records.transient_resources);
    assert_eq!((message.region, message.seed), (A, seed_of(A)));
    assert_eq!(message.baseline_hash, records.baseline_hash);
    match message.overlay {
        OverlayPayload::Full(sent) => assert!(Arc::ptr_eq(&sent, overlay)),
        other => panic!("a changed region is sent in full, not {other:?}"),
    }
}

#[test]
fn the_cooked_state_is_no_overlay_and_a_cooked_message() {
    let registry = Arc::new(registry());
    let table = SchemaTable::from_registry(&registry).unwrap();
    let mut world = WorldBuilder::new(WorldKind::Server, registry).build().unwrap();
    let config = RegionConfig { id: A, seed: 9, mode: RegionMode::Simulated };
    world.create_region(config).unwrap();
    world.load_region(A, &baseline_a(), None).unwrap();
    world.insert_region_resource(A, Weather(1)).unwrap();
    // A transient value written over the baseline's is still the cooked state.
    world.insert(net(c(A, 7)), Contacts(1234)).unwrap();
    let export = RegionExport { region: A, rows: world.export_region(A).unwrap() };

    let overlay = diff(&export.rows, &baseline_a(), &table).unwrap();
    assert!(overlay.records().is_empty());
    assert_eq!(overlay.baseline_hash(), baseline_a().content_hash());
    let records = RegionRecords::from_export(&export, &baseline_a(), &table).unwrap();
    assert!(records.overlay.is_none());
    assert_eq!(records.layouts.keys().copied().collect::<Vec<_>>(), vec![Weather::KEY]);
    let message = records.to_relevance(None);
    assert!(matches!(message.overlay, OverlayPayload::Cooked));
    assert_eq!(message.resources, vec![data(Weather(1))]);
}

/// The save digests of the program at tick 120, under one layout and worker count.
fn digests(layout: lactose_ecs::LayoutConfig, workers: u32) -> BTreeMap<RegionId, Option<u64>> {
    let executor =
        lactose_ecs::Executor::builder().frame_workers(workers).background_workers(1).build();
    let executor = executor.unwrap();
    let mut driver = Driver::default();
    let mut simulation = program(layout);
    for tick in 1..=120 {
        let frame = program_frame(tick, &mut driver);
        simulation = step(&executor, simulation, frame);
        driver.observe(&mut simulation);
    }
    let (_, save) = save_of(&executor, simulation);
    save.regions
        .iter()
        .map(|export| {
            let records = records_of(export);
            (export.region, records.overlay.map(|overlay| overlay.digest()))
        })
        .collect()
}

#[test]
fn the_digest_is_the_same_under_every_layout_and_worker_count() {
    let production = digests(layout_mode(ChunkCapacity::Production, 0).unwrap(), 1);
    assert!(production.len() > 1, "the program loads several regions");
    assert!(production.values().any(Option::is_some), "and changes some of them");
    let capacities = [
        (ChunkCapacity::Rows(1), 11),
        (ChunkCapacity::Rows(7), 12),
        (ChunkCapacity::Rows(64), 13),
        (ChunkCapacity::Bytes(16 * 1024), 14),
    ];
    for (capacity, seed) in capacities {
        for workers in [1, 4] {
            let layout = layout_mode(capacity, seed).unwrap();
            assert_eq!(digests(layout, workers), production, "{capacity:?}, {workers} workers");
        }
    }
    assert_eq!(digests(layout_mode(ChunkCapacity::Production, 0).unwrap(), 4), production);
}

#[test]
fn a_key_the_table_lacks_is_not_found() {
    let Changed { world, .. } = changed();
    let rows = world.export_region(A).unwrap();
    // A table of a registry without `Velocity`: the export carries it.
    let mut lacking = Registry::new();
    lacking.register::<OwnedBy>().unwrap();
    lacking.register::<Position>().unwrap();
    lacking.register::<Mass>().unwrap();
    lacking.register::<Health>().unwrap();
    lacking.register::<Age>().unwrap();
    lacking.register::<Marker>().unwrap();
    lacking.register::<Contacts>().unwrap();
    lacking.register::<Cache>().unwrap();
    let table = SchemaTable::from_registry(&lacking).unwrap();
    let error = diff(&rows, &baseline_a(), &table).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound, "{error}");

    // On the baseline side: an unchanged region whose baseline has a column the table
    // lacks.
    let mut no_cache = Registry::new();
    no_cache.register::<OwnedBy>().unwrap();
    no_cache.register::<Position>().unwrap();
    no_cache.register::<Contacts>().unwrap();
    let table = SchemaTable::from_registry(&no_cache).unwrap();
    let cached = Baseline::from_rows(
        AssetSet::new(),
        cooked_schemas(),
        [BaselineRow { id: c(A, 1), components: vec![data(pos(1, 1)), data(Cache(3))] }],
    )
    .unwrap();
    let error = diff(&RegionRows::default(), &cached, &table).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound, "{error}");
}

#[test]
fn a_resource_among_components_or_a_value_of_another_size_is_invalid() {
    let Changed { world, table, .. } = changed();
    let rows = world.export_region(A).unwrap();

    let mut resource = rows.clone();
    let row = resource.rows.iter_mut().find(|row| row.id == c(A, 6)).unwrap();
    row.components.push(data(Weather(1)));
    row.components.sort_by_key(|data| data.component);
    let error = diff(&resource, &baseline_a(), &table).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");

    let mut short = rows.clone();
    let row = short.rows.iter_mut().find(|row| row.id == c(A, 3)).unwrap();
    let mass = row.components.iter_mut().find(|data| data.component == Mass::KEY).unwrap();
    mass.bytes.pop();
    let error = diff(&short, &baseline_a(), &table).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");

    // A resource of another size beside the rows: `from_export` refuses it.
    let mut export = RegionExport { region: A, rows };
    export.rows.resources[0].bytes.push(0);
    let error = RegionRecords::from_export(&export, &baseline_a(), &table).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
}

#[test]
fn a_spawned_moved_in_or_gained_component_is_written_canonical() {
    let registry = Arc::new(registry());
    let table = SchemaTable::from_registry(&registry).unwrap();
    let mut world = WorldBuilder::new(WorldKind::Server, registry).build().unwrap();
    for (region, baseline) in [(A, baseline_a()), (B, baseline_b())] {
        let config =
            RegionConfig { id: region, seed: seed_of(region), mode: RegionMode::Simulated };
        world.create_region(config).unwrap();
        world.load_region(region, &baseline, None).unwrap();
    }
    let canonical_nan = Scalar::from_canonical_bits(odd_nan().to_canonical_bits());
    let odd = Position { x: odd_nan(), y: Scalar::ONE };
    let canonical = Position { x: canonical_nan, y: Scalar::ONE };
    assert_ne!(data(odd).bytes, data(canonical).bytes);
    // Spawned at runtime, moved in from B, and gained by a baseline row that lacked it.
    let spawned = world.allocate_net_id().unwrap();
    world.spawn(net(spawned), A, (odd,)).unwrap();
    world.insert(net(c(B, 1)), odd).unwrap();
    world.move_to_region(net(c(B, 1)), A).unwrap();
    world.insert(net(c(A, 6)), Velocity { x: odd_nan(), y: Scalar::ONE }).unwrap();
    let rows = world.export_region(A).unwrap();

    let overlay = diff(&rows, &baseline_a(), &table).unwrap();
    let records = overlay.records();
    let spawn = records.iter().find_map(|record| match record {
        Record::Spawn { id, components } if *id == spawned => Some(components),
        _ => None,
    });
    assert_eq!(spawn, Some(&vec![data(canonical)]));
    let moved = records.iter().find_map(|record| match record {
        Record::SpawnFromBaseline { id, components } if *id == c(B, 1) => Some(components),
        _ => None,
    });
    assert_eq!(moved, Some(&sorted(vec![data(canonical), data(Age(3))])));
    let added = records.iter().find_map(|record| match record {
        Record::AddComponent { id, data } if *id == c(A, 6) && data.component == Velocity::KEY => {
            Some(data)
        }
        _ => None,
    });
    assert_eq!(added, Some(&data(Velocity { x: canonical_nan, y: Scalar::ONE })));
}

/// A persistent region resource of scalars, which no type of the shared game is.
#[resource(key = "diff::wind", persistent)]
#[derive(PartialEq, Debug)]
pub struct Wind {
    /// Along x.
    #[field(id = 1)]
    pub x: Scalar,
    /// Along y.
    #[field(id = 2)]
    pub y: Scalar,
}

#[test]
fn a_scalar_resource_is_written_canonical_by_the_save_and_the_load() {
    let mut registry = registry();
    registry.register::<Wind>().unwrap();
    let table = SchemaTable::from_registry(&registry).unwrap();
    let odd = Wind { x: odd_nan(), y: Scalar::from_f64(-0.0) };
    let canonical = Wind {
        x: Scalar::from_canonical_bits(odd_nan().to_canonical_bits()),
        y: Scalar::from_f64(-0.0),
    };
    assert_ne!(data(odd).bytes, data(canonical).bytes);
    let rows = RegionRows { seed: 1, resources: vec![data(odd)], ..RegionRows::default() };
    let export = RegionExport { region: A, rows };
    let empty = Baseline::from_rows(AssetSet::new(), [], []).unwrap();
    let saved = RegionRecords::from_export(&export, &empty, &table).unwrap();
    // The NaN is made canonical; the negative zero is a value and stays.
    assert_eq!(saved.resources, vec![data(canonical)]);

    // Records whose resource is not canonical: the loader writes it canonical.
    let mut odd_records = saved.clone();
    odd_records.resources = vec![data(odd)];
    let migrations = Migrations::new();
    let loaded = Loader::new(&table, &migrations).migrate(&odd_records).unwrap();
    assert_eq!(loaded.resources, vec![data(canonical)]);
}

/// `cooked_schemas()` with `component`'s version moved on: a cook another build made.
fn stale_schemas(component: ComponentKey) -> Vec<ComponentSchema> {
    cooked_schemas()
        .into_iter()
        .map(|schema| {
            if schema.component() != component {
                return schema;
            }
            let fields = schema.fields().iter().copied();
            let version = schema.schema_version() + 1;
            ComponentSchema::new(component, version, schema.element_size(), fields).unwrap()
        })
        .collect()
}

#[test]
fn a_baseline_column_in_another_schema_is_refused_as_stated() {
    let id = c(A, 1);
    let records_over = |baseline: &Baseline| RegionRecords {
        region: A,
        seed: 1,
        baseline_hash: baseline.content_hash(),
        overlay: None,
        resources: Vec::new(),
        resource_schemas: Vec::new(),
        transient_resources: Vec::new(),
        layouts: BTreeMap::new(),
    };
    let migrations = Migrations::new();
    let loader = Loader::new(schemas(), &migrations);
    let export = |components: Vec<ComponentData>| RegionExport {
        region: A,
        rows: RegionRows {
            seed: 1,
            rows: vec![ExportedRow { id, owner: None, components, transient: Vec::new() }],
            ..RegionRows::default()
        },
    };

    // A saved column: the diff cannot compare it byte for byte (`InvalidData`), and the
    // loader refuses the out-of-date cook (`VersionMismatch`).
    let row = BaselineRow { id, components: vec![data(pos(1, 1))] };
    let stale = Arc::new(
        Baseline::from_rows(AssetSet::new(), stale_schemas(Position::KEY), [row]).unwrap(),
    );
    let error =
        RegionRecords::from_export(&export(vec![data(pos(1, 1))]), &stale, schemas()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
    let records = records_over(&stale);
    let error = loader.bundle(&records, stale, RegionMode::Simulated, None).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::VersionMismatch, "{error}");

    // A derived column: the diff never compares it, so it accepts it; the loader still
    // refuses the cook.
    let row = BaselineRow { id, components: vec![data(pos(1, 1)), data(Cache(3))] };
    let stale =
        Arc::new(Baseline::from_rows(AssetSet::new(), stale_schemas(Cache::KEY), [row]).unwrap());
    let saved =
        RegionRecords::from_export(&export(vec![data(pos(1, 1))]), &stale, schemas()).unwrap();
    assert!(saved.overlay.is_none());
    let error =
        loader.bundle(&records_over(&stale), stale, RegionMode::Simulated, None).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::VersionMismatch, "{error}");
}
