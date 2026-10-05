//! The conformance suite run against the reference store, and against stores that break
//! one rule each, to show the suite notices.

#![forbid(unsafe_code)]

use lactose_asset_api::conformance::{self, Harness, ReferenceStore};
use lactose_asset_api::{
    AssetCatalog, AssetId, AssetSet, AssetVersion, CookedAsset, LoadPriority, PublishedAssets,
    Readiness, UntypedHandle, VersionTable,
};

#[test]
fn reference_store_conforms() {
    conformance::run(ReferenceStore::new);
}

/// How a [`Broken`] store departs from the reference.
#[derive(Clone, Copy)]
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
}

struct Broken {
    inner: ReferenceStore,
    fault: Fault,
    first_served: std::sync::Mutex<Vec<(AssetId, UntypedHandle)>>,
}

impl Harness for Broken {
    fn catalog(&self) -> &dyn AssetCatalog {
        self
    }

    fn provide(&self, id: AssetId, version: AssetVersion, cooked: CookedAsset) {
        self.inner.provide(id, version, cooked);
    }

    fn settle(&self) {
        self.inner.settle();
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
        self.inner.request(id, version, priority);
    }

    fn get(&self, id: AssetId, version: AssetVersion) -> Readiness<UntypedHandle> {
        self.hide_failure(self.inner.get(id, version))
    }
}

impl AssetCatalog for Broken {
    fn request_newest(&self, id: AssetId, priority: LoadPriority) {
        self.inner.request_newest(id, priority);
    }

    fn newest(&self, id: AssetId) -> Readiness<UntypedHandle> {
        let newest = self.hide_failure(self.inner.newest(id));
        if let (Fault::NewestIsStale, Readiness::Ready(handle)) = (self.fault, &newest) {
            let mut served = self.first_served.lock().unwrap();
            if let Some((_, first)) = served.iter().find(|(served_id, _)| *served_id == id) {
                return Readiness::Ready(first.clone());
            }
            served.push((id, handle.clone()));
        }
        newest
    }

    fn resolve(&self, set: &AssetSet) -> Readiness<VersionTable> {
        let own_members =
            || set.iter().filter_map(|id| Some((id, self.inner.newest(id).ready()?.version())));
        match (self.fault, self.inner.resolve(set)) {
            (Fault::ResolveSkipsUnpublished, _) => Readiness::Ready(own_members().collect()),
            (Fault::ResolveOmitsDependencies, Readiness::Ready(_)) => {
                Readiness::Ready(own_members().collect())
            }
            (_, resolved) => self.hide_failure(resolved),
        }
    }

    fn generation(&self) -> u64 {
        match self.fault {
            Fault::GenerationIsConstant => 0,
            _ => self.inner.generation(),
        }
    }
}

fn run_broken(fault: Fault) {
    conformance::run(|loaders| Broken {
        inner: ReferenceStore::new(loaders),
        fault,
        first_served: std::sync::Mutex::default(),
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
