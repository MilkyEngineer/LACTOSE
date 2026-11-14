//! The generation's promise (`README.md`, "Publication, current versions and the
//! generation"): while it stands still, `get`, `newest` and `resolve` answer as they did,
//! and it moves exactly when one of them changes.

#![forbid(unsafe_code)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use common::*;
use lactose_asset::Store;
use lactose_asset_api::{
    AssetCatalog, AssetFailure, AssetId, AssetSet, AssetVersion, LoadPriority, PublishedAssets,
    Readiness,
};
use lactose_core::ErrorKind;

const NOW: LoadPriority = LoadPriority(0);

/// What a reader sees of `ids`: each one's `newest` and `get` of `exact`, and `resolve`
/// of all of them.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Answers {
    newest: Vec<Readiness<AssetVersion>>,
    exact: Vec<Readiness<AssetVersion>>,
    resolved: Readiness<Vec<(AssetId, AssetVersion)>>,
}

fn answers(store: &Store, ids: &[AssetId], exact: &[(AssetId, AssetVersion)]) -> Answers {
    let mut set = AssetSet::new();
    for &id in ids {
        set.insert(id);
    }
    Answers {
        newest: ids.iter().map(|&id| store.newest(id).map(|handle| handle.version())).collect(),
        exact: exact
            .iter()
            .map(|&(id, version)| store.get(id, version).map(|handle| handle.version()))
            .collect(),
        resolved: store.resolve(&set).map(|table| table.iter().collect()),
    }
}

#[test]
fn a_held_retry_and_a_held_reload_change_nothing_until_they_end() {
    let env = Env::new("generation_held");
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let x = id("x");
    let refused = gated(b"refuse", []);
    env.provide(&store, &[(x, refused.clone())]);
    store.request_newest(x, NOW);
    env.settle(&store);
    let failed = answers(&store, &[x], &[(x, refused.version())]);
    let generation = store.generation();

    // A retry, held in flight: nothing changes until it ends, failed again.
    gate.hold(b"refuse");
    store.request_newest(x, NOW);
    gate.wait_entered(b"refuse");
    assert_eq!(store.generation(), generation);
    assert_eq!(answers(&store, &[x], &[(x, refused.version())]), failed);
    gate.open(b"refuse");
    env.settle(&store);
    assert_eq!(store.generation(), generation + 1, "a retry that fails again moves it once");
    assert_eq!(answers(&store, &[x], &[(x, refused.version())]), failed);

    // A mount that renames the failed asset, its load held.
    let generation = store.generation();
    let intact = gated(b"intact", []);
    gate.hold(b"intact");
    env.provide(&store, &[(x, intact.clone())]);
    gate.wait_entered(b"intact");
    assert_eq!(store.generation(), generation);
    assert_eq!(answers(&store, &[x], &[(x, refused.version())]), failed);
    gate.open(b"intact");
    env.settle(&store);
    assert_eq!(store.generation(), generation + 1, "one publish, one move");
    let published = answers(&store, &[x], &[(x, refused.version())]);
    assert_eq!(published.newest, [Readiness::Ready(intact.version())]);
    assert_eq!(published.resolved, Readiness::Ready(vec![(x, intact.version())]));
    env.shutdown();
}

#[test]
fn a_request_moves_it_only_when_an_answer_changes() {
    let env = Env::new("generation_requests");
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let unknown = id("unknown");
    for _ in 0..3 {
        let generation = store.generation();
        store.request_newest(unknown, NOW);
        assert_eq!(store.generation(), generation + 1, "each unknown request records a failure");
    }
    let failure = AssetFailure { id: unknown, version: None, kind: ErrorKind::NotFound };
    assert_eq!(store.newest(unknown).map(|_| ()), Readiness::Failed(failure));

    let (x, held) = (id("x"), id("held"));
    let cooked = text(b"x", []);
    env.provide(&store, &[(x, cooked.clone()), (held, gated(b"held", []))]);
    store.request_newest(x, NOW);
    env.settle(&store);
    gate.hold(b"held");
    store.request_newest(held, NOW);
    gate.wait_entered(b"held");
    let generation = store.generation();
    for _ in 0..3 {
        store.request_newest(x, NOW);
        store.request(x, cooked.version(), LoadPriority(9));
        store.request_newest(held, LoadPriority(9));
        store.request(AssetId::NONE, cooked.version(), NOW);
        store.request_newest(AssetId::NONE, NOW);
    }
    assert_eq!(store.generation(), generation, "published, loading or nothing at all");
    gate.open(b"held");
    env.settle(&store);
    env.shutdown();
}

/// Readers on several threads read the generation, look, and read it again; between two
/// equal reads they must all have seen one answer, while packs are mounted and loads end
/// under them.
#[test]
fn readers_never_see_two_answers_under_one_generation() {
    const VERSIONS: usize = 40;
    let env = Env::new("generation_readers");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let (a, b, c) = (id("a"), id("b"), id("c"));
    let ids = [a, b, c];
    let version = |name: AssetId, at: usize| text(format!("{name:?} {at}").as_bytes(), []);
    let mut exact = Vec::new();
    for at in 0..VERSIONS {
        exact.push((a, version(a, at).version()));
    }
    env.provide(&store, &[(a, version(a, 0)), (b, text(b"b", [c])), (c, version(c, 0))]);
    for id in ids {
        store.request_newest(id, NOW);
    }

    let stop = Arc::new(AtomicBool::new(false));
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let (store, stop, exact) = (store.clone(), Arc::clone(&stop), exact.clone());
            std::thread::spawn(move || {
                let mut seen: Vec<(u64, Answers)> = Vec::new();
                while !stop.load(Ordering::Relaxed) {
                    let before = store.generation();
                    let answers = answers(&store, &ids, &exact);
                    if store.generation() == before {
                        seen.push((before, answers));
                    }
                }
                seen
            })
        })
        .collect();

    for at in 1..VERSIONS {
        env.provide(&store, &[(a, version(a, at)), (c, version(c, at))]);
        if at % 3 == 0 {
            store.request(a, version(a, at / 2).version(), NOW);
        }
        if at % 5 == 0 {
            env.settle(&store);
        }
    }
    env.settle(&store);
    stop.store(true, Ordering::Relaxed);

    let mut by_generation = std::collections::BTreeMap::new();
    let mut observations = 0;
    for reader in readers {
        for (generation, answers) in reader.join().unwrap() {
            observations += 1;
            let first = by_generation.entry(generation).or_insert_with(|| answers.clone());
            assert_eq!(*first, answers, "two answers under generation {generation}");
        }
    }
    assert!(observations > 0);
    assert!(by_generation.len() > 1, "the readers saw the generation move");
    let last = answers(&store, &ids, &exact);
    assert_eq!(last.newest[0], Readiness::Ready(version(a, VERSIONS - 1).version()));
    env.shutdown();
}
