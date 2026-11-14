//! Mounting (`README.md`, "Mounting" and "Hot reload"): the order of places, a publish
//! seen through the generation, what a mount follows, and a new source for a failed
//! version.

#![forbid(unsafe_code)]

mod common;

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Waker};

use common::capture::capture;
use common::format::{dependency_count, entry_at, reseal};
use common::*;
use lactose_asset::{Storage, Store};
use lactose_asset_api::{
    AssetCatalog, AssetFailure, AssetId, AssetRef, AssetSet, AssetVersion, CookedAsset,
    LoadPriority, PinnedTable, PublishedAssets, Readiness,
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
    // Held until both dropped mounts are gone, so the one polled once is dropped with its
    // open still queued: a pool fast enough to finish the open and both reads within that
    // poll would resolve it there and mount the pack.
    let (store, pool) = env.held_store(loaders);
    // Places 0 to 3, taken at the calls; nothing is opened before a mount's first poll.
    let mounted = store.mount(first_path);
    let missing = store.mount(env.dir.join("no such pack"));
    drop(store.mount(second_path.clone()));
    {
        let mut mount = pin!(store.mount(second_path));
        let poll = mount.as_mut().poll(&mut Context::from_waker(Waker::noop()));
        assert!(poll.is_pending(), "the open is queued on the held pool");
    }
    pool.release();
    env.wait(mounted).unwrap();
    store.request_newest(x, NOW);
    env.settle(&store);
    assert_eq!(newest_version(&store, x), Readiness::Ready(first.version()));
    let generation = store.generation();

    let missing = env.wait(missing).unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::NotFound);
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

/// Flips the first byte of the payload of the entry `at` (in ID order) of a raw pack.
fn damage(pack: &mut [u8], at: usize) {
    let entry = entry_at(pack, at);
    let field = entry + 33 + 8 * dependency_count(pack, entry);
    let offset = u64::from_le_bytes(pack[field..field + 8].try_into().unwrap()) as usize;
    pack[offset] ^= 0x20;
}

