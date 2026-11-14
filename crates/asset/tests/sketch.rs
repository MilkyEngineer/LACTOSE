//! Compile-only consumer sketches against the surface: what `app` writes to build the store
//! and hand it on, the bridge a milestone 1 binary puts behind `app`'s `Cooked` stand-in,
//! the driver that answers the region lifecycle at milestone 3, what a boundary world
//! reads, what a client or a replay asks for, what the cooker writes, how a cooked pack
//! reaches a running store, and what `persist` calls for a baseline. Type-checked and
//! never run: the store's behaviour is proved by the other tests of this directory, and
//! the two tests at the end pin the log target, the format version and the baseline
//! kind's key. A change that breaks a sketch is a signature change to list.

#![forbid(unsafe_code)]
#![allow(dead_code, unused_variables, missing_docs)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use lactose_asset::{
    LOG_TARGET, Mount, PACK_FORMAT_VERSION, PackBuilder, RegionBaseline, RegionBaselineLoader,
    Storage, Store,
};
use lactose_asset_api::{
    Asset, AssetCatalog, AssetFailure, AssetId, AssetKind, AssetRef, AssetSet, CookedAsset,
    ErasedLoader, LoadPriority, Loader, PinnedTable, Processor, PublishedAssets, Readiness,
    SimulationAsset, VersionTable,
};
use lactose_bundle::Baseline;
use lactose_core::{Error, ErrorKind, Result};
use lactose_ecs::{RegionId, RegionMode, World};
use lactose_io::Io;
use lactose_sim::{RegionBundle, TickInput};
use lactose_task::{Executor, Task, WorkerClass};

// ---- 0. Two asset kinds, as the crates that own them define them ----

/// A kind the simulation reads: its loader is a pure decoder of its bytes.
struct SpawnTable {
    rolls: u32,
}

impl Asset for SpawnTable {
    const KIND: AssetKind = AssetKind::from_name("sketch.spawn_table");
    const FORMAT_VERSION: u32 = 1;
}

impl SimulationAsset for SpawnTable {}

struct SpawnTableLoader;

impl Loader for SpawnTableLoader {
    type Asset = SpawnTable;

    fn load(&self, bytes: &[u8], _dependencies: &[AssetId]) -> Result<SpawnTable> {
        let rolls: [u8; 4] = bytes
            .try_into()
            .map_err(|_| Error::new(ErrorKind::InvalidData, "a spawn table is four bytes"))?;
        Ok(SpawnTable { rolls: u32::from_le_bytes(rolls) })
    }
}

/// A presentation kind, which only code outside the boundary reads.
struct Mesh(Vec<u8>);

impl Asset for Mesh {
    const KIND: AssetKind = AssetKind::from_name("sketch.mesh");
    const FORMAT_VERSION: u32 = 1;
}

struct MeshLoader;

impl Loader for MeshLoader {
    type Asset = Mesh;

    fn load(&self, bytes: &[u8], _dependencies: &[AssetId]) -> Result<Mesh> {
        Ok(Mesh(bytes.to_vec()))
    }
}

struct MeshProcessor;

impl Processor for MeshProcessor {
    type Asset = Mesh;

    fn process(&self, source: &[u8]) -> Result<CookedAsset> {
        Ok(CookedAsset::new::<Mesh>(source.to_vec(), []))
    }
}

// ---- 1. app: the store, built once and handed on ----

/// One loader per asset kind, collected from the crates that define the kinds. The store
/// adds the region baseline's itself; a second loader for one kind is `InvalidData`.
fn app_start(executor: &Executor, io: &Io) -> Result<Store> {
    let loaders = [ErasedLoader::new(SpawnTableLoader), ErasedLoader::new(MeshLoader)];
    Store::new(executor.clone(), io.clone(), loaders)
}

/// The packs of a fresh process, oldest first: a later pack's version of an asset is the
/// newest. Awaited one after another, each from the calling thread.
fn app_mount(executor: &Executor, store: &Store, packs: &[PathBuf]) -> Result<()> {
    for pack in packs {
        executor.block_on(store.mount(pack))?;
    }
    Ok(())
}

