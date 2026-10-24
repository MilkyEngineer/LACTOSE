//! Migrations (README, "Migrations" and "The registration a game crate writes"): a save
//! written under an older schema version loads through the steps a game crate registers and
//! hashes as one saved at the new version; each refusal the rule states refuses the region
//! before anything loads; a transient, a newly persistent and a newly derived type each load
//! by the rule of their persistence now.
//!
//! Two builds of one type are two Rust types with one key, each in its own module, and a
//! build is the schema table of a registry holding one of them.

#![forbid(unsafe_code)]

mod common;
mod variants;

use std::num::NonZeroU64;
use std::sync::Arc;

use common::{
    A, INTEGRATE, Inputs, Position, Sessions, Weather, data, executor, first_table, load_into, net,
    pos, register_systems, seed_of, setup, step,
};
use lactose_asset_api::AssetSet;
use lactose_bundle::{Baseline, BaselineRow, ComponentData, Overlay, Record};
use lactose_core::{ComponentKey, Error, ErrorKind, FieldId, NetId, Result, StableKey, Tick};
use lactose_ecs::{
    ClaimedBy, Commands, InRegion, OwnedBy, PredictedOrigin, RegionConfig, RegionId, RegionMode,
    StableKeyOf, Without, World, WorldBuilder, WorldKind, system,
};
use lactose_persist::{
    CONTAINER_FORMAT_VERSION, Container, ContainerHeader, EngineVersion, FieldValues, Loader,
    MigrateFn, Migration, Migrations, RegionRecords, SchemaTable,
};
use lactose_reflect::{Component, Padding, Reflect, Registry, component, resource, value};
use lactose_sim::{
    Hashers, RegionBundle, RegionExport, SimulationBuilder, Stage, StreamingAnchor, TickInput,
    TypeTable,
};

// ---- The README's example: `Health` from version 1 to version 2 ----

/// Version 1: the game's `Health`, `i32` points under field 1.
mod v1 {
    pub use super::common::Health;
}

/// Version 2, as the README's example declares it.
mod v2 {
    use super::*;

    #[component(key = "game::health", persistent, replicated, version = 2, retired = [1])]
    #[derive(PartialEq, Debug)]
    pub struct Health {
        #[field(id = 2)]
        pub points: u16,
        #[field(id = 3)]
        pub max: u16,
    }
}

/// Version 3: a field added after the README's version 2.
mod v3 {
    use super::*;

    #[component(key = "game::health", persistent, replicated, version = 3, retired = [1])]
    pub struct Health {
        #[field(id = 2)]
        pub points: u16,
        #[field(id = 3)]
        pub max: u16,
        #[field(id = 4)]
        pub regen: u16,
        #[field(padding)]
        _pad: Padding<2>,
    }
}

/// A version 2 with a field whose zero is valid and whose other values mostly are not.
mod moody {
    use super::*;

    #[value]
    #[repr(u8)]
    pub enum Mood {
        Calm = 0,
        Cross = 1,
    }

    #[component(key = "game::health", persistent, replicated, version = 2, retired = [1])]
    pub struct Health {
        #[field(id = 2)]
        pub points: u16,
        #[field(id = 3)]
        pub mood: Mood,
        #[field(padding)]
        _pad: Padding<1>,
    }
}

/// The README's step, verbatim.
fn health_v1_to_v2(value: &mut FieldValues) -> Result<()> {
    let points: i32 = value.read(FieldId(1))?;
    value.take(FieldId(1)); // a step takes what it retires
    let points = u16::try_from(points.max(0)).unwrap_or(u16::MAX);
    value.write(FieldId(2), points);
    value.write(FieldId(3), points);
    Ok(())
}

/// Reads a field only version 2 has, so the chain holds only in version order.
fn health_v2_to_v3(value: &mut FieldValues) -> Result<()> {
    let max: u16 = value.read(FieldId(3))?;
    value.write(FieldId(4), max / 20);
    Ok(())
}

/// The README's step with its `take` forgotten.
fn keeps_field_1(value: &mut FieldValues) -> Result<()> {
    let points: i32 = value.read(FieldId(1))?;
    value.write(FieldId(2), points as u16);
    value.write(FieldId(3), points as u16);
    Ok(())
}

/// The README's step that never writes the maximum.
fn forgets_max(value: &mut FieldValues) -> Result<()> {
    let points: i32 = value.read(FieldId(1))?;
    value.take(FieldId(1));
    value.write(FieldId(2), points as u16);
    Ok(())
}

/// The README's step writing the points four bytes wide.
fn writes_wide(value: &mut FieldValues) -> Result<()> {
    let points: i32 = value.read(FieldId(1))?;
    value.take(FieldId(1));
    value.write(FieldId(2), points as u32);
    value.write(FieldId(3), points as u16);
    Ok(())
}

/// A step to `moody::Health` writing a mood no variant has.
fn crosses_wrongly(value: &mut FieldValues) -> Result<()> {
    let points: i32 = value.read(FieldId(1))?;
    value.take(FieldId(1));
    value.write(FieldId(2), points as u16);
    value.write(FieldId(3), 7_u8);
    Ok(())
}

/// A step that refuses on its own terms.
fn refuses(_: &mut FieldValues) -> Result<()> {
    Err(Error::new(ErrorKind::Unsupported, "this save predates the guild system"))
}

fn with(steps: &[(u32, MigrateFn)]) -> Migrations {
    let mut migrations = Migrations::new();
    for &(from, run) in steps {
        migrations.register(Migration { component: v1::Health::KEY, from, run }).unwrap();
    }
    migrations
}

/// The region the health saves are of.
const HOME: RegionId = RegionId(7);

/// One build of the health game: the world's own types, `Position`, one `Health` and
/// `Weather`.
struct Build {
    registry: Arc<Registry>,
    table: SchemaTable,
    hashers: Hashers,
}

