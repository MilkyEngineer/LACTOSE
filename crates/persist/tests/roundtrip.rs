//! Round trip (README, "Schemas", "The container"): the diff's records load back through
//! the ECS's loader into a world that exports the same rows; the table's schemas are the
//! ones the ECS loader accepts, for nested structs, arrays, enums and padding; a container
//! decodes to itself, section by section from its index too, and assembles to the same
//! bytes whatever order its sections were made in.

#![forbid(unsafe_code)]

mod common;

use std::sync::Arc;

use common::*;
use lactose_asset_api::AssetSet;
use lactose_bundle::{
    Baseline, BaselineRow, ComponentData, ComponentSchema, FieldLayout, Overlay, Record,
};
use lactose_core::{FieldId, NetId, PlainArray, StableKey};
use lactose_ecs::{
    ClaimedBy, InRegion, OwnedBy, PredictedOrigin, RegionConfig, RegionId, RegionMode,
    WorldBuilder, WorldKind,
};
use lactose_math::Scalar;
use lactose_persist::{
    Container, ContainerHeader, FieldValues, Loader, Migration, Migrations, PREAMBLE_BYTES,
    RegionRecords, SchemaTable,
};
use lactose_reflect::{Padding, Reflect, Registry, TypeInfo, component, resource, value};
use lactose_sim::RegionExport;

/// The bundle of `records` through the loader of `table`.
fn bundle_with(
    table: &SchemaTable,
    records: &RegionRecords,
    baseline: Arc<Baseline>,
) -> lactose_sim::RegionBundle {
    let migrations = Migrations::new();
    let loader = Loader::new(table, &migrations);
    loader.bundle(records, baseline, RegionMode::Simulated, None).unwrap()
}

#[test]
fn every_change_kind_loads_back_to_the_same_records() {
    let Changed { world, table, .. } = changed();
    let mut loaded = bare_world();
    for (region, baseline) in [(A, baseline_a()), (B, baseline_b())] {
        let export = RegionExport { region, rows: world.export_region(region).unwrap() };
        let records = RegionRecords::from_export(&export, &baseline, &table).unwrap();
        let bundle = bundle_with(&table, &records, Arc::new(baseline.clone()));
        load_into(&mut loaded, region, &bundle);
        let again = RegionExport { region, rows: loaded.export_region(region).unwrap() };
        // The records agree, and so does every row's key, owner and transient presence.
        assert_eq!(RegionRecords::from_export(&again, &baseline, &table).unwrap(), records);
        assert_eq!(again.rows.seed, export.rows.seed);
        assert_eq!(again.rows.resources, export.rows.resources);
        assert_eq!(again.rows.transient_resources, export.rows.transient_resources);
        let ids = |rows: &lactose_ecs::RegionRows| -> Vec<_> {
            rows.rows.iter().map(|row| (row.id, row.owner, row.transient.clone())).collect()
        };
        assert_eq!(ids(&again.rows), ids(&export.rows));
    }
}

#[test]
fn a_saved_program_loads_back_to_the_same_export() {
    let save = saved_at(160);
    assert!(save.regions.len() > 2);
    let mut loaded = bare_world();
    for export in &save.regions {
        let records = records_of(export);
        let bundle = bundle_of(&records, None);
        load_into(&mut loaded, export.region, &bundle);
        assert_eq!(
            loaded.export_region(export.region).unwrap(),
            export.rows,
            "{:?}",
            export.region
        );
    }
}

// ---- A registry with nested structs, arrays, enums and padding ----

/// A mood, stored as a byte.
#[value]
#[repr(u8)]
#[derive(PartialEq, Eq, Debug)]
pub enum Mood {
    /// Calm.
    Calm = 0,
    /// Angry.
    Angry = 5,
}

/// A nested struct with padding.
#[value]
#[derive(PartialEq, Debug)]
pub struct Hit {
    /// When.
    #[field(id = 1)]
    pub at: i64,
    /// How hard.
    #[field(id = 2)]
    pub amount: u16,
    /// The mood it left.
    #[field(id = 3)]
    pub mood: Mood,
    #[field(padding)]
    _pad: Padding<5>,
}

/// A nested scalar, so canonicalisation reaches inside an array of structs.
#[value]
#[derive(PartialEq, Debug)]
pub struct Speed {
    /// The value.
    #[field(id = 1)]
    pub value: Scalar,
}

