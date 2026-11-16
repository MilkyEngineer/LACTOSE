//! Failures (`README.md`, "Failures" and "Requests and loads", steps 5 and 6): each kind
//! produced, which kind a dependant reports, cycles, and chains long enough to overflow a
//! recursive walk.

#![forbid(unsafe_code)]

use crate::common;

use std::future::Future;
use std::pin::pin;
use std::sync::mpsc;
use std::task::{Context, Waker};

use common::capture::capture;
use common::format::index_range;
use common::*;
use lactose_asset::{Storage, Store};
use lactose_asset_api::{
    AssetCatalog, AssetId, AssetSet, CookedAsset, LoadPriority, PublishedAssets, Readiness,
};
use lactose_core::ErrorKind;
use lactose_task::WorkerClass;
use tracing::Level;

const NOW: LoadPriority = LoadPriority(0);

fn failed_kind(store: &Store, id: AssetId) -> Option<ErrorKind> {
    match store.newest(id) {
        Readiness::Failed(failure) => Some(failure.kind),
        _ => None,
    }
}

fn ordered(first: &str, second: &str) -> (AssetId, AssetId) {
    let (a, b) = (id(first), id(second));
    if a < b { (a, b) } else { (b, a) }
}

/// The `dependency` field of the one `warn` logged for `asset` since event `from`.
fn warned_dependency(from: usize, asset: AssetId) -> Option<String> {
    let asset = format!("{:#018x}", asset.to_bits());
    let warned: Vec<_> = capture()
        .store_events(from, Level::WARN)
        .into_iter()
        .filter(|event| event.fields.get("asset") == Some(&asset))
        .collect();
    assert_eq!(warned.len(), 1, "one warn per recorded failure: {warned:#?}");
    warned[0].fields.get("dependency").cloned()
}

#[test]
fn a_request_after_the_pool_stopped_is_cancelled() {
    let env = Env::new("failures_io_shutdown");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let x = id("x");
    env.provide(&store, &[(x, text(b"x", []))]);
    env.io.clone().shutdown();
    store.request_newest(x, NOW);
    env.settle(&store);
    assert_eq!(failed_kind(&store, x), Some(ErrorKind::Cancelled));
    env.shutdown();
}

#[test]
fn a_request_after_the_executor_stopped_returns_and_is_cancelled() {
    let env = Env::new("failures_executor_shutdown");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let x = id("x");
    env.provide(&store, &[(x, text(b"x", []))]);
    env.executor.clone().shutdown();

    // The future is dropped inside `spawn`, on this thread, and its guard records the
    // failure: the request returns, bounded so that a deadlock fails instead.
    let (done, returned) = mpsc::channel();
    let requester = store.clone();
    std::thread::spawn(move || {
        requester.request_newest(x, NOW);
        done.send(()).unwrap();
    });
    returned.recv_timeout(LIMIT).expect("the request returned");
    assert_eq!(failed_kind(&store, x), Some(ErrorKind::Cancelled));
    let mut settled = pin!(store.settled());
    let poll = settled.as_mut().poll(&mut Context::from_waker(Waker::noop()));
    assert!(poll.is_ready(), "settled after an executor shutdown");
    env.io.clone().shutdown();
}

#[test]
fn loads_queued_behind_a_held_loader_are_cancelled_at_shutdown() {
    let env = Env::with_background_workers("failures_queued", 1);
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let names = ["one", "two", "three"];
    let assets: Vec<_> = names.iter().map(|name| (id(name), gated(name.as_bytes(), []))).collect();
    env.provide(&store, &assets);
    gate.hold_all();
    for (asset, _) in &assets {
        store.request_newest(*asset, NOW);
    }
    eventually("a loader call began", || !gate.entered().is_empty());
    let ran = gate.entered()[0].clone();

    // Shutdown cancels the queued tasks at once and lets the running call finish: it is
    // opened from here once the queued loads have ended.
    let executor = env.executor.clone();
    let stopping = std::thread::spawn(move || executor.shutdown());
    eventually("the queued loads cancelled", || {
        let queued = assets.iter().filter(|(_, cooked)| cooked.bytes != ran);
        queued.into_iter().all(|(asset, _)| failed_kind(&store, *asset).is_some())
    });
    gate.open_all();
    stopping.join().unwrap();

    assert_eq!(gate.entered(), std::slice::from_ref(&ran), "no queued loader call began");
    let ran = &ran;
    let mut kinds = Vec::new();
    for (asset, cooked) in &assets {
        if &cooked.bytes == ran {
            assert!(store.newest(*asset).ready().is_some());
        } else {
            kinds.push(failed_kind(&store, *asset));
        }
    }
    assert_eq!(kinds, [Some(ErrorKind::Cancelled); 2]);
    env.io.clone().shutdown();
}

