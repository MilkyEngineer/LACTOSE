//! The conformance suite run against the reference store, and against stores that break
//! one rule each, to show the suite notices.
//!
//! The five oldest break broad rules and fail several checks; each test names the first
//! message. The seven that break a rule D136 added each fail exactly one check, on that
//! check's own message, and pass the other twenty. Each of the six checks D136 added fails
//! at least one of them: the first fails two, a store whose generation stands still at a
//! cook and one whose generation moves.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use lactose_asset_api::conformance::{self, Harness, ReferenceStore};
use lactose_asset_api::{
    Asset, AssetCatalog, AssetFailure, AssetId, AssetKind, AssetSet, AssetVersion, CookedAsset,
    ErasedLoader, LoadPriority, Loader, PublishedAssets, Readiness, UntypedHandle, VersionTable,
};
use lactose_core::{ErrorKind, Result};

#[test]
fn reference_store_conforms() {
    conformance::run(ReferenceStore::new);
}

/// How a [`Broken`] store departs from the reference.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fault {
    /// `newest` keeps serving the first version it ever served for an asset.
    NewestIsStale,
    /// Failures are reported as still pending.
    FailuresArePending,
    /// `resolve` ignores every asset that has no published version.
    ResolveSkipsUnpublished,
    /// `resolve` returns the set's own members without their dependencies.
    ResolveOmitsDependencies,
    /// `generation` never moves.
    GenerationIsConstant,
    /// A failure is forgotten once a cook renames its asset: `newest` answers `Pending`
    /// until the next load ends, and the generation has not moved.
    RenamedFailureIsPending,
    /// The same forgetting, by a store that reloads by itself: for an asset whose newest
    /// version was asked for, `newest` and `resolve` answer `Pending` from the cook until
    /// the next load ends, and the generation moves at the cook.
    RenamedFailureIsPendingAndMoves,
    /// A dependant reports the kind of its failing dependency with the highest ID, as a
    /// walk over dependencies in descending order would, where the rule is the lowest.
    HighestDependencyFirst,
    /// A repeated request for an asset the index does not name moves the generation once.
    RepeatedUnknownMovesOnce,
    /// An exact request of a published version the index names makes it current.
    ExactRequestMakesCurrent,
    /// A failed exact request of the version the index names is no answer for `newest`.
    ExactFailureIsNotNewest,
    /// Lookups of the empty ID are `Pending`.
    EmptyIdIsPending,
}

/// What a [`Broken`] store remembers of the calls made to it, to break its one rule.
#[derive(Default)]
struct Calls {
    /// The first handle `newest` served per asset.
    first_served: Vec<(AssetId, UntypedHandle)>,
    /// The version last provided per asset: what the index names.
    named: BTreeMap<AssetId, AssetVersion>,
    /// The assets provided since the last `settle`.
    renamed: BTreeSet<AssetId>,
    /// The assets whose newest version was ever requested.
    asked_newest: BTreeSet<AssetId>,
    /// The assets requested by exact version alone.
    exact_only: BTreeSet<AssetId>,
    /// The assets whose newest version was requested while the index did not name them.
    asked_unknown: BTreeSet<AssetId>,
    /// Repeats of such requests: those not yet settled, and those the generation hides.
    repeats_requested: u64,
    repeats_hidden: u64,
    /// The published versions an exact request made current.
    forced: BTreeMap<AssetId, AssetVersion>,
    /// The assets whose newest version was asked for and had failed when a cook renamed
    /// them, since the last `settle`; and how many such cooks there were.
    forgotten: BTreeSet<AssetId>,
    forgotten_moves: u64,
    /// The dependencies each provided version declares.
    declared: BTreeMap<(AssetId, u64), Vec<AssetId>>,
}

struct Broken {
    inner: ReferenceStore,
    fault: Fault,
    calls: Mutex<Calls>,
}

impl Harness for Broken {
    fn catalog(&self) -> &dyn AssetCatalog {
        self
    }