/// Every shape a field may take.
#[component(key = "roundtrip::gear", persistent)]
#[derive(PartialEq, Debug)]
pub struct Gear {
    /// An array of nested structs.
    #[field(id = 1)]
    pub hits: PlainArray<Hit, 2>,
    /// An array of structs holding a scalar.
    #[field(id = 2)]
    pub speeds: PlainArray<Speed, 2>,
    /// An array of enums.
    #[field(id = 3)]
    pub moods: PlainArray<Mood, 3>,
    /// A byte.
    #[field(id = 4)]
    pub level: u8,
    #[field(padding)]
    _pad: Padding<4>,
}

/// A resource with padding.
#[resource(key = "roundtrip::stock", persistent)]
#[derive(PartialEq, Debug)]
pub struct Stock {
    /// How many.
    #[field(id = 1)]
    pub count: u32,
    /// Its mood.
    #[field(id = 2)]
    pub mood: Mood,
    #[field(padding)]
    _pad: Padding<3>,
}

fn shapes() -> Registry {
    let mut registry = Registry::new();
    registry.register::<InRegion>().unwrap();
    registry.register::<OwnedBy>().unwrap();
    registry.register::<ClaimedBy>().unwrap();
    registry.register::<PredictedOrigin>().unwrap();
    registry.register::<Gear>().unwrap();
    registry.register::<Stock>().unwrap();
    registry
}

fn hit(at: i64, mood: Mood) -> Hit {
    Hit { at, amount: 3, mood, _pad: Padding::default() }
}

fn gear(level: u8, speed: Scalar) -> Gear {
    Gear {
        hits: PlainArray([hit(1, Mood::Calm), hit(-2, Mood::Angry)]),
        speeds: PlainArray([Speed { value: Scalar::ONE }, Speed { value: speed }]),
        moods: PlainArray([Mood::Angry, Mood::Calm, Mood::Angry]),
        level,
        _pad: Padding::default(),
    }
}

/// The document schema of `info`, the ECS's one derivation, which its loader compares against.
fn ecs_schema(info: &TypeInfo) -> ComponentSchema {
    lactose_ecs::document_schema(info).unwrap()
}

#[test]
fn the_tables_schemas_are_the_ones_the_loader_accepts() {
    let registry = Arc::new(shapes());
    let table = SchemaTable::from_registry(&registry).unwrap();
    for info in registry.iter() {
        if let Some(schema) = table.schema(info.key()) {
            assert_eq!(schema, &ecs_schema(info), "{}", info.name());
        }
    }
    let gear_schema = table.schema(Gear::KEY).unwrap();
    // Padding is not a field of the schema; a nested struct or array is one field.
    let ids: Vec<u16> = gear_schema.fields().iter().map(|field| field.id.0).collect();
    assert_eq!(ids, vec![1, 2, 3, 4]);
    assert_eq!(gear_schema.element_size() as usize, size_of::<Gear>());

    // A cooked row of the type, a runtime row with a scalar nested in an array, a resource.
    let region = RegionId(7);
    let cooked_row =
        BaselineRow { id: cooked(region, 1), components: vec![data(gear(1, Scalar::TWO))] };
    let schemas = registry.iter().map(ecs_schema);
    let baseline = Arc::new(Baseline::from_rows(AssetSet::new(), schemas, [cooked_row]).unwrap());
    let mut world = WorldBuilder::new(WorldKind::Server, Arc::clone(&registry)).build().unwrap();
    let config = RegionConfig { id: region, seed: 3, mode: RegionMode::Simulated };
    world.create_region(config).unwrap();
    world.load_region(region, &baseline, None).unwrap();
    let id = world.allocate_net_id().unwrap();
    world.spawn(StableKey::Net(id), region, (gear(9, Scalar::from_i32(-9)),)).unwrap();
    world.insert(StableKey::Net(cooked(region, 1)), gear(2, Scalar::ONE)).unwrap();
    let stock = Stock { count: 4, mood: Mood::Angry, _pad: Padding::default() };
    world.insert_region_resource(region, stock).unwrap();
    let export = RegionExport { region, rows: world.export_region(region).unwrap() };
    let records = RegionRecords::from_export(&export, &baseline, &table).unwrap();

    // The overlay and the resources load under the table's schemas.
    let bundle = bundle_with(&table, &records, Arc::clone(&baseline));
    let mut loaded = WorldBuilder::new(WorldKind::Server, Arc::clone(&registry)).build().unwrap();
    loaded.create_region(RegionConfig { id: region, seed: bundle.seed, ..config }).unwrap();
    loaded.load_region(region, &bundle.baseline, bundle.overlay.as_deref()).unwrap();
    for data in &bundle.resources {
        loaded.insert_region_resource_erased(region, data.clone()).unwrap();
    }
    let again = RegionExport { region, rows: loaded.export_region(region).unwrap() };
    assert_eq!(RegionRecords::from_export(&again, &baseline, &table).unwrap(), records);
    assert_eq!(loaded.region_resource::<Stock>(region), Some(&stock_of(4)));

    // Decoded records saved under the type's previous version whose runtime row holds a
    // NaN of odd bits nested in the array: the loader's step writes it canonical, and a
    // debug build asserts nothing, since a file is not a live world (architecture section
    // 8.2). The world's own export is read, not saved.
    let odd_nan = Scalar::from_f64(f64::from_bits(0x7FF8_0000_0000_0042));
    let overlay = records.overlay.as_deref().unwrap();
    let decoded: Vec<Record> = overlay
        .records()
        .iter()
        .map(|record| match record {
            Record::Spawn { id: spawned, .. } if *spawned == id => {
                Record::Spawn { id, components: vec![data(gear(9, odd_nan))] }
            }
            other => other.clone(),
        })
        .collect();
    let from = gear_schema.schema_version() - 1;
    let previous = |schema: &ComponentSchema| {
        if schema.component() != Gear::KEY {
            return schema.clone();
        }
        let fields = schema.fields().iter().copied();
        let (size, digest) = (schema.element_size(), schema.layout_digest());
        ComponentSchema::new(Gear::KEY, from, size, digest, fields).unwrap()
    };
    let schemas = overlay.schemas().iter().map(previous);
    let mut odd = records.clone();
    let rebuilt = Overlay::new(overlay.baseline_hash(), schemas, decoded);
    odd.overlay = Some(Arc::new(rebuilt.unwrap()));
    let mut migrations = Migrations::new();
    migrations.register(Migration { component: Gear::KEY, from, run: unchanged }).unwrap();
    let loader = Loader::new(&table, &migrations);
    let bundle = loader.bundle(&odd, Arc::clone(&baseline), RegionMode::Simulated, None).unwrap();
    let mut loaded = WorldBuilder::new(WorldKind::Server, Arc::clone(&registry)).build().unwrap();
    loaded.create_region(RegionConfig { id: region, seed: bundle.seed, ..config }).unwrap();
    loaded.load_region(region, &bundle.baseline, bundle.overlay.as_deref()).unwrap();
    let rows = loaded.export_region(region).unwrap();
    let canonical = Scalar::from_canonical_bits(odd_nan.to_canonical_bits());
    let row = rows.rows.iter().find(|row| row.id == id).unwrap();
    assert_eq!(row.components, vec![data(gear(9, canonical))]);
}

