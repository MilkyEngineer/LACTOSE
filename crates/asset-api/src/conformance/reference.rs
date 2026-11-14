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
/// It is the smallest store that passes the conformance suite, and a stand-in for tests of
/// crates that need a store. It is not a backend: it performs no IO, never unloads
/// anything, ignores budgets and serialises every call behind one lock. Where the traits
/// leave a choice to the store it makes the smallest one: it loads nothing by itself when
/// its source changes, and it stops at an asset's first failing dependency without looking
/// at the ones after it.
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
    /// What `newest` answers for an asset without a current version: the failure of the
    /// last load to end of a version the index named when it ended, or of the last request
    /// for its newest version that found the index naming none. A change of the index
    /// leaves it alone: only the end of a later load replaces it.
    newest_failure: BTreeMap<AssetId, AssetFailure>,
    generation: u64,
}

/// One load in the middle of its dependencies, on the explicit stack of [`State::load`].
struct Walk {
    id: AssetId,
    version: AssetVersion,
    cooked: CookedAsset,
    /// The next declared dependency to look at.
    next: usize,
}

/// The loads a [`State::load`] is in the middle of: a stack, since a dependency is loaded
/// before the next is looked at, and the same loads as a set, to notice a cycle.
#[derive(Default)]
struct Walks {
    stack: Vec<Walk>,
    members: BTreeSet<VersionKey>,
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
                newest_failure: BTreeMap::new(),
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
            let _ = state.load(request.id, request.version);
        }
    }
}

/// The failure every lookup of [`AssetId::NONE`] answers with.
fn no_asset() -> AssetFailure {
    AssetFailure { id: AssetId::NONE, version: None, kind: ErrorKind::NotFound }
}

impl State {
    fn published(&self, id: AssetId, version: AssetVersion) -> Option<&UntypedHandle> {
        self.outcomes.get(&(id, version.to_bits()))?.as_ref().ok()
    }

    fn newest(&self, id: AssetId) -> Readiness<UntypedHandle> {
        if id.is_none() {
            return Readiness::Failed(no_asset());
        }
        if let Some(handle) = self.current.get(&id).and_then(|&version| self.published(id, version))
        {
            return Readiness::Ready(handle.clone());
        }
        match self.newest_failure.get(&id) {
            Some(failure) => Readiness::Failed(*failure),
            None => Readiness::Pending,
        }
    }

    /// Loads and publishes `version` of `id`, or the version the index names for a request
    /// that names none, with the dependencies it needs first.
    ///
    /// The walk is depth-first over declared dependencies in ascending order and stops at a
    /// load's first failing dependency, on an explicit stack: a dependency chain is as long
    /// as a cook makes it, and a thread's stack is not.
    fn load(&mut self, id: AssetId, version: Option<AssetVersion>) -> Result<(), ErrorKind> {
        let mut walks = Walks::default();
        // The result of the load that ended last: the dependency the load on top of the
        // stack was waiting for, or at the end the request's own.
        let mut ended = self.begin(id, version, &mut walks);
        while let Some(walk) = walks.stack.last_mut() {
            if let Some(Err(kind)) = ended.take() {
                // Its first failing dependency fails the dependant, with the dependency's
                // kind and before anything of the dependant's own is looked at.
                let walk = walks.pop();
                let failure = AssetFailure { id: walk.id, version: Some(walk.version), kind };
                ended = Some(self.end(walk.id, walk.version, Err(failure)));
                continue;
            }
            let dependencies = walk.cooked.header.dependencies();
            let waiting = dependencies[walk.next..]
                .iter()
                .position(|dependency| !self.current.contains_key(dependency));
            match waiting {
                Some(skipped) => {
                    let dependency = dependencies[walk.next + skipped];
                    walk.next += skipped + 1;
                    ended = self.begin(dependency, None, &mut walks);
                }
                None => {
                    let walk = walks.pop();
                    ended = Some(self.finish(walk));
                }
            }
        }
        ended.expect("the request's own load ended")
    }

