//! Compile-only consumer sketches against the surface: what the server's `app` writes to
//! save and to load, in a fresh process, in place and one region at a time; what it writes
//! to build the relevance message from a region's records; what a game crate writes to
//! migrate a component; and the shape of the section 15.5 tests, which `reload.rs`,
//! `migrate.rs` and `schema.rs` run. Type-checked and never run. They pin the surface the
//! README promises: a change that breaks one is a signature change to list.

#![forbid(unsafe_code)]
#![allow(dead_code, unused_variables, missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;

use lactose_asset_api::{PinnedTable, VersionTable};
use lactose_bundle::Baseline;
use lactose_core::{FieldId, NetId, PlayerId, Result, Tick};
use lactose_ecs::{Executor, RegionId, RegionMode, WorldKind};
use lactose_math::Scalar;
use lactose_net_api::RegionRelevance;
use lactose_persist::{
    Container, ContainerHeader, ContainerIndex, EncodedSection, FieldValues, LayoutDigest, Loader,
    Migration, Migrations, PREAMBLE_BYTES, RegionRecords, SchemaHash, SchemaTable, diff,
};
use lactose_reflect::{Reflect, Registry, component, resource};
use lactose_sim::{
    RegionBundle, RegionExport, SavePoint, SessionEvent, Simulation, SimulationBuilder, TickInput,
    TypeTable,
};

// ---- The game's types, as a game crate declares them ----

/// Where an entity is: two scalars, saved at full precision.
#[component(key = "sketch::position", persistent, replicated)]
pub struct Position {
    #[field(id = 1)]
    pub x: Scalar,
    #[field(id = 2)]
    pub y: Scalar,
}

/// Hit points, at schema version 2: version 1 held `i32` points under field 1; version 2
/// holds `u16` points under field 2 and a `u16` maximum under field 3.
#[component(key = "sketch::health", persistent, replicated, version = 2, retired = [1])]
pub struct Health {
    #[field(id = 2)]
    pub points: u16,
    #[field(id = 3)]
    pub max: u16,
}

