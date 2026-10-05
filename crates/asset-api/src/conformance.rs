//! The conformance suite every [`AssetStore`] implementation must pass (architecture
//! section 20), and the [`ReferenceStore`] the suite itself is tested against.
//!
//! A backend implements [`Harness`] around its store in a test and calls [`run`].

mod reference;

use lactose_core::{Error, ErrorKind, Result};

pub use self::reference::ReferenceStore;
use crate::handle::PinnedTable;
use crate::id::{Asset, AssetId, AssetKind, AssetRef};
use crate::pipeline::{CookedAsset, CookedHeader, ErasedLoader, Loader};
use crate::store::{AssetFailure, AssetStore, LoadRequest, Readiness};
use crate::version::{AssetSet, AssetVersion, VersionTable};

/// What the suite needs around a store under test.
///
/// The suite relies on a store loading only what it was asked for: a requested asset, a
/// dependency of one, or a newer version of an asset it has already loaded.
pub trait Harness {
    /// The store under test.
    fn store(&self) -> &dyn AssetStore;

    /// Makes one cooked asset version available to the store's source, as a cook that
    /// published it would. This alone publishes nothing.
    fn provide(&mut self, id: AssetId, version: AssetVersion, cooked: CookedAsset);

    /// Returns once every load the store has accepted so far has been published or has
    /// failed.
    fn settle(&mut self);
}

/// Runs the whole suite. `make` builds a fresh, empty harness whose store loads with the
/// given loaders; it is called once per check.
///
/// # Panics
///
/// Panics on the first violated requirement.
pub fn run<H: Harness>(make: impl Fn(Vec<ErasedLoader>) -> H) {
    let checks: [fn(&mut H); 10] = [
        publishes_what_was_requested,
        typed_lookups_check_the_kind,
        unknown_assets_fail_as_not_found,
        published_versions_never_change,
        failures_are_values,
        a_failed_newer_version_keeps_the_older_one,
        mismatched_cooked_data_is_refused,
        dependencies_are_published_first,
        resolve_reports_the_whole_set,
        pinned_tables_ignore_later_publications,
    ];
    for check in checks {
        check(&mut make(vec![ErasedLoader::new(TextLoader), ErasedLoader::new(FlagLoader)]));
    }
}

/// The suite's asset kind: cooked bytes are UTF-8 text.
#[derive(Debug, PartialEq)]
struct Text(String);

impl Asset for Text {
    const KIND: AssetKind = AssetKind::from_name("conformance.text");
    const FORMAT_VERSION: u32 = 1;
}

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
const MISSING: AssetId = AssetId::from_name("conformance/missing");
const V1: AssetVersion = AssetVersion(1);
const V2: AssetVersion = AssetVersion(2);

fn text(content: &str) -> CookedAsset {
    CookedAsset::new::<Text>(content.as_bytes().to_vec(), [])
}

fn corrupt_text() -> CookedAsset {
    CookedAsset::new::<Text>(vec![0xFF, 0xFE], [])
}

fn failed(id: AssetId, version: Option<AssetVersion>, kind: ErrorKind) -> AssetFailure {
    AssetFailure { id, version, kind }
}

/// Provides `cooked` as `version` of `id`, requests the newest version and settles.
fn publish(harness: &mut impl Harness, id: AssetId, version: AssetVersion, cooked: CookedAsset) {
    harness.provide(id, version, cooked);
    harness.store().request(LoadRequest::newest(id));
    harness.settle();
}

/// The text of the newest published version of `id`, with that version.
fn newest_text(harness: &impl Harness, id: AssetId) -> Readiness<(AssetVersion, String)> {
    harness.store().newest_typed(AssetRef::<Text>::new(id)).map(|h| (h.version(), h.0.clone()))
}

fn text_at(harness: &impl Harness, id: AssetId, version: AssetVersion) -> Readiness<String> {
    harness.store().get_typed(AssetRef::<Text>::new(id), version).map(|h| h.0.clone())
}

fn failure_of<T>(readiness: Readiness<T>) -> Option<AssetFailure> {
    match readiness {
        Readiness::Failed(failure) => Some(failure),
        Readiness::Ready(_) | Readiness::Pending => None,
    }
}

