//! The conformance suite run against the reference store, and against stores that break
//! one rule each, to show the suite notices.

use lactose_asset_api::conformance::{self, Harness, ReferenceStore};
use lactose_asset_api::{
    AssetId, AssetSet, AssetStore, AssetVersion, CookedAsset, LoadRequest, Readiness,
    UntypedHandle, VersionTable,
};

#[test]
fn reference_store_conforms() {
    conformance::run(ReferenceStore::new);
}

/// How a [`Broken`] store departs from the reference.
#[derive(Clone, Copy)]
enum Fault {
    /// `newest` serves version 1 whatever else is published.
    NewestIsOldest,
    /// Failures are reported as still pending.
    FailuresArePending,
    /// `resolve` ignores every asset that has no published version.
    ResolveSkipsUnpublished,
}

struct Broken {
    inner: ReferenceStore,
    fault: Fault,
}

impl Harness for Broken {
    fn store(&self) -> &dyn AssetStore {
        self
    }

    fn provide(&mut self, id: AssetId, version: AssetVersion, cooked: CookedAsset) {
        self.inner.provide(id, version, cooked);
    }

    fn settle(&mut self) {
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

impl AssetStore for Broken {
    fn request(&self, request: LoadRequest) {
        self.inner.request(request);
    }

    fn get(&self, id: AssetId, version: AssetVersion) -> Readiness<UntypedHandle> {
        self.hide_failure(self.inner.get(id, version))
    }

    fn newest(&self, id: AssetId) -> Readiness<UntypedHandle> {
        if let Fault::NewestIsOldest = self.fault
            && let Readiness::Ready(first) = self.inner.get(id, AssetVersion(1))
        {
            return Readiness::Ready(first);
        }
        self.hide_failure(self.inner.newest(id))
    }

    fn resolve(&self, set: &AssetSet) -> Readiness<VersionTable> {
        if let Fault::ResolveSkipsUnpublished = self.fault {
            let published =
                set.iter().filter_map(|id| Some((id, self.inner.newest(id).ready()?.version())));
            return Readiness::Ready(published.collect());
        }
        self.hide_failure(self.inner.resolve(set))
    }
}

fn run_broken(fault: Fault) {
    conformance::run(|loaders| Broken { inner: ReferenceStore::new(loaders), fault });
}

#[test]
#[should_panic(expected = "newest moves on")]
fn suite_rejects_a_store_whose_newest_is_stale() {
    run_broken(Fault::NewestIsOldest);
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
