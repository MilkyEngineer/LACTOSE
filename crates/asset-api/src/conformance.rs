//! The conformance suite every store must pass (architecture section 20), and the
//! [`ReferenceStore`] the suite itself is tested against.
//!
//! A backend implements [`Harness`] around its store in a test and calls [`run`].
//!
//! This module exists only under the `conformance` feature, which is for
//! `[dev-dependencies]` alone: nothing that ships may be able to name [`ReferenceStore`].

mod reference;

use std::sync::atomic::{AtomicBool, Ordering};

use lactose_core::{Error, ErrorKind, Result};

pub use self::reference::ReferenceStore;
use crate::budget::LoadPriority;
use crate::handle::PinnedTable;
use crate::id::{Asset, AssetId, AssetKind, AssetRef, SimulationAsset};
use crate::pipeline::{CookedAsset, CookedHeader, ErasedLoader, Loader};
use crate::store::{AssetCatalog, AssetFailure, PublishedAssets, Readiness};
use crate::version::{AssetSet, AssetVersion, VersionTable};

/// What the suite needs around a store under test. The suite calls it from several
/// threads at once.
pub trait Harness: Sync {
    /// The store under test.
    fn catalog(&self) -> &dyn AssetCatalog;

    /// Makes `cooked` available to the store's source as `version` of `id`, and makes that
    /// the version the pack index names for `id`, as a cook that published it would. This
    /// alone loads and publishes nothing. `version` is `cooked.version()` except where the
    /// suite checks that a store notices the difference.
    fn provide(&self, id: AssetId, version: AssetVersion, cooked: CookedAsset);

    /// Returns once every load the store has accepted so far has been published or has
    /// failed.
    fn settle(&self);
}

/// Runs the whole suite. `make` builds a fresh, empty harness whose store loads with the
/// given loaders; it is called once per check.
///
/// # Panics
///
/// Panics on the first violated requirement.
pub fn run<H: Harness>(make: impl Fn(Vec<ErasedLoader>) -> H) {
    let checks: [fn(&H); 15] = [
        publishes_what_was_requested,
        typed_lookups_check_the_kind,
        unknown_assets_fail_as_not_found,
        published_versions_never_change,
        failures_are_values,
        a_failed_newer_version_keeps_the_older_one,
        mismatched_cooked_data_is_refused,
        a_retry_can_succeed,
        dependencies_are_published_first,
        dependency_cycles_fail,
        resolve_closes_over_dependencies,
        resolve_reports_the_whole_set,
        pinned_tables_ignore_later_publications,
        generation_moves_only_with_publication,
        concurrent_use_is_consistent,
    ];
    for check in checks {
        check(&make(vec![ErasedLoader::new(TextLoader), ErasedLoader::new(FlagLoader)]));
    }
}

/// The suite's asset kind: cooked bytes are UTF-8 text.
#[derive(Debug, PartialEq)]
struct Text(String);

impl Asset for Text {
    const KIND: AssetKind = AssetKind::from_name("conformance.text");
    const FORMAT_VERSION: u32 = 1;
}

impl SimulationAsset for Text {}

struct TextLoader;

impl Loader for TextLoader {
    type Asset = Text;

    fn load(&self, bytes: &[u8], _dependencies: &[AssetId]) -> Result<Text> {
        match core::str::from_utf8(bytes) {
            Ok(text) => Ok(Text(text.to_owned())),
            Err(_) => Err(Error::new(ErrorKind::InvalidData, "text is not UTF-8")),
        }
    }
}

/// A second kind, so a reference of the wrong type has something to point at.
#[derive(Debug)]
struct Flag;

impl Asset for Flag {
    const KIND: AssetKind = AssetKind::from_name("conformance.flag");
    const FORMAT_VERSION: u32 = 1;
}

impl SimulationAsset for Flag {}

struct FlagLoader;

impl Loader for FlagLoader {
    type Asset = Flag;

    fn load(&self, _bytes: &[u8], _dependencies: &[AssetId]) -> Result<Flag> {
        Ok(Flag)
    }
}

