//! The in-memory store the conformance suite is tested against.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use lactose_core::ErrorKind;

use super::Harness;
use crate::handle::UntypedHandle;
use crate::id::{AssetId, AssetKind};
use crate::pipeline::{CookedAsset, ErasedLoader};
use crate::store::{AssetFailure, AssetStore, LoadRequest, Readiness};
use crate::version::{AssetSet, AssetVersion, VersionTable};

/// An [`AssetStore`] for tests: cooked assets are handed to it in memory, and every
/// requested load runs synchronously inside [`Harness::settle`].
///
/// It is the executable statement of the store contract, and a stand-in for tests of crates
/// that need a store. It is not a backend: it performs no IO, never unloads anything,
/// ignores budgets and serialises every call behind one lock.
pub struct ReferenceStore {
    state: Mutex<State>,
}

type Outcome = Result<UntypedHandle, AssetFailure>;

struct State {
    loaders: BTreeMap<AssetKind, ErasedLoader>,
    source: BTreeMap<(AssetId, AssetVersion), CookedAsset>,
    requests: Vec<LoadRequest>,
    outcomes: BTreeMap<(AssetId, AssetVersion), Outcome>,
    // Requests for the newest version of an asset the source holds no version of.
    unknown: BTreeMap<AssetId, AssetFailure>,
}

impl ReferenceStore {
    /// A store that loads with `loaders`, one per asset kind.
    ///
    /// # Panics
    ///
    /// Panics if two loaders produce the same kind.
    #[must_use]
    pub fn new(loaders: Vec<ErasedLoader>) -> Self {
        let mut by_kind = BTreeMap::new();
        for loader in loaders {
            let kind = loader.kind();
            assert!(by_kind.insert(kind, loader).is_none(), "two loaders for {kind:?}");
        }
        Self {
            state: Mutex::new(State {
                loaders: by_kind,
                source: BTreeMap::new(),
                requests: Vec::new(),
                outcomes: BTreeMap::new(),
                unknown: BTreeMap::new(),
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("a loader panicked while the reference store was loading")
    }
}

impl Harness for ReferenceStore {
    fn store(&self) -> &dyn AssetStore {
        self
    }

    fn provide(&mut self, id: AssetId, version: AssetVersion, cooked: CookedAsset) {
        self.state().source.insert((id, version), cooked);
    }

    fn settle(&mut self) {
        let mut state = self.state();
        let mut requests = core::mem::take(&mut state.requests);
        // Stable, so equal priorities keep request order.
        requests.sort_by_key(|request| core::cmp::Reverse(request.priority));
        for request in requests {
            let _ = state.load(request.id, request.version, &mut Vec::new());
        }
    }
}

impl State {
    fn versions(&self, id: AssetId) -> impl DoubleEndedIterator<Item = (AssetVersion, &Outcome)> {
        self.outcomes
            .range((id, AssetVersion(0))..=(id, AssetVersion(u32::MAX)))
            .map(|(&(_, version), outcome)| (version, outcome))
    }

    fn newest_published(&self, id: AssetId) -> Option<&UntypedHandle> {
        self.versions(id).rev().find_map(|(_, outcome)| outcome.as_ref().ok())
    }

    /// The failure to report for an asset with no published version, if a load failed.
    fn failure(&self, id: AssetId) -> Option<AssetFailure> {
        let newest_failed = self.versions(id).rev().find_map(|(_, outcome)| outcome.as_ref().err());
        self.unknown.get(&id).or(newest_failed).copied()
    }

    fn newest(&self, id: AssetId) -> Readiness<UntypedHandle> {
        match (self.newest_published(id), self.failure(id)) {
            (Some(handle), _) => Readiness::Ready(handle.clone()),
            (None, Some(failure)) => Readiness::Failed(failure),
            (None, None) => Readiness::Pending,
        }
    }

    /// Loads and publishes `version` of `id`, or the newest version the source holds.
    /// `loading` is the chain of dependants, which detects a dependency cycle.
    fn load(
        &mut self,
        id: AssetId,
        version: Option<AssetVersion>,
        loading: &mut Vec<AssetId>,
    ) -> Result<(), ErrorKind> {
        let newest_in_source = || {
            let all = (id, AssetVersion(0))..=(id, AssetVersion(u32::MAX));
            self.source.range(all).next_back().map(|(&(_, version), _)| version)
        };
        let Some(version) = version.or_else(newest_in_source) else {
            let kind = ErrorKind::NotFound;
            self.unknown.insert(id, AssetFailure { id, version: None, kind });
            return Err(kind);
        };
        if matches!(self.outcomes.get(&(id, version)), Some(Ok(_))) {
            return Ok(());
        }
        let outcome = self.load_version(id, version, loading);
        let result = outcome.as_ref().map(|_| ()).map_err(|failure| failure.kind);
        self.outcomes.insert((id, version), outcome);
        self.unknown.remove(&id);
        result
    }

    fn load_version(
        &mut self,
        id: AssetId,
        version: AssetVersion,
        loading: &mut Vec<AssetId>,
    ) -> Outcome {
        let fail = |kind| AssetFailure { id, version: Some(version), kind };
        let Some(cooked) = self.source.get(&(id, version)).cloned() else {
            return Err(fail(ErrorKind::NotFound));
        };
        if loading.contains(&id) {
            return Err(fail(ErrorKind::InvalidData));
        }
        loading.push(id);
        for &dependency in cooked.header.dependencies() {
            if self.newest_published(dependency).is_none() {
                self.load(dependency, None, loading).map_err(fail)?;
            }
        }
        loading.pop();
        let Some(loader) = self.loaders.get(&cooked.header.kind()) else {
            return Err(fail(ErrorKind::Unsupported));
        };
        loader.load(id, version, &cooked.header, &cooked.bytes).map_err(|error| fail(error.kind()))
    }
}

impl AssetStore for ReferenceStore {
    fn request(&self, request: LoadRequest) {
        self.state().requests.push(request);
    }

    fn get(&self, id: AssetId, version: AssetVersion) -> Readiness<UntypedHandle> {
        match self.state().outcomes.get(&(id, version)) {
            Some(Ok(handle)) => Readiness::Ready(handle.clone()),
            Some(Err(failure)) => Readiness::Failed(*failure),
            None => Readiness::Pending,
        }
    }

    fn newest(&self, id: AssetId) -> Readiness<UntypedHandle> {
        self.state().newest(id)
    }

    fn resolve(&self, set: &AssetSet) -> Readiness<VersionTable> {
        let state = self.state();
        let mut table = VersionTable::new();
        let mut pending = false;
        for id in set.iter() {
            match state.newest(id) {
                Readiness::Ready(handle) => {
                    table.set(id, handle.version());
                }
                Readiness::Pending => pending = true,
                Readiness::Failed(failure) => return Readiness::Failed(failure),
            }
        }
        if pending { Readiness::Pending } else { Readiness::Ready(table) }
    }
}
