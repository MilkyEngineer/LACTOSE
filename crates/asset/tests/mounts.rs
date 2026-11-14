//! Mounting (`README.md`, "Mounting" and "Hot reload"): the order of places, a publish
//! seen through the generation, what a mount follows, and a new source for a failed
//! version.

#![forbid(unsafe_code)]

mod common;

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Waker};

use common::capture::capture;
use common::format::index_range;
use common::*;
use lactose_asset::{Storage, Store};
use lactose_asset_api::{
    AssetCatalog, AssetFailure, AssetId, AssetRef, AssetSet, AssetVersion, LoadPriority,
    PinnedTable, PublishedAssets, Readiness,
};
use lactose_core::ErrorKind;

const NOW: LoadPriority = LoadPriority(0);

fn newest_version(store: &Store, id: AssetId) -> Readiness<AssetVersion> {
    store.newest(id).map(|handle| handle.version())
}

fn set(ids: &[AssetId]) -> AssetSet {
    let mut set = AssetSet::new();
    for &id in ids {
        set.insert(id);
    }
    set
}

#[test]
fn the_named_version_is_the_last_calls_whatever_order_the_mounts_end_in() {
    let env = Env::new("mounts_order");
    let x = id("x");
    let versions = [text(b"one", []), text(b"two", []), text(b"three", [])];
    let paths: Vec<_> =
        versions.iter().map(|cooked| env.pack(&[(x, cooked.clone())], Storage::Raw)).collect();

    // Mounted in turn.
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    for path in &paths {
        env.mount(&store, path.clone()).unwrap();
    }
    store.request_newest(x, NOW);
    env.settle(&store);
    assert_eq!(newest_version(&store, x), Readiness::Ready(versions[2].version()));
    for cooked in &versions {
        store.request(x, cooked.version(), NOW);
    }
    env.settle(&store);
    for cooked in &versions {
        let handle = store.get(x, cooked.version()).ready().expect("each version is found");
        let catalog: &dyn AssetCatalog = &store;
        let typed = catalog.get_typed(AssetRef::<Text>::new(x), handle.version()).ready();
        assert_eq!(typed.unwrap().0, cooked.bytes);
    }

    // Created in order, awaited in reverse.
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let mounts: Vec<_> = paths.iter().map(|path| store.mount(path.clone())).collect();
    for mount in mounts.into_iter().rev() {
        env.wait(mount).unwrap();
    }
    store.request_newest(x, NOW);
    env.settle(&store);
    assert_eq!(newest_version(&store, x), Readiness::Ready(versions[2].version()));
    env.shutdown();
}

#[test]
fn a_failed_and_a_dropped_mount_leave_their_place_empty_and_change_nothing() {
    let env = Env::new("mounts_empty_places");
    let x = id("x");
    let (first, second) = (text(b"first", []), text(b"second", []));
    let first_path = env.pack(&[(x, first.clone())], Storage::Raw);
    let second_path = env.pack(&[(x, second.clone())], Storage::Raw);
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    env.mount(&store, first_path).unwrap();
    store.request_newest(x, NOW);
    env.settle(&store);
    let generation = store.generation();

    let missing = env.mount(&store, env.dir.join("no such pack")).unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::NotFound);
    drop(store.mount(second_path.clone()));
    // Polled once, then dropped: its open may have started.
    {
        let mut mount = pin!(store.mount(second_path));
        let poll = mount.as_mut().poll(&mut Context::from_waker(Waker::noop()));
        assert!(poll.is_pending(), "a mount resolves no sooner than its reads");
    }
    env.settle(&store);
    assert_eq!(store.generation(), generation);
    store.request_newest(x, NOW);
    env.settle(&store);
    assert_eq!(newest_version(&store, x), Readiness::Ready(first.version()));
    assert_eq!(store.generation(), generation);
    store.request(x, second.version(), NOW);
    env.settle(&store);
    let failure =
        AssetFailure { id: x, version: Some(second.version()), kind: ErrorKind::NotFound };
    assert_eq!(store.get(x, second.version()).map(|_| ()), Readiness::Failed(failure));
    env.shutdown();
}