#[test]
fn a_kind_without_a_loader_is_unsupported_and_costs_no_read() {
    let capture = capture();
    let env = Env::new("failures_unsupported");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let x = id("x");
    env.provide(&store, &[(x, unloadable(b"no loader", []))]);
    let from = capture.span_count();
    store.request_newest(x, NOW);
    env.settle(&store);
    assert_eq!(failed_kind(&store, x), Some(ErrorKind::Unsupported));
    assert!(capture.named(from, "io.read").is_empty());
    env.shutdown();
}

/// A dependant of two failing dependencies, `low` below `high`, each held at its loader
/// until the test opens it in the order given: the dependant reports `low`'s failure.
#[test]
fn the_lower_failing_dependency_is_reported_whichever_fails_first() {
    let capture = capture();
    let (low, high) = ordered("first failing", "second failing");
    for (run, high_first) in [false, true].into_iter().enumerate() {
        let env = Env::with_background_workers(&format!("failures_lowest_{run}"), 4);
        let (loaders, gate) = test_loaders();
        let store = env.store(loaders);
        let dependant = id("dependant");
        env.provide(
            &store,
            &[
                (dependant, text(b"dependant", [low, high])),
                (low, gated(b"refuse low", [])),
                (high, gated(b"refuse high", [])),
            ],
        );
        gate.hold(b"refuse low");
        gate.hold(b"refuse high");
        let from = capture.event_count();
        store.request_newest(dependant, NOW);
        let (first, second): (&[u8], &[u8]) = if high_first {
            (b"refuse high", b"refuse low")
        } else {
            (b"refuse low", b"refuse high")
        };
        gate.wait_entered(first);
        gate.wait_entered(second);
        gate.open(first);
        let failed_first = if high_first { high } else { low };
        eventually("the first failure recorded", || {
            matches!(store.newest(failed_first), Readiness::Failed(_))
        });
        assert_eq!(store.newest(dependant).map(|_| ()), Readiness::Pending);
        gate.open_all();
        env.settle(&store);
        assert_eq!(failed_kind(&store, dependant), Some(ErrorKind::InvalidData));
        let expected = format!("{:#018x}", low.to_bits());
        assert_eq!(warned_dependency(from, dependant), Some(expected), "high first: {high_first}");
        env.shutdown();
    }

    // Kinds that differ: the higher fails at once with `NotFound`, the lower later with
    // `InvalidData`, and the dependant reports `InvalidData`; then the other way round.
    for (run, low_missing) in [false, true].into_iter().enumerate() {
        let env = Env::with_background_workers(&format!("failures_lowest_kinds_{run}"), 4);
        let (loaders, gate) = test_loaders();
        let store = env.store(loaders);
        let dependant = id("dependant");
        let (missing, refused) = if low_missing { (low, high) } else { (high, low) };
        env.provide(
            &store,
            &[(dependant, text(b"dependant", [low, high])), (refused, gated(b"refuse", []))],
        );
        gate.hold(b"refuse");
        store.request_newest(dependant, NOW);
        gate.wait_entered(b"refuse");
        assert_eq!(failed_kind(&store, missing), Some(ErrorKind::NotFound));
        assert_eq!(store.newest(dependant).map(|_| ()), Readiness::Pending);
        gate.open_all();
        env.settle(&store);
        let expected = if low_missing { ErrorKind::NotFound } else { ErrorKind::InvalidData };
        assert_eq!(failed_kind(&store, dependant), Some(expected));
        env.shutdown();
    }
}