const A: AssetId = AssetId::from_name("conformance/a");
const B: AssetId = AssetId::from_name("conformance/b");
const C: AssetId = AssetId::from_name("conformance/c");
const D: AssetId = AssetId::from_name("conformance/d");
const MISSING: AssetId = AssetId::from_name("conformance/missing");
const PRIORITY: LoadPriority = LoadPriority(0);

fn text(content: &str) -> CookedAsset {
    text_with(content, [])
}

fn text_with(content: &str, dependencies: impl IntoIterator<Item = AssetId>) -> CookedAsset {
    CookedAsset::new::<Text>(content.as_bytes().to_vec(), dependencies)
}

fn corrupt_text() -> CookedAsset {
    CookedAsset::new::<Text>(vec![0xFF, 0xFE], [])
}

fn failed(id: AssetId, version: Option<AssetVersion>, kind: ErrorKind) -> AssetFailure {
    AssetFailure { id, version, kind }
}

/// Provides `cooked` under its own version without requesting it.
fn offer(harness: &impl Harness, id: AssetId, cooked: CookedAsset) -> AssetVersion {
    let version = cooked.version();
    harness.provide(id, version, cooked);
    version
}

/// Provides `cooked`, requests the newest version of `id` and settles.
fn publish(harness: &impl Harness, id: AssetId, cooked: CookedAsset) -> AssetVersion {
    let version = offer(harness, id, cooked);
    harness.catalog().request_newest(id, PRIORITY);
    harness.settle();
    version
}

/// The text of the current published version of `id`, with that version.
fn newest_text(harness: &impl Harness, id: AssetId) -> Readiness<(AssetVersion, String)> {
    harness.catalog().newest_typed(AssetRef::<Text>::new(id)).map(|h| (h.version(), h.0.clone()))
}

fn text_at(harness: &impl Harness, id: AssetId, version: AssetVersion) -> Readiness<String> {
    harness.catalog().get_typed(AssetRef::<Text>::new(id), version).map(|h| h.0.clone())
}

fn ready_text(version: AssetVersion, content: &str) -> Readiness<(AssetVersion, String)> {
    Readiness::Ready((version, content.into()))
}

fn failure_of<T>(readiness: Readiness<T>) -> Option<AssetFailure> {
    match readiness {
        Readiness::Failed(failure) => Some(failure),
        Readiness::Ready(_) | Readiness::Pending => None,
    }
}

fn publishes_what_was_requested<H: Harness>(harness: &H) {
    let a = offer(harness, A, text("alpha"));
    let b_old = offer(harness, B, text("beta"));
    offer(harness, B, text("beta two"));
    let c = offer(harness, C, text("gamma"));
    harness.catalog().request_newest(A, PRIORITY);
    harness.catalog().request(B, b_old, PRIORITY);
    harness.settle();

    let handle = harness.catalog().newest(A).ready().expect("a requested asset is published");
    assert_eq!((handle.id(), handle.version(), handle.kind()), (A, a, Text::KIND));
    assert_eq!(newest_text(harness, A), ready_text(a, "alpha"));
    assert_eq!(text_at(harness, A, a), Readiness::Ready("alpha".into()));
    assert_eq!(text_at(harness, B, b_old), Readiness::Ready("beta".into()), "an exact request");
    let newest_b = harness.catalog().newest(B);
    assert!(matches!(newest_b, Readiness::Pending), "the index names another version of B");

    harness.catalog().request_newest(A, PRIORITY);
    harness.settle();
    assert_eq!(newest_text(harness, A), ready_text(a, "alpha"), "a repeat request");

    assert!(matches!(harness.catalog().get(C, c), Readiness::Pending), "never requested");
    assert!(matches!(harness.catalog().newest(C), Readiness::Pending), "never requested");
}