/// A step that changes nothing.
fn unchanged(_: &mut FieldValues) -> lactose_core::Result<()> {
    Ok(())
}

fn stock_of(count: u32) -> Stock {
    Stock { count, mood: Mood::Angry, _pad: Padding::default() }
}

// ---- The container ----

fn container_at(ticks: u32) -> Container {
    let save = saved_at(ticks);
    let mut container = Container::new(ContainerHeader::of(&save, schemas()));
    for export in &save.regions {
        assert_eq!(container.insert(records_of(export)).unwrap(), None);
    }
    container
}

#[test]
fn a_container_decodes_to_itself() {
    let container = container_at(160);
    assert!(container.len() > 2);
    assert!(container.regions().any(|records| records.overlay.is_some()));
    let bytes = container.encode().unwrap();
    assert_eq!(Container::decode(&bytes).unwrap(), container);
    // Equal inputs give equal bytes.
    assert_eq!(container.encode().unwrap(), bytes);
    // An empty container too.
    let empty = Container::new(*container.header());
    assert_eq!(Container::decode(&empty.encode().unwrap()).unwrap(), empty);
}

#[test]
fn the_index_and_each_section_agree_with_decode() {
    let container = container_at(160);
    let bytes = container.encode().unwrap();
    let len = Container::index_len(&bytes[..PREAMBLE_BYTES]).unwrap();
    let index = Container::index(&bytes[..len]).unwrap();
    assert_eq!(index, Container::index(&bytes).unwrap());
    assert_eq!(index.header, *container.header());
    let regions: Vec<RegionId> = index.regions.iter().map(|entry| entry.region).collect();
    let expected: Vec<RegionId> = container.regions().map(|records| records.region).collect();
    assert_eq!(regions, expected);
    for entry in &index.regions {
        let start = entry.offset as usize;
        let section = &bytes[start..start + entry.compressed_bytes as usize];
        let records = Container::decode_section(entry, section).unwrap();
        assert_eq!(Some(&records), container.region(entry.region));
        assert_eq!(Container::decode_region(&bytes, entry.region).unwrap(), records);
        assert_eq!(index.entry(entry.region), Some(entry));
        assert_eq!(entry.seed, records.seed);
        assert_eq!(entry.baseline_hash, records.baseline_hash);
        assert_eq!(entry.overlay_digest, records.overlay.as_ref().map(|o| o.digest()));
    }
    // The sections follow the block back to back, in index order.
    let mut at = len as u64;
    for entry in &index.regions {
        assert_eq!(entry.offset, at);
        at += entry.compressed_bytes;
    }
    assert_eq!(at as usize, bytes.len());
}