/// A per-tick accumulator: present on some rows, never saved by value, so a change to it
/// needs no version bump and no step.
#[component(key = "sketch::contacts", transient)]
pub struct Contacts(#[field(id = 1)] pub u32);

/// World-global persistent state, a resource of a pinned region.
#[resource(key = "sketch::weather", persistent)]
pub struct Weather(#[field(id = 1)] pub u32);

/// Every component and resource of the game, as `app` calls it.
fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<Position>()?;
    types.register::<Health>()?;
    types.register::<Contacts>()?;
    types.register::<Weather>()?;
    Ok(())
}

// ---- 1. A game crate's migrations ----

/// `Health` from version 1 to 2: the `i32` points become `u16` points under a new ID, and
/// the maximum starts at the points. A step reads what the value has, writes what the next
/// version has, and takes what it retires.
fn health_v1_to_v2(value: &mut FieldValues) -> Result<()> {
    let points: i32 = value.read(FieldId(1))?;
    value.take(FieldId(1));
    let points = u16::try_from(points.max(0)).unwrap_or(u16::MAX);
    value.write(FieldId(2), points);
    value.write(FieldId(3), points);
    Ok(())
}

/// The function a game crate exports beside `register_types`, which `app` calls on the one
/// `Migrations` it hands the loader.
fn register_migrations(migrations: &mut Migrations) -> Result<()> {
    migrations.register(Migration { component: Health::KEY, from: 1, run: health_v1_to_v2 })
}

// ---- 2. The server's `app`: a save ----

/// What `app` keeps for saving and loading: built once from the world's registry.
struct SaveContext {
    schemas: SchemaTable,
    migrations: Migrations,
    /// The cooked baseline of every region, from the asset store.
    baselines: BTreeMap<RegionId, Arc<Baseline>>,
}

impl SaveContext {
    fn new(registry: &Registry, baselines: BTreeMap<RegionId, Arc<Baseline>>) -> Result<Self> {
        let mut migrations = Migrations::new();
        register_migrations(&mut migrations)?;
        Ok(Self { schemas: SchemaTable::from_registry(registry)?, migrations, baselines })
    }

    fn baseline(&self, region: RegionId) -> Arc<Baseline> {
        Arc::clone(&self.baselines[&region])
    }
}

/// One region's work, as a background task runs it: the diff and the section.
fn section_of(export: &RegionExport, context: &SaveContext) -> Result<EncodedSection> {
    RegionRecords::from_export(export, &context.baseline(export.region), &context.schemas)?
        .encode_section()
}

/// The save, as the timer root's task writes it: the save copy at a sync point, then one
/// diff and one section per region (each independent, so `app` runs them as background
/// tasks), then the assembled bytes, which `app` writes through `io` to a temporary file,
/// syncs and renames over the slot.
async fn save(
    executor: &Executor,
    simulation: Simulation,
    context: &SaveContext,
) -> Result<(Simulation, Vec<u8>)> {
    let (simulation, save): (Simulation, SavePoint) =
        simulation.save_point(executor).await.map_err(lactose_core::Error::from)?;
    let header = ContainerHeader::of(&save, &context.schemas);
    // One task per region in `app`; sequential here, since the sketch is never run.
    let sections =
        save.regions.iter().map(|export| section_of(export, context)).collect::<Result<_>>()?;
    let bytes = Container::assemble(&header, sections)?;
    Ok((simulation, bytes))
}

/// The relevance message for a region that became relevant to a client: the same records
/// a save would hold, built by the server's `app` on the export it takes for the region,
/// handed to `net` to send. One `Cooked` rule and one canonicalisation, both
/// `from_export`'s.
fn relevance(
    export: &RegionExport,
    assets: Option<VersionTable>,
    context: &SaveContext,
) -> Result<RegionRelevance> {
    let records =
        RegionRecords::from_export(export, &context.baseline(export.region), &context.schemas)?;
    Ok(records.to_relevance(assets))
}

// ---- 3. The server's `app`: loads ----

/// A read of the front of the file, as `io` serves it: the preamble, then the rest of the
/// prefix the index needs, and no section.
fn read_index(file: &[u8]) -> Result<ContainerIndex> {
    let len = Container::index_len(&file[..PREAMBLE_BYTES.min(file.len())])?;
    Container::index(&file[..len.min(file.len())])
}

/// What a save-slot browser shows without reading a section.
fn browse(file: &[u8]) -> Result<(Tick, usize)> {
    let index = read_index(file)?;
    Ok((index.header.tick, index.regions.len()))
}

/// A fresh process: the header into the builder, every region's bundle into the first
/// frame, every player joining again (a save carries no session state).
fn load_fresh(
    executor: &Executor,
    bytes: &[u8],
    context: &SaveContext,
    tables: &BTreeMap<RegionId, PinnedTable>,
    rejoin: BTreeMap<PlayerId, SessionEvent>,
) -> Result<Simulation> {
    let container = Container::decode(bytes)?;
    let header = container.header();
    let mut types = TypeTable::new();
    register_types(&mut types)?;
    let simulation = SimulationBuilder::new(WorldKind::Server, types)
        .tick(header.tick)
        .next_net_id(header.next_net_id)
        .build()?;
    let frame = TickInput {
        loaded: bundles(&container, context, tables)?,
        sessions: rejoin,
        ..TickInput::default()
    };
    executor.block_on(simulation.step(executor, frame)).map_err(lactose_core::Error::from)
}

/// One bundle per region of the container, migrated and validated before anything loads.
fn bundles(
    container: &Container,
    context: &SaveContext,
    tables: &BTreeMap<RegionId, PinnedTable>,
) -> Result<BTreeMap<RegionId, RegionBundle>> {
    let loader = Loader::new(&context.schemas, &context.migrations);
    container
        .regions()
        .map(|records| {
            let bundle = loader.bundle(
                records,
                context.baseline(records.region),
                RegionMode::Simulated,
                tables.get(&records.region).cloned(),
            )?;
            Ok((records.region, bundle))
        })
        .collect()
}

/// One region of a save, read alone: the index from the front of the file, then that
/// region's bytes at its offset, and nothing else. What a streaming load at milestone 3
/// does per region.
fn load_one(file: &[u8], region: RegionId, context: &SaveContext) -> Result<RegionBundle> {
    let index = read_index(file)?;
    let entry = index.entry(region).ok_or_else(|| {
        lactose_core::Error::new(lactose_core::ErrorKind::NotFound, "no section for the region")
    })?;
    let start = usize::try_from(entry.offset).unwrap_or(usize::MAX);
    let len = usize::try_from(entry.compressed_bytes).unwrap_or(usize::MAX);
    let section = &file[start..start + len]; // `io` reads exactly this range
    let records = Container::decode_section(entry, section)?;
    let loader = Loader::new(&context.schemas, &context.migrations);
    loader.bundle(&records, context.baseline(region), RegionMode::Frozen, None)
}

/// The corrupt-save path of architecture section 19, as `app` reads what it can: every
/// section that decodes, and the regions whose sections do not, for its policy.
fn recover(file: &[u8]) -> Result<(Vec<RegionRecords>, Vec<RegionId>)> {
    let index = read_index(file)?;
    let (mut readable, mut damaged) = (Vec::new(), Vec::new());
    for entry in &index.regions {
        let start = usize::try_from(entry.offset).unwrap_or(usize::MAX);
        let len = usize::try_from(entry.compressed_bytes).unwrap_or(usize::MAX);
        match file.get(start..start + len).map(|section| Container::decode_section(entry, section))
        {
            Some(Ok(records)) => readable.push(records),
            _ => damaged.push(entry.region),
        }
    }
    Ok((readable, damaged))
}

/// A save whose baselines were re-cooked since: each section is carried over before it
/// is loaded, and the records the rebase dropped go to the cross-region pass that is not
/// in this crate yet.
fn rebase_all(container: &mut Container, context: &SaveContext) -> Result<()> {
    let regions: Vec<RegionId> = container.regions().map(|records| records.region).collect();
    for region in regions {
        let mut records = container.remove(region).expect("listed above");
        let patched = context.baseline(region);
        if patched.content_hash() != records.baseline_hash {
            let dropped = records.rebase(&patched)?;
            let _ = dropped;
        }
        container.insert(records)?;
    }
    Ok(())
}

// ---- 4. The server's `app`: the reload paths of section 15.5 and D124 ----

/// The one-frame in-place reload: every region unloaded and loaded again in one frame,
/// through the same bundles a fresh process takes.
fn reload_in_place(
    executor: &Executor,
    simulation: Simulation,
    bytes: &[u8],
    context: &SaveContext,
    tables: &BTreeMap<RegionId, PinnedTable>,
    rejoin: BTreeMap<PlayerId, SessionEvent>,
) -> Result<Simulation> {
    let container = Container::decode(bytes)?;
    let frame = TickInput {
        unloaded: simulation.world().regions().collect(),
        loaded: bundles(&container, context, tables)?,
        sessions: rejoin,
        ..TickInput::default()
    };
    executor.block_on(simulation.step(executor, frame)).map_err(lactose_core::Error::from)
}

/// The two-frame reload: unloaded at one tick, loaded again at the next from the exports
/// the unload left in the lifecycle, diffed and loaded through the same path.
fn reload_two_frames(
    executor: &Executor,
    mut simulation: Simulation,
    context: &SaveContext,
    tables: &BTreeMap<RegionId, PinnedTable>,
) -> Result<Simulation> {
    let unload =
        TickInput { unloaded: simulation.world().regions().collect(), ..TickInput::default() };
    simulation = executor.block_on(simulation.step(executor, unload))?;
    let exports: Vec<RegionExport> = simulation.lifecycle_mut().take_exports();
    let loader = Loader::new(&context.schemas, &context.migrations);
    let mut load = TickInput::default();
    for export in &exports {
        let baseline = context.baseline(export.region);
        let records = RegionRecords::from_export(export, &baseline, &context.schemas)?;
        let bundle = loader.bundle(
            &records,
            baseline,
            RegionMode::Simulated,
            tables.get(&export.region).cloned(),
        )?;
        load.loaded.insert(export.region, bundle);
    }
    executor.block_on(simulation.step(executor, load)).map_err(lactose_core::Error::from)
}

// ---- 5. The section 15.5 tests, as the implementation lane writes them ----

/// Test 1 and 4 of section 15.5 (D57, D124): the original reloads in place at tick T, a
/// fresh process loads the save, an untouched twin never reloads; all three run K ticks and
/// hash equal every tick, under the condition D129 states (the same store, no publish
/// between the save and the load, the mode the policy gives), since a save carries no
/// table and no mode; with physics, the fresh process and the reload are bit-equal to each
/// other through D124's fresh solver, and within D124's tolerance of the twin while bodies
/// are in contact.
fn test_reload_at(executor: &Executor, at: u32, k: u32) {
    let _ = (executor, at, k);
}

/// Test 2: a region unloaded and loaded again through `RegionRecords::from_export` and
/// `Loader::bundle` exports the same `RegionExport` afterwards.
fn test_unload_reload_keeps_columns(executor: &Executor, region: RegionId) {
    let _ = (executor, region);
}

/// Test 3: a save written under `Health` version 1 loads on this build through
/// `health_v1_to_v2`; a save with a version this build lacks a step for is refused with
/// `VersionMismatch` and nothing loads; a save whose `Contacts` layout differs loads with
/// no step, since a transient type is taken by presence.
fn test_previous_schema_version(executor: &Executor, bytes_from_v1: &[u8]) {
    let _ = (executor, bytes_from_v1);
}

/// A layout changed without a version bump: equal document schemas, different digests,
/// refused with `VersionMismatch` and nothing loads.
fn test_layout_change_without_bump(old: &Registry, new: &Registry) -> bool {
    let (old, new) = (SchemaTable::from_registry(old), SchemaTable::from_registry(new));
    let digest = |table: Result<SchemaTable>| table.ok().and_then(|t| t.layout(Health::KEY));
    let (a, b): (Option<LayoutDigest>, Option<LayoutDigest>) = (digest(old), digest(new));
    a != b
}

/// The schema hash: two registries with the same saved types hash equal whatever their
/// registration order; a derived type or a tracking flag changes nothing; a version bump, a
/// field, a size or a persistence change each change it.
fn test_schema_hash(a: &Registry, b: &Registry) -> bool {
    SchemaHash::of(a) == SchemaHash::of(b)
}

/// A hand-written check of the record rule on one export: the record set is the one the
/// README states, under every layout and worker count, and its digest is one number.
fn test_diff_is_canonical(export: &RegionExport, baseline: &Baseline, schemas: &SchemaTable) {
    let a = diff(&export.rows, baseline, schemas).map(|overlay| overlay.digest());
    let b = diff(&export.rows, baseline, schemas).map(|overlay| overlay.digest());
    let _ = (a, b, NetId::cooked(1));
}