fn typed_lookups_check_the_kind<H: Harness>(harness: &H) {
    let a = publish(harness, A, text("alpha"));
    let catalog = harness.catalog();
    let wrong_kind = Some(failed(A, Some(a), ErrorKind::InvalidData));
    assert_eq!(failure_of(catalog.newest_typed(AssetRef::<Flag>::new(A))), wrong_kind);
    assert_eq!(failure_of(catalog.get_typed(AssetRef::<Flag>::new(A), a)), wrong_kind);
    let empty = Some(failed(AssetId::NONE, None, ErrorKind::NotFound));
    assert_eq!(failure_of(catalog.newest_typed(AssetRef::<Text>::NONE)), empty);
    assert_eq!(failure_of(catalog.get_typed(AssetRef::<Text>::NONE, a)), empty);
}

fn unknown_assets_fail_as_not_found<H: Harness>(harness: &H) {
    offer(harness, A, text("alpha"));
    let absent = text("never provided").version();
    harness.catalog().request_newest(MISSING, PRIORITY);
    harness.catalog().request(A, absent, PRIORITY);
    harness.settle();
    let catalog = harness.catalog();
    assert_eq!(
        failure_of(catalog.newest(MISSING)),
        Some(failed(MISSING, None, ErrorKind::NotFound))
    );
    assert_eq!(
        failure_of(catalog.get(A, absent)),
        Some(failed(A, Some(absent), ErrorKind::NotFound))
    );
}

fn published_versions_never_change<H: Harness>(harness: &H) {
    let one = publish(harness, A, text("one"));
    let first = harness.catalog().newest_typed(AssetRef::<Text>::new(A)).ready().unwrap();

    let two = publish(harness, A, text("two"));
    assert_ne!(one, two);
    assert_eq!(newest_text(harness, A), ready_text(two, "two"), "newest moves on");
    assert_eq!((first.version(), first.0.as_str()), (one, "one"), "a held handle is unaffected");
    assert_eq!(text_at(harness, A, one), Readiness::Ready("one".into()), "a held version stays");

    harness.catalog().request(A, one, PRIORITY);
    harness.settle();
    assert_eq!(newest_text(harness, A), ready_text(two, "two"), "an exact request moves nothing");

    // Versions have no order: the index may name an earlier one again.
    assert_eq!(publish(harness, A, text("one")), one);
    assert_eq!(newest_text(harness, A), ready_text(one, "one"), "newest follows the index");
}

fn failures_are_values<H: Harness>(harness: &H) {
    let bad = publish(harness, A, corrupt_text());
    let failure = Some(failed(A, Some(bad), ErrorKind::InvalidData));
    assert_eq!(failure_of(harness.catalog().get(A, bad)), failure);
    assert_eq!(failure_of(harness.catalog().newest(A)), failure, "nothing to fall back on");

    harness.catalog().request(A, bad, PRIORITY);
    harness.settle();
    assert_eq!(failure_of(harness.catalog().get(A, bad)), failure, "a retry of the same bytes");

    let fixed = publish(harness, A, text("fixed"));
    assert_eq!(newest_text(harness, A), ready_text(fixed, "fixed"));
    assert_eq!(
        failure_of(harness.catalog().get(A, bad)),
        failure,
        "the failed version stays failed"
    );
}

fn a_failed_newer_version_keeps_the_older_one<H: Harness>(harness: &H) {
    let good = publish(harness, A, text("good"));
    let bad = publish(harness, A, corrupt_text());
    assert_eq!(newest_text(harness, A), ready_text(good, "good"));
    let failure = Some(failed(A, Some(bad), ErrorKind::InvalidData));
    assert_eq!(failure_of(harness.catalog().get(A, bad)), failure);
    let resolved = harness.catalog().resolve(&AssetSet::from_iter([A]));
    assert_eq!(resolved, Readiness::Ready(VersionTable::from_iter([(A, good)])));
}

