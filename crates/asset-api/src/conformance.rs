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
    /// the version the pack index names for `id`, as a cook that published it would. By
    /// itself this changes no answer, and it loads nothing of an asset whose newest version
    /// nobody has asked for. A store that reloads by itself may, for an asset somebody has
    /// asked the newest version of, start loading the new version, or make it current if
    /// it is published already, moving the generation with any answer it changes.
    /// `version` is `cooked.version()` except where the suite checks that a store notices
    /// the difference.
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
    let checks: [fn(&H); 21] = [
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
        answers_change_only_with_the_generation,
        every_failed_retry_moves_the_generation,
        an_exact_request_of_a_published_version_changes_nothing,
        the_end_of_a_load_answers_newest_whoever_asked,
        the_empty_id_names_no_asset,
        failures_follow_dependencies_in_id_order,
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

/// A cook that renames an asset changes no answer by itself, whatever the asset's state: a
/// ready one, a failed one, one the index did not name. The failure of a renamed asset
/// stays the answer until a load of the newly named version ends, and is never `Pending`
/// in between.
///
/// A store that reloads by itself may have the new versions loading, or loaded, by the
/// time the suite looks. So for an asset somebody asked the newest version of, each answer
/// is held to the one from before the cook or the newly named version, and to the one from
/// before exactly while the generation stands still. For an asset nobody asked the newest
/// version of, no store loads anything at a cook, and its answers are compared outright.
fn answers_change_only_with_the_generation<H: Harness>(harness: &H) {
    let catalog = harness.catalog();
    let set = AssetSet::from_iter([A]);
    let one = publish(harness, A, text("one"));
    let bad = publish(harness, B, corrupt_text());
    let failure = failed(B, Some(bad), ErrorKind::InvalidData);
    catalog.request_newest(MISSING, PRIORITY);
    // D fails by an exact request of the version the index names: nobody asks for its newest.
    let lone = offer(harness, D, corrupt_text());
    catalog.request(D, lone, PRIORITY);
    harness.settle();
    let unknown = failed(MISSING, None, ErrorKind::NotFound);
    assert_eq!(failure_of(catalog.newest(B)), Some(failure));
    assert_eq!(failure_of(catalog.newest(MISSING)), Some(unknown));
    let lone_answers = || (newest_text(harness, D), text_at(harness, D, lone));
    let lone_before = lone_answers();

    let before = catalog.generation();
    let two = offer(harness, A, text("two"));
    let fixed = offer(harness, B, text("fixed"));
    let found = offer(harness, MISSING, text("found"));
    let mended = offer(harness, D, text("mended"));
    let renamed = newest_text(harness, A);
    let table = catalog.resolve(&set);
    let stale = newest_text(harness, B);
    let stale_at = failure_of(catalog.get(B, bad));
    let stale_unknown = newest_text(harness, MISSING);
    let lone_after = lone_answers();
    let unmoved = catalog.generation() == before;

    let old_table = Readiness::Ready(VersionTable::from_iter([(A, one)]));
    let new_table = Readiness::Ready(VersionTable::from_iter([(A, two)]));
    let (old_failure, old_unknown) = (Readiness::Failed(failure), Readiness::Failed(unknown));
    if unmoved {
        // While the generation stands still, every answer is the one from before the cook.
        assert_eq!(renamed, ready_text(one, "one"), "a cook alone does not change newest");
        assert_eq!(table, old_table, "nor resolve");
        assert_eq!(
            stale, old_failure,
            "a failure stays the answer after the index renamed its asset"
        );
        assert_eq!(stale_unknown, old_unknown, "and for an asset the index did not name before");
    }
    // Whether or not a reload of the store's own has ended: the answer from before the cook
    // or the newly named version, and never `Pending` in between.
    assert!(
        renamed == ready_text(one, "one") || renamed == ready_text(two, "two"),
        "newest of a renamed asset is its old version or its new one, got {renamed:?}"
    );
    assert!(
        table == old_table || table == new_table,
        "resolve of a renamed asset is its old table or its new one, got {table:?}"
    );
    assert!(
        stale == old_failure || stale == ready_text(fixed, "fixed"),
        "a renamed failure is the old failure or the new version, never pending, got {stale:?}"
    );
    assert_eq!(stale_at, Some(failure), "the failed version stays failed by exact version");
    assert!(
        stale_unknown == old_unknown || stale_unknown == ready_text(found, "found"),
        "an unnamed asset is its old failure or its first version, got {stale_unknown:?}"
    );
    // Nobody asked for the newest D, so no store loads anything of it at a cook.
    assert_eq!(lone_after, lone_before, "a cook changes no answer of an asset nobody asked for");
    assert!(matches!(catalog.get(D, mended), Readiness::Pending), "and loads nothing of it");

    catalog.request_newest(A, PRIORITY);
    catalog.request_newest(B, PRIORITY);
    catalog.request_newest(MISSING, PRIORITY);
    harness.settle();
    assert!(catalog.generation() > before, "the ends of the loads move the generation");
    assert_eq!(newest_text(harness, A), ready_text(two, "two"));
    assert_eq!(newest_text(harness, B), ready_text(fixed, "fixed"), "a later load's outcome");
    assert_eq!(newest_text(harness, MISSING), ready_text(found, "found"));
    assert_eq!(failure_of(catalog.get(B, bad)), Some(failure), "the failed version stays failed");

    // A failure is replaced by a later failure too.
    let first = publish(harness, C, corrupt_text());
    let second = publish(harness, C, CookedAsset::new::<Text>(vec![0xC3], []));
    assert_ne!(first, second);
    let replaced = Some(failed(C, Some(second), ErrorKind::InvalidData));
    assert_eq!(failure_of(catalog.newest(C)), replaced, "the last load to end");
}

/// A retry that fails again is a recorded failure like the first: the generation moves, so
/// a caller that retries and waits for the generation to move never waits for ever.
fn every_failed_retry_moves_the_generation<H: Harness>(harness: &H) {
    let catalog = harness.catalog();
    catalog.request_newest(MISSING, PRIORITY);
    harness.settle();
    let unknown = Some(failed(MISSING, None, ErrorKind::NotFound));
    let first = catalog.generation();
    catalog.request_newest(MISSING, PRIORITY);
    harness.settle();
    let second = catalog.generation();
    assert!(second > first, "a repeated request for an asset the index does not name");
    assert_eq!(failure_of(catalog.newest(MISSING)), unknown);

    let bad = publish(harness, A, corrupt_text());
    let failure = Some(failed(A, Some(bad), ErrorKind::InvalidData));
    let failed_at = catalog.generation();
    catalog.request_newest(A, PRIORITY);
    harness.settle();
    let retried = catalog.generation();
    assert!(retried > failed_at, "a retry of the named version that fails again");
    catalog.request(A, bad, PRIORITY);
    harness.settle();
    assert!(catalog.generation() > retried, "a retry by exact version that fails again");
    assert_eq!(failure_of(catalog.newest(A)), failure);
}

/// A request by exact version for something already published does nothing, also when the
/// index has come to name that version: only a request for the newest version makes an
/// already published version current.
fn an_exact_request_of_a_published_version_changes_nothing<H: Harness>(harness: &H) {
    let catalog = harness.catalog();
    let one = offer(harness, A, text("one"));
    offer(harness, A, text("two"));
    catalog.request(A, one, PRIORITY);
    harness.settle();
    assert_eq!(text_at(harness, A, one), Readiness::Ready("one".into()));
    assert!(matches!(catalog.newest(A), Readiness::Pending), "the index named another version");

    // The index comes back to the published version. Nobody has asked for the newest A.
    assert_eq!(offer(harness, A, text("one")), one);
    let before = catalog.generation();
    catalog.request(A, one, PRIORITY);
    harness.settle();
    let newest = catalog.newest(A);
    assert!(matches!(newest, Readiness::Pending), "an exact request of a published version");
    assert_eq!(catalog.generation(), before, "moves nothing");

    catalog.request_newest(A, PRIORITY);
    harness.settle();
    assert_eq!(newest_text(harness, A), ready_text(one, "one"), "request_newest makes it current");
    assert!(catalog.generation() > before);
}

/// The end of a load of the version the index names is the asset's answer for `newest`,
/// whoever asked for the load: the current version if it was published, its failure if it
/// failed. The end of a load of any other version is an answer for `get` alone.
fn the_end_of_a_load_answers_newest_whoever_asked<H: Harness>(harness: &H) {
    let catalog = harness.catalog();
    let good = offer(harness, A, text("alpha"));
    let bad = offer(harness, B, corrupt_text());
    offer(harness, C, text("gamma"));
    let absent = text("never provided").version();
    catalog.request(A, good, PRIORITY);
    catalog.request(B, bad, PRIORITY);
    catalog.request(C, absent, PRIORITY);
    harness.settle();

    assert_eq!(newest_text(harness, A), ready_text(good, "alpha"), "published while named");
    let failure = Some(failed(B, Some(bad), ErrorKind::InvalidData));
    assert_eq!(failure_of(catalog.newest(B)), failure, "failed by exact request while named");
    let missing = Some(failed(C, Some(absent), ErrorKind::NotFound));
    assert_eq!(failure_of(catalog.get(C, absent)), missing);
    assert!(matches!(catalog.newest(C), Readiness::Pending), "a version the index does not name");
}

/// [`AssetId::NONE`] names no asset in any store: a request for it does nothing, and a
/// lookup of it fails as a typed lookup through an empty reference does.
fn the_empty_id_names_no_asset<H: Harness>(harness: &H) {
    let catalog = harness.catalog();
    let version = text("anything").version();
    let before = catalog.generation();
    catalog.request(AssetId::NONE, version, PRIORITY);
    catalog.request_newest(AssetId::NONE, PRIORITY);
    harness.settle();
    let none = Some(failed(AssetId::NONE, None, ErrorKind::NotFound));
    assert_eq!(failure_of(catalog.get(AssetId::NONE, version)), none, "get of the empty ID");
    assert_eq!(failure_of(catalog.newest(AssetId::NONE)), none, "newest of the empty ID");
    assert_eq!(catalog.generation(), before, "a request for the empty ID does nothing");
}

/// The three IDs of one scenario: two dependencies, the lower ID first, and the asset that
/// declares them.
fn scenario(name: &str) -> (AssetId, AssetId, AssetId) {
    let id = |role: &str| AssetId::from_name(&format!("conformance/{name}/{role}"));
    let (a, b) = (id("a"), id("b"));
    (a.min(b), a.max(b), id("root"))
}

/// Which failure a load reports when several apply is the same in every store and in every
/// order of requests, for an index that does not change while the load is in flight: the
/// kind of its first dependency, in ascending [`AssetId`] order, that cannot be published,
/// before anything of its own; and for a dependency cycle `InvalidData`, unless a failing
/// dependency with a lower ID decides first.
fn failures_follow_dependencies_in_id_order<H: Harness>(harness: &H) {
    let catalog = harness.catalog();
    let kind_of = |id: AssetId| failure_of(catalog.newest(id)).map(|failure| failure.kind);
    let request = |ids: &[AssetId]| {
        for &id in ids {
            catalog.request_newest(id, PRIORITY);
        }
        harness.settle();
    };

    // Two failing dependencies: the lower ID's kind, whichever kind that is.
    let (low, high, root) = scenario("corrupt-then-missing");
    offer(harness, low, corrupt_text());
    offer(harness, root, text_with("root", [low, high]));
    request(&[root]);
    assert_eq!(kind_of(root), Some(ErrorKind::InvalidData), "the lower dependency is corrupt");
    let (low, high, root) = scenario("missing-then-corrupt");
    offer(harness, high, corrupt_text());
    offer(harness, root, text_with("root", [low, high]));
    request(&[root]);
    assert_eq!(kind_of(root), Some(ErrorKind::NotFound), "the lower dependency is missing");

    // A dependency's failure comes before the dependant's own: corrupt bytes, and no loader.
    let (missing, _, root) = scenario("corrupt-root");
    offer(harness, root, CookedAsset::new::<Text>(vec![0xFF, 0xFE], [missing]));
    request(&[root]);
    assert_eq!(kind_of(root), Some(ErrorKind::NotFound), "before the dependant's own bytes");
    let (missing, _, root) = scenario("unloadable-root");
    let no_loader = AssetKind::from_name("conformance.unregistered");
    let header = CookedHeader::new(no_loader, 1, [missing]);
    offer(harness, root, CookedAsset { header, bytes: Vec::new() });
    request(&[root]);
    assert_eq!(kind_of(root), Some(ErrorKind::NotFound), "before the dependant's own loader");

    // A cycle beside a missing dependency. With the missing one lower, it decides the
    // root's kind and, through the root, the kind of the dependency that closes the cycle,
    // in either order of requests.
    for (name, root_first) in [("cycle-after-missing", true), ("cycle-after-missing-2", false)] {
        let (low, high, root) = scenario(name);
        offer(harness, high, text_with("back to the root", [root]));
        offer(harness, root, text_with("root", [low, high]));
        request(&if root_first { [root, high] } else { [high, root] });
        assert_eq!(kind_of(root), Some(ErrorKind::NotFound), "{name}: the lower dependency");
        assert_eq!(kind_of(high), Some(ErrorKind::NotFound), "{name}: through the root");
    }
    // With the dependency that closes the cycle lower, the cycle decides both.
    for (name, root_first) in [("cycle-before-missing", true), ("cycle-before-missing-2", false)] {
        let (low, high, root) = scenario(name);
        offer(harness, low, text_with("back to the root", [root]));
        offer(harness, root, text_with("root", [low, high]));
        request(&if root_first { [root, low] } else { [low, root] });
        assert_eq!(kind_of(root), Some(ErrorKind::InvalidData), "{name}: the cycle comes first");
        assert_eq!(kind_of(low), Some(ErrorKind::InvalidData), "{name}: a member of the cycle");
    }
}