/// The same packs with every mount in flight at once: the place in the mount order is
/// taken by the call, so the packs name the same versions as when mounted in turn.
fn app_mount_at_once(
    executor: &Executor,
    store: &Store,
    packs: &[PathBuf],
) -> Vec<Task<Result<()>>> {
    packs.iter().map(|pack| executor.spawn(WorkerClass::Background, store.mount(pack))).collect()
}

/// The two `dyn` resources of `lactose-asset-api`: the catalogue for everything outside
/// the boundary, and from it the exact-version view a boundary world receives.
fn app_views(store: &Store) -> (Arc<dyn AssetCatalog>, Arc<dyn PublishedAssets>) {
    let catalog: Arc<dyn AssetCatalog> = Arc::new(store.clone());
    let published: Arc<dyn PublishedAssets> = catalog.clone();
    (catalog, published)
}

/// The store owns no thread: it is dropped, then the pool stops, then the executor.
fn app_shutdown(store: Store, io: Io, executor: Executor) {
    drop(store);
    io.shutdown();
    executor.shutdown();
}

// ---- 2. Milestone 1: a binary's bridge behind `app`'s `Cooked` stand-in ----

/// `lactose_app::Cooked`'s three calls, restated: a backend crate's tests cannot name the
/// assembly. They are synchronous and called between and during steps, so the bridge
/// answers only from what was loaded before the first step.
trait Cooked: Send + Sync + 'static {
    fn baseline(&self, content_hash: u64) -> Result<Arc<Baseline>>;
    fn table(&self, region: RegionId) -> Option<VersionTable>;
    fn pin(&self, table: &VersionTable) -> Result<PinnedTable>;
}

struct Preloaded {
    store: Store,
    sets: BTreeMap<RegionId, AssetSet>,
}

/// Before the first step, from the thread that owns the app: mount, ask for every
/// baseline and every region's set, and wait until all of it is published or failed.
fn preload(
    executor: &Executor,
    store: &Store,
    packs: &[PathBuf],
    baselines: &[u64],
    sets: &BTreeMap<RegionId, AssetSet>,
) -> Result<()> {
    app_mount(executor, store, packs)?;
    for &content_hash in baselines {
        store.request_newest(RegionBaseline::id(content_hash), LoadPriority::default());
    }
    for id in sets.values().flat_map(AssetSet::iter) {
        store.request_newest(id, LoadPriority::default());
    }
    executor.block_on(store.settled());
    Ok(())
}

fn loaded<T>(readiness: Readiness<T>) -> Result<T> {
    match readiness {
        Readiness::Ready(value) => Ok(value),
        Readiness::Pending => {
            Err(Error::new(ErrorKind::NotFound, "not loaded before the first step"))
        }
        Readiness::Failed(failure) => Err(failure.into()),
    }
}

impl Cooked for Preloaded {
    fn baseline(&self, content_hash: u64) -> Result<Arc<Baseline>> {
        loaded(RegionBaseline::lookup(&self.store, content_hash))
    }

    fn table(&self, region: RegionId) -> Option<VersionTable> {
        self.store.resolve(self.sets.get(&region)?).ready()
    }

    fn pin(&self, table: &VersionTable) -> Result<PinnedTable> {
        loaded(PinnedTable::pin(&self.store, table))
    }
}

// ---- 3. Milestone 3: the driver that answers the region lifecycle ----

/// What the driver keeps for one region the lifecycle asked for.
struct Wanted {
    seed: u64,
    baseline: Arc<Baseline>,
    /// The baseline's set plus the assets of migrated residents.
    set: AssetSet,
    /// The table the region last entered the tick with.
    table: Option<VersionTable>,
    /// The failure last reported for it, so one failure is one event.
    reported: Option<AssetFailure>,
    loaded: bool,
}

/// A `RegionRequest::Load`: the baseline first, by the content hash the atlas or the
/// region's saved records name.
fn driver_request_baseline(catalog: &dyn AssetCatalog, baseline_hash: u64) {
    catalog.request_newest(RegionBaseline::id(baseline_hash), LoadPriority(1));
}