#[test]
fn sections_made_in_any_order_assemble_to_the_same_bytes() {
    let container = container_at(160);
    let bytes = container.encode().unwrap();
    let sections: Vec<_> =
        container.regions().map(|records| records.encode_section().unwrap()).collect();
    let header = container.header();
    let mut reversed = sections.clone();
    reversed.reverse();
    assert_eq!(Container::assemble(header, reversed).unwrap(), bytes);
    let mut rotated = sections.clone();
    rotated.rotate_left(sections.len() / 2);
    assert_eq!(Container::assemble(header, rotated).unwrap(), bytes);
    // Offsets are assigned by the assembly; a section's entry starts unplaced.
    assert!(sections.iter().all(|section| section.entry().offset == 0));
    // A section's entry states what its records do.
    for (section, records) in sections.iter().zip(container.regions()) {
        let entry = section.entry();
        assert_eq!(entry.region, records.region);
        assert_eq!(entry.compressed_bytes as usize, section.bytes().len());
    }
}

/// A persistent component with a zero-length array field, which no document schema can
/// state.
#[component(key = "roundtrip::zero_array", persistent)]
pub struct ZeroArray {
    /// A field of four bytes.
    #[field(id = 1)]
    pub count: u32,
    /// The field of no bytes.
    #[field(id = 2)]
    pub none: PlainArray<u8, 0>,
}

/// A derived component with the same field.
#[component(key = "roundtrip::zero_derived", derived)]
pub struct ZeroDerived {
    /// A field of four bytes.
    #[field(id = 1)]
    pub count: u32,
    /// The field of no bytes.
    #[field(id = 2)]
    pub none: PlainArray<u8, 0>,
}

/// The world's own types and `Position`.
fn engine_registry() -> Registry {
    let mut registry = Registry::new();
    registry.register::<InRegion>().unwrap();
    registry.register::<OwnedBy>().unwrap();
    registry.register::<ClaimedBy>().unwrap();
    registry.register::<PredictedOrigin>().unwrap();
    registry.register::<Position>().unwrap();
    registry
}

#[test]
fn a_zero_size_field_is_refused_at_registration_so_no_table_or_load_meets_it() {
    // Registration refuses both, naming the type and the field, and the registry is as it
    // was: the table builds, where before it refused the whole game naming no type.
    let mut registry = engine_registry();
    for (error, name) in [
        (registry.register::<ZeroArray>().unwrap_err(), "`roundtrip::zero_array`: field `none`"),
        (
            registry.register::<ZeroDerived>().unwrap_err(),
            "`roundtrip::zero_derived`: field `none`",
        ),
    ] {
        assert_eq!(error.kind(), lactose_core::ErrorKind::InvalidData, "{error}");
        assert!(error.message().contains(name), "`{error}` names `{name}`");
    }
    assert_eq!(registry.len(), engine_registry().len());
    SchemaTable::from_registry(&registry).unwrap();

    // A baseline that names the derived type's key under a well-formed schema, as any
    // document can: the world, which cannot have registered the type, refuses the region
    // with an error instead of panicking on the registry's schema of it.
    let schema = ComponentSchema::new(
        ZeroDerived::KEY,
        1,
        4,
        0,
        [FieldLayout { id: FieldId(1), offset: 0, size: 4 }],
    )
    .unwrap();
    let position = Position::KEY;
    let position = schemas().schema(position).unwrap().clone();
    let rows = [BaselineRow {
        id: NetId::cooked(1),
        components: vec![
            data(pos(1, 1)),
            ComponentData { component: ZeroDerived::KEY, bytes: vec![0; 4] },
        ],
    }];
    let baseline = Baseline::from_rows(AssetSet::new(), [schema, position], rows).unwrap();
    let baseline = Baseline::decode(&baseline.encode()).unwrap();
    let mut world = WorldBuilder::new(WorldKind::Server, Arc::new(registry)).build().unwrap();
    let region = RegionId(1);
    let config = RegionConfig { id: region, seed: 1, mode: RegionMode::Simulated };
    world.create_region(config).unwrap();
    let error = world.load_region(region, &baseline, None).unwrap_err();
    assert_eq!(error.kind(), lactose_core::ErrorKind::NotFound, "{error}");
    assert!(error.message().contains("is not registered"), "{error}");
}