#[test]
fn a_path_replaced_by_rename_and_mounted_again_serves_both_contents() {
    let env = Env::new("mounts_rename");
    let (x, y) = (id("x"), id("y"));
    let (old, new, only_old) = (text(b"old", []), text(b"new", []), text(b"only in the old", []));
    let path = env.pack(&[(x, old.clone()), (y, only_old.clone())], Storage::Raw);
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    env.mount(&store, path.clone()).unwrap();
    store.request_newest(x, NOW);
    env.settle(&store);
    assert_eq!(newest_version(&store, x), Readiness::Ready(old.version()));

    env.write_at(path.clone(), pack(&[(x, new.clone())], Storage::Raw));
    env.mount(&store, path).unwrap();
    env.settle(&store);
    assert_eq!(newest_version(&store, x), Readiness::Ready(new.version()));
    // The first file stays open under its place: what only it holds is still read from it.
    store.request(y, only_old.version(), NOW);
    env.settle(&store);
    let catalog: &dyn AssetCatalog = &store;
    let typed = catalog.get_typed(AssetRef::<Text>::new(y), only_old.version()).ready();
    assert_eq!(typed.unwrap().0, b"only in the old");
    assert_eq!(store.get(x, old.version()).ready().unwrap().version(), old.version());
    env.shutdown();
}

#[test]
fn a_reload_is_seen_through_the_generation_and_old_readers_keep_the_old_version() {
    let env = Env::new("mounts_publish");
    let x = id("x");
    let (old, new) = (gated(b"old", []), gated(b"new", []));
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let old_path = env.pack(&[(x, old.clone())], Storage::Raw);
    env.mount(&store, old_path.clone()).unwrap();
    store.request_newest(x, NOW);
    env.settle(&store);
    let handle = store.newest(x).ready().unwrap();
    let table = store.resolve(&set(&[x])).ready().unwrap();
    let published: &dyn PublishedAssets = &store;
    let pinned = PinnedTable::pin(published, &table).ready().unwrap();
    let generation = store.generation();

    gate.hold(b"new");
    env.mount(&store, env.pack(&[(x, new.clone())], Storage::Raw)).unwrap();
    assert_eq!(store.generation(), generation, "the mount itself publishes nothing");
    gate.wait_entered(b"new");
    assert_eq!(store.generation(), generation);
    assert_eq!(newest_version(&store, x), Readiness::Ready(old.version()));
    gate.open(b"new");
    env.settle(&store);
    assert!(store.generation() > generation);
    assert_eq!(newest_version(&store, x), Readiness::Ready(new.version()));
    let reloaded = store.resolve(&set(&[x])).ready().unwrap();
    assert_eq!(reloaded.get(x), Some(new.version()));
    assert_eq!(table.get(x), Some(old.version()));
    assert_eq!(handle.downcast::<Gated>().unwrap().0, b"old");
    assert_eq!(pinned.get(AssetRef::<Gated>::new(x)).unwrap().0, b"old");

    // A pack that renames nothing followed moves nothing.
    let generation = store.generation();
    env.mount(&store, env.pack(&[(id("unfollowed"), text(b"z", []))], Storage::Raw)).unwrap();
    env.settle(&store);
    assert_eq!(store.generation(), generation);

    // A pack naming an earlier, published version makes it current at its mount.
    env.mount(&store, old_path).unwrap();
    let moved = store.generation();
    assert!(moved > generation, "current changed at the mount");
    assert_eq!(newest_version(&store, x), Readiness::Ready(old.version()));
    env.settle(&store);
    assert_eq!(store.generation(), moved, "and nothing was loaded");
    assert_eq!(gate.entered().len(), 2);
    env.shutdown();
}