fn mismatched_cooked_data_is_refused<H: Harness>(harness: &H) {
    let other_format = CookedHeader::new(Text::KIND, Text::FORMAT_VERSION + 1, []);
    let a = publish(harness, A, CookedAsset { header: other_format, bytes: b"alpha".to_vec() });
    let failure = Some(failed(A, Some(a), ErrorKind::VersionMismatch));
    assert_eq!(failure_of(harness.catalog().get(A, a)), failure);

    let no_loader = CookedHeader::new(AssetKind::from_name("conformance.unregistered"), 1, []);
    let b = publish(harness, B, CookedAsset { header: no_loader, bytes: Vec::new() });
    let failure = Some(failed(B, Some(b), ErrorKind::Unsupported));
    assert_eq!(failure_of(harness.catalog().get(B, b)), failure);

    // Content stored under a version it does not hash to, as after disk corruption.
    let claimed = text("what the index promised").version();
    harness.provide(C, claimed, text("what the disk holds"));
    harness.catalog().request_newest(C, PRIORITY);
    harness.settle();
    let failure = Some(failed(C, Some(claimed), ErrorKind::InvalidData));
    assert_eq!(failure_of(harness.catalog().get(C, claimed)), failure, "the hash is verified");
    assert_eq!(failure_of(harness.catalog().newest(C)), failure);
}

fn a_retry_can_succeed<H: Harness>(harness: &H) {
    let cooked = text("late");
    let version = cooked.version();
    harness.catalog().request(A, version, PRIORITY);
    harness.catalog().request_newest(B, PRIORITY);
    harness.settle();
    let missing = Some(failed(A, Some(version), ErrorKind::NotFound));
    assert_eq!(failure_of(harness.catalog().get(A, version)), missing);
    assert_eq!(failure_of(harness.catalog().newest(B)), Some(failed(B, None, ErrorKind::NotFound)));

    harness.provide(A, version, cooked);
    let b = offer(harness, B, text("also late"));
    assert_eq!(failure_of(harness.catalog().get(A, version)), missing, "no load without a request");
    harness.catalog().request(A, version, PRIORITY);
    harness.catalog().request_newest(B, PRIORITY);
    harness.settle();
    assert_eq!(text_at(harness, A, version), Readiness::Ready("late".into()));
    assert_eq!(newest_text(harness, B), ready_text(b, "also late"));
}

fn dependencies_are_published_first<H: Harness>(harness: &H) {
    let b = offer(harness, B, text("leaf"));
    let c = offer(harness, C, text_with("middle", [B]));
    let a = publish(harness, A, text_with("root", [C]));
    assert_eq!(newest_text(harness, A), ready_text(a, "root"));
    assert_eq!(newest_text(harness, C), ready_text(c, "middle"), "a dependency");
    assert_eq!(newest_text(harness, B), ready_text(b, "leaf"), "transitively");

    let orphan = publish(harness, D, text_with("orphan", [MISSING]));
    let failure = Some(failed(D, Some(orphan), ErrorKind::NotFound));
    assert_eq!(failure_of(harness.catalog().newest(D)), failure, "the dependency's kind");
}

fn dependency_cycles_fail<H: Harness>(harness: &H) {
    let b = offer(harness, B, text_with("b needs a", [A]));
    let a = publish(harness, A, text_with("a needs b", [B]));
    let catalog = harness.catalog();
    assert_eq!(failure_of(catalog.newest(A)), Some(failed(A, Some(a), ErrorKind::InvalidData)));
    assert_eq!(failure_of(catalog.newest(B)), Some(failed(B, Some(b), ErrorKind::InvalidData)));
    assert!(failure_of(catalog.resolve(&AssetSet::from_iter([A]))).is_some());
}

fn resolve_closes_over_dependencies<H: Harness>(harness: &H) {
    let c = offer(harness, C, text("leaf"));
    let b = offer(harness, B, text_with("middle", [C]));
    let a = publish(harness, A, text_with("root", [B]));
    let d = publish(harness, D, text("apart"));
    let only_a = AssetSet::from_iter([A]);

    let closure = VersionTable::from_iter([(A, a), (B, b), (C, c)]);
    assert_eq!(harness.catalog().resolve(&only_a), Readiness::Ready(closure), "dependencies too");
    let both = VersionTable::from_iter([(A, a), (B, b), (C, c), (D, d)]);
    assert_eq!(harness.catalog().resolve(&AssetSet::from_iter([A, D])), Readiness::Ready(both));
    let listed_twice = AssetSet::from_iter([A, C]);
    let closure = VersionTable::from_iter([(A, a), (B, b), (C, c)]);
    assert_eq!(harness.catalog().resolve(&listed_twice), Readiness::Ready(closure));

    // A new version of A that adds a dependency nobody requested by name.
    let extra = AssetId::from_name("conformance/extra");
    let e = offer(harness, extra, text("added later"));
    let a_two = publish(harness, A, text_with("root two", [B, extra]));
    let closure = VersionTable::from_iter([(A, a_two), (B, b), (C, c), (extra, e)]);
    assert_eq!(harness.catalog().resolve(&only_a), Readiness::Ready(closure), "the new dependency");
}