    fn provide(&self, id: AssetId, version: AssetVersion, cooked: CookedAsset) {
        let mut calls = self.calls.lock().unwrap();
        calls.named.insert(id, version);
        calls.renamed.insert(id);
        calls.declared.insert((id, version.to_bits()), cooked.header.dependencies().to_vec());
        let failed = matches!(self.inner.newest(id), Readiness::Failed(_));
        if failed && calls.asked_newest.contains(&id) {
            calls.forgotten.insert(id);
            calls.forgotten_moves += 1;
        }
        self.inner.provide(id, version, cooked);
    }

    fn settle(&self) {
        self.inner.settle();
        let mut calls = self.calls.lock().unwrap();
        calls.renamed.clear();
        calls.forgotten.clear();
        calls.repeats_hidden += core::mem::take(&mut calls.repeats_requested);
    }
}

impl Broken {
    fn hide_failure<T>(&self, readiness: Readiness<T>) -> Readiness<T> {
        match (self.fault, readiness) {
            (Fault::FailuresArePending, Readiness::Failed(_)) => Readiness::Pending,
            (_, readiness) => readiness,
        }
    }
}

impl PublishedAssets for Broken {
    fn request(&self, id: AssetId, version: AssetVersion, priority: LoadPriority) {
        let mut calls = self.calls.lock().unwrap();
        if !calls.asked_newest.contains(&id) {
            calls.exact_only.insert(id);
        }
        let published = matches!(self.inner.get(id, version), Readiness::Ready(_));
        let current =
            matches!(self.inner.newest(id), Readiness::Ready(now) if now.version() == version);
        if published && !current && calls.named.get(&id) == Some(&version) {
            calls.forced.insert(id, version);
        }
        self.inner.request(id, version, priority);
    }

    fn get(&self, id: AssetId, version: AssetVersion) -> Readiness<UntypedHandle> {
        if self.fault == Fault::EmptyIdIsPending && id.is_none() {
            return Readiness::Pending;
        }
        self.hide_failure(self.inner.get(id, version))
    }
}

impl AssetCatalog for Broken {
    fn request_newest(&self, id: AssetId, priority: LoadPriority) {
        let mut calls = self.calls.lock().unwrap();
        calls.asked_newest.insert(id);
        calls.exact_only.remove(&id);
        if !calls.named.contains_key(&id) && !calls.asked_unknown.insert(id) {
            calls.repeats_requested += 1;
        }
        self.inner.request_newest(id, priority);
    }

    fn newest(&self, id: AssetId) -> Readiness<UntypedHandle> {
        let newest = self.hide_failure(self.inner.newest(id));
        let mut calls = self.calls.lock().unwrap();
        match (self.fault, &newest) {
            (Fault::NewestIsStale, Readiness::Ready(handle)) => {
                if let Some((_, first)) =
                    calls.first_served.iter().find(|(served, _)| *served == id)
                {
                    return Readiness::Ready(first.clone());
                }
                calls.first_served.push((id, handle.clone()));
            }
            (Fault::RenamedFailureIsPending, Readiness::Failed(_))
                if calls.renamed.contains(&id) =>
            {
                return Readiness::Pending;
            }
            (Fault::ExactFailureIsNotNewest, Readiness::Failed(_))
                if calls.exact_only.contains(&id) =>
            {
                return Readiness::Pending;
            }
            (Fault::ExactRequestMakesCurrent, Readiness::Pending) => {
                if let Some(&version) = calls.forced.get(&id) {
                    return self.inner.get(id, version);
                }
            }
            (Fault::EmptyIdIsPending, _) if id.is_none() => return Readiness::Pending,
            (Fault::RenamedFailureIsPendingAndMoves, _) if calls.forgotten.contains(&id) => {
                return Readiness::Pending;
            }
            (Fault::HighestDependencyFirst, Readiness::Failed(failure)) => {
                let key = failure.version.map(|version| (id, version.to_bits()));
                let declared = key.and_then(|key| calls.declared.get(&key));
                let unpublished =
                    |dependency: &&AssetId| self.inner.newest(**dependency).ready().is_none();
                if let Some(&highest) = declared.and_then(|all| all.iter().rev().find(unpublished))
                {
                    // The reference store never looked at it if a lower one failed first:
                    // a dependency no cook provided is not found, as a walk would find.
                    let kind = match self.inner.newest(highest) {
                        Readiness::Failed(of_dependency) => of_dependency.kind,
                        _ if calls.named.contains_key(&highest) => failure.kind,
                        _ => ErrorKind::NotFound,
                    };
                    return Readiness::Failed(AssetFailure { kind, ..*failure });
                }
            }
            _ => {}
        }
        newest
    }

