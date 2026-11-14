//! The region baseline kind (`README.md`, "The region baseline"), and what a boundary
//! world reads of a table resolved from the store.

#![forbid(unsafe_code)]

mod common;

use std::sync::Arc;

use common::*;
use lactose_asset::{RegionBaseline, RegionBaselineLoader, Storage};
use lactose_asset_api::conformance::{Harness, ReferenceStore};
use lactose_asset_api::{
    Asset, AssetCatalog, AssetFailure, AssetRef, AssetSet, CookedAsset, ErasedLoader, LoadPriority,
    Loader, PinnedTable, PublishedAssets, Readiness,
};
use lactose_bundle::Baseline;
use lactose_core::ErrorKind;
use lactose_ecs::{RegionConfig, RegionId, RegionMode, WorldKind};
use lactose_sim::{SimulationBuilder, TypeTable};

const NOW: LoadPriority = LoadPriority(0);

/// A baseline with no rows whose asset set names `names`: enough to differ by content.
fn baseline(names: &[&str]) -> Baseline {
    let mut assets = AssetSet::new();
    for name in names {
        assets.insert(id(name));
    }
    Baseline::from_rows(assets, [], []).expect("a baseline")
}

#[test]
fn the_name_and_the_id_are_golden() {
    assert_eq!(RegionBaseline::name(255), "region_baseline/00000000000000ff");
    assert_eq!(RegionBaseline::name(0x0123_4567_89AB_CDEF), "region_baseline/0123456789abcdef");
    assert_eq!(RegionBaseline::name(u64::MAX), "region_baseline/ffffffffffffffff");
    assert_eq!(RegionBaseline::name(0).len(), 32);
    for hash in [0, 255, 0x0123_4567_89AB_CDEF, u64::MAX] {
        let name = RegionBaseline::name(hash);
        assert_eq!(RegionBaseline::id(hash), lactose_asset_api::AssetId::from_name(&name));
    }
    let (id, cooked) = RegionBaseline::cook(&baseline(&["x"]));
    assert_eq!(id, RegionBaseline::id(baseline(&["x"]).content_hash()));
    assert_eq!(cooked.header.kind(), RegionBaseline::KIND);
    assert_eq!(RegionBaseline::KIND, lactose_asset_api::AssetKind::from_name("region_baseline"));
    assert_eq!(cooked.header.format_version(), 1);
    assert!(cooked.header.dependencies().is_empty());
    assert_eq!(cooked.bytes, baseline(&["x"]).encode());
}

#[test]
fn a_cooked_baseline_loads_equal_through_the_store_and_the_reference_store() {
    let original = baseline(&["a", "b"]);
    let hash = original.content_hash();
    let (id, cooked) = RegionBaseline::cook(&original);

    let env = Env::new("baseline_load");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    env.mount(&store, env.pack(&[(id, cooked.clone())], Storage::Zstd { level: 3 })).unwrap();
    assert_eq!(RegionBaseline::lookup(&store, hash), Readiness::Pending, "before the request");
    store.request_newest(id, NOW);
    env.settle(&store);
    let loaded = RegionBaseline::lookup(&store, hash).ready().expect("published");
    assert_eq!(*loaded, original);
    let again = RegionBaseline::lookup(&store, hash).ready().unwrap();
    assert!(Arc::ptr_eq(&loaded, &again), "shared, not copied");

    let reference = ReferenceStore::new(vec![ErasedLoader::new(RegionBaselineLoader)]);
    reference.provide(id, cooked.version(), cooked);
    reference.catalog().request_newest(id, NOW);
    reference.settle();
    let loaded = RegionBaseline::lookup(reference.catalog(), hash).ready().expect("published");
    assert_eq!(*loaded, original);
    env.shutdown();
}

#[test]
fn lookup_refuses_another_kind_and_a_baseline_under_the_wrong_id() {
    let env = Env::new("baseline_lookup");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let (named, other) = (baseline(&["named"]), baseline(&["other"]));
    let id = RegionBaseline::id(named.content_hash());

    // Another kind under the ID.
    let not_a_baseline = text(b"text", []);
    env.provide(&store, &[(id, not_a_baseline.clone())]);
    store.request_newest(id, NOW);
    env.settle(&store);
    let failure =
        AssetFailure { id, version: Some(not_a_baseline.version()), kind: ErrorKind::InvalidData };
    assert_eq!(RegionBaseline::lookup(&store, named.content_hash()), Readiness::Failed(failure));

    // A baseline under another baseline's ID.
    let (_, wrong) = RegionBaseline::cook(&other);
    env.provide(&store, &[(id, wrong.clone())]);
    env.settle(&store);
    let failure = AssetFailure { id, version: Some(wrong.version()), kind: ErrorKind::InvalidData };
    assert_eq!(RegionBaseline::lookup(&store, named.content_hash()), Readiness::Failed(failure));
    env.shutdown();
}