/// Once the baseline is in: its declared set, with the assets of the migrated residents
/// `persist` read from the region's records, requested by name. Dependencies need no
/// request: the store follows them.
fn driver_request_set(
    catalog: &dyn AssetCatalog,
    seed: u64,
    baseline_hash: u64,
    residents: &AssetSet,
) -> Readiness<Wanted> {
    RegionBaseline::lookup(catalog, baseline_hash).map(|baseline| {
        let mut set = baseline.assets().clone();
        set.extend(residents.iter());
        for id in set.iter() {
            catalog.request_newest(id, LoadPriority(1));
        }
        Wanted { seed, baseline, set, table: None, reported: None, loaded: false }
    })
}

/// Once per frame, between runs: nothing is resolved while the generation stands still.
/// A first table enters with the region's bundle; a later one, after a pack was mounted
/// and its reloads published, enters through `TickInput::assets`, tick-stamped by the
/// frame that carries it; a failure enters as the event of architecture section 14.
fn driver_poll(
    catalog: &dyn AssetCatalog,
    published: &dyn PublishedAssets,
    seen: &mut u64,
    regions: &mut BTreeMap<RegionId, Wanted>,
    input: &mut TickInput,
) {
    let generation = catalog.generation();
    if generation == *seen {
        return;
    }
    *seen = generation;
    for (&region, wanted) in regions.iter_mut() {
        match catalog.resolve(&wanted.set) {
            Readiness::Ready(table) if wanted.table.as_ref() != Some(&table) => {
                // A table `resolve` returned always pins: nothing is unloaded.
                let Readiness::Ready(pinned) = PinnedTable::pin(published, &table) else {
                    continue;
                };
                if wanted.loaded {
                    input.assets.insert(region, pinned);
                } else {
                    input.loaded.insert(
                        region,
                        RegionBundle {
                            seed: wanted.seed,
                            mode: RegionMode::Frozen,
                            baseline: Arc::clone(&wanted.baseline),
                            overlay: None,
                            resources: Vec::new(),
                            transient_resources: Vec::new(),
                            assets: Some(pinned),
                        },
                    );
                    wanted.loaded = true;
                }
                wanted.table = Some(table);
            }
            Readiness::Ready(_) | Readiness::Pending => {}
            Readiness::Failed(failure) => {
                if wanted.reported != Some(failure) {
                    input.failures.push(failure);
                    wanted.reported = Some(failure);
                }
            }
        }
    }
}

/// Policy decided a failed asset is worth another try (a new pack was mounted by hand, a
/// player asked): the retry ends in a publication or a recorded failure, and either moves
/// the generation, so the driver looks again when it moves and never before.
fn driver_retry(catalog: &dyn AssetCatalog, failure: AssetFailure) {
    catalog.request_newest(failure.id, LoadPriority(1));
}

// ---- 4. A boundary world: exact versions, read through the pinned table ----

/// The table reaches the world at the input sync point (`sim` does this from
/// `TickInput::assets`); the one it replaces is handed back.
fn world_takes_table(
    world: &mut World,
    region: RegionId,
    pinned: PinnedTable,
) -> Result<Option<PinnedTable>> {
    world.set_region_assets(region, pinned)
}

/// What a kernel does with `Ctx::assets()`: a borrow from the region's table, no store,
/// no version, no atomic. Only a `SimulationAsset` can be read this way.
fn kernel_reads(
    assets: Option<&PinnedTable>,
    table: AssetRef<SpawnTable>,
) -> Result<u32, AssetFailure> {
    let Some(assets) = assets else { return Ok(0) };
    Ok(assets.get(table)?.rolls)
}

/// Outside the boundary a reader asks the catalogue and holds a handle; it asks again,
/// after the generation moved, to see a reload.
fn renderer_reads(catalog: &dyn AssetCatalog, mesh: AssetRef<Mesh>) -> Option<usize> {
    catalog.newest_typed(mesh).ready().map(|handle| handle.0.len())
}