    fn resolve(&self, set: &AssetSet) -> Readiness<VersionTable> {
        let own_members =
            || set.iter().filter_map(|id| Some((id, self.inner.newest(id).ready()?.version())));
        let forgotten = |id| self.calls.lock().unwrap().forgotten.contains(&id);
        match (self.fault, self.inner.resolve(set)) {
            (Fault::ResolveSkipsUnpublished, _) => Readiness::Ready(own_members().collect()),
            (Fault::ResolveOmitsDependencies, Readiness::Ready(_)) => {
                Readiness::Ready(own_members().collect())
            }
            (Fault::RenamedFailureIsPendingAndMoves, _) if set.iter().any(forgotten) => {
                Readiness::Pending
            }
            (_, resolved) => self.hide_failure(resolved),
        }
    }

    fn generation(&self) -> u64 {
        let calls = self.calls.lock().unwrap();
        match self.fault {
            Fault::GenerationIsConstant => 0,
            Fault::RepeatedUnknownMovesOnce => self.inner.generation() - calls.repeats_hidden,
            Fault::RenamedFailureIsPendingAndMoves => {
                self.inner.generation() + calls.forgotten_moves
            }
            _ => self.inner.generation(),
        }
    }
}

fn run_broken(fault: Fault) {
    conformance::run(|loaders| Broken {
        inner: ReferenceStore::new(loaders),
        fault,
        calls: Mutex::default(),
    });
}

#[test]
#[should_panic(expected = "newest moves on")]
fn suite_rejects_a_store_whose_newest_is_stale() {
    run_broken(Fault::NewestIsStale);
}

#[test]
#[should_panic(expected = "assertion")]
fn suite_rejects_a_store_that_hides_failures() {
    run_broken(Fault::FailuresArePending);
}

#[test]
#[should_panic(expected = "assertion")]
fn suite_rejects_a_store_that_resolves_a_partial_set() {
    run_broken(Fault::ResolveSkipsUnpublished);
}

#[test]
#[should_panic(expected = "dependencies too")]
fn suite_rejects_a_store_that_resolves_without_dependencies() {
    run_broken(Fault::ResolveOmitsDependencies);
}

#[test]
#[should_panic(expected = "a publication")]
fn suite_rejects_a_store_whose_generation_is_stuck() {
    run_broken(Fault::GenerationIsConstant);
}

/// What the reference store did before D136: its `newest` read the index live, so a cook
/// that renamed a failed asset turned `Failed` into `Pending` with the generation unmoved.
#[test]
#[should_panic(expected = "a failure stays the answer after the index renamed its asset")]
fn suite_rejects_a_store_that_forgets_a_failure_when_its_asset_is_renamed() {
    run_broken(Fault::RenamedFailureIsPending);
}

/// A store that reloads by itself may move its generation at a cook. It may not answer
/// `Pending` for the failed asset it is reloading: the check that holds a store to an
/// unchanged answer only while its generation stands still would pass this one.
#[test]
#[should_panic(expected = "a renamed failure is the old failure or the new version, never pending")]
fn suite_rejects_a_store_that_forgets_a_failure_at_a_cook_and_moves_its_generation() {
    run_broken(Fault::RenamedFailureIsPendingAndMoves);
}