#[test]
fn a_mount_reloads_what_is_followed_and_nothing_else() {
    let capture = capture();
    let env = Env::new("mounts_following");
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let (exact, dependant, dependency) = (id("exact"), id("dependant"), id("dependency"));
    let exact_v1 = text(b"exact one", []);
    let dependency_v1 = text(b"dependency one", []);
    env.provide(
        &store,
        &[
            (exact, exact_v1.clone()),
            (dependant, text(b"dependant", [dependency])),
            (dependency, dependency_v1.clone()),
        ],
    );
    store.request(exact, exact_v1.version(), NOW);
    store.request_newest(dependant, NOW);
    env.settle(&store);
    assert!(store.get(exact, exact_v1.version()).ready().is_some());
    let generation = store.generation();

    // An asset requested only by exact version is not reloaded.
    let exact_v2 = text(b"exact two", []);
    env.provide(&store, &[(exact, exact_v2.clone())]);
    env.settle(&store);
    assert_eq!(store.generation(), generation);
    assert_eq!(store.get(exact, exact_v2.version()).map(|_| ()), Readiness::Pending);

    // A dependency nobody named is.
    let dependency_v2 = text(b"dependency two", []);
    env.provide(&store, &[(dependency, dependency_v2.clone())]);
    env.settle(&store);
    assert_eq!(newest_version(&store, dependency), Readiness::Ready(dependency_v2.version()));

    // An asset requested before any pack held it loads when one does.
    let late = id("late");
    store.request_newest(late, NOW);
    env.settle(&store);
    assert_eq!(
        store.newest(late).map(|_| ()),
        Readiness::Failed(AssetFailure { id: late, version: None, kind: ErrorKind::NotFound })
    );
    let late_v1 = text(b"late", []);
    env.provide(&store, &[(late, late_v1.clone())]);
    env.settle(&store);
    assert_eq!(newest_version(&store, late), Readiness::Ready(late_v1.version()));

    // An asset that failed for itself is not read again by a mount that does not hold it;
    // `request_newest` reads it again.
    let refused = id("refused");
    env.provide(&store, &[(refused, gated(b"refuse me", []))]);
    store.request_newest(refused, NOW);
    env.settle(&store);
    assert_eq!(gate.entered().len(), 1);
    let failed_for_it = id("failed for it");
    env.provide(&store, &[(failed_for_it, text(b"needs the refused", [refused]))]);
    store.request_newest(failed_for_it, NOW);
    env.settle(&store);
    assert!(matches!(store.newest(failed_for_it), Readiness::Failed(_)));
    // Asked for as a dependency of a new load, as by `request_newest`, it was read again.
    assert_eq!(gate.entered().len(), 2);
    let generation = store.generation();
    let from = capture.span_count();
    env.provide(&store, &[(id("unrelated"), text(b"unrelated", []))]);
    env.settle(&store);
    assert_eq!(store.generation(), generation);
    assert_eq!(capture.closed(from, "io.read").len(), 2, "the mount's two reads alone");
    assert_eq!(gate.entered().len(), 2);

    store.request_newest(refused, NOW);
    env.settle(&store);
    assert_eq!(gate.entered().len(), 3);
    assert!(store.generation() > generation, "a retry that fails again moves it");
    env.shutdown();
}

#[test]
fn a_missing_dependency_added_later_repairs_its_dependants_at_that_mount() {
    let env = Env::new("mounts_repair");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let (top, middle, bottom) = (id("top"), id("middle"), id("bottom"));
    env.provide(&store, &[(top, text(b"top", [middle])), (middle, text(b"middle", [bottom]))]);
    store.request_newest(top, NOW);
    env.settle(&store);
    for asset in [top, middle] {
        let failure = store.newest(asset).map(|_| ());
        assert!(
            matches!(failure, Readiness::Failed(failure) if failure.kind == ErrorKind::NotFound)
        );
    }
    env.provide(&store, &[(bottom, text(b"bottom", []))]);
    env.settle(&store);
    for asset in [top, middle, bottom] {
        assert!(store.newest(asset).ready().is_some(), "{asset:?}");
    }
    let table = store.resolve(&set(&[top])).ready().unwrap();
    assert_eq!(table.len(), 3);
    env.shutdown();
}