fn build<H: Reflect>() -> Build {
    let mut registry = Registry::new();
    registry.register::<InRegion>().unwrap();
    registry.register::<OwnedBy>().unwrap();
    registry.register::<ClaimedBy>().unwrap();
    registry.register::<PredictedOrigin>().unwrap();
    registry.register::<StreamingAnchor>().unwrap();
    registry.register::<Position>().unwrap();
    registry.register::<H>().unwrap();
    registry.register::<Weather>().unwrap();
    let mut types = TypeTable::new();
    types.register::<Position>().unwrap();
    types.register::<H>().unwrap();
    types.register::<Weather>().unwrap();
    let table = SchemaTable::from_registry(&registry).unwrap();
    Build { registry: Arc::new(registry), table, hashers: types.hashers().clone() }
}

impl Build {
    fn world(&self) -> World {
        WorldBuilder::new(WorldKind::Server, Arc::clone(&self.registry)).build().unwrap()
    }

    fn loader<'a>(&'a self, migrations: &'a Migrations) -> Loader<'a> {
        Loader::new(&self.table, migrations)
    }
}

/// The home region's baseline, the same under every build: two cooked rows with a
/// position and no health.
fn home_baseline() -> Arc<Baseline> {
    let rows = [
        BaselineRow { id: NetId::cooked(1), components: vec![data(pos(1, 1))] },
        BaselineRow { id: NetId::cooked(2), components: vec![data(pos(2, 2))] },
    ];
    let schemas = [common::schemas().schema(Position::KEY).unwrap().clone()];
    Arc::new(Baseline::from_rows(AssetSet::new(), schemas, rows).unwrap())
}

/// The home region played under one build: a health added to a cooked row, two entities
/// spawned with one each, and the weather.
fn home_world<H: Component>(build: &Build, [added, low, high]: [H; 3]) -> World {
    let mut world = build.world();
    let config = RegionConfig { id: HOME, seed: 0x5eed, mode: RegionMode::Simulated };
    world.create_region(config).unwrap();
    world.load_region(HOME, &home_baseline(), None).unwrap();
    world.insert(net(NetId::cooked(1)), added).unwrap();
    for (at, health) in [(3, low), (4, high)] {
        let id = world.allocate_net_id().unwrap();
        world.spawn(net(id), HOME, (pos(at, at), health)).unwrap();
    }
    world.insert_region_resource(HOME, Weather(4)).unwrap();
    world
}

fn v1_world(build: &Build) -> World {
    home_world(build, [v1::Health(40), v1::Health(-5), v1::Health(70_000)])
}

fn v2_world(build: &Build) -> World {
    let health = |points| v2::Health { points, max: points };
    home_world(build, [health(40), health(0), health(u16::MAX)])
}

fn records_of(build: &Build, world: &World) -> RegionRecords {
    let export = RegionExport { region: HOME, rows: world.export_region(HOME).unwrap() };
    RegionRecords::from_export(&export, &home_baseline(), &build.table).unwrap()
}

/// The container a version 1 build writes of [`v1_world`].
fn v1_container(build: &Build, world: &World) -> Vec<u8> {
    let header = ContainerHeader {
        format: CONTAINER_FORMAT_VERSION,
        engine: EngineVersion { major: 1, minor: 0, patch: 0 },
        schema_hash: build.table.hash(),
        tick: Tick(12),
        next_net_id: world.next_net_id(),
    };
    let mut container = Container::new(header);
    container.insert(records_of(build, world)).unwrap();
    container.encode().unwrap()
}

/// The container a version 1 build wrote, pinned: the records of [`v1_world`] (a health
/// of 40 added to a cooked row, -5 and 70000 spawned) under the version 1 table. The
/// schema hash and the zstd frame make these bytes move with the world's own types and
/// with zstd (pinned by `Cargo.lock`); the test below says which.
const V1_CONTAINER: &str = concat!(
    // The preamble: magic, format 1, zero, a block of 103 bytes.
    "4c534156",
    "0100",
    "0000",
    "6700000000000000",
    // The header: engine 1.0.0, the version 1 table's schema hash, tick 12, counter 3, one
    // entry.
    "010000000000",
    "60433682e3f17fab",
    "0c000000",
    "0300000000000000",
    "01000000",
    // The entry: region 7, seed, baseline hash, an overlay and its digest, offset 119, 201
    // bytes compressed, 356 decompressed, the section's hash.
    "0700000000000000",
    "ed5e000000000000",
    "82c9cd95f07eb7c6",
    "01",
    "b086fae8876c1e43",
    "7700000000000000",
    "c900000000000000",
    "6401000000000000",
    "3dde686148938735",
    // The block's hash.
    "369430b684ab52b8",
    // The section: one zstd frame.
    "28b52ffd606400fd050024070700ed5e0082c9cd95f07eb7c601dd004c4f564c0102000000c6d01cb2c7",
    "c78c05100000000000082e56cab3f382b2fe04030100000840fbffffff01101070110100040180280000",
    "00b086fae8876c1e43d16d465f758a0a3203202489c6470340927cc96c20349a22170f0a5ff9b7402091",
    "23208002a2346e2d849f4f5820c660186d21421000b5f22f24865c46554e19d3a97d1b54311226d13c80",
    "d6057c437b2e8ea10d1ba7bc0fc6c7221888eab90bee25fb4a7cd4a2b70d6c6201",
);

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(hex: &str) -> Vec<u8> {
    (0..hex.len()).step_by(2).map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap()).collect()
}

/// The pinned version 1 records.
fn v1_records() -> RegionRecords {
    let container = Container::decode(&unhex(V1_CONTAINER)).unwrap();
    container.region(HOME).unwrap().clone()
}

fn load(build: &Build, migrations: &Migrations, records: &RegionRecords) -> Result<RegionBundle> {
    let bundle =
        build.loader(migrations).bundle(records, home_baseline(), RegionMode::Simulated, None);
    if let Err(error) = &bundle {
        // A refusal is the loader's whole answer: `migrate` refuses the same records the
        // same way, unless the refusal is the bundle's own check against the baseline.
        if let Err(migrated) = build.loader(migrations).migrate(records) {
            assert_eq!(migrated.kind(), error.kind(), "{migrated} / {error}");
        }
    }
    bundle
}

fn loaded(build: &Build, bundle: &RegionBundle) -> World {
    let mut world = build.world();
    load_into(&mut world, HOME, bundle);
    world
}