fn publishes_what_was_requested<H: Harness>(harness: &mut H) {
    harness.provide(A, V1, text("alpha"));
    harness.provide(B, V1, text("beta"));
    harness.provide(B, V2, text("beta two"));
    harness.provide(C, V1, text("gamma"));
    harness.store().request(LoadRequest::newest(A));
    harness.store().request(LoadRequest::exact(B, V1));
    harness.settle();

    let handle = harness.store().newest(A).ready().expect("a requested asset is published");
    assert_eq!((handle.id(), handle.version(), handle.kind()), (A, V1, Text::KIND));
    assert_eq!(newest_text(harness, A), Readiness::Ready((V1, "alpha".into())));
    assert_eq!(text_at(harness, A, V1), Readiness::Ready("alpha".into()));
    assert_eq!(text_at(harness, B, V1), Readiness::Ready("beta".into()), "an exact request");

    harness.store().request(LoadRequest::newest(A));
    harness.settle();
    assert_eq!(newest_text(harness, A), Readiness::Ready((V1, "alpha".into())), "a repeat request");

    assert!(matches!(harness.store().get(C, V1), Readiness::Pending), "never requested");
    assert!(matches!(harness.store().newest(C), Readiness::Pending), "never requested");
}

fn typed_lookups_check_the_kind<H: Harness>(harness: &mut H) {
    publish(harness, A, V1, text("alpha"));
    let store = harness.store();
    let wrong_kind = Some(failed(A, Some(V1), ErrorKind::InvalidData));
    assert_eq!(failure_of(store.newest_typed(AssetRef::<Flag>::new(A))), wrong_kind);
    assert_eq!(failure_of(store.get_typed(AssetRef::<Flag>::new(A), V1)), wrong_kind);
    let empty = Some(failed(AssetId::NONE, None, ErrorKind::NotFound));
    assert_eq!(failure_of(store.newest_typed(AssetRef::<Text>::NONE)), empty);
    assert_eq!(failure_of(store.get_typed(AssetRef::<Text>::NONE, V1)), empty);
}

fn unknown_assets_fail_as_not_found<H: Harness>(harness: &mut H) {
    harness.provide(A, V1, text("alpha"));
    harness.store().request(LoadRequest::newest(MISSING));
    harness.store().request(LoadRequest::exact(A, V2));
    harness.settle();
    let store = harness.store();
    assert_eq!(failure_of(store.newest(MISSING)), Some(failed(MISSING, None, ErrorKind::NotFound)));
    assert_eq!(failure_of(store.get(A, V2)), Some(failed(A, Some(V2), ErrorKind::NotFound)));
}

fn published_versions_never_change<H: Harness>(harness: &mut H) {
    publish(harness, A, V1, text("one"));
    let first = harness.store().newest_typed(AssetRef::<Text>::new(A)).ready().unwrap();

    publish(harness, A, V2, text("two"));
    assert_eq!(newest_text(harness, A), Readiness::Ready((V2, "two".into())), "newest moves on");
    assert_eq!((first.version(), first.0.as_str()), (V1, "one"), "a held handle is unaffected");
    assert_eq!(text_at(harness, A, V1), Readiness::Ready("one".into()), "a held version stays");

    harness.store().request(LoadRequest::exact(A, V1));
    harness.settle();
    assert_eq!(
        newest_text(harness, A),
        Readiness::Ready((V2, "two".into())),
        "newest never regresses"
    );
}

fn failures_are_values<H: Harness>(harness: &mut H) {
    publish(harness, A, V1, corrupt_text());
    let failure = Some(failed(A, Some(V1), ErrorKind::InvalidData));
    assert_eq!(failure_of(harness.store().get(A, V1)), failure);
    assert_eq!(failure_of(harness.store().newest(A)), failure, "nothing older to fall back on");

    harness.store().request(LoadRequest::exact(A, V1));
    harness.settle();
    assert_eq!(failure_of(harness.store().get(A, V1)), failure, "a retry of the same bytes");

    publish(harness, A, V2, text("fixed"));
    assert_eq!(newest_text(harness, A), Readiness::Ready((V2, "fixed".into())));
    assert_eq!(failure_of(harness.store().get(A, V1)), failure, "the failed version stays failed");
}

fn a_failed_newer_version_keeps_the_older_one<H: Harness>(harness: &mut H) {
    publish(harness, A, V1, text("good"));
    publish(harness, A, V2, corrupt_text());
    assert_eq!(newest_text(harness, A), Readiness::Ready((V1, "good".into())));
    let failure = Some(failed(A, Some(V2), ErrorKind::InvalidData));
    assert_eq!(failure_of(harness.store().get(A, V2)), failure);
    let set = AssetSet::from_iter([A]);
    assert_eq!(harness.store().resolve(&set), Readiness::Ready(VersionTable::from_iter([(A, V1)])));
}