/// Flips a byte of the payload of the first entry of a raw pack.
fn damage(pack: &mut [u8]) {
    let (start, _) = index_range(pack);
    let dependencies = u32::from_le_bytes(pack[start + 32..start + 36].try_into().unwrap());
    let entry = start + 4 + 32 + 8 * dependencies as usize + 1;
    let offset = u64::from_le_bytes(pack[entry..entry + 8].try_into().unwrap()) as usize;
    pack[offset] ^= 0x20;
}

#[test]
fn a_dependants_own_failure_yields_to_its_failed_dependency() {
    let env = Env::new("failures_precedence");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let missing = id("missing");
    let (corrupt, unsupported) = (id("corrupt"), id("unsupported"));
    let mut bytes = pack(&[(corrupt, text(b"corrupt", [missing]))], Storage::Raw);
    damage(&mut bytes);
    env.mount(&store, env.write(bytes)).unwrap();
    env.provide(&store, &[(unsupported, unloadable(b"no loader", [missing]))]);
    store.request_newest(corrupt, NOW);
    store.request_newest(unsupported, NOW);
    env.settle(&store);
    assert_eq!(failed_kind(&store, corrupt), Some(ErrorKind::NotFound));
    assert_eq!(failed_kind(&store, unsupported), Some(ErrorKind::NotFound));
    env.shutdown();
}

// ---- Cycles ----

/// A declares [B, C], no pack holds B, C declares [A]; A and C are held at their loaders
/// and opened in the order given. With `member_lower`, C is below B, so that A's lowest
/// failed dependency is the cycle's.
fn cycle(name: &str, member_lower: bool, c_first: bool) -> (ErrorKind, ErrorKind, ErrorKind) {
    let env = Env::with_background_workers(name, 4);
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let (low, high) = ordered("cycle b", "cycle c");
    let (b, c) = if member_lower { (high, low) } else { (low, high) };
    let (a, outside) = (id("cycle a"), id("outside"));
    env.provide(
        &store,
        &[(a, gated(b"a", [b, c])), (c, gated(b"c", [a])), (outside, text(b"outside", [c]))],
    );
    gate.hold(b"a");
    gate.hold(b"c");
    store.request_newest(a, NOW);
    store.request_newest(outside, NOW);
    gate.wait_entered(b"a");
    gate.wait_entered(b"c");
    let generation = store.generation();
    let first: &[u8] = if c_first { b"c" } else { b"a" };
    gate.open(first);
    gate.wait_returned(first);
    // The first member's steps 3 and 4 have ended once the three workers the other's held
    // loader leaves free have each returned from their polls; neither member's failure is
    // recorded while the other's loader is held.
    env.drain(3);
    assert_eq!(store.generation(), generation);
    assert_eq!(store.newest(a).map(|_| ()), Readiness::Pending);
    assert_eq!(store.newest(c).map(|_| ()), Readiness::Pending);
    gate.open_all();
    env.settle(&store);
    let kinds = (
        failed_kind(&store, a).unwrap(),
        failed_kind(&store, c).unwrap(),
        failed_kind(&store, outside).unwrap(),
    );
    env.shutdown();
    kinds
}

#[test]
fn a_cycle_reports_its_lowest_failed_dependency_whichever_member_ends_first() {
    for c_first in [false, true] {
        let name = format!("failures_cycle_outside_{c_first}");
        let kinds = cycle(&name, false, c_first);
        assert_eq!(kinds, (ErrorKind::NotFound, ErrorKind::NotFound, ErrorKind::NotFound));
    }
}

#[test]
fn a_cycle_that_comes_back_round_is_invalid_data_whichever_member_ends_first() {
    for c_first in [false, true] {
        let name = format!("failures_cycle_round_{c_first}");
        let kinds = cycle(&name, true, c_first);
        let invalid = ErrorKind::InvalidData;
        assert_eq!(kinds, (invalid, invalid, invalid));
    }
}