#[test]
fn the_v1_container_is_what_a_v1_build_writes() {
    let build = build::<v1::Health>();
    let written = v1_container(&build, &v1_world(&build));
    assert_eq!(
        hex(&written),
        V1_CONTAINER,
        "a change to the world's own types moves the schema hash at offset 22, a zstd \
         upgrade moves the frame: re-pin; a change to the records is a format change"
    );
    let records = v1_records();
    assert_eq!(
        records.overlay.as_ref().unwrap().schema(v1::Health::KEY).unwrap().schema_version(),
        1
    );
}

#[test]
fn a_v1_save_loads_through_the_readme_step_and_hashes_as_a_v2_save() {
    let old = v1_records();
    let new = build::<v2::Health>();
    let migrations = with(&[(1, health_v1_to_v2)]);

    // The records migrate to exactly what a version 2 build saves of the same play.
    let native = v2_world(&new);
    let saved = records_of(&new, &native);
    assert_eq!(new.loader(&migrations).migrate(&old).unwrap(), saved);
    // A version 2 save under the same loader is the fast path: unchanged, the same `Arc`.
    let again = new.loader(&migrations).migrate(&saved).unwrap();
    assert!(Arc::ptr_eq(again.overlay.as_ref().unwrap(), saved.overlay.as_ref().unwrap()));

    // And the world loaded from the version 1 container hashes as the version 2 world
    // does, and as that world saved and loaded does.
    let from_v1 = loaded(&new, &load(&new, &migrations, &old).unwrap());
    let from_v2 = loaded(&new, &load(&new, &migrations, &saved).unwrap());
    let hash = new.hashers.world_hash(&native, HOME).unwrap();
    assert_eq!(new.hashers.world_hash(&from_v1, HOME).unwrap(), hash);
    assert_eq!(new.hashers.world_hash(&from_v2, HOME).unwrap(), hash);
    let spawned = NetId::from_bits(2).unwrap();
    assert_eq!(
        from_v1.get::<v2::Health>(net(spawned)),
        Some(&v2::Health { points: u16::MAX, max: u16::MAX })
    );

    // Through two steps, in version order.
    let newer = build::<v3::Health>();
    let migrations = with(&[(2, health_v2_to_v3), (1, health_v1_to_v2)]);
    let world = loaded(&newer, &load(&newer, &migrations, &old).unwrap());
    let health = world.get::<v3::Health>(net(NetId::cooked(1))).unwrap();
    assert_eq!((health.points, health.max, health.regen), (40, 40, 2));
}

/// Loads `records` under `build` and `migrations`, which must refuse with `kind` and a
/// message naming `named`.
fn refused(
    what: &str,
    build: &Build,
    migrations: &Migrations,
    records: &RegionRecords,
    kind: ErrorKind,
    named: &str,
) {
    let error = load(build, migrations, records).unwrap_err();
    assert_eq!(error.kind(), kind, "{what}: {error}");
    assert!(error.to_string().contains(named), "{what}: `{error}` names `{named}`");
}

#[test]
fn each_refusal_of_the_step_path_refuses_the_region() {
    let old = v1_records();
    let new = build::<v2::Health>();
    let v1_build = build::<v1::Health>();
    let saved_v2 = records_of(&new, &v2_world(&new));
    let mismatch = ErrorKind::VersionMismatch;
    refused("a newer save", &v1_build, &Migrations::new(), &saved_v2, mismatch, "newer");
    refused(
        "a missing step",
        &new,
        &Migrations::new(),
        &old,
        mismatch,
        "no migration of `game::health` from version 1",
    );
    let newer = build::<v3::Health>();
    refused(
        "the second step missing",
        &newer,
        &with(&[(1, health_v1_to_v2)]),
        &old,
        mismatch,
        "from version 2",
    );
    refused(
        "the first step missing",
        &newer,
        &with(&[(2, health_v2_to_v3)]),
        &old,
        mismatch,
        "from version 1",
    );
    refused(
        "a field left over",
        &new,
        &with(&[(1, keeps_field_1)]),
        &old,
        mismatch,
        "left field 1, which the type lacks",
    );
    refused("a field unset", &new, &with(&[(1, forgets_max)]), &old, mismatch, "`max` (3) unset");
    refused(
        "a field of another size",
        &new,
        &with(&[(1, writes_wide)]),
        &old,
        mismatch,
        "`points` (2) 4 bytes, not 2",
    );
    refused(
        "an invalid value",
        &build::<moody::Health>(),
        &with(&[(1, crosses_wrongly)]),
        &old,
        ErrorKind::InvalidData,
        "",
    );
    refused(
        "the step's own error",
        &new,
        &with(&[(1, refuses)]),
        &old,
        ErrorKind::Unsupported,
        "predates the guild system",
    );
}

#[test]
fn a_set_field_under_a_changed_schema_is_unsupported() {
    let old = v1_records();
    let v1_build = build::<v1::Health>();
    let set = Record::SetField {
        id: NetId::cooked(1),
        component: v1::Health::KEY,
        field: FieldId(1),
        bytes: 9_i32.to_le_bytes().to_vec(),
    };
    let schemas = v1_build.table.schemas().cloned();
    let overlay = Overlay::new(old.baseline_hash, schemas, [set]).unwrap();
    let mut records = RegionRecords { overlay: Some(Arc::new(overlay)), ..old };
    records.layouts.remove(&Position::KEY);
    records.check().unwrap();
    let new = build::<v2::Health>();
    let migrations = with(&[(1, health_v1_to_v2)]);
    refused(
        "a `SetField` of a migrated type",
        &new,
        &migrations,
        &records,
        ErrorKind::Unsupported,
        "without the rest of its value",
    );
    // At the registry's version and digest it passes through, the overlay untouched.
    let same = v1_build.loader(&migrations).migrate(&records).unwrap();
    assert!(Arc::ptr_eq(same.overlay.as_ref().unwrap(), records.overlay.as_ref().unwrap()));
}

