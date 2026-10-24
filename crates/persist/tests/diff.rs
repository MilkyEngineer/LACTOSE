//! The diff (README, "The diff"): every change kind gives exactly its records in canonical
//! order, with transient bytes zeroed and scalars canonical; the digest is a function of
//! region state alone, whatever the layout and the worker count; the empty diff is `None`
//! and the relevance message's `Cooked`; what the table cannot account for is refused; and
//! every ownership state a history of legal writes reaches saves, and its records load.

#![forbid(unsafe_code)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::*;
use lactose_asset_api::AssetSet;
use lactose_bundle::{Baseline, BaselineRow, ComponentData, ComponentSchema, Record};
use lactose_core::{ComponentKey, ErrorKind, NetId, Result, StableKey};
use lactose_ecs::{
    ExportedRow, OwnedBy, RegionConfig, RegionId, RegionMode, RegionRows, World, WorldBuilder,
    WorldKind,
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

// ---- Ownership (`crates/ecs/README.md`, Regions, "Ownership is a forest") ----

/// A and B as `common` cooks them, B with an owner inside it and one in A.
fn owner_baselines() -> BTreeMap<RegionId, Arc<Baseline>> {
    let b = [
        BaselineRow { id: c(B, 1), components: vec![data(pos(20, 20)), data(Age(3))] },
        BaselineRow { id: c(B, 2), components: vec![data(pos(21, 21)), data(owned_by(c(B, 1)))] },
        BaselineRow { id: c(B, 3), components: vec![data(pos(22, 22)), data(owned_by(c(A, 3)))] },
    ];
    let b = Baseline::from_rows(AssetSet::new(), cooked_schemas(), b).unwrap();
    [(A, Arc::new(baseline_a())), (B, Arc::new(b))].into_iter().collect()
}

/// A server world of `baselines`' regions, each loaded with no overlay.
fn server_of(baselines: &BTreeMap<RegionId, Arc<Baseline>>) -> World {
    let mut world = bare_world();
    for (&region, baseline) in baselines {
        let config =
            RegionConfig { id: region, seed: seed_of(region), mode: RegionMode::Simulated };
        world.create_region(config).unwrap();
        world.load_region(region, baseline, None).unwrap();
    }
    world
}

/// The records of `region` of `world` against its cooked baseline.
fn save(
    world: &World,
    region: RegionId,
    baselines: &BTreeMap<RegionId, Arc<Baseline>>,
    table: &SchemaTable,
) -> Result<RegionRecords> {
    let export = RegionExport { region, rows: world.export_region(region)? };
    RegionRecords::from_export(&export, &baselines[&region], table)
}

/// Loads `records` into `world` as the sync point does, through `Loader::bundle`.
fn load(
    world: &mut World,
    records: &RegionRecords,
    baselines: &BTreeMap<RegionId, Arc<Baseline>>,
    table: &SchemaTable,
) -> Result<()> {
    let migrations = Migrations::new();
    let baseline = Arc::clone(&baselines[&records.region]);
    let bundle =
        Loader::new(table, &migrations).bundle(records, baseline, RegionMode::Simulated, None)?;
    let config =
        RegionConfig { id: records.region, seed: bundle.seed, mode: RegionMode::Simulated };
    world.create_region(config)?;
    world.load_region(records.region, &bundle.baseline, bundle.overlay.as_deref())?;
    for data in &bundle.resources {
        world.insert_region_resource_erased(records.region, data.clone())?;
    }
    for &key in &bundle.transient_resources {
        world.insert_region_resource_zeroed(records.region, key)?;
    }
    Ok(())
}

/// Panics if some live entity's chain of owners returns to it.
fn assert_forest(world: &World, context: &str) {
    let live: Vec<StableKey> = [A, B]
        .into_iter()
        .filter(|&region| world.region(region).is_some())
        .flat_map(|region| world.entities_in(region))
        .collect();
    for &start in &live {
        let mut at = start;
        for _ in 0..=live.len() {
            let owner = world.get::<OwnedBy>(at).and_then(|owned| owned.owner.get());
            let Some(owner) = owner.and_then(|owner| world.origin(owner)) else { break };
            assert_ne!(owner, start, "{context}: a cycle of owners through {start:?}");
            at = owner;
        }
    }
}

fn records_for(records: &RegionRecords, id: NetId) -> Vec<Record> {
    let all = records.overlay.iter().flat_map(|overlay| overlay.records());
    all.filter(|record| record.id() == id).cloned().collect()
}

fn owner_of(world: &World, key: StableKey) -> Option<StableKey> {
    world.get::<OwnedBy>(key).and_then(|owned| owned.owner.get())
}

/// Each entity of `region` and its owner, live or not.
fn owners_in(world: &World, region: RegionId) -> BTreeMap<StableKey, Option<StableKey>> {
    world.entities_in(region).map(|key| (key, owner_of(world, key))).collect()
}

/// `records` but their owners: what a load that breaks a cycle leaves as it was.
fn but_owners(records: &RegionRecords) -> (Vec<Record>, u64, Vec<ComponentData>) {
    let all = records.overlay.iter().flat_map(|overlay| overlay.records());
    let others = all.filter(|record| !matches!(record, Record::OwnershipChange { .. }));
    (others.cloned().collect(), records.seed, records.resources.clone())
}

#[test]
fn a_cross_region_owner_saves_and_reloads_in_either_order() {
    let baselines = owner_baselines();
    let table = SchemaTable::from_registry(&registry()).unwrap();
    let mut world = server_of(&baselines);
    // A cooked owner that walked to B, and a runtime owner spawned there.
    world.move_to_region(net(c(A, 2)), B).unwrap();
    world.insert(net(c(A, 6)), owned_by(c(A, 2))).unwrap();
    let runtime = world.allocate_net_id().unwrap();
    world.spawn(net(runtime), B, (pos(40, 40),)).unwrap();
    world.insert(net(c(A, 5)), owned_by(runtime)).unwrap();
    assert_eq!(world.region_of(net(c(A, 6))), Some(A), "writing `OwnedBy` moves nothing");

    let saved_a = save(&world, A, &baselines, &table).unwrap();
    assert_eq!(records_for(&saved_a, c(A, 2)), [Record::Destroy { id: c(A, 2) }]);
    let owner = Record::OwnershipChange { id: c(A, 6), owner: Some(c(A, 2)) };
    assert_eq!(records_for(&saved_a, c(A, 6)), [owner]);
    let owner = Record::OwnershipChange { id: c(A, 5), owner: Some(runtime) };
    assert_eq!(records_for(&saved_a, c(A, 5)), [owner]);
    let saved_b = save(&world, B, &baselines, &table).unwrap();

    // The owners' region unloads; what they own stays, and saves the same.
    world.unload_region(B).unwrap();
    assert_eq!(save(&world, A, &baselines, &table).unwrap(), saved_a);

    for order in [[&saved_a, &saved_b], [&saved_b, &saved_a]] {
        let context = format!("order {:?}", order.map(|records| records.region));
        let mut fresh = bare_world();
        for records in order {
            load(&mut fresh, records, &baselines, &table).expect(&context);
        }
        assert_eq!(save(&fresh, A, &baselines, &table).unwrap(), saved_a, "{context}");
        assert_eq!(save(&fresh, B, &baselines, &table).unwrap(), saved_b, "{context}");
        let owned: Vec<StableKey> = fresh.owned_by(net(c(A, 2))).collect();
        assert_eq!(owned, [net(c(A, 6))], "{context}");
        let owned: Vec<StableKey> = fresh.owned_by(net(runtime)).collect();
        assert_eq!(owned, [net(c(A, 5))], "{context}");
    }
}

#[test]
fn what_the_ecs_refuses_never_reaches_the_diff() {
    let baselines = owner_baselines();
    let table = SchemaTable::from_registry(&registry()).unwrap();
    let mut world = server_of(&baselines);
    let cooked = save(&world, A, &baselines, &table).unwrap();

    let error = world.insert(net(c(A, 6)), owned_by(c(A, 6))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
    assert_eq!(save(&world, A, &baselines, &table).unwrap(), cooked);

    world.insert(net(c(A, 6)), owned_by(c(A, 7))).unwrap();
    let owned = save(&world, A, &baselines, &table).unwrap();
    let error = world.insert(net(c(A, 7)), owned_by(c(A, 6))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
    assert_eq!(save(&world, A, &baselines, &table).unwrap(), owned);
    assert_forest(&world, "after the refusals");
}

/// splitmix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

/// One seeded history of spawns, owner writes (refusals ignored), moves, despawns, unloads
/// and reloads over A and B, checked after every step and by a restart at the end. Returns
/// how many owner edges the reloads broke: each a row of the reloaded region left without
/// the owner it had at the unload, that owner live in the other region and owned, through
/// its chain, by the row, so that the edge would close a cycle; nothing else of the region
/// may differ from the records it loaded.
fn history(seed: u64, baselines: &BTreeMap<RegionId, Arc<Baseline>>, table: &SchemaTable) -> usize {
    let mut rng = Rng(seed);
    let mut world = server_of(baselines);
    let loaded = |world: &World| -> Vec<RegionId> {
        [A, B].into_iter().filter(|&region| world.region(region).is_some()).collect()
    };
    let live = |world: &World| -> Vec<StableKey> {
        loaded(world).into_iter().flat_map(|region| world.entities_in(region)).collect()
    };
    // Every key the history may name as an owner: the cooked rows, two never spawned, and
    // every entity it spawns, live or not, as a direct write may name them.
    let mut pool: Vec<NetId> = (1..=8).map(|n| c(A, n)).chain((1..=3).map(|n| c(B, n))).collect();
    pool.push(world.allocate_net_id().unwrap());
    pool.push(world.allocate_net_id().unwrap());
    let mut saved: BTreeMap<RegionId, RegionRecords> = BTreeMap::new();
    let mut owners: BTreeMap<RegionId, BTreeMap<StableKey, Option<StableKey>>> = BTreeMap::new();
    let mut log: Vec<String> = Vec::new();
    let mut broken = 0;
    for _ in 0..20 + rng.below(60) {
        let (keys, regions) = (live(&world), loaded(&world));
        let owner = pool[rng.below(pool.len())];
        match rng.below(100) {
            0..=14 if !regions.is_empty() => {
                let region = regions[rng.below(regions.len())];
                let id = world.allocate_net_id().unwrap();
                pool.push(id);
                if rng.chance(50) {
                    log.push(format!("spawn {id:?} in {region:?} owned by {owner:?}"));
                    let _ = world.spawn(net(id), region, (pos(1, 1), owned_by(owner)));
                } else {
                    log.push(format!("spawn {id:?} in {region:?}"));
                    world.spawn(net(id), region, (pos(1, 1),)).unwrap();
                }
            }
            15..=49 if !keys.is_empty() => {
                let key = keys[rng.below(keys.len())];
                log.push(format!("set {key:?} owned by {owner:?}"));
                let _ = world.insert(key, owned_by(owner));
            }
            50..=54 if !keys.is_empty() => {
                let key = keys[rng.below(keys.len())];
                log.push(format!("clear the owner of {key:?}"));
                world.remove::<OwnedBy>(key).unwrap();
            }
            55..=64 if !keys.is_empty() => {
                let key = keys[rng.below(keys.len())];
                let region = regions[rng.below(regions.len())];
                log.push(format!("move {key:?} to {region:?}"));
                world.move_to_region(key, region).unwrap();
            }
            65..=72 if !keys.is_empty() => {
                let key = keys[rng.below(keys.len())];
                log.push(format!("despawn {key:?}"));
                world.despawn(key).unwrap();
            }
            73..=86 if !regions.is_empty() => {
                let region = regions[rng.below(regions.len())];
                log.push(format!("unload {region:?}"));
                let records = save(&world, region, baselines, table)
                    .unwrap_or_else(|error| panic!("{error}: seed {seed:#x} {log:#?}"));
                owners.insert(region, owners_in(&world, region));
                world.unload_region(region).unwrap();
                saved.insert(region, records);
            }
            87..=99 if !saved.is_empty() => {
                let unloaded: Vec<RegionId> = saved.keys().copied().collect();
                let region = unloaded[rng.below(unloaded.len())];
                log.push(format!("reload {region:?}"));
                let records = saved.remove(&region).unwrap();
                load(&mut world, &records, baselines, table)
                    .unwrap_or_else(|error| panic!("{error}: seed {seed:#x} {log:#?}"));
                let context = format!("seed {seed:#x} {log:#?}");
                let before = owners.remove(&region).unwrap();
                let now = owners_in(&world, region);
                assert!(before.keys().eq(now.keys()), "{context}");
                let mut breaks = 0;
                for (&key, &owner) in &before {
                    if now[&key] == owner {
                        continue;
                    }
                    assert_eq!(now[&key], None, "{context}: {key:?}");
                    let owner = owner.unwrap();
                    let elsewhere = world.region_of(owner).is_some_and(|at| at != region);
                    assert!(elsewhere, "{context}: {key:?} lost {owner:?}");
                    let closes = core::iter::successors(Some(owner), |&at| owner_of(&world, at))
                        .take(live(&world).len() + 1)
                        .any(|at| at == key);
                    assert!(closes, "{context}: {key:?} lost {owner:?}, which no cycle needed");
                    breaks += 1;
                }
                let again = save(&world, region, baselines, table).unwrap();
                if breaks == 0 {
                    assert_eq!(again, records, "{context}");
                } else {
                    assert_eq!(but_owners(&again), but_owners(&records), "{context}");
                }
                broken += breaks;
            }
            _ => continue,
        }
        let context = format!("seed {seed:#x} {log:#?}");
        assert_forest(&world, &context);
        for region in loaded(&world) {
            if let Err(error) = save(&world, region, baselines, table) {
                panic!("{error}: {context}");
            }
        }
    }

    // A restart from the saves in either order, then a second round trip of that world's
    // own records (saved, loaded fresh in the same order, saved), which is exact.
    for region in loaded(&world) {
        saved.insert(region, save(&world, region, baselines, table).unwrap());
    }
    for order in [[A, B], [B, A]] {
        let context = format!("seed {seed:#x} restart {order:?} {log:#?}");
        let mut fresh = bare_world();
        for region in order {
            load(&mut fresh, &saved[&region], baselines, table).expect(&context);
        }
        assert_forest(&fresh, &context);
        let first = order.map(|region| save(&fresh, region, baselines, table).expect(&context));
        let mut again = bare_world();
        for records in &first {
            load(&mut again, records, baselines, table).expect(&context);
        }
        let second = order.map(|region| save(&again, region, baselines, table).expect(&context));
        assert_eq!(second, first, "{context}");
    }
    broken
}

/// What every save, unload and relevance message rests on: whatever a history of legal
/// writes leaves, `from_export` succeeds and its records load, the world staying a forest. A
/// cycle that two regions' documents close is broken by the load (logged at `warn`, target
/// `casein`); the breaks are counted, here from the world's owners before the unload and
/// after the reload rather than from the log, and the seeds are fixed so that some history
/// reaches one.
#[test]
fn every_reachable_state_saves() {
    let baselines = owner_baselines();
    let table = SchemaTable::from_registry(&registry()).unwrap();
    let histories = 200_u64;
    let broken: usize = (0..histories)
        .map(|i| history(i.wrapping_mul(0x2545_F491_4F6C_DD1D), &baselines, &table))
        .sum();
    println!("{histories} histories, {broken} owner edges broken by a reload");
    assert!(broken > 0, "no history closed a cycle across the two regions");
}