/// A cycle formed by a rename, among loads one of which was found to wait for a load
/// still running before the rename: A declares B, B at its first version declares C,
/// whose loader is held, and a mount renames B to a version that declares A. A and B's
/// new version fail as a cycle while C's call is still held.
#[test]
fn a_cycle_a_rename_forms_behind_a_running_load_is_decided_while_it_runs() {
    let env = Env::with_background_workers("failures_cycle_renamed", 4);
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let (a, b, c) = (id("renamed a"), id("renamed b"), id("renamed c"));
    env.provide(&store, &[(a, gated(b"a", [b])), (b, gated(b"b1", [c])), (c, gated(b"c", []))]);
    gate.hold(b"a");
    gate.hold(b"c");
    store.request_newest(a, NOW);
    gate.wait_entered(b"a");
    gate.wait_entered(b"c");
    gate.wait_returned(b"b1");
    // B's first version's steps 3 and 4 have ended: it waits for C, which runs.
    env.drain(2);
    gate.open(b"a");
    gate.wait_returned(b"a");
    // A's have ended too: A was found to wait, through B, for C.
    env.drain(3);
    assert_eq!(store.newest(a).map(|_| ()), Readiness::Pending);

    env.provide(&store, &[(b, gated(b"b2", [a]))]);
    gate.wait_returned(b"b2");
    env.drain(3);
    assert_eq!(failed_kind(&store, a), Some(ErrorKind::InvalidData));
    assert_eq!(failed_kind(&store, b), Some(ErrorKind::InvalidData));
    assert!(gate.returned().iter().all(|returned| returned != b"c"), "C's call is held");
    gate.open(b"c");
    env.settle(&store);
    env.shutdown();
}

/// A cycle a rename forms onto a version whose load waits already: W declares D, named at
/// a version whose dependency's loader is held; D's other version, requested exactly,
/// declares W. A mount that renames D to that version decides W and it at once, while
/// the held call still runs, since the rename gives W another look.
#[test]
fn a_rename_onto_a_waiting_version_that_closes_a_cycle_decides_it_at_the_mount() {
    let env = Env::with_background_workers("failures_cycle_rename_waiting", 4);
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let (w, d, c) = (id("waiting w"), id("waiting d"), id("waiting c"));
    let d2 = gated(b"d2", [w]);
    let version = d2.version();
    env.provide(&store, &[(w, gated(b"w", [d])), (d, d2.clone())]);
    env.provide(&store, &[(d, gated(b"d1", [c])), (c, gated(b"c", []))]);
    gate.hold(b"c");
    store.request_newest(w, NOW);
    gate.wait_entered(b"c");
    store.request(d, version, NOW);
    for bytes in [b"w".as_slice(), b"d1", b"d2"] {
        gate.wait_returned(bytes);
    }
    // Every call but C's has ended its steps 3 and 4: W waits, through D's first
    // version, for C, and D's second version waits for W.
    env.drain(3);
    assert_eq!(store.newest(w).map(|_| ()), Readiness::Pending);

    env.provide(&store, &[(d, d2)]);
    assert_eq!(failed_kind(&store, w), Some(ErrorKind::InvalidData));
    assert_eq!(failed_kind(&store, d), Some(ErrorKind::InvalidData));
    assert!(gate.returned().iter().all(|returned| returned != b"c"), "C's call is held");
    gate.open(b"c");
    env.settle(&store);
    env.shutdown();
}

/// The successors the cycle searches visit in all, while the links of a chain of `n` end
/// their steps 3 and 4 one by one, each also declaring a leaf that publishes in between;
/// the chain's last link declares a load whose call is held until every leaf published.
/// `tail_first` requests the links from the tail, so that each ending link was found
/// behind the held load by the one after it; otherwise from the head.
fn search_visits(n: usize, tail_first: bool) -> u64 {
    let capture = capture();
    let env = Env::with_background_workers(&format!("failures_walk_{tail_first}"), 4);
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let link = |at: usize| id(&format!("walk link {at}"));
    let leaf = |at: usize| id(&format!("walk leaf {at}"));
    let held = id("walk held");
    let mut assets = vec![(held, gated(b"held", []))];
    for at in 0..n {
        let next = if at + 1 < n { link(at + 1) } else { held };
        assets.push((link(at), text(format!("link {at}").as_bytes(), [next, leaf(at)])));
        assets.push((leaf(at), text(format!("leaf {at}").as_bytes(), [])));
    }
    env.provide(&store, &assets);
    gate.hold(b"held");
    let events = capture.event_count();
    let order: Vec<usize> = if tail_first { (0..n).rev().collect() } else { (0..n).collect() };
    for at in order {
        store.request_newest(link(at), NOW);
    }
    eventually("every leaf published", || store.generation() >= n as u64);
    gate.open(b"held");
    env.settle(&store);
    assert!(store.newest(link(0)).ready().is_some());
    capture.closed(0, "silo.load");
    let searches = capture.events()[events..]
        .iter()
        .filter(|event| event.name == "silo.search")
        .map(|event| event.fields["visits"].parse::<u64>().expect("a count"))
        .sum();
    env.shutdown();
    searches
}