    /// Starts the load of `version` of `id`, or of the version the index names. `None`
    /// means the load is now on the stack; otherwise it has ended and this is its result.
    fn begin(
        &mut self,
        id: AssetId,
        version: Option<AssetVersion>,
        walks: &mut Walks,
    ) -> Option<Result<(), ErrorKind>> {
        let newest = version.is_none();
        let Some(version) = version.or_else(|| self.index.get(&id).copied()) else {
            // A request for the newest version of an asset the index does not name is a
            // failed load, recorded every time it is made.
            let kind = ErrorKind::NotFound;
            self.newest_failure.insert(id, AssetFailure { id, version: None, kind });
            self.generation += 1;
            return Some(Err(kind));
        };
        let key = (id, version.to_bits());
        if self.published(id, version).is_some() {
            // Only a request for the newest version makes a published one current: the
            // index may have come back to a version that was published earlier.
            if newest && self.current.insert(id, version) != Some(version) {
                self.newest_failure.remove(&id);
                self.generation += 1;
            }
            return Some(Ok(()));
        }
        if walks.members.contains(&key) {
            // The walk came back to a load it is in the middle of: a dependency cycle.
            // That load records its own end when the failure reaches it.
            return Some(Err(ErrorKind::InvalidData));
        }
        let Some(cooked) = self.source.get(&key).cloned() else {
            let failure = AssetFailure { id, version: Some(version), kind: ErrorKind::NotFound };
            return Some(self.end(id, version, Err(failure)));
        };
        walks.members.insert(key);
        walks.stack.push(Walk { id, version, cooked, next: 0 });
        None
    }

    /// Runs the loader of a load whose dependencies all have a current version.
    fn finish(&mut self, walk: Walk) -> Result<(), ErrorKind> {
        let Walk { id, version, cooked, .. } = walk;
        let fail = |kind| AssetFailure { id, version: Some(version), kind };
        let outcome = match self.loaders.get(&cooked.header.kind()) {
            None => Err(fail(ErrorKind::Unsupported)),
            Some(loader) => loader
                .load(id, version, &cooked.header, &cooked.bytes)
                .map_err(|error| fail(error.kind())),
        };
        if outcome.is_ok() {
            let dependencies = cooked.header.dependencies().to_vec();
            self.dependencies.insert((id, version.to_bits()), dependencies);
        }
        self.end(id, version, outcome)
    }

    /// Records the end of a load, in one step: the outcome for `get`; for `newest`, if the
    /// index names the version now, the new current version or the failure; and the
    /// generation.
    fn end(
        &mut self,
        id: AssetId,
        version: AssetVersion,
        outcome: Outcome,
    ) -> Result<(), ErrorKind> {
        let result = outcome.as_ref().map(drop).map_err(|failure| failure.kind);
        if self.index.get(&id) == Some(&version) {
            match &outcome {
                Ok(_) => {
                    self.current.insert(id, version);
                    self.newest_failure.remove(&id);
                }
                Err(failure) => {
                    self.newest_failure.insert(id, *failure);
                }
            }
        }
        self.outcomes.insert((id, version.to_bits()), outcome);
        self.generation += 1;
        result
    }
}

impl Walks {
    /// Takes the load on top of the stack, which has ended.
    fn pop(&mut self) -> Walk {
        let walk = self.stack.pop().expect("a load on the stack");
        self.members.remove(&(walk.id, walk.version.to_bits()));
        walk
    }
}

impl PublishedAssets for ReferenceStore {
    fn request(&self, id: AssetId, version: AssetVersion, priority: LoadPriority) {
        if !id.is_none() {
            self.state().requests.push(Request { id, version: Some(version), priority });
        }
    }

    fn get(&self, id: AssetId, version: AssetVersion) -> Readiness<UntypedHandle> {
        if id.is_none() {
            return Readiness::Failed(no_asset());
        }
        match self.state().outcomes.get(&(id, version.to_bits())) {
            Some(Ok(handle)) => Readiness::Ready(handle.clone()),
            Some(Err(failure)) => Readiness::Failed(*failure),
            None => Readiness::Pending,
        }
    }
}

impl AssetCatalog for ReferenceStore {
    fn request_newest(&self, id: AssetId, priority: LoadPriority) {
        if !id.is_none() {
            self.state().requests.push(Request { id, version: None, priority });
        }
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