#[test]
fn a_matching_overlay_comes_back_as_the_same_arc() {
    let old = v1_records();
    let v1_build = build::<v1::Health>();
    let migrations = with(&[(1, health_v1_to_v2)]);
    let migrated = v1_build.loader(&migrations).migrate(&old).unwrap();
    assert_eq!(migrated, old);
    let (before, after) = (old.overlay.as_ref().unwrap(), migrated.overlay.as_ref().unwrap());
    assert!(Arc::ptr_eq(before, after));
    let bundle = load(&v1_build, &migrations, &old).unwrap();
    let overlay = bundle.overlay.as_ref().unwrap();
    assert!(Arc::ptr_eq(before, overlay));
    assert_eq!(overlay.digest(), before.digest());
    // A migrated overlay is a new document with a new digest.
    let new = build::<v2::Health>();
    let migrated = new.loader(&migrations).migrate(&old).unwrap();
    assert_ne!(migrated.overlay.as_ref().unwrap().digest(), before.digest());
}

#[test]
fn a_second_step_for_one_type_and_version_is_refused() {
    let mut migrations = with(&[(1, health_v1_to_v2)]);
    let again = Migration { component: v1::Health::KEY, from: 1, run: keeps_field_1 };
    let error = migrations.register(again).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert_eq!(migrations.len(), 1);
    // The step kept is the first: the second would leave field 1 over.
    let new = build::<v2::Health>();
    load(&new, &migrations, &v1_records()).unwrap();
    // Another version of the type is another step.
    migrations
        .register(Migration { component: v1::Health::KEY, from: 2, run: health_v2_to_v3 })
        .unwrap();
    assert_eq!(migrations.len(), 2);
}

// ---- Equal versions with another digest ----

/// The original base type's records: one spawned entity with a base value of zeroes.
fn base_records() -> (RegionRecords, Arc<Baseline>) {
    let table = table_of::<variants::original::Base>();
    let baseline = Arc::new(Baseline::from_rows(AssetSet::new(), [], []).unwrap());
    let size = table.schema(variants::original::Base::KEY).unwrap().element_size() as usize;
    let spawn = Record::Spawn {
        id: NetId::from_bits(1).unwrap(),
        components: vec![ComponentData {
            component: variants::original::Base::KEY,
            bytes: vec![0; size],
        }],
    };
    (records(&table, &baseline, vec![spawn], vec![], vec![]), baseline)
}

fn table_of<T: Reflect>() -> SchemaTable {
    let mut registry = Registry::new();
    registry.register::<T>().unwrap();
    SchemaTable::from_registry(&registry).unwrap()
}

#[test]
fn equal_versions_with_another_digest_are_refused() {
    let (records, baseline) = base_records();
    let migrations = Migrations::new();
    let bundle = |table: &SchemaTable| {
        Loader::new(table, &migrations).bundle(
            &records,
            Arc::clone(&baseline),
            RegionMode::Simulated,
            None,
        )
    };
    bundle(&table_of::<variants::original::Base>()).unwrap();
    for (what, table) in [
        ("a nested reorder", table_of::<variants::nested_reorder::Base>()),
        ("discriminants renumbered", table_of::<variants::renumbered::Base>()),
        ("names swapped over their values", table_of::<variants::names_swapped::Base>()),
        ("an element's names swapped", table_of::<variants::element_names_swapped::Base>()),
        ("declarations reordered", table_of::<variants::declarations_reordered::Base>()),
        ("a variant renamed", table_of::<variants::variant_renamed::Base>()),
        ("u64 to i64", table_of::<variants::signed::Base>()),
        ("u64 to Scalar", table_of::<variants::scalar::Base>()),
        ("an element's primitive", table_of::<variants::element_signed::Base>()),
        ("an element's nested order", table_of::<variants::element_reorder::Base>()),
        ("an element's enum", table_of::<variants::element_enum::Base>()),
    ] {
        let error = bundle(&table).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::VersionMismatch, "{what}: {error}");
        assert!(error.to_string().contains("without a version bump"), "{what}: {error}");
        let error = Loader::new(&table, &migrations).migrate(&records).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::VersionMismatch, "{what}: {error}");
    }
    // A change neither the schema nor the digest sees loads as it is.
    for table in [
        table_of::<variants::replicated::Base>(),
        table_of::<variants::field_renamed::Base>(),
        table_of::<variants::struct_renamed::Base>(),
        table_of::<variants::enum_renamed::Base>(),
    ] {
        let migrated = Loader::new(&table, &migrations).migrate(&records).unwrap();
        assert!(Arc::ptr_eq(migrated.overlay.as_ref().unwrap(), records.overlay.as_ref().unwrap()));
    }
}

// ---- Persistence changed since the save ----