#[test]
fn the_cycle_searches_of_a_chain_ending_link_by_link_visit_linear_successors() {
    const N: usize = 2000;
    for tail_first in [true, false] {
        let visits = search_visits(N, tail_first);
        eprintln!("{N} links, tail first {tail_first}: {visits} successors visited");
        assert!(visits <= 8 * N as u64, "{visits} successors visited, tail first {tail_first}");
    }
}

// ---- Long chains ----

const LINKS: usize = 100_000;

fn link(at: usize) -> AssetId {
    id(&format!("link {at}"))
}

/// A chain of `LINKS` links, each declaring the next; the last declares `last`.
fn chain(last: &[AssetId]) -> Vec<(AssetId, CookedAsset)> {
    (0..LINKS)
        .map(|at| {
            let next: Vec<AssetId> =
                if at + 1 == LINKS { last.to_vec() } else { vec![link(at + 1)] };
            (link(at), text(&(at as u32).to_le_bytes(), next))
        })
        .collect()
}

/// `request_newest` of the chain's head from a task on a background worker; returns how
/// many loads that one call accepted (their spans).
fn request_head_on_a_worker(env: &Env, store: &Store) -> usize {
    let capture = capture();
    let requester = store.clone();
    let task = env.executor.spawn(WorkerClass::Background, async move {
        let from = capture.span_count();
        requester.request_newest(link(0), NOW);
        capture.named(from, "silo.load").len()
    });
    env.wait(task).expect("the requesting task")
}

#[test]
fn a_chain_of_100000_links_is_accepted_in_one_call_and_published() {
    let env = Env::long("failures_chain");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    env.provide(&store, &chain(&[]));
    assert_eq!(request_head_on_a_worker(&env, &store), LINKS);
    env.settle(&store);
    let mut head = AssetSet::new();
    head.insert(link(0));
    let table = store.resolve(&head).ready().expect("every link published");
    assert_eq!(table.len(), LINKS);
    let catalog: &dyn AssetCatalog = &store;
    assert!(catalog.newest(link(LINKS - 1)).ready().is_some());
    env.shutdown();
}

#[test]
fn a_chain_of_100000_links_closed_into_a_cycle_fails_every_link_invalid_data() {
    let env = Env::long("failures_chain_cycle");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    env.provide(&store, &chain(&[link(0)]));
    assert_eq!(request_head_on_a_worker(&env, &store), LINKS);
    env.settle(&store);
    for at in 0..LINKS {
        assert_eq!(failed_kind(&store, link(at)), Some(ErrorKind::InvalidData), "link {at}");
    }
    env.shutdown();
}

#[test]
fn a_chain_of_100000_links_missing_its_tail_fails_and_is_repaired_at_one_mount() {
    let env = Env::long("failures_chain_repair");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let tail = id("the missing tail");
    env.provide(&store, &chain(&[tail]));
    // Every link, and the tail's load that ended in the call.
    assert_eq!(request_head_on_a_worker(&env, &store), LINKS + 1);
    env.settle(&store);
    for at in 0..LINKS {
        assert_eq!(failed_kind(&store, link(at)), Some(ErrorKind::NotFound), "link {at}");
    }
    let accepted = {
        let capture = capture();
        let from = capture.span_count();
        env.provide(&store, &[(tail, text(b"tail", []))]);
        capture.named(from, "silo.load").len()
    };
    assert_eq!(accepted, LINKS + 1, "the tail and every link, at the mount");
    env.settle(&store);
    for at in 0..LINKS {
        assert!(store.newest(link(at)).ready().is_some(), "link {at}");
    }
    assert!(store.newest(tail).ready().is_some());
    env.shutdown();
}