/// Flips a byte of the payload of the entry `at` (in ID order) of a raw pack.
fn damage(pack: &mut [u8], at: usize) -> usize {
    let (start, _) = index_range(pack);
    let entry = start + 4 + 57 * at + 33;
    let offset = u64::from_le_bytes(pack[entry..entry + 8].try_into().unwrap()) as usize;
    pack[offset] ^= 0x20;
    offset
}

fn ordered(first: &str, second: &str) -> (AssetId, AssetId) {
    let (a, b) = (id(first), id(second));
    if a < b { (a, b) } else { (b, a) }
}

#[test]
fn a_second_source_for_a_damaged_version_repairs_it_and_its_dependant() {
    let env = Env::new("mounts_new_source");
    // The dependency below its dependant, so that it is entry 0.
    let (dependency, dependant) = ordered("a dependency", "a dependant");
    let assets = [
        (dependency, text(b"intact dependency", [])),
        (dependant, text(b"its dependant", [dependency])),
    ];
    for cut in [false, true] {
        let (loaders, _) = test_loaders();
        let store = env.store(loaders);
        let path = if cut {
            // The range made to run past the file: the file shortened in place after its
            // mount, as damage would.
            let path = env.write(pack(&assets, Storage::Raw));
            env.mount(&store, path.clone()).unwrap();
            let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            file.set_len(3).unwrap();
            path
        } else {
            let mut bytes = pack(&assets, Storage::Raw);
            damage(&mut bytes, 0);
            let path = env.write(bytes);
            env.mount(&store, path.clone()).unwrap();
            path
        };
        store.request_newest(dependant, NOW);
        env.settle(&store);
        for asset in [dependency, dependant] {
            assert!(matches!(store.newest(asset), Readiness::Failed(_)), "cut: {cut}");
        }
        let generation = store.generation();
        env.provide(&store, &assets);
        env.settle(&store);
        assert!(store.generation() > generation);
        for (asset, cooked) in &assets {
            let version = newest_version(&store, *asset);
            assert_eq!(version, Readiness::Ready(cooked.version()), "cut: {cut}");
        }
        drop(path);
    }
    env.shutdown();
}

#[test]
fn a_second_source_reads_nothing_for_a_failure_the_bytes_did_not_cause() {
    let capture = capture();
    let env = Env::new("mounts_same_failure");
    let cases = [
        ("the loader refuses the bytes", gated(b"refuse these", [])),
        ("another cooked format", stale_text(b"stale")),
        ("no loader for the kind", unloadable(b"unloadable", [])),
    ];
    for (what, cooked) in cases {
        let (loaders, _) = test_loaders();
        let store = env.store(loaders);
        let x = id("x");
        env.provide(&store, &[(x, cooked.clone())]);
        store.request_newest(x, NOW);
        env.settle(&store);
        assert!(matches!(store.newest(x), Readiness::Failed(_)), "{what}");
        let generation = store.generation();
        let from = capture.span_count();
        env.provide(&store, &[(x, cooked)]);
        env.settle(&store);
        assert_eq!(store.generation(), generation, "{what}");
        assert_eq!(capture.closed(from, "io.read").len(), 2, "{what}: the mount's reads alone");
    }

    // A second source given an earlier place than the damaged one is not where the
    // version is read from.
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let x = id("x");
    let intact = [(x, text(b"intact", []))];
    let mut damaged = pack(&intact, Storage::Raw);
    damage(&mut damaged, 0);
    let early = store.mount(env.pack(&intact, Storage::Raw));
    env.mount(&store, env.write(damaged)).unwrap();
    store.request_newest(x, NOW);
    env.settle(&store);
    assert!(matches!(store.newest(x), Readiness::Failed(_)));
    let generation = store.generation();
    let from = capture.span_count();
    env.wait(early).unwrap();
    env.settle(&store);
    assert_eq!(store.generation(), generation);
    assert_eq!(capture.closed(from, "io.read").len(), 2);
    env.shutdown();
}