/// What a store new to the process answers for the newest version of `ask` after
/// mounting `packs` in order and requesting it: what the store that mounted them as it
/// ran must answer too, once it settles.
fn fresh(env: &Env, packs: &[std::path::PathBuf], ask: AssetId) -> Readiness<AssetVersion> {
    let (loaders, gate) = test_loaders();
    gate.open_all();
    let store = env.store(loaders);
    for path in packs {
        env.mount(&store, path.clone()).unwrap();
    }
    store.request_newest(ask, NOW);
    env.settle(&store);
    newest_version(&store, ask)
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

/// Rule 2 in flight: a later pack holds a version intact while its load from a damaged
/// source is still in flight, held there by a dependency whose loader is held. The load
/// reads again from the new source when its own steps fail for the bytes, as a store new
/// to the process that mounted the same packs would read it.
#[test]
fn a_new_source_mounted_while_a_damaged_load_is_in_flight_repairs_it() {
    let env = Env::new("mounts_new_source_in_flight");
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let (held, damaged) = (id("a held dependency"), id("a damaged dependant"));
    let mut assets = vec![(held, gated(b"hold", [])), (damaged, text(b"intact", [held]))];
    assets.sort_by_key(|(id, _)| *id);
    let at = assets.iter().position(|(id, _)| *id == damaged).unwrap();
    let mut bytes = pack(&assets, Storage::Raw);
    damage(&mut bytes, at);
    let first = env.write(bytes);
    let second = env.pack(&assets, Storage::Raw);

    env.mount(&store, first.clone()).unwrap();
    gate.hold(b"hold");
    store.request_newest(damaged, NOW);
    gate.wait_entered(b"hold");
    env.mount(&store, second.clone()).unwrap();
    let generation = store.generation();
    gate.open(b"hold");
    env.settle(&store);
    let expected = Readiness::Ready(assets[at].1.version());
    assert_eq!(fresh(&env, &[first, second], damaged), expected);
    assert_eq!(newest_version(&store, damaged), expected);
    // The same header: the load reads again as the same load, and records no failure.
    assert_eq!(store.generation(), generation + 2, "two publishes and nothing between");
    env.shutdown();
}

/// Rule 3 for a dependency renamed back to a version still in flight: the dependant that
/// failed for the version in between is read again, as a store new to the process that
/// mounted the same packs would read it.
#[test]
fn a_dependency_renamed_back_to_a_version_in_flight_repairs_its_dependant() {
    // The held call keeps one background worker; the dependant's own steps need another.
    let env = Env::with_background_workers("mounts_renamed_back", 2);
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let (dependant, dependency) = (id("a dependant"), id("a dependency renamed back"));
    let good = gated(b"hold the good one", []);
    let first = env
        .pack(&[(dependant, text(b"a", [dependency])), (dependency, good.clone())], Storage::Raw);
    let bad = env.pack(&[(dependency, unloadable(b"of a kind with no loader", []))], Storage::Raw);
    let back = env.pack(&[(dependency, good.clone())], Storage::Raw);

    env.mount(&store, first.clone()).unwrap();
    gate.hold(b"hold the good one");
    store.request_newest(dependant, NOW);
    gate.wait_entered(b"hold the good one");
    env.mount(&store, bad.clone()).unwrap();
    eventually("the dependant failed for the version in between", || {
        matches!(store.newest(dependant), Readiness::Failed(_))
    });
    env.mount(&store, back.clone()).unwrap();
    gate.open(b"hold the good one");
    env.settle(&store);
    assert_eq!(newest_version(&store, dependency), Readiness::Ready(good.version()));
    let expected = fresh(&env, &[first, bad, back], dependant);
    assert!(matches!(expected, Readiness::Ready(_)), "{expected:?}");
    assert_eq!(newest_version(&store, dependant), expected);
    env.shutdown();
}

/// Rule 2 for a load in flight whose newer source has another header: the first pack's
/// entry is the second's with `edit` applied to its index entry and the index resealed,
/// damage the version's hash catches, since it covers the header. `a` waits for a held
/// dependency while the second pack is mounted; the store must end where a store new to
/// the process that mounted both packs ends.
fn newer_source_with_another_header(
    name: &str,
    assets: &[(AssetId, CookedAsset)],
    damaged: &[(AssetId, CookedAsset)],
    a: AssetId,
    edit: impl FnOnce(&mut [u8], usize),
) -> Readiness<AssetVersion> {
    // The held call keeps one background worker; the damaged load's own steps need another.
    let env = Env::with_background_workers(name, 2);
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let at = damaged.iter().position(|(id, _)| *id == a).unwrap();
    let mut bytes = pack(damaged, Storage::Raw);
    let entry = entry_at(&bytes, at);
    edit(&mut bytes, entry);
    reseal(&mut bytes);
    let (first, second) = (env.write(bytes), env.pack(assets, Storage::Raw));
    let expected = fresh(&env, &[first.clone(), second.clone()], a);

    env.mount(&store, first).unwrap();
    gate.hold(b"hold");
    store.request_newest(a, NOW);
    gate.wait_entered(b"hold");
    env.mount(&store, second).unwrap();
    // A wait first polled while `a`'s first load is in flight covers the load that
    // replaces it.
    let mut settled = pin!(store.settled());
    assert!(settled.as_mut().poll(&mut Context::from_waker(Waker::noop())).is_pending());
    gate.open(b"hold");
    env.wait(settled);
    assert_eq!(newest_version(&store, a), expected);
    env.shutdown();
    expected
}

/// Another kind: the damaged entry names the text kind, the intact one a kind with no
/// loader. The load ends, and the newest is asked for again from the intact entry:
/// `Unsupported`, with no loader call.
#[test]
fn a_newer_source_of_another_kind_ends_the_load_and_asks_again() {
    let (held, a) = (id("held"), id("of another kind"));
    let mut assets = vec![(held, gated(b"hold", [])), (a, unloadable(b"payload", [held]))];
    assets.sort_by_key(|(id, _)| *id);
    let text_kind = text(b"any", []).header.kind().0.to_le_bytes();
    let answer = newer_source_with_another_header("mounts_newer_kind", &assets, &assets, a, {
        move |bytes, entry| bytes[entry + 16..entry + 24].copy_from_slice(&text_kind)
    });
    let failure = AssetFailure {
        id: a,
        version: Some(assets.iter().find(|(id, _)| *id == a).unwrap().1.version()),
        kind: ErrorKind::Unsupported,
    };
    assert_eq!(answer, Readiness::Failed(failure));
}

/// Another dependency list: the damaged entry declares `held` alone under the intact
/// entry's version, the intact one `held` and `extra`. The load ends, and the newest is
/// asked for again from the intact entry, which asks for `extra` and is published.
#[test]
fn a_newer_source_with_another_dependency_ends_the_load_and_asks_again() {
    let (held, a, extra) = (id("held"), id("with another dependency"), id("extra"));
    let mut assets = vec![
        (held, gated(b"hold", [])),
        (a, text(b"payload", [held, extra])),
        (extra, text(b"extra", [])),
    ];
    assets.sort_by_key(|(id, _)| *id);
    let at = assets.iter().position(|(id, _)| *id == a).unwrap();
    let version = assets[at].1.version();
    let mut damaged = assets.clone();
    damaged[at].1 = text(b"payload", [held]);
    let answer = newer_source_with_another_header("mounts_newer_deps", &assets, &damaged, a, {
        move |bytes, entry| {
            bytes[entry + 8..entry + 16].copy_from_slice(&version.to_bits().to_le_bytes())
        }
    });
    assert_eq!(answer, Readiness::Ready(version));
}

/// A pack of `assets` whose entry for `a` claims `version`, the version of another header:
/// damage the version's hash catches.
fn claiming(assets: &[(AssetId, CookedAsset)], a: AssetId, version: AssetVersion) -> Vec<u8> {
    let mut bytes = pack(assets, Storage::Raw);
    let entry = entry_at(&bytes, assets.iter().position(|(id, _)| *id == a).unwrap());
    bytes[entry + 8..entry + 16].copy_from_slice(&version.to_bits().to_le_bytes());
    reseal(&mut bytes);
    bytes
}

fn sorted(mut assets: Vec<(AssetId, CookedAsset)>) -> Vec<(AssetId, CookedAsset)> {
    assets.sort_by_key(|(id, _)| *id);
    assets
}

fn version_of(assets: &[(AssetId, CookedAsset)], a: AssetId) -> AssetVersion {
    assets.iter().find(|(id, _)| *id == a).unwrap().1.version()
}

fn pending<F: Future>(future: std::pin::Pin<&mut F>) -> bool {
    future.poll(&mut Context::from_waker(Waker::noop())).is_pending()
}

/// Rule 2 onto another header renews the load whatever asked for it: a version requested
/// by exact version alone, not followed, is published as a store new to the process
/// publishes it, by a load of cause `"renew"` at the request's priority.
#[test]
fn an_exact_request_onto_another_header_is_renewed() {
    let capture = capture();
    // The held call keeps one background worker; the damaged load's own steps need another.
    let env = Env::with_background_workers("mounts_renew_exact", 2);
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let (held, a, extra) = (id("held"), id("asked by exact version"), id("extra"));
    let assets = sorted(vec![
        (held, gated(b"hold", [])),
        (a, text(b"payload", [held, extra])),
        (extra, text(b"extra", [])),
    ]);
    let version = version_of(&assets, a);
    let damaged = sorted(vec![
        (held, gated(b"hold", [])),
        (a, text(b"payload", [held])),
        (extra, text(b"extra", [])),
    ]);
    let first = env.write(claiming(&damaged, a, version));
    let second = env.pack(&assets, Storage::Raw);
    let priority = LoadPriority(3);
    let fresh = {
        let (loaders, gate) = test_loaders();
        gate.open_all();
        let fresh = env.store(loaders);
        env.mount(&fresh, first.clone()).unwrap();
        env.mount(&fresh, second.clone()).unwrap();
        fresh.request(a, version, priority);
        env.settle(&fresh);
        fresh.get(a, version).map(|handle| handle.version())
    };
    assert_eq!(fresh, Readiness::Ready(version));

    env.mount(&store, first).unwrap();
    gate.hold(b"hold");
    store.request(a, version, priority);
    gate.wait_entered(b"hold");
    env.mount(&store, second).unwrap();
    let from = capture.span_count();
    gate.open(b"hold");
    env.settle(&store);
    assert_eq!(store.get(a, version).map(|handle| handle.version()), fresh);
    let renewals: Vec<_> = capture
        .named(from, "silo.load")
        .into_iter()
        .filter(|(_, span)| span.field("cause") == Some("renew"))
        .collect();
    assert_eq!(renewals.len(), 1, "one renewal");
    assert_eq!(renewals[0].1.field("priority"), Some("3"), "at the request's priority");
    env.shutdown();
}

/// A dependant of a renewed version waits through the renewal, `Pending` while the ended
/// load's failure is the version's answer, and is published as a store new to the process
/// publishes it; a wait first polled before covers the renewal.
#[test]
fn a_dependant_waits_through_the_renewal() {
    // Two held calls at once at most, and a load's own steps beside them.
    let env = Env::with_background_workers("mounts_renew_dependant", 3);
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let (held, a, extra, d) = (id("held"), id("renewed"), id("extra"), id("dependant"));
    let assets = sorted(vec![
        (held, gated(b"hold", [])),
        (a, text(b"payload", [held, extra])),
        (extra, gated(b"extra", [])),
        (d, text(b"dependant", [a])),
    ]);
    let damaged = sorted(vec![
        (held, gated(b"hold", [])),
        (a, text(b"payload", [held])),
        (extra, gated(b"extra", [])),
        (d, text(b"dependant", [a])),
    ]);
    let first = env.write(claiming(&damaged, a, version_of(&assets, a)));
    let second = env.pack(&assets, Storage::Raw);
    let expected = fresh(&env, &[first.clone(), second.clone()], d);
    assert_eq!(expected, Readiness::Ready(version_of(&assets, d)));

    env.mount(&store, first).unwrap();
    gate.hold(b"hold");
    gate.hold(b"extra");
    store.request_newest(d, NOW);
    gate.wait_entered(b"hold");
    env.mount(&store, second).unwrap();
    let mut settled = pin!(store.settled());
    assert!(pending(settled.as_mut()));
    gate.open(b"hold");
    // The renewal asks for `extra`, which only the intact header declares.
    gate.wait_entered(b"extra");
    assert!(matches!(store.newest(a), Readiness::Failed(_)), "the ended load's failure");
    assert_eq!(newest_version(&store, d), Readiness::Pending, "the dependant waits");
    assert!(pending(settled.as_mut()), "the wait covers the renewal");
    gate.open(b"extra");
    env.wait(settled);
    assert_eq!(newest_version(&store, d), expected);
    env.shutdown();
}

/// Two loads renewed in one step each keep their own place: a wait first polled before
/// both resolves only once both renewals have ended.
#[test]
fn two_renewals_in_one_step_each_keep_their_place() {
    // Two held calls at once at most, and a load's own steps beside them.
    let env = Env::with_background_workers("mounts_renew_two", 3);
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let (held, a1, a2, x1, x2) = (id("held"), id("a1"), id("a2"), id("x1"), id("x2"));
    let make = |one: Vec<AssetId>, two: Vec<AssetId>| {
        sorted(vec![
            (held, gated(b"hold", [])),
            (a1, text(b"one", one)),
            (a2, text(b"two", two)),
            (x1, gated(b"x1", [])),
            (x2, gated(b"x2", [])),
        ])
    };
    let assets = make(vec![held, x1], vec![held, x2]);
    let (v1, v2) = (version_of(&assets, a1), version_of(&assets, a2));
    let damaged = make(vec![held], vec![held]);
    let mut bytes = claiming(&damaged, a1, v1);
    let entry = entry_at(&bytes, damaged.iter().position(|(id, _)| *id == a2).unwrap());
    bytes[entry + 8..entry + 16].copy_from_slice(&v2.to_bits().to_le_bytes());
    reseal(&mut bytes);
    let first = env.write(bytes);
    let second = env.pack(&assets, Storage::Raw);

    env.mount(&store, first).unwrap();
    for held in [&b"hold"[..], b"x1", b"x2"] {
        gate.hold(held);
    }
    store.request_newest(a1, NOW);
    store.request_newest(a2, NOW);
    gate.wait_entered(b"hold");
    env.mount(&store, second).unwrap();
    let mut settled = pin!(store.settled());
    assert!(pending(settled.as_mut()));
    gate.open(b"hold");
    gate.wait_entered(b"x1");
    gate.wait_entered(b"x2");
    gate.open(b"x1");
    eventually("a1 published", || newest_version(&store, a1) == Readiness::Ready(v1));
    assert!(pending(settled.as_mut()), "a2's renewal is still in flight");
    gate.open(b"x2");
    env.wait(settled);
    assert_eq!(newest_version(&store, a2), Readiness::Ready(v2));
    env.shutdown();
}

/// A renewal renewed again, by a third pack mounted while it is in flight, keeps the
/// first load's place throughout and ends where a store new to the process ends.
#[test]
fn a_renewal_renewed_again_keeps_its_place() {
    // Two held calls at once at most, and a load's own steps beside them.
    let env = Env::with_background_workers("mounts_renew_twice", 3);
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let (held, h2, a) = (id("held"), id("h2"), id("renewed twice"));
    let make = |dependencies: Vec<AssetId>| {
        sorted(vec![
            (held, gated(b"hold", [])),
            (h2, gated(b"h2", [])),
            (a, text(b"payload", dependencies)),
        ])
    };
    let intact = make(vec![held, h2]);
    let version = version_of(&intact, a);
    let p0 = env.write(claiming(&make(vec![held]), a, version));
    let p1 = env.write(claiming(&make(vec![h2]), a, version));
    let p2 = env.pack(&intact, Storage::Raw);
    let expected = fresh(&env, &[p0.clone(), p1.clone(), p2.clone()], a);
    assert_eq!(expected, Readiness::Ready(version));

    env.mount(&store, p0).unwrap();
    gate.hold(b"hold");
    gate.hold(b"h2");
    store.request_newest(a, NOW);
    gate.wait_entered(b"hold");
    env.mount(&store, p1).unwrap();
    let mut settled = pin!(store.settled());
    assert!(pending(settled.as_mut()));
    gate.open(b"hold");
    gate.wait_entered(b"h2");
    env.mount(&store, p2).unwrap();
    assert!(pending(settled.as_mut()), "the first renewal is waited for");
    gate.open(b"h2");
    env.wait(settled);
    assert_eq!(newest_version(&store, a), expected);
    env.shutdown();
}

fn entered(gate: &Gate, bytes: &[u8]) -> usize {
    gate.entered().iter().filter(|entered| entered.as_slice() == bytes).count()
}

/// Rule 3 reads again only a dependant whose named version is the one that failed: one
/// renamed since to a version that failed for itself is not read again.
#[test]
fn rule_3_skips_a_dependant_renamed_since_it_failed() {
    let env = Env::new("mounts_rule_3_renamed");
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let (dependant, dependency) = (id("a dependant renamed"), id("its dependency"));
    env.provide(&store, &[(dependant, text(b"first", [dependency]))]);
    store.request_newest(dependant, NOW);
    env.settle(&store);
    assert!(matches!(store.newest(dependant), Readiness::Failed(_)));
    env.provide(&store, &[(dependant, gated(b"refuse the second", []))]);
    env.settle(&store);
    assert_eq!(entered(&gate, b"refuse the second"), 1);
    env.provide(&store, &[(dependency, text(b"dependency", []))]);
    env.settle(&store);
    assert_eq!(entered(&gate, b"refuse the second"), 1, "a failure of its own read again");
    env.shutdown();
}

/// Rule 3 reads again only what is followed: an asset requested by exact version alone
/// that failed for a missing dependency stays failed when a mount adds the dependency.
#[test]
fn rule_3_skips_a_dependant_requested_only_by_exact_version() {
    let env = Env::new("mounts_rule_3_exact");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let (dependant, dependency) = (id("an exact dependant"), id("its dependency"));
    let cooked = text(b"exact", [dependency]);
    let version = cooked.version();
    env.provide(&store, &[(dependant, cooked)]);
    store.request(dependant, version, NOW);
    env.settle(&store);
    assert!(matches!(store.get(dependant, version), Readiness::Failed(_)));
    env.provide(&store, &[(dependency, text(b"dependency", []))]);
    env.settle(&store);
    assert!(matches!(store.get(dependant, version), Readiness::Failed(_)), "not followed");
    env.shutdown();
}