// ---- 5. A client or a replay: the table it was given, by exact version ----

fn client_requests(published: &dyn PublishedAssets, table: &VersionTable) {
    for (id, version) in table.iter() {
        published.request(id, version, LoadPriority(2));
    }
}

fn client_pins(published: &dyn PublishedAssets, table: &VersionTable) -> Readiness<PinnedTable> {
    PinnedTable::pin(published, table)
}

// ---- 6. The cooker: cooked assets into one pack, written whole ----

/// One cook's output: every processed source and the region's baseline in one pack. An
/// ID added again with other content is `InvalidData` from `add`, which is where two
/// names that hash to one ID surface. The builder compresses; the cooker names no frame
/// rule.
fn cook_pack(
    executor: &Executor,
    io: &Io,
    sources: &[(&str, Vec<u8>)],
    baseline: &Baseline,
    out: PathBuf,
) -> Result<()> {
    let mut pack = PackBuilder::new();
    for (name, source) in sources {
        let cooked = MeshProcessor.process(source)?;
        pack.add(AssetId::from_name(name), &cooked, Storage::Zstd { level: 3 })?;
    }
    let (id, cooked) = RegionBaseline::cook(baseline);
    pack.add(id, &cooked, Storage::Zstd { level: 3 })?;
    executor.block_on(io.replace(out, pack.finish()?))?;
    Ok(())
}

/// The cooker's collision check runs over every name it publishes, the baseline's among
/// them: two names that hash to one ID, or a name that hashes to zero, are reported by
/// name. The baseline's name comes from the kind, not from a format string of the
/// cooker's.
fn cook_names(sources: &[(&str, Vec<u8>)], baseline: &Baseline) -> Vec<(AssetId, String)> {
    let baseline_name = RegionBaseline::name(baseline.content_hash());
    let mut names: Vec<(AssetId, String)> =
        sources.iter().map(|(name, _)| (AssetId::from_name(name), (*name).to_owned())).collect();
    names.push((RegionBaseline::id(baseline.content_hash()), baseline_name));
    names.sort();
    names
}

// ---- 7. Publication: the cooker's output reaching a running store ----

/// Whoever knows the cooker ran mounts its pack. The mount asks again for every followed
/// asset the pack renamed, and each of those loads moves the generation when it ends: a
/// frame loop sees the move in `driver_poll`; a tool waits for the loads and reads it.
async fn dev_reload(store: &Store, pack: PathBuf) -> Result<bool> {
    let before = store.generation();
    store.mount(pack).await?;
    store.settled().await;
    Ok(store.generation() != before)
}

/// A mount no longer wanted is dropped: nothing is mounted. The last handle dropped
/// cancels every load in flight; handles and pinned tables already taken stay valid.
fn cancel(mount: Mount, store: Store, kept: PinnedTable) -> PinnedTable {
    drop(mount);
    drop(store);
    kept
}

// ---- 8. persist, milestone 3: the baseline a region's records were made against ----

fn persist_baseline(
    catalog: &dyn AssetCatalog,
    records_baseline_hash: u64,
) -> Readiness<Arc<Baseline>> {
    RegionBaseline::lookup(catalog, records_baseline_hash)
}

/// A test of another crate that needs baselines from the reference store registers the
/// loader itself.
fn reference_store_loaders() -> Vec<ErasedLoader> {
    vec![ErasedLoader::new(RegionBaselineLoader)]
}

// ---- What is real now ----

#[test]
fn the_log_target_is_the_codename() {
    assert_eq!(LOG_TARGET, "silo");
}

#[test]
fn the_format_version_and_the_baseline_kind_are_pinned() {
    assert_eq!(PACK_FORMAT_VERSION, 1);
    assert_eq!(RegionBaseline::KIND, AssetKind::from_name("region_baseline"));
    assert_eq!(RegionBaseline::FORMAT_VERSION, 1);
    assert_ne!(RegionBaseline::KIND, SpawnTable::KIND);
}