#[test]
fn the_loader_refuses_a_baseline_with_a_dependency() {
    let original = baseline(&["a"]);
    let dependency = id("a dependency");
    let bytes = original.encode();
    let refused = RegionBaselineLoader.load(&bytes, &[dependency]).unwrap_err();
    assert_eq!(refused.kind(), ErrorKind::InvalidData);
    assert_eq!(*RegionBaselineLoader.load(&bytes, &[]).unwrap().baseline().as_ref(), original);

    let env = Env::new("baseline_dependency");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let id = RegionBaseline::id(original.content_hash());
    let cooked = CookedAsset::new::<RegionBaseline>(bytes, [dependency]);
    env.provide(&store, &[(id, cooked), (dependency, text(b"loads", []))]);
    store.request_newest(id, NOW);
    env.settle(&store);
    assert!(store.newest(dependency).ready().is_some());
    assert!(matches!(
        RegionBaseline::lookup(&store, original.content_hash()),
        Readiness::Failed(failure) if failure.kind == ErrorKind::InvalidData
    ));
    env.shutdown();
}

/// A table resolved and pinned from the store enters a world through
/// `World::set_region_assets`, and the world reads the version it was given until a new
/// table is set. No system runs: running one steps a simulation world, which is
/// `[harness]` work (see the crate's README, "Proof for milestone 1").
#[test]
fn a_world_reads_the_table_it_was_given_until_the_next_is_set() {
    let env = Env::new("baseline_world");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let spawns = id("spawn table");
    let (one, two) = (text(b"one", []), text(b"two", []));
    env.provide(&store, &[(spawns, one.clone())]);
    store.request_newest(spawns, NOW);
    env.settle(&store);

    let mut set = AssetSet::new();
    set.insert(spawns);
    let published: &dyn PublishedAssets = &store;
    let pin = |set: &AssetSet| {
        let table = store.resolve(set).ready().expect("resolved");
        PinnedTable::pin(published, &table).ready().expect("pinned")
    };

    let mut simulation =
        SimulationBuilder::new(WorldKind::Server, TypeTable::new()).build().expect("a world");
    let world = simulation.world_mut();
    let region = RegionId(1);
    world.create_region(RegionConfig { id: region, seed: 7, mode: RegionMode::Simulated }).unwrap();
    let reads = |world: &lactose_ecs::World| -> Vec<u8> {
        let table = world.region_assets(region).expect("a table");
        table.get(AssetRef::<Text>::new(spawns)).expect("a simulation asset").0.clone()
    };
    assert!(world.set_region_assets(region, pin(&set)).unwrap().is_none());
    assert_eq!(reads(world), b"one");

    env.provide(&store, &[(spawns, two.clone())]);
    env.settle(&store);
    assert_eq!(store.newest(spawns).ready().unwrap().version(), two.version());
    assert_eq!(reads(world), b"one", "the world keeps its table across the reload");
    let previous = world.set_region_assets(region, pin(&set)).unwrap().expect("the old table");
    assert_eq!(previous.table().get(spawns), Some(one.version()));
    assert_eq!(reads(world), b"two");
    env.shutdown();
}

/// The store registers the baseline's loader itself: a second one is refused like a second
/// loader of any kind.
#[test]
fn a_second_loader_of_one_kind_is_refused_the_baseline_kind_included() {
    let env = Env::new("baseline_second_loader");
    let two = vec![ErasedLoader::new(TextLoader), ErasedLoader::new(TextLoader)];
    let refused = lactose_asset::Store::new(env.executor.clone(), env.io.clone(), two);
    assert_eq!(refused.err().map(|error| error.kind()), Some(ErrorKind::InvalidData));
    let baseline = vec![ErasedLoader::new(RegionBaselineLoader)];
    let refused = lactose_asset::Store::new(env.executor.clone(), env.io.clone(), baseline);
    assert_eq!(refused.err().map(|error| error.kind()), Some(ErrorKind::InvalidData));
    env.shutdown();
}