fn resolve_reports_the_whole_set<H: Harness>(harness: &H) {
    let empty = harness.catalog().resolve(&AssetSet::new());
    assert_eq!(empty, Readiness::Ready(VersionTable::new()));

    publish(harness, A, text("a one"));
    let a = publish(harness, A, text("a two"));
    let b = publish(harness, B, text("b one"));
    offer(harness, C, text("c one"));
    let table = VersionTable::from_iter([(A, a), (B, b)]);
    let resolved = harness.catalog().resolve(&AssetSet::from_iter([A, B]));
    assert_eq!(resolved, Readiness::Ready(table), "the current version of each, no more");

    let with_unloaded = AssetSet::from_iter([A, B, C]);
    assert_eq!(harness.catalog().resolve(&with_unloaded), Readiness::Pending);

    let also_missing = AssetId::from_name("conformance/also-missing");
    harness.catalog().request_newest(MISSING, PRIORITY);
    harness.catalog().request_newest(also_missing, PRIORITY);
    harness.settle();
    let lowest = failed(MISSING.min(also_missing), None, ErrorKind::NotFound);
    let with_failures = AssetSet::from_iter([A, B, C, MISSING, also_missing]);
    let resolved = harness.catalog().resolve(&with_failures);
    assert_eq!(failure_of(resolved), Some(lowest), "failed beats pending, lowest ID first");
}

fn pinned_tables_ignore_later_publications<H: Harness>(harness: &H) {
    let a = publish(harness, A, text("a one"));
    let b = publish(harness, B, CookedAsset::new::<Flag>(Vec::new(), []));
    let c = offer(harness, C, text("c one"));
    let published: &dyn PublishedAssets = harness.catalog();

    let table = harness.catalog().resolve(&AssetSet::from_iter([A, B])).ready().unwrap();
    let pinned = PinnedTable::pin(published, &table).ready().expect("a resolved table pins");
    assert_eq!(pinned.table(), &table);

    publish(harness, A, text("a two"));
    assert_eq!(pinned.get(AssetRef::<Text>::new(A)), Ok(&Text("a one".into())), "still pinned");
    assert!(pinned.get(AssetRef::<Flag>::new(B)).is_ok());
    let wrong_kind = failed(B, Some(b), ErrorKind::InvalidData);
    assert_eq!(pinned.get(AssetRef::<Text>::new(B)), Err(wrong_kind));
    let outside = failed(C, None, ErrorKind::NotFound);
    assert_eq!(pinned.get(AssetRef::<Text>::new(C)), Err(outside), "not in the table");
    let empty = failed(AssetId::NONE, None, ErrorKind::NotFound);
    assert_eq!(pinned.get(AssetRef::<Text>::NONE), Err(empty));

    let unloaded = VersionTable::from_iter([(A, a), (C, c)]);
    assert!(matches!(PinnedTable::pin(published, &unloaded), Readiness::Pending));

    let absent = text("never provided").version();
    harness.catalog().request(B, absent, PRIORITY);
    harness.settle();
    let table = VersionTable::from_iter([(A, a), (B, absent), (C, c)]);
    let failure = Some(failed(B, Some(absent), ErrorKind::NotFound));
    assert_eq!(failure_of(PinnedTable::pin(published, &table)), failure, "failed beats pending");
}