fn mismatched_cooked_data_is_refused<H: Harness>(harness: &mut H) {
    let other_format = CookedHeader::new(Text::KIND, Text::FORMAT_VERSION + 1, []);
    publish(harness, A, V1, CookedAsset { header: other_format, bytes: b"alpha".to_vec() });
    let failure = Some(failed(A, Some(V1), ErrorKind::VersionMismatch));
    assert_eq!(failure_of(harness.store().get(A, V1)), failure);

    let no_loader = CookedHeader::new(AssetKind::from_name("conformance.unregistered"), 1, []);
    publish(harness, B, V1, CookedAsset { header: no_loader, bytes: Vec::new() });
    let failure = Some(failed(B, Some(V1), ErrorKind::Unsupported));
    assert_eq!(failure_of(harness.store().get(B, V1)), failure);
}

fn dependencies_are_published_first<H: Harness>(harness: &mut H) {
    harness.provide(B, V1, text("leaf"));
    harness.provide(C, V1, CookedAsset::new::<Text>(b"middle".to_vec(), [B]));
    publish(harness, A, V1, CookedAsset::new::<Text>(b"root".to_vec(), [C]));
    assert_eq!(newest_text(harness, A), Readiness::Ready((V1, "root".into())));
    assert_eq!(newest_text(harness, C), Readiness::Ready((V1, "middle".into())), "a dependency");
    assert_eq!(newest_text(harness, B), Readiness::Ready((V1, "leaf".into())), "transitively");

    let orphan = AssetId::from_name("conformance/orphan");
    publish(harness, orphan, V1, CookedAsset::new::<Text>(b"orphan".to_vec(), [MISSING]));
    let failure = Some(failed(orphan, Some(V1), ErrorKind::NotFound));
    assert_eq!(failure_of(harness.store().newest(orphan)), failure, "the dependency's kind");
}

fn resolve_reports_the_whole_set<H: Harness>(harness: &mut H) {
    assert_eq!(harness.store().resolve(&AssetSet::new()), Readiness::Ready(VersionTable::new()));

    publish(harness, A, V1, text("a one"));
    publish(harness, A, V2, text("a two"));
    publish(harness, B, V1, text("b one"));
    harness.provide(C, V1, text("c one"));
    let ready = AssetSet::from_iter([A, B]);
    let table = VersionTable::from_iter([(A, V2), (B, V1)]);
    assert_eq!(harness.store().resolve(&ready), Readiness::Ready(table), "newest of each, no more");

    let with_unloaded = AssetSet::from_iter([A, B, C]);
    assert_eq!(harness.store().resolve(&with_unloaded), Readiness::Pending);

    let also_missing = AssetId::from_name("conformance/also-missing");
    harness.store().request(LoadRequest::newest(MISSING));
    harness.store().request(LoadRequest::newest(also_missing));
    harness.settle();
    let lowest = failed(MISSING.min(also_missing), None, ErrorKind::NotFound);
    let with_failures = AssetSet::from_iter([A, B, C, MISSING, also_missing]);
    let resolved = harness.store().resolve(&with_failures);
    assert_eq!(failure_of(resolved), Some(lowest), "failed beats pending, lowest ID first");
}

fn pinned_tables_ignore_later_publications<H: Harness>(harness: &mut H) {
    publish(harness, A, V1, text("a one"));
    publish(harness, B, V1, CookedAsset::new::<Flag>(Vec::new(), []));
    harness.provide(C, V1, text("c one"));

    let table = harness.store().resolve(&AssetSet::from_iter([A, B])).ready().unwrap();
    let pinned = PinnedTable::pin(harness.store(), &table).ready().expect("a resolved table pins");
    assert_eq!(pinned.table(), &table);

    publish(harness, A, V2, text("a two"));
    assert_eq!(
        pinned.get(AssetRef::<Text>::new(A)),
        Ok(&Text("a one".into())),
        "still version one"
    );
    assert!(pinned.get(AssetRef::<Flag>::new(B)).is_ok());
    let wrong_kind = failed(B, Some(V1), ErrorKind::InvalidData);
    assert_eq!(pinned.get(AssetRef::<Text>::new(B)), Err(wrong_kind));
    let outside = failed(C, None, ErrorKind::NotFound);
    assert_eq!(pinned.get(AssetRef::<Text>::new(C)), Err(outside), "not in the table");
    let empty = failed(AssetId::NONE, None, ErrorKind::NotFound);
    assert_eq!(pinned.get(AssetRef::<Text>::NONE), Err(empty));

    let unloaded = VersionTable::from_iter([(A, V1), (C, V1)]);
    assert!(matches!(PinnedTable::pin(harness.store(), &unloaded), Readiness::Pending));

    harness.store().request(LoadRequest::exact(B, V2));
    harness.settle();
    let absent = VersionTable::from_iter([(A, V1), (B, V2), (C, V1)]);
    let failure = Some(failed(B, Some(V2), ErrorKind::NotFound));
    assert_eq!(
        failure_of(PinnedTable::pin(harness.store(), &absent)),
        failure,
        "failed beats pending"
    );
}