/// A component in every build, the same in each: the cooked row's.
#[component(key = "migrate::tag", persistent)]
#[derive(PartialEq, Debug)]
pub struct Tag(#[field(id = 1)] pub u32);

/// A value whose zero no variant has.
#[value]
#[repr(u8)]
pub enum Level {
    /// Low.
    Low = 1,
    /// High.
    High = 2,
}

/// The build the saves below were written by.
mod saved {
    use super::*;

    /// A transient component.
    #[component(key = "migrate::buzz", transient)]
    pub struct Buzz {
        #[field(id = 1)]
        pub a: u32,
    }

    /// A transient region resource.
    #[resource(key = "migrate::hum", transient)]
    pub struct Hum {
        #[field(id = 1)]
        pub a: u32,
    }

    /// A persistent region resource.
    #[resource(key = "migrate::stock", persistent)]
    pub struct Stock {
        #[field(id = 1)]
        pub n: u32,
    }

    /// A transient component and a transient resource of a byte, which the next build
    /// makes a [`Level`], whose zero is invalid (a transient type's zero must be valid,
    /// so they become `Level`s as they become persistent).
    #[component(key = "migrate::gate", transient)]
    pub struct Gate {
        #[field(id = 1)]
        pub level: u8,
    }

    #[resource(key = "migrate::lock", transient)]
    pub struct Lock {
        #[field(id = 1)]
        pub level: u8,
    }

    /// A persistent component and a persistent resource the next build derives.
    #[component(key = "migrate::memo", persistent)]
    pub struct Memo(#[field(id = 1)] pub u64);

    #[resource(key = "migrate::tally", persistent)]
    pub struct Tally(#[field(id = 1)] pub u64);
}

/// The transient types relaid out, with no bump, and the persistent resource made
/// transient.
mod relaid {
    use super::*;

    #[component(key = "migrate::buzz", transient)]
    pub struct Buzz {
        #[field(id = 1)]
        pub a: u32,
        #[field(id = 2)]
        pub b: u32,
    }

    #[resource(key = "migrate::hum", transient)]
    pub struct Hum {
        #[field(id = 1)]
        pub a: u32,
        #[field(id = 2)]
        pub b: u32,
    }

    #[resource(key = "migrate::stock", transient)]
    pub struct Stock {
        #[field(id = 1)]
        pub n: u32,
    }
}

/// The transient types made persistent: `Buzz` as it was saved, `Hum` at a new layout and
/// version, `Gate` at version 2 and `Lock` at version 1, each holding a `Level`.
mod kept {
    use super::*;

    #[component(key = "migrate::buzz", persistent)]
    #[derive(PartialEq, Debug)]
    pub struct Buzz {
        #[field(id = 1)]
        pub a: u32,
    }

    #[resource(key = "migrate::hum", persistent, version = 3)]
    #[derive(PartialEq, Debug)]
    pub struct Hum {
        #[field(id = 1)]
        pub a: u32,
        #[field(id = 2)]
        pub b: u32,
    }

    #[component(key = "migrate::gate", persistent, version = 2)]
    pub struct Gate {
        #[field(id = 1)]
        pub level: Level,
    }

    #[resource(key = "migrate::lock", persistent)]
    pub struct Lock {
        #[field(id = 1)]
        pub level: Level,
    }
}

/// `Buzz` made persistent at version 2, with a step from 1.
mod bumped {
    use super::*;

    #[component(key = "migrate::buzz", persistent, version = 2)]
    #[derive(PartialEq, Debug)]
    pub struct Buzz {
        #[field(id = 1)]
        pub a: u32,
        #[field(id = 3)]
        pub c: u32,
    }
}

/// `Buzz` made persistent at version 1 with another layout: moved without a bump.
mod moved {
    use super::*;

    #[component(key = "migrate::buzz", persistent)]
    pub struct Buzz {
        #[field(id = 1)]
        pub a: u32,
        #[field(id = 2)]
        pub b: u32,
    }
}

/// The types made derived.
mod derived {
    use super::*;

    #[component(key = "migrate::memo", derived)]
    #[derive(PartialEq, Debug)]
    pub struct Memo(#[field(id = 1)] pub u64);

    #[resource(key = "migrate::tally", derived)]
    pub struct Tally(#[field(id = 1)] pub u64);

    #[resource(key = "migrate::hum", derived)]
    pub struct Hum {
        #[field(id = 1)]
        pub a: u32,
    }
}

/// `Stock` at version 2, widened, with a step; and at version 1 relaid without a bump.
mod stock {
    use super::*;

    #[resource(key = "migrate::stock", persistent, version = 2, retired = [1])]
    pub struct Wide {
        #[field(id = 2)]
        pub n: u64,
    }

    #[resource(key = "migrate::stock", persistent)]
    pub struct Signed {
        #[field(id = 1)]
        pub n: i32,
    }
}

fn buzz_v1_to_v2(value: &mut FieldValues) -> Result<()> {
    assert_eq!(value.get(FieldId(1)), Some(&[0_u8; 4][..]), "the step runs over the zero value");
    value.write(FieldId(3), 9_u32);
    Ok(())
}

/// A step that changes nothing: the byte is read as a `Level` as it is.
fn unchanged(_: &mut FieldValues) -> Result<()> {
    Ok(())
}

fn stock_v1_to_v2(value: &mut FieldValues) -> Result<()> {
    let n: u32 = value.read(FieldId(1))?;
    value.take(FieldId(1));
    value.write(FieldId(2), u64::from(n) << 32);
    Ok(())
}

/// A registry of the world's own types, `Tag` and `T`.
macro_rules! registry {
    ($($t:ty),* $(,)?) => {{
        let mut registry = Registry::new();
        registry.register::<InRegion>().unwrap();
        registry.register::<OwnedBy>().unwrap();
        registry.register::<ClaimedBy>().unwrap();
        registry.register::<PredictedOrigin>().unwrap();
        registry.register::<StreamingAnchor>().unwrap();
        registry.register::<Tag>().unwrap();
        $(registry.register::<$t>().unwrap();)*
        registry
    }};
}

/// The baseline of the persistence cases: one cooked row with a tag.
fn tag_baseline() -> Arc<Baseline> {
    let schema = table_of::<Tag>().schema(Tag::KEY).unwrap().clone();
    let row = BaselineRow { id: NetId::cooked(1), components: vec![data(Tag(1))] };
    Arc::new(Baseline::from_rows(AssetSet::new(), [schema], [row]).unwrap())
}

/// Canonical records under `table`: the overlay of `overlay` (none if it is empty), the
/// resources and the transient keys, with the schemas and digests the table states.
fn records(
    table: &SchemaTable,
    baseline: &Baseline,
    overlay: Vec<Record>,
    mut resources: Vec<ComponentData>,
    mut transient_resources: Vec<ComponentKey>,
) -> RegionRecords {
    let hash = baseline.content_hash();
    let overlay = (!overlay.is_empty())
        .then(|| Arc::new(Overlay::new(hash, table.schemas().cloned(), overlay).unwrap()));
    resources.sort_by_key(|data| data.component);
    transient_resources.sort_unstable();
    let carried =
        overlay.iter().flat_map(|overlay| overlay.schemas()).map(|schema| schema.component());
    let layouts = carried
        .chain(resources.iter().map(|data| data.component))
        .map(|component| (component, table.layout(component).unwrap()))
        .collect();
    let resource_schemas =
        resources.iter().map(|data| table.schema(data.component).unwrap().clone()).collect();
    let records = RegionRecords {
        region: HOME,
        seed: 3,
        baseline_hash: hash,
        overlay,
        resources,
        resource_schemas,
        transient_resources,
        layouts,
    };
    records.check().unwrap();
    records
}

fn runtime(n: u64) -> NetId {
    NetId::from_bits(n).unwrap()
}

/// The saved build's table.
fn saved_table() -> SchemaTable {
    use saved::*;
    SchemaTable::from_registry(&registry!(Buzz, Hum, Stock, Gate, Lock, Memo, Tally)).unwrap()
}

/// What `table`'s loader makes of `records`, with `migrations`.
fn bundle_under(
    table: &SchemaTable,
    migrations: &Migrations,
    records: &RegionRecords,
) -> Result<RegionBundle> {
    Loader::new(table, migrations).bundle(records, tag_baseline(), RegionMode::Simulated, None)
}

fn world_of(registry: Registry, bundle: &RegionBundle) -> World {
    let mut world = WorldBuilder::new(WorldKind::Server, Arc::new(registry)).build().unwrap();
    load_into(&mut world, HOME, bundle);
    world
}

fn resource_bytes(world: &World, component: ComponentKey) -> Option<Vec<u8>> {
    world
        .region_resources_erased(HOME)
        .find(|(key, _)| *key == component)
        .map(|(_, bytes)| bytes.to_vec())
}

#[test]
fn a_transient_type_relaid_loads_zeroed_with_no_step() {
    let table = saved_table();
    let buzz = ComponentData { component: saved::Buzz::KEY, bytes: 5_u32.to_le_bytes().to_vec() };
    let spawn = Record::Spawn { id: runtime(1), components: vec![data(Tag(2)), buzz.clone()] };
    let records = records(&table, &tag_baseline(), vec![spawn], vec![], vec![saved::Hum::KEY]);
    let migrations = Migrations::new();

    // Unchanged: the overlay is the same `Arc`, its bytes kept whatever they are.
    let same = Loader::new(&table, &migrations).migrate(&records).unwrap();
    assert!(Arc::ptr_eq(same.overlay.as_ref().unwrap(), records.overlay.as_ref().unwrap()));

    // Relaid with no bump and no step: zeroes of the new size under the new schema.
    let relaid_registry = || registry!(relaid::Buzz, relaid::Hum, saved::Stock);
    let relaid = SchemaTable::from_registry(&relaid_registry()).unwrap();
    let migrated = Loader::new(&relaid, &migrations).migrate(&records).unwrap();
    let overlay = migrated.overlay.as_ref().unwrap();
    assert_eq!(overlay.schema(saved::Buzz::KEY), relaid.schema(saved::Buzz::KEY));
    let Record::Spawn { components, .. } = &overlay.records()[0] else { panic!() };
    assert_eq!(components[1], ComponentData { component: saved::Buzz::KEY, bytes: vec![0; 8] });
    assert_eq!(migrated.transient_resources, vec![saved::Hum::KEY]);
    assert_eq!(migrated.layouts[&saved::Buzz::KEY], relaid.layout(saved::Buzz::KEY).unwrap());

    let bundle = bundle_under(&relaid, &migrations, &records).unwrap();
    let world = world_of(relaid_registry(), &bundle);
    assert_eq!(world.get_erased(net(runtime(1)), saved::Buzz::KEY), Some(&[0_u8; 8][..]));
    assert_eq!(resource_bytes(&world, saved::Hum::KEY), Some(vec![0; 8]));
}

#[test]
fn a_transient_set_field_is_kept_under_its_schema_and_dropped_under_another() {
    let table = saved_table();
    let buzz =
        |a: u32| ComponentData { component: saved::Buzz::KEY, bytes: a.to_le_bytes().to_vec() };
    let schemas = [Tag::KEY, saved::Buzz::KEY].map(|key| table.schema(key).unwrap().clone());
    let row = BaselineRow { id: NetId::cooked(1), components: vec![data(Tag(1)), buzz(5)] };
    let baseline = Arc::new(Baseline::from_rows(AssetSet::new(), schemas, [row]).unwrap());
    let set = Record::SetField {
        id: NetId::cooked(1),
        component: saved::Buzz::KEY,
        field: FieldId(1),
        bytes: 7_u32.to_le_bytes().to_vec(),
    };
    let tag = Record::AddComponent { id: NetId::cooked(1), data: data(Tag(2)) };
    let records = records(&table, &baseline, vec![tag.clone(), set], vec![], vec![]);
    let migrations = Migrations::new();

    // Under its own schema the record passes through, and the value still loads zeroed.
    let loader = Loader::new(&table, &migrations);
    let same = loader.migrate(&records).unwrap();
    assert!(Arc::ptr_eq(same.overlay.as_ref().unwrap(), records.overlay.as_ref().unwrap()));
    let bundle = loader.bundle(&records, baseline, RegionMode::Simulated, None).unwrap();
    let registry = || {
        use saved::*;
        registry!(Buzz, Hum, Stock, Gate, Lock, Memo, Tally)
    };
    let world = world_of(registry(), &bundle);
    let key = net(NetId::cooked(1));
    assert_eq!(world.get_erased(key, saved::Buzz::KEY), Some(&[0_u8; 4][..]));
    assert_eq!(world.get::<Tag>(key), Some(&Tag(2)));

    // Under another schema it is dropped, not refused, and the rest of the overlay stays.
    let relaid = SchemaTable::from_registry(&registry!(relaid::Buzz, relaid::Hum)).unwrap();
    let migrated = Loader::new(&relaid, &migrations).migrate(&records).unwrap();
    assert_eq!(migrated.overlay.as_ref().unwrap().records(), [tag].as_slice());
}

#[test]
fn a_persistent_resource_made_transient_moves_to_the_transient_keys() {
    let table = saved_table();
    let stock = ComponentData { component: saved::Stock::KEY, bytes: 3_u32.to_le_bytes().to_vec() };
    let records = records(&table, &tag_baseline(), vec![], vec![stock], vec![saved::Hum::KEY]);
    let relaid_registry = || registry!(relaid::Buzz, relaid::Hum, relaid::Stock);
    let relaid = SchemaTable::from_registry(&relaid_registry()).unwrap();
    let migrations = Migrations::new();
    let migrated = Loader::new(&relaid, &migrations).migrate(&records).unwrap();
    assert!(migrated.resources.is_empty() && migrated.resource_schemas.is_empty());
    assert!(migrated.layouts.is_empty());
    let mut keys = vec![saved::Hum::KEY, saved::Stock::KEY];
    keys.sort_unstable();
    assert_eq!(migrated.transient_resources, keys);
    let bundle = bundle_under(&relaid, &migrations, &records).unwrap();
    let world = world_of(relaid_registry(), &bundle);
    assert_eq!(resource_bytes(&world, saved::Stock::KEY), Some(vec![0; 4]));
}

#[test]
fn a_transient_type_made_persistent_loads_from_a_zero_value() {
    let table = saved_table();
    // The diff writes a transient component's presence as zeroes.
    let buzz = ComponentData { component: saved::Buzz::KEY, bytes: vec![0; 4] };
    let spawn = Record::Spawn { id: runtime(1), components: vec![data(Tag(2)), buzz] };
    let records = records(&table, &tag_baseline(), vec![spawn], vec![], vec![saved::Hum::KEY]);
    let migrations = Migrations::new();

    // Kept at the version and layout it was saved under: the zero value, and the key
    // the zero value under the registry's schema and digest, with no step though `Hum`
    // is at version 3 now.
    let kept_registry = || registry!(kept::Buzz, kept::Hum);
    let kept = SchemaTable::from_registry(&kept_registry()).unwrap();
    let migrated = Loader::new(&kept, &migrations).migrate(&records).unwrap();
    assert!(migrated.transient_resources.is_empty());
    assert_eq!(
        migrated.resources,
        vec![ComponentData { component: saved::Hum::KEY, bytes: vec![0; 8] }]
    );
    assert_eq!(migrated.resource_schemas, vec![kept.schema(saved::Hum::KEY).unwrap().clone()]);
    assert_eq!(migrated.layouts[&saved::Hum::KEY], kept.layout(saved::Hum::KEY).unwrap());
    let world = world_of(kept_registry(), &bundle_under(&kept, &migrations, &records).unwrap());
    assert_eq!(world.get::<kept::Buzz>(net(runtime(1))), Some(&kept::Buzz { a: 0 }));
    assert_eq!(world.region_resource::<kept::Hum>(HOME), Some(&kept::Hum { a: 0, b: 0 }));

    // At a later version: through its steps, over the zero value.
    let bumped_registry = || registry!(bumped::Buzz);
    let bumped = SchemaTable::from_registry(&bumped_registry()).unwrap();
    let mut steps = Migrations::new();
    let step = Migration { component: saved::Buzz::KEY, from: 1, run: buzz_v1_to_v2 };
    steps.register(step).unwrap();
    let hum_free = RegionRecords { transient_resources: vec![], ..records.clone() };
    let world = world_of(bumped_registry(), &bundle_under(&bumped, &steps, &hum_free).unwrap());
    assert_eq!(world.get::<bumped::Buzz>(net(runtime(1))), Some(&bumped::Buzz { a: 0, c: 9 }));
    let error = bundle_under(&bumped, &migrations, &hum_free).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::VersionMismatch, "{error}");

    // Relaid while transient and made persistent without a bump: refused.
    let moved = SchemaTable::from_registry(&registry!(moved::Buzz)).unwrap();
    let error = bundle_under(&moved, &migrations, &hum_free).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::VersionMismatch, "{error}");
    assert!(error.to_string().contains("without a version bump"), "{error}");
}

#[test]
fn a_restored_zero_that_fails_validation_is_invalid_data() {
    let table = saved_table();
    let gate = ComponentData { component: saved::Gate::KEY, bytes: vec![0] };
    let spawn = Record::Spawn { id: runtime(1), components: vec![gate] };
    let as_component = records(&table, &tag_baseline(), vec![spawn], vec![], vec![]);
    let as_key = records(&table, &tag_baseline(), vec![], vec![], vec![saved::Lock::KEY]);
    // The component's version moved, so a step runs over its zero value; the key has none.
    let mut migrations = Migrations::new();
    let step = Migration { component: saved::Gate::KEY, from: 1, run: unchanged };
    migrations.register(step).unwrap();
    bundle_under(&table, &migrations, &as_component).unwrap();
    bundle_under(&table, &migrations, &as_key).unwrap();
    let kept = SchemaTable::from_registry(&registry!(kept::Gate, kept::Lock)).unwrap();
    for (what, records) in [("a component", as_component), ("a resource", as_key)] {
        let error = bundle_under(&kept, &migrations, &records).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{what}: {error}");
    }
}

#[test]
fn a_type_made_derived_is_dropped() {
    use saved::{Memo, Tally};
    let table = saved_table();
    let baseline = tag_baseline();
    let memo = |n: u64| data(Memo(n));
    let added = Record::AddComponent { id: NetId::cooked(1), data: memo(7) };
    let spawn = Record::Spawn { id: runtime(1), components: vec![data(Tag(2)), memo(8)] };
    let tally = data(Tally(9));
    let derived_registry = || registry!(derived::Memo, derived::Tally, derived::Hum);
    let derived = SchemaTable::from_registry(&derived_registry()).unwrap();
    let migrations = Migrations::new();

    // A spawn keeps its record and loses the component; the resource and the transient
    // key leave the records.
    let both =
        records(&table, &baseline, vec![added.clone(), spawn], vec![tally], vec![saved::Hum::KEY]);
    let migrated = Loader::new(&derived, &migrations).migrate(&both).unwrap();
    let overlay = migrated.overlay.as_ref().unwrap();
    assert_eq!(
        overlay.records(),
        &[Record::Spawn { id: runtime(1), components: vec![data(Tag(2))] }]
    );
    assert!(migrated.resources.is_empty() && migrated.transient_resources.is_empty());
    assert_eq!(migrated.layouts.keys().copied().collect::<Vec<_>>(), vec![Tag::KEY]);
    let world = world_of(derived_registry(), &bundle_under(&derived, &migrations, &both).unwrap());
    assert_eq!(world.get::<derived::Memo>(net(runtime(1))), None);
    assert!(world.region_resources_erased(HOME).next().is_none());

    // An overlay left with no record is no overlay.
    let removed = Record::RemoveComponent { id: NetId::cooked(1), component: Tag::KEY };
    let only_memo = records(&table, &baseline, vec![added], vec![], vec![]);
    let migrated = Loader::new(&derived, &migrations).migrate(&only_memo).unwrap();
    assert_eq!(migrated.overlay, None);
    assert!(migrated.layouts.is_empty());
    migrated.check().unwrap();
    // A removal of the derived type goes too; one of a kept type stays.
    let memo_removed = Record::RemoveComponent { id: NetId::cooked(1), component: Memo::KEY };
    let baseline_with_memo = {
        let schemas =
            [table.schema(Tag::KEY).unwrap().clone(), table.schema(Memo::KEY).unwrap().clone()];
        let row = BaselineRow { id: NetId::cooked(1), components: vec![data(Tag(1)), memo(1)] };
        Baseline::from_rows(AssetSet::new(), schemas, [row]).unwrap()
    };
    let removals =
        records(&table, &baseline_with_memo, vec![memo_removed, removed.clone()], vec![], vec![]);
    let migrated = Loader::new(&derived, &migrations).migrate(&removals).unwrap();
    assert_eq!(migrated.overlay.as_ref().unwrap().records(), &[removed]);
}

/// `Memo` derived, as the game maintains it: every positioned row without one gets one.
#[system(name = "migrate::remember", worlds = [server], after = integrate)]
fn remember(key: StableKeyOf, pos: &Position, cmd: Commands, _memo: Without<derived::Memo>) {
    cmd.set(*key, derived::Memo(10 * u64::from(pos.x.to_i32().unsigned_abs()) + 1));
}

#[test]
fn the_world_rebuilds_a_type_made_derived() {
    // The game's region `A`, saved with `Memo` persistent on an entity it spawned.
    let mut saved_schemas: Vec<_> = common::schemas().schemas().cloned().collect();
    saved_schemas.push(saved_table().schema(saved::Memo::KEY).unwrap().clone());
    let spawned = runtime(500);
    let spawn =
        Record::Spawn { id: spawned, components: vec![data(pos(5, 6)), data(saved::Memo(0xdead))] };
    let baseline = common::baseline(A);
    let overlay = Overlay::new(baseline.content_hash(), saved_schemas, [spawn]).unwrap();
    let layouts = [
        (Position::KEY, common::schemas().layout(Position::KEY).unwrap()),
        (saved::Memo::KEY, saved_table().layout(saved::Memo::KEY).unwrap()),
        (Weather::KEY, common::schemas().layout(Weather::KEY).unwrap()),
    ];
    let records = RegionRecords {
        region: A,
        seed: seed_of(A),
        baseline_hash: baseline.content_hash(),
        overlay: Some(Arc::new(overlay)),
        resources: vec![data(Weather(1))],
        resource_schemas: vec![common::schemas().schema(Weather::KEY).unwrap().clone()],
        transient_resources: vec![common::Rain::KEY],
        layouts: layouts.into_iter().collect(),
    };
    records.check().unwrap();

    // This build derives `Memo`.
    let mut types = common::types();
    types.register::<derived::Memo>().unwrap();
    let table = SchemaTable::from_registry(types.registry()).unwrap();
    let migrations = Migrations::new();
    let bundle = Loader::new(&table, &migrations)
        .bundle(&records, baseline, RegionMode::Simulated, Some(first_table(A)))
        .unwrap();
    let Record::Spawn { components, .. } = &bundle.overlay.as_ref().unwrap().records()[0] else {
        panic!()
    };
    assert_eq!(components, &[data(pos(5, 6))]);

    let simulation = SimulationBuilder::new(WorldKind::Server, types)
        .setup(setup)
        .systems(|registration| {
            register_systems(registration);
            registration.stage(Stage::Simulation).add(&REMEMBER);
        })
        .inputs(Box::new(Inputs))
        .sessions(Box::new(Sessions))
        .next_net_id(NonZeroU64::new(1000).unwrap())
        .build()
        .unwrap();
    let executor = executor();
    let frame = TickInput { loaded: [(A, bundle)].into(), ..TickInput::default() };
    let simulation = step(&executor, simulation, frame);
    let key: StableKey = net(spawned);
    let rebuilt = 10 * 5 + 1;
    assert_eq!(simulation.world().get::<derived::Memo>(key), Some(&derived::Memo(rebuilt)));
}

// ---- Resources migrate under the section's schema and digest ----

#[test]
fn a_resource_migrates_under_the_schema_and_digest_the_section_carries() {
    let table = saved_table();
    let stock = ComponentData { component: saved::Stock::KEY, bytes: 3_u32.to_le_bytes().to_vec() };
    let records = records(&table, &tag_baseline(), vec![], vec![stock], vec![]);

    let wide_registry = || registry!(stock::Wide);
    let wide = SchemaTable::from_registry(&wide_registry()).unwrap();
    let mut migrations = Migrations::new();
    let step = Migration { component: saved::Stock::KEY, from: 1, run: stock_v1_to_v2 };
    migrations.register(step).unwrap();
    let migrated = Loader::new(&wide, &migrations).migrate(&records).unwrap();
    assert_eq!(migrated.resources[0].bytes, (3_u64 << 32).to_le_bytes());
    assert_eq!(migrated.resource_schemas[0], *wide.schema(saved::Stock::KEY).unwrap());
    assert_eq!(migrated.layouts[&saved::Stock::KEY], wide.layout(saved::Stock::KEY).unwrap());
    let world = world_of(wide_registry(), &bundle_under(&wide, &migrations, &records).unwrap());
    assert_eq!(
        resource_bytes(&world, saved::Stock::KEY),
        Some((3_u64 << 32).to_le_bytes().to_vec())
    );

    // The digest the section carries is the one compared: at version 1 relaid as `i32`, the
    // document schema is the same and the digest is not.
    let signed = SchemaTable::from_registry(&registry!(stock::Signed)).unwrap();
    assert_eq!(signed.schema(saved::Stock::KEY), table.schema(saved::Stock::KEY));
    let error = bundle_under(&signed, &Migrations::new(), &records).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::VersionMismatch, "{error}");
    // And a section that states the new digest for the old bytes is believed: the digest is
    // the section's word for its layout.
    let mut lying = records.clone();
    lying.layouts.insert(saved::Stock::KEY, signed.layout(saved::Stock::KEY).unwrap());
    bundle_under(&signed, &Migrations::new(), &lying).unwrap();
}
