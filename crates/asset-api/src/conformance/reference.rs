//! The in-memory store the conformance suite is tested against.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard};

use lactose_core::ErrorKind;

use super::Harness;
use crate::budget::LoadPriority;
use crate::handle::UntypedHandle;
use crate::id::{AssetId, AssetKind};
use crate::pipeline::{CookedAsset, ErasedLoader};
use crate::store::{AssetCatalog, AssetFailure, PublishedAssets, Readiness};
use crate::version::{AssetSet, AssetVersion, VersionTable};

/// A store for tests: cooked assets are handed to it in memory, and every requested load
/// runs synchronously inside [`Harness::settle`].
///
/// It is the executable statement of the store contract, and a stand-in for tests of crates
/// that need a store. It is not a backend: it performs no IO, never unloads anything,
/// ignores budgets and serialises every call behind one lock.
pub struct ReferenceStore {
    state: Mutex<State>,
}

type Outcome = Result<UntypedHandle, AssetFailure>;
/// Versions have no order of their own; maps are keyed by their bits.
type VersionKey = (AssetId, u64);

struct Request {
    id: AssetId,
    /// `None` asks for the version the index names when the request is served.
    version: Option<AssetVersion>,
    priority: LoadPriority,
}

struct State {
    loaders: BTreeMap<AssetKind, ErasedLoader>,
    /// Every cooked version handed over, and the one the pack index names per asset.
    source: BTreeMap<VersionKey, CookedAsset>,
    index: BTreeMap<AssetId, AssetVersion>,
    requests: Vec<Request>,
    outcomes: BTreeMap<VersionKey, Outcome>,
    /// The declared dependencies of every published version.
    dependencies: BTreeMap<VersionKey, Vec<AssetId>>,
    /// The current published version per asset.
    current: BTreeMap<AssetId, AssetVersion>,
    /// Requests for the newest version of an asset the index does not name.
    unknown: BTreeMap<AssetId, AssetFailure>,
    generation: u64,
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
                index: BTreeMap::new(),
                requests: Vec::new(),
                outcomes: BTreeMap::new(),
                dependencies: BTreeMap::new(),
                current: BTreeMap::new(),
                unknown: BTreeMap::new(),
                generation: 0,
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("a loader panicked while the reference store was loading")
    }
}

impl Harness for ReferenceStore {
    fn catalog(&self) -> &dyn AssetCatalog {
        self
    }

    fn provide(&self, id: AssetId, version: AssetVersion, cooked: CookedAsset) {
        let mut state = self.state();
        state.source.insert((id, version.to_bits()), cooked);
        state.index.insert(id, version);
    }

    fn settle(&self) {
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
    fn published(&self, id: AssetId, version: AssetVersion) -> Option<&UntypedHandle> {
        self.outcomes.get(&(id, version.to_bits()))?.as_ref().ok()
    }

    fn newest(&self, id: AssetId) -> Readiness<UntypedHandle> {
        if let Some(handle) = self.current.get(&id).and_then(|&version| self.published(id, version))
        {
            return Readiness::Ready(handle.clone());
        }
        let named =
            self.index.get(&id).and_then(|version| self.outcomes.get(&(id, version.to_bits())));
        match (named, self.unknown.get(&id)) {
            (Some(Err(failure)), _) | (None, Some(failure)) => Readiness::Failed(*failure),
            _ => Readiness::Pending,
        }
    }

    /// Loads and publishes `version` of `id`, or the version the index names.
    /// `loading` is the chain of dependants, which detects a dependency cycle.
    fn load(
        &mut self,
        id: AssetId,
        version: Option<AssetVersion>,
        loading: &mut Vec<AssetId>,
    ) -> Result<(), ErrorKind> {
        let Some(version) = version.or_else(|| self.index.get(&id).copied()) else {
            let kind = ErrorKind::NotFound;
            let failure = AssetFailure { id, version: None, kind };
            if self.unknown.insert(id, failure).is_none() {
                self.generation += 1;
            }
            return Err(kind);
        };
        self.unknown.remove(&id);
        let named = self.index.get(&id) == Some(&version);
        if self.published(id, version).is_some() {
            // The index may have come back to a version that was published earlier.
            if named && self.current.insert(id, version) != Some(version) {
                self.generation += 1;
            }
            return Ok(());
        }
        let outcome = self.load_version(id, version, loading);
        let result = outcome.as_ref().map(drop).map_err(|failure| failure.kind);
        if result.is_ok() && named {
            self.current.insert(id, version);
        }
        self.outcomes.insert((id, version.to_bits()), outcome);
        self.generation += 1;
        result
    }

    fn load_version(
        &mut self,
        id: AssetId,
        version: AssetVersion,
        loading: &mut Vec<AssetId>,
    ) -> Outcome {
        let fail = |kind| AssetFailure { id, version: Some(version), kind };
        let Some(cooked) = self.source.get(&(id, version.to_bits())).cloned() else {
            return Err(fail(ErrorKind::NotFound));
        };
        if loading.contains(&id) {
            return Err(fail(ErrorKind::InvalidData));
        }
        loading.push(id);
        for &dependency in cooked.header.dependencies() {
            if !self.current.contains_key(&dependency) {
                self.load(dependency, None, loading).map_err(fail)?;
            }
        }
        loading.pop();
        let Some(loader) = self.loaders.get(&cooked.header.kind()) else {
            return Err(fail(ErrorKind::Unsupported));
        };
        let handle = loader
            .load(id, version, &cooked.header, &cooked.bytes)
            .map_err(|error| fail(error.kind()))?;
        self.dependencies.insert((id, version.to_bits()), cooked.header.dependencies().to_vec());
        Ok(handle)
    }
}

impl PublishedAssets for ReferenceStore {
    fn request(&self, id: AssetId, version: AssetVersion, priority: LoadPriority) {
        self.state().requests.push(Request { id, version: Some(version), priority });
    }

    fn get(&self, id: AssetId, version: AssetVersion) -> Readiness<UntypedHandle> {
        match self.state().outcomes.get(&(id, version.to_bits())) {
            Some(Ok(handle)) => Readiness::Ready(handle.clone()),
            Some(Err(failure)) => Readiness::Failed(*failure),
            None => Readiness::Pending,
        }
    }
}

impl AssetCatalog for ReferenceStore {
    fn request_newest(&self, id: AssetId, priority: LoadPriority) {
        self.state().requests.push(Request { id, version: None, priority });
    }

    fn newest(&self, id: AssetId) -> Readiness<UntypedHandle> {
        self.state().newest(id)
    }

    fn resolve(&self, set: &AssetSet) -> Readiness<VersionTable> {
        let state = self.state();
        let mut pending = false;
        for id in set.iter() {
            match state.newest(id) {
                Readiness::Ready(_) => {}
                Readiness::Pending => pending = true,
                Readiness::Failed(failure) => return Readiness::Failed(failure),
            }
        }
        if pending {
            return Readiness::Pending;
        }
        // Every member has a current version, and a version is published only once its
        // dependencies have one, so the walk below finds a version for everything.
        let mut table = VersionTable::new();
        let mut reached: BTreeSet<AssetId> = set.iter().collect();
        let mut queue: Vec<AssetId> = set.iter().collect();
        while let Some(id) = queue.pop() {
            let version = state.current[&id];
            table.set(id, version);
            for &dependency in &state.dependencies[&(id, version.to_bits())] {
                if reached.insert(dependency) {
                    queue.push(dependency);
                }
            }
        }
        Readiness::Ready(table)
    }

    fn generation(&self) -> u64 {
        self.state().generation
    }
}