fn generation_moves_only_with_publication<H: Harness>(harness: &H) {
    let start = harness.catalog().generation();
    let a = offer(harness, A, text("alpha"));
    let _ = harness.catalog().newest(A);
    let _ = harness.catalog().resolve(&AssetSet::from_iter([A]));
    harness.settle();
    assert_eq!(harness.catalog().generation(), start, "providing and looking publish nothing");

    harness.catalog().request_newest(A, PRIORITY);
    harness.settle();
    let published = harness.catalog().generation();
    assert!(published > start, "a publication");

    harness.catalog().request_newest(A, PRIORITY);
    harness.catalog().request(A, a, PRIORITY);
    harness.settle();
    assert_eq!(harness.catalog().generation(), published, "requests for what is published");

    harness.catalog().request_newest(MISSING, PRIORITY);
    harness.settle();
    let failed = harness.catalog().generation();
    assert!(failed > published, "a recorded failure");

    publish(harness, A, text("alpha two"));
    assert!(harness.catalog().generation() > failed, "a new current version");
}

/// Readers on several threads while assets are published under them. Every answer must be
/// one a sequential store could have given: content always hashes to its version, the
/// generation never goes back, and a resolved table always holds the dependency of the
/// version of `A` it names, although each round publishes that pair anew.
#[expect(
    clippy::disallowed_methods,
    reason = "the suite drives a store from several threads; its feature is for dev-dependencies alone"
)]
fn concurrent_use_is_consistent<H: Harness>(harness: &H) {
    const ROUNDS: usize = 40;
    let dependency = |round: usize| AssetId::from_name(&format!("conformance/dependency-{round}"));
    let root = |round: usize| text_with(&format!("root {round}"), [dependency(round)]);
    let leaf = |round: usize| text(&format!("leaf {round}"));
    // Which dependency each version of A declares.
    let expected: Vec<_> =
        (0..ROUNDS).map(|round| (root(round).version(), dependency(round))).collect();
    let set = AssetSet::from_iter([A]);
    let done = AtomicBool::new(false);

    let read = || {
        let mut generation = 0;
        let mut resolved = 0;
        while !done.load(Ordering::Acquire) || resolved == 0 {
            let now = harness.catalog().generation();
            assert!(now >= generation, "generation went back");
            generation = now;

            if let Readiness::Ready(handle) = harness.catalog().newest(A) {
                let text = handle.clone().downcast::<Text>().expect("A is text");
                assert_eq!(
                    text_with(&text.0, [expected_dependency(&expected, handle.version())])
                        .version(),
                    handle.version()
                );
            }
            match harness.catalog().resolve(&set) {
                Readiness::Ready(table) => {
                    let version = table.get(A).expect("the set's own member");
                    let needs = expected_dependency(&expected, version);
                    assert!(table.get(needs).is_some(), "resolve saw A without its dependency");
                    assert_eq!(table.len(), 2);
                    resolved += 1;
                }
                Readiness::Pending => {}
                Readiness::Failed(failure) => panic!("resolve failed: {failure}"),
            }
        }
    };
    let request = || {
        while !done.load(Ordering::Acquire) {
            for (version, _) in &expected {
                harness.catalog().request(A, *version, PRIORITY);
            }
            harness.catalog().request_newest(A, PRIORITY);
            harness.settle();
            std::thread::yield_now();
        }
    };

    // The first round is published before anyone looks, so A always has a current version
    // and no reader can see the failure of a request that raced ahead of the first cook.
    offer(harness, dependency(0), leaf(0));
    publish(harness, A, root(0));

    std::thread::scope(|scope| {
        for _ in 0..3 {
            scope.spawn(read);
        }
        scope.spawn(request);
        for round in 1..ROUNDS {
            offer(harness, dependency(round), leaf(round));
            offer(harness, A, root(round));
            harness.catalog().request_newest(A, PRIORITY);
            harness.settle();
        }
        done.store(true, Ordering::Release);
    });

    let last = ROUNDS - 1;
    assert_eq!(newest_text(harness, A), ready_text(root(last).version(), &format!("root {last}")));
}

fn expected_dependency(expected: &[(AssetVersion, AssetId)], version: AssetVersion) -> AssetId {
    let found = expected.iter().find(|(known, _)| *known == version);
    found.expect("a version of A that was never provided").1
}