#[test]
#[should_panic(expected = "a repeated request for an asset the index does not name")]
fn suite_rejects_a_store_whose_repeated_failure_moves_nothing() {
    run_broken(Fault::RepeatedUnknownMovesOnce);
}

#[test]
#[should_panic(expected = "an exact request of a published version")]
fn suite_rejects_a_store_whose_exact_request_makes_a_version_current() {
    run_broken(Fault::ExactRequestMakesCurrent);
}

#[test]
#[should_panic(expected = "failed by exact request while named")]
fn suite_rejects_a_store_whose_newest_ignores_an_exact_failure() {
    run_broken(Fault::ExactFailureIsNotNewest);
}

#[test]
#[should_panic(expected = "get of the empty ID")]
fn suite_rejects_a_store_that_waits_for_the_empty_id() {
    run_broken(Fault::EmptyIdIsPending);
}

#[test]
#[should_panic(expected = "the lower dependency is corrupt")]
fn suite_rejects_a_store_that_reports_its_highest_failing_dependency() {
    run_broken(Fault::HighestDependencyFirst);
}

/// One link of a dependency chain: an asset with nothing in it.
struct Link;

impl Asset for Link {
    const KIND: AssetKind = AssetKind::from_name("conformance.link");
    const FORMAT_VERSION: u32 = 1;
}

struct LinkLoader;

impl Loader for LinkLoader {
    type Asset = Link;

    fn load(&self, _bytes: &[u8], _dependencies: &[AssetId]) -> Result<Link> {
        Ok(Link)
    }
}

/// A chain of `LINKS` assets, each declaring the next; with `closed`, the last declares
/// the first. Requests the head and settles.
fn chain(closed: bool) -> (ReferenceStore, AssetId, AssetId) {
    const LINKS: usize = 100_000;
    let link = |n: usize| AssetId::from_name(&format!("conformance/chain/{n}"));
    let store = ReferenceStore::new(vec![ErasedLoader::new(LinkLoader)]);
    for n in 0..LINKS {
        let next = if n + 1 < LINKS { Some(link(n + 1)) } else { closed.then(|| link(0)) };
        let cooked = CookedAsset::new::<Link>(Vec::new(), next);
        store.provide(link(n), cooked.version(), cooked);
    }
    store.request_newest(link(0), LoadPriority(0));
    store.settle();
    (store, link(0), link(LINKS - 1))
}

/// A dependency chain is as long as a cook makes it: the reference store walks one of
/// 100,000 links, open and closed into a cycle, on a thread with 256 KiB of stack. A walk
/// that recursed per link overflowed 2 MiB at ten thousand links before D136.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the walk is proved against a stack of a known, small size: a thread of its own"
)]
fn a_chain_of_100_000_dependencies_is_walked_without_recursion() {
    let walk = || {
        let (store, head, tail) = chain(false);
        assert!(matches!(store.newest(head), Readiness::Ready(_)), "the head is published");
        assert!(matches!(store.newest(tail), Readiness::Ready(_)), "and the tail before it");
        let table = store.resolve(&AssetSet::from_iter([head])).ready().expect("published");
        assert_eq!(table.len(), 100_000, "resolve closes over the whole chain");

        let (store, head, tail) = chain(true);
        let kind = |id| match store.newest(id) {
            Readiness::Failed(failure) => Some(failure.kind),
            Readiness::Ready(_) | Readiness::Pending => None,
        };
        assert_eq!(kind(head), Some(ErrorKind::InvalidData), "a cycle of 100,000");
        assert_eq!(kind(tail), Some(ErrorKind::InvalidData), "fails every asset in it");
    };
    let thread = std::thread::Builder::new().stack_size(256 * 1024).spawn(walk);
    thread.expect("a thread").join().expect("the walk neither overflowed nor failed");
}
