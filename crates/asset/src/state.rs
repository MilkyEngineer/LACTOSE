//! What the store decides under its lock: the mounted packs, the loads in flight, the
//! published versions and failures, the generation, and every rule of `README.md`,
//! "Behaviour the signatures cannot express", that says when a load is accepted and how it
//! ends.
//!
//! Nothing here reads, spawns, wakes, waits, drops what a step lets go of, or tells a
//! `tracing` subscriber anything: a [`Step`] collects each of those as [`Effects`], which
//! the store carries out once the lock is released. A lock holder runs the store's own map
//! work and nothing else.
//!
//! Every walk over dependencies (accepting a load's dependencies, a mount's rule 3, a
//! cycle's search and decision, `resolve`) is a loop over a work list, never a recursion
//! per link, so a chain is as long as a cook makes it.

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;
use std::task::Waker;

use lactose_asset_api::{
    AssetFailure, AssetId, AssetKind, AssetSet, AssetVersion, ErasedLoader, LoadPriority,
    Readiness, UntypedHandle, VersionTable,
};
use lactose_core::{Error, ErrorKind};
use lactose_io::File;
use lactose_task::Task;

use crate::pack::Entry;
use crate::trace::{LoadTrace, Trace};

/// The loaders, one per kind, fixed for the store's life.
pub(crate) type Loaders = BTreeMap<AssetKind, ErasedLoader>;

/// A load, a published version or a recorded failure: one `(AssetId, AssetVersion)`.
pub(crate) type Key = (AssetId, u64);

pub(crate) fn key(id: AssetId, version: AssetVersion) -> Key {
    (id, version.to_bits())
}

/// An ID, a version or a kind as `AssetFailure`'s `Display` writes them.
pub(crate) struct Hex(pub(crate) u64);

impl fmt::Display for Hex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#018x}", self.0)
    }
}

/// A mounted pack: its open file and its decoded index, sorted by ID.
pub(crate) struct Pack {
    pub(crate) file: File,
    pub(crate) entries: Vec<Entry>,
}

impl Pack {
    fn find(&self, id: AssetId) -> Option<usize> {
        self.entries.binary_search_by_key(&id, |entry| entry.id).ok()
    }
}

/// One entry of one mounted pack.
#[derive(Clone)]
pub(crate) struct Source {
    pub(crate) pack: Arc<Pack>,
    index: usize,
}

impl Source {
    pub(crate) fn entry(&self) -> &Entry {
        &self.pack.entries[self.index]
    }
}

/// Why a version failed, for the rules that decide what a mount retries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Cause {
    /// No mounted pack held the version: neither of the version nor of the source.
    Absent,
    /// The same content would fail the same way from any pack.
    Version,
    /// Another pack that holds the version might load it.
    Source,
    /// A declared dependency failed: this one, the lowest that did.
    Dependency(AssetId),
}

enum Outcome {
    Published { handle: UntypedHandle, source: Source },
    Failed { failure: AssetFailure, cause: Cause },
}

#[derive(Default)]
struct AssetState {
    followed: bool,
    current: Option<AssetVersion>,
    newest_failure: Option<AssetFailure>,
}

/// Steps 2 to 4 of a load, as far as they have gone.
pub(crate) enum Own {
    Running,
    Done(std::result::Result<UntypedHandle, (Error, Cause)>),
}

/// A load in flight that another was found to wait for, directly or through others.
#[derive(Clone, Copy)]
struct Behind {
    target: Key,
    /// The target's `seq` and the mount count when it was found: the finding holds while
    /// that load's steps 3 and 4 still run and no mount has renamed anything since.
    seq: u64,
    mounts: u64,
}

struct Load {
    id: AssetId,
    version: AssetVersion,
    source: Source,
    own: Own,
    /// The order of acceptance, for [`Settled`](crate::Settled).
    seq: u64,
    /// Which run of steps 3 and 4 is the load's: a load that reads again from a newer
    /// source (rule 2) runs them again, and the earlier run's task and result are stale.
    run: u64,
    task: Option<Task<()>>,
    trace: Arc<LoadTrace>,
    /// Lets a cycle search that reaches this load jump to a load that still runs instead
    /// of walking the same path again, while the finding holds.
    behind: Option<Behind>,
}

impl Load {
    fn dependencies(&self) -> &[AssetId] {
        self.source.entry().header.dependencies()
    }
}

/// What a declared dependency is to a load that waits for it.
enum DepState {
    Current,
    Failed(ErrorKind),
    /// No current version; the load of the named version is in flight.
    Waiting(Key),
}

enum Ending {
    /// Steps 3 and 4 decide: published, or failed for itself.
    Own,
    /// Failed for `dependency`, with `kind`.
    Dependency { dependency: AssetId, kind: ErrorKind, message: Cow<'static, str> },
}

/// A run of steps 3 and 4 to spawn once the lock is released.
pub(crate) struct Spawn {
    pub(crate) id: AssetId,
    pub(crate) version: AssetVersion,
    pub(crate) run: u64,
    pub(crate) source: Source,
    pub(crate) trace: Arc<LoadTrace>,
}

/// What a step leaves for after the lock, carried out in this order: what to tell the
/// subscriber, waits to wake, values to drop, task handles of ended loads to let go, and
/// loads to spawn.
#[derive(Default)]
pub(crate) struct Effects {
    pub(crate) traces: Vec<Trace>,
    pub(crate) wakers: Vec<Waker>,
    /// What the step let go of whose drop may run code that is not the store's: an
    /// asset, an error's source, a span, a pack's file.
    pub(crate) dropped: Vec<Box<dyn Send>>,
    pub(crate) tasks: Vec<Task<()>>,
    pub(crate) spawns: Vec<Spawn>,
}

/// A spawned run's task that its load does not keep, for the spawner to let go of after
/// the lock.
pub(crate) enum Leftover {
    /// The load ended or reads again: the run finishes by itself.
    Detach(Task<()>),
    /// The store is closed: dropping the task cancels the run.
    Cancel(Task<()>),
}

#[derive(Default)]
pub(crate) struct State {
    /// One per call of `Store::mount`, in call order: the pack once its mount resolved
    /// `Ok`, `None` before that and for good after a failure or a drop.
    places: Vec<Option<Arc<Pack>>>,
    /// For each asset a mounted pack holds, where its named entry is: the latest place
    /// that holds it, and the entry's index there.
    named: BTreeMap<AssetId, (usize, usize)>,
    /// Mounts resolved so far; a mount is the only step that renames.
    mounts: u64,
    versions: BTreeMap<Key, Outcome>,
    assets: BTreeMap<AssetId, AssetState>,
    loads: BTreeMap<Key, Load>,
    /// For each asset, the loads in flight that declare it.
    waiters: BTreeMap<AssetId, BTreeSet<Key>>,
    /// For each asset, the versions whose last load failed for it (rule 3). Entries go
    /// stale when a version is retried and are checked against `versions` when read.
    failed_for: BTreeMap<AssetId, BTreeSet<Key>>,
    generation: u64,
    next_seq: u64,
    next_run: u64,
    /// The `seq` of every load in flight.
    in_flight: BTreeSet<u64>,
    /// Each `Settled` waiting: its mark (the loads accepted before its first poll) and its
    /// waker, by a number of its own.
    settling: BTreeMap<u64, (u64, Waker)>,
    next_settling: u64,
    /// Set once the last handle is gone: nothing is published, and no loader is called
    /// by a load that has not passed its check, from then on.
    closed: bool,
}

impl State {
    /// Takes the next place in the mount order.
    pub(crate) fn take_place(&mut self) -> usize {
        self.places.push(None);
        self.places.len() - 1
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn closed(&self) -> bool {
        self.closed
    }

    /// The last handle is gone: marks the store closed and hands back every load's task,
    /// to be dropped, which cancels it, once the lock is released.
    pub(crate) fn close(&mut self) -> Vec<Task<()>> {
        self.closed = true;
        self.loads.values_mut().filter_map(|load| load.task.take()).collect()
    }

    fn pack_at(&self, place: usize) -> &Arc<Pack> {
        self.places[place].as_ref().expect("a place in the index is mounted")
    }

    /// The asset's named entry: in the mounted pack with the latest place that holds it.
    fn named(&self, id: AssetId) -> Option<Source> {
        let &(place, index) = self.named.get(&id)?;
        Some(Source { pack: Arc::clone(self.pack_at(place)), index })
    }

    fn named_version(&self, id: AssetId) -> Option<AssetVersion> {
        let &(place, index) = self.named.get(&id)?;
        Some(self.pack_at(place).entries[index].version)
    }

    /// The mounted pack with the latest place that holds exactly `version` of `id`: one
    /// search of each mounted pack's index, for an exact request and for a load that
    /// failed for its source.
    fn source(&self, id: AssetId, version: AssetVersion) -> Option<Source> {
        self.places.iter().rev().flatten().find_map(|pack| {
            let index = pack.find(id).filter(|&index| pack.entries[index].version == version)?;
            Some(Source { pack: Arc::clone(pack), index })
        })
    }

    fn published(&self, key: Key) -> bool {
        matches!(self.versions.get(&key), Some(Outcome::Published { .. }))
    }

    fn handle(&self, key: Key) -> Option<&UntypedHandle> {
        match self.versions.get(&key) {
            Some(Outcome::Published { handle, .. }) => Some(handle),
            _ => None,
        }
    }

    pub(crate) fn get(&self, id: AssetId, version: AssetVersion) -> Readiness<UntypedHandle> {
        match self.versions.get(&key(id, version)) {
            Some(Outcome::Published { handle, .. }) => Readiness::Ready(handle.clone()),
            Some(Outcome::Failed { failure, .. }) => Readiness::Failed(*failure),
            None => Readiness::Pending,
        }
    }

    pub(crate) fn newest(&self, id: AssetId) -> Readiness<UntypedHandle> {
        let Some(asset) = self.assets.get(&id) else { return Readiness::Pending };
        if let Some(current) = asset.current {
            let handle = self.handle(key(id, current)).expect("a current version is published");
            return Readiness::Ready(handle.clone());
        }
        match asset.newest_failure {
            Some(failure) => Readiness::Failed(failure),
            None => Readiness::Pending,
        }
    }

    pub(crate) fn resolve(&self, set: &AssetSet) -> Readiness<VersionTable> {
        let mut pending = false;
        for id in set.iter() {
            match self.newest(id) {
                Readiness::Ready(_) => {}
                Readiness::Pending => pending = true,
                Readiness::Failed(failure) => return Readiness::Failed(failure),
            }
        }
        if pending {
            return Readiness::Pending;
        }
        let mut table = VersionTable::new();
        let mut walk: Vec<AssetId> = set.iter().collect();
        while let Some(id) = walk.pop() {
            if table.get(id).is_some() {
                continue;
            }
            // A version is published only once each dependency has a current version, and
            // a current version is never withdrawn.
            let current = self.assets.get(&id).and_then(|asset| asset.current);
            let current = current.expect("a dependency of a published version is current");
            table.set(id, current);
            match self.versions.get(&key(id, current)) {
                Some(Outcome::Published { source, .. }) => {
                    walk.extend(source.entry().header.dependencies());
                }
                _ => unreachable!("a current version is published"),
            }
        }
        Readiness::Ready(table)
    }

    /// Whether every load accepted before `mark` has ended.
    fn settled(&self, mark: u64) -> bool {
        self.in_flight.first().is_none_or(|&seq| seq >= mark)
    }

    /// For a `Settled` at its poll, with a clone of its waker made before the lock: whether
    /// it is done, else the waker registered. Hands back the wakers it no longer keeps, to
    /// be dropped after the lock. `slot` and `mark` are taken at the first poll.
    pub(crate) fn poll_settled(
        &mut self,
        slot: &mut Option<(u64, u64)>,
        waker: Waker,
    ) -> (bool, [Option<Waker>; 2]) {
        let (number, mark) = *slot.get_or_insert_with(|| {
            self.next_settling += 1;
            (self.next_settling, self.next_seq)
        });
        if self.settled(mark) {
            let old = self.settling.remove(&number).map(|(_, old)| old);
            return (true, [old, Some(waker)]);
        }
        if let Some((_, old)) = self.settling.get(&number)
            && old.will_wake(&waker)
        {
            return (false, [Some(waker), None]);
        }
        let old = self.settling.insert(number, (mark, waker)).map(|(_, old)| old);
        (false, [old, None])
    }

    /// A `Settled` dropped before it resolved: its waker, to be dropped after the lock.
    pub(crate) fn forget_settled(&mut self, number: u64) -> Option<Waker> {
        self.settling.remove(&number).map(|(_, waker)| waker)
    }

    /// Hands a spawned run its load's task slot, unless the load is no longer the one
    /// spawned, has no more need of it, or the store is closed; then the task comes back.
    pub(crate) fn give_task(&mut self, key: Key, run: u64, task: Task<()>) -> Option<Leftover> {
        if self.closed {
            return Some(Leftover::Cancel(task));
        }
        match self.loads.get_mut(&key) {
            Some(load) if load.run == run && matches!(load.own, Own::Running) => {
                load.task.replace(task).map(Leftover::Detach)
            }
            _ => Some(Leftover::Detach(task)),
        }
    }
}

/// One critical section that may change something.
pub(crate) struct Step<'a> {
    pub(crate) state: &'a mut State,
    pub(crate) loaders: &'a Loaders,
    pub(crate) effects: Effects,
    /// Loads whose end may have become decidable.
    work: Vec<Key>,
    /// Dependencies to ask for as by `request_newest`, at their dependant's priority.
    asks: Vec<(AssetId, LoadPriority)>,
    /// Cycle searches this step made, and the loads they visited.
    searches: u64,
    visits: Cell<u64>,
}

impl<'a> Step<'a> {
    pub(crate) fn new(state: &'a mut State, loaders: &'a Loaders) -> Self {
        Self {
            state,
            loaders,
            effects: Effects::default(),
            work: Vec::new(),
            asks: Vec::new(),
            searches: 0,
            visits: Cell::new(0),
        }
    }

    /// The step's effects, `silo.search` among them if it searched.
    pub(crate) fn finish(mut self) -> Effects {
        if self.searches > 0 {
            let (searches, visits) = (self.searches, self.visits.get());
            self.effects.traces.push(Trace::Search { searches, visits });
        }
        self.effects
    }

    fn asset(&mut self, id: AssetId) -> &mut AssetState {
        self.state.assets.entry(id).or_default()
    }

    fn move_generation(&mut self) -> u64 {
        self.state.generation += 1;
        self.state.generation
    }

    /// Records `outcome` for `key`; an outcome it replaces is dropped after the lock.
    fn record(&mut self, key: Key, outcome: Outcome) {
        if let Some(old) = self.state.versions.insert(key, outcome) {
            self.effects.dropped.push(Box::new(old));
        }
    }

    /// Queues the loads that wait for `id` for another look.
    fn touch(&mut self, id: AssetId) {
        if let Some(waiting) = self.state.waiters.get(&id) {
            self.work.extend(waiting.iter().copied());
        }
    }

    /// `PublishedAssets::request`.
    pub(crate) fn exact(&mut self, id: AssetId, version: AssetVersion, priority: LoadPriority) {
        let key = key(id, version);
        if self.state.published(key) || self.state.loads.contains_key(&key) {
            return;
        }
        match self.state.source(id, version) {
            Some(source) => {
                self.accept(source, priority, "exact");
                self.ask_dependencies();
            }
            None => {
                let load = LoadTrace::new(id, Some(version), None, priority, "exact");
                let failure =
                    AssetFailure { id, version: Some(version), kind: ErrorKind::NotFound };
                self.forget_failed_for(key);
                self.record(key, Outcome::Failed { failure, cause: Cause::Absent });
                self.move_generation();
                let message = "no mounted pack holds the version".into();
                self.effects.traces.push(Trace::Failure {
                    load,
                    failure,
                    dependency: None,
                    message,
                });
            }
        }
    }

    /// `AssetCatalog::request_newest`, and every request that is "as by" it.
    pub(crate) fn newest(&mut self, id: AssetId, priority: LoadPriority, cause: &'static str) {
        self.newest_once(id, priority, cause);
        self.ask_dependencies();
    }

    fn ask_dependencies(&mut self) {
        while let Some((id, priority)) = self.asks.pop() {
            self.newest_once(id, priority, "dependency");
        }
    }

    fn newest_once(&mut self, id: AssetId, priority: LoadPriority, cause: &'static str) {
        self.asset(id).followed = true;
        let Some(source) = self.state.named(id) else {
            let load = LoadTrace::new(id, None, None, priority, cause);
            let failure = AssetFailure { id, version: None, kind: ErrorKind::NotFound };
            self.asset(id).newest_failure = Some(failure);
            self.move_generation();
            let message = "no mounted pack holds the asset".into();
            self.effects.traces.push(Trace::Failure { load, failure, dependency: None, message });
            self.touch(id);
            return;
        };
        let version = source.entry().version;
        let key = key(id, version);
        if self.state.published(key) {
            if self.asset(id).current != Some(version) {
                self.asset(id).current = Some(version);
                let generation = self.move_generation();
                self.effects.traces.push(Trace::Current { id, version, generation });
                self.touch(id);
            }
        } else if !self.state.loads.contains_key(&key) {
            self.accept(source, priority, cause);
        }
    }

    /// Accepts a load of `source`'s entry: steps 1 and 2 in this call, steps 3 and 4 on a
    /// task spawned after it.
    fn accept(&mut self, source: Source, priority: LoadPriority, cause: &'static str) {
        let entry = source.entry();
        let (id, version, kind) = (entry.id, entry.version, entry.header.kind());
        let key = key(id, version);
        let trace = LoadTrace::new(id, Some(version), Some(kind), priority, cause);
        self.effects.traces.push(Trace::Open(Arc::clone(&trace)));
        let seq = self.state.next_seq;
        self.state.next_seq += 1;
        self.state.in_flight.insert(seq);
        let run = self.state.next_run;
        self.state.next_run += 1;
        // A retried failure stays the answer until the retry ends; only rule 3's index
        // forgets it now.
        self.forget_failed_for(key);
        for &dependency in entry.header.dependencies() {
            self.state.waiters.entry(dependency).or_default().insert(key);
            let asset = self.state.assets.get(&dependency);
            if asset.is_none_or(|asset| !asset.followed || asset.current.is_none()) {
                self.asks.push((dependency, priority));
            }
        }
        let own = if self.loaders.contains_key(&kind) {
            let trace = Arc::clone(&trace);
            self.effects.spawns.push(Spawn { id, version, run, source: source.clone(), trace });
            Own::Running
        } else {
            self.work.push(key);
            let message = format!("no loader is registered for asset kind {}", Hex(kind.0));
            Own::Done(Err((Error::new(ErrorKind::Unsupported, message), Cause::Version)))
        };
        let load = Load { id, version, source, own, seq, run, task: None, trace, behind: None };
        self.state.loads.insert(key, load);
    }

    fn forget_failed_for(&mut self, key: Key) {
        if let Some(Outcome::Failed { cause: Cause::Dependency(dependency), .. }) =
            self.state.versions.get(&key)
            && let Some(dependants) = self.state.failed_for.get_mut(dependency)
        {
            dependants.remove(&key);
        }
    }

    /// Run `run` of steps 3 and 4 of the load `key` ended with `result`. A stale run's
    /// result, or any once the store is closed, is dropped after the lock unrecorded.
    pub(crate) fn own_done(
        &mut self,
        key: Key,
        run: u64,
        result: std::result::Result<UntypedHandle, (Error, Cause)>,
    ) {
        if !self.state.closed
            && let Some(load) = self.state.loads.get_mut(&key)
            && load.run == run
            && matches!(load.own, Own::Running)
        {
            load.own = Own::Done(result);
            self.work.push(key);
            return;
        }
        self.effects.dropped.push(Box::new(result));
    }

    /// The pack at `place` is mounted: one step that names its versions and asks again
    /// for what the three rules of `README.md`, "Mounting", say. Returns the number of
    /// loads accepted.
    pub(crate) fn mounted(&mut self, place: usize, pack: &Arc<Pack>) -> u64 {
        let before = self.state.next_seq;
        self.state.places[place] = Some(Arc::clone(pack));
        self.state.mounts += 1;

        // Rule 3's frontier: what the mount asks for again, and the renamed versions it
        // does not ask for because they are loading already. Only `asked` is asked.
        let mut frontier = Vec::new();
        let mut asked = Vec::new();
        let mut seen = BTreeSet::new();
        for (index, entry) in pack.entries.iter().enumerate() {
            let id = entry.id;
            let previous = self.state.named.get(&id).copied();
            if previous.is_some_and(|(at, _)| at > place) {
                // A pack with a later place names the asset: nothing is renamed.
                continue;
            }
            let earlier = previous.map(|(at, index)| self.state.pack_at(at).entries[index].version);
            self.state.named.insert(id, (place, index));
            let Some(asset) = self.state.assets.get(&id).filter(|asset| asset.followed) else {
                continue;
            };
            let key = key(id, entry.version);
            let renamed = earlier != Some(entry.version);
            let loading = self.state.loads.contains_key(&key);
            let ask = if loading {
                false
            } else if renamed {
                // Rule 1.
                asset.current != Some(entry.version)
            } else {
                // Rule 2: this pack is now where the version is read from.
                matches!(
                    self.state.versions.get(&key),
                    Some(Outcome::Failed { cause: Cause::Source, .. })
                )
            };
            if ask {
                asked.push(id);
            }
            if ask || (renamed && loading) {
                frontier.push(id);
                seen.insert(id);
            }
            if renamed {
                self.touch(id);
            }
        }
        // Rule 3, transitively, in the order the dependants are found.
        let mut next = 0;
        while let Some(&dependency) = frontier.get(next) {
            next += 1;
            let Some(dependants) = self.state.failed_for.get(&dependency) else { continue };
            for &key in dependants {
                let id = key.0;
                let failed_for_it = matches!(
                    self.state.versions.get(&key),
                    Some(Outcome::Failed { cause: Cause::Dependency(of), .. }) if *of == dependency
                );
                if failed_for_it
                    && !seen.contains(&id)
                    && self.state.assets.get(&id).is_some_and(|asset| asset.followed)
                    && self.state.named_version(id).is_some_and(|named| key.1 == named.to_bits())
                    && !self.state.loads.contains_key(&key)
                {
                    asked.push(id);
                    frontier.push(id);
                    seen.insert(id);
                }
            }
        }
        for id in asked {
            self.newest(id, LoadPriority::default(), "mount");
        }
        self.state.next_seq - before
    }

    /// Ends every load that can end, deciding cycles once nothing else can, until
    /// nothing changes.
    pub(crate) fn settle(&mut self) {
        let mut blocked = Vec::new();
        loop {
            while let Some(key) = self.work.pop() {
                match self.evaluate(key) {
                    None => {}
                    Some(None) => blocked.push(key),
                    Some(Some(ending)) => self.end(key, ending),
                }
            }
            let mut progress = false;
            while let Some(root) = blocked.pop() {
                if self.cycles_from(root) {
                    progress = true;
                    break;
                }
            }
            if !progress {
                break;
            }
        }
    }

    /// `None` if `key` is not in flight or its steps 3 and 4 run; `Some(None)` if it waits
    /// for a dependency; otherwise how it ends (step 5).
    #[expect(clippy::option_option, reason = "three answers, each used once")]
    fn evaluate(&self, key: Key) -> Option<Option<Ending>> {
        let load = self.state.loads.get(&key)?;
        if matches!(load.own, Own::Running) {
            return None;
        }
        let mut failed = None;
        for &dependency in load.dependencies() {
            match self.dep_state(dependency) {
                DepState::Current => {}
                DepState::Failed(kind) => {
                    failed.get_or_insert((dependency, kind));
                }
                DepState::Waiting(_) => return Some(None),
            }
        }
        Some(Some(match failed {
            Some((dependency, kind)) => Ending::Dependency {
                dependency,
                kind,
                message: format!("its dependency {} failed", Hex(dependency.to_bits())).into(),
            },
            None => Ending::Own,
        }))
    }

    /// Rule 2 for a load in flight: a load whose own steps failed for its source reads
    /// again, as the same load, from the pack with the latest place that holds its
    /// version, if a mount since its acceptance made that another pack. Returns whether it
    /// does.
    fn read_again(&mut self, key: Key, ending: &Ending) -> bool {
        let load = &self.state.loads[&key];
        if !matches!(ending, Ending::Own) || !matches!(load.own, Own::Done(Err((_, Cause::Source))))
        {
            return false;
        }
        let Some(source) = self.state.source(load.id, load.version) else { return false };
        if Arc::ptr_eq(&source.pack, &load.source.pack) {
            return false;
        }
        let run = self.state.next_run;
        self.state.next_run += 1;
        let load = self.state.loads.get_mut(&key).expect("a load reading again is in flight");
        let own = std::mem::replace(&mut load.own, Own::Running);
        let read = std::mem::replace(&mut load.source, source.clone());
        load.run = run;
        let (id, version, trace) = (load.id, load.version, Arc::clone(&load.trace));
        self.effects.tasks.extend(load.task.take());
        self.effects.dropped.push(Box::new(own));
        self.effects.dropped.push(Box::new(read));
        self.effects.spawns.push(Spawn { id, version, run, source, trace });
        true
    }

    fn dep_state(&self, dependency: AssetId) -> DepState {
        let asset = self.state.assets.get(&dependency);
        if asset.is_some_and(|asset| asset.current.is_some()) {
            return DepState::Current;
        }
        if let Some(named) = self.state.named_version(dependency) {
            let key = key(dependency, named);
            if self.state.loads.contains_key(&key) {
                return DepState::Waiting(key);
            }
        }
        // Every dependency was asked for when its dependant was accepted, and every mount
        // since asked again for those it renamed, so one that has neither a current
        // version nor a load of its named version has a newest failure.
        let failure = asset.and_then(|asset| asset.newest_failure);
        debug_assert!(failure.is_some(), "a settled dependency without a failure");
        DepState::Failed(failure.map_or(ErrorKind::NotFound, |failure| failure.kind))
    }

    /// The loads in flight `key` waits for. Empty if it waits for none, and so could end.
    fn waits(&self, key: Key) -> Vec<Key> {
        let Some(load) = self.state.loads.get(&key) else { return Vec::new() };
        let mut waits = Vec::new();
        for &dependency in load.dependencies() {
            if let DepState::Waiting(target) = self.dep_state(dependency) {
                waits.push(target);
            }
        }
        waits
    }

    fn running(&self, key: Key) -> bool {
        self.state.loads.get(&key).is_some_and(|load| matches!(load.own, Own::Running))
    }

    /// Step 6 for the waits reachable from `root`: decides every cycle that waits for
    /// nothing outside it, if every load `root` reaches has ended its steps 3 and 4.
    /// Returns whether anything changed.
    fn cycles_from(&mut self, root: Key) -> bool {
        if !self.state.loads.contains_key(&root) {
            return false;
        }
        self.searches += 1;
        match self.find_running(root) {
            Some(running) => {
                if !self.running(running) {
                    // A load that waits for nothing and has not ended yet: end it first.
                    self.work.push(running);
                    return true;
                }
                false
            }
            None => {
                let cycles = self.sink_cycles(root);
                let changed = !cycles.is_empty();
                for cycle in cycles {
                    self.decide(&cycle);
                }
                changed
            }
        }
    }

    /// A load reachable from `root` through waits whose steps 3 and 4 still run, or one
    /// that waits for nothing; `None` if there is none. Jumps along `Load::behind` where
    /// the finding still holds, and records it for the path it walked.
    ///
    /// A finding holds while its target runs, since no load on a path to a load that still
    /// runs can end (step 5 needs every dependency settled, and step 6 decides only cycles
    /// that reach no such load), and only a mount renames, so the path stays a path.
    fn find_running(&mut self, root: Key) -> Option<Key> {
        let mut visited = BTreeSet::from([root]);
        let mut path: Vec<(Key, Vec<Key>)> = Vec::new();
        let mut found = None;
        let mut node = root;
        loop {
            match self.successors(node) {
                Err(running) => {
                    found = Some(running);
                    path.push((node, Vec::new()));
                    break;
                }
                Ok(next) => path.push((node, next)),
            }
            let mut advanced = false;
            while let Some((_, next)) = path.last_mut() {
                match next.pop() {
                    Some(candidate) if visited.insert(candidate) => {
                        node = candidate;
                        advanced = true;
                        break;
                    }
                    Some(_) => {}
                    None => {
                        path.pop();
                    }
                }
            }
            if !advanced {
                break;
            }
        }
        let running = found?;
        // Only a load whose steps 3 and 4 still run is worth remembering; one that waits
        // for nothing is about to end.
        if let Some(target) = self.state.loads.get(&running)
            && matches!(target.own, Own::Running)
        {
            let behind = Behind { target: running, seq: target.seq, mounts: self.state.mounts };
            for (node, _) in path {
                if node == running {
                    continue;
                }
                if let Some(load) = self.state.loads.get_mut(&node) {
                    load.behind = Some(behind);
                }
            }
        }
        Some(running)
    }

    /// The loads `node` waits for; `Err` with a load that still runs (`node` itself, the
    /// one it is known to be behind, or one it waits for directly), or with `node` if it
    /// waits for nothing.
    fn successors(&self, node: Key) -> std::result::Result<Vec<Key>, Key> {
        self.visits.set(self.visits.get() + 1);
        let load = &self.state.loads[&node];
        if matches!(load.own, Own::Running) {
            return Err(node);
        }
        if let Some(behind) = load.behind
            && behind.mounts == self.state.mounts
            && self.state.loads.get(&behind.target).is_some_and(|target| {
                target.seq == behind.seq && matches!(target.own, Own::Running)
            })
        {
            return Err(behind.target);
        }
        let waits = self.waits(node);
        if waits.is_empty() {
            return Err(node);
        }
        match waits.iter().find(|&&target| self.running(target)) {
            Some(&running) => Err(running),
            None => Ok(waits),
        }
    }

    /// The strongly connected sets of waits reachable from `root` that wait for nothing
    /// outside themselves, once every load reachable has ended its steps 3 and 4 (an
    /// iterative Tarjan over the waits). Empty if one still runs after all.
    fn sink_cycles(&self, root: Key) -> Vec<Vec<Key>> {
        struct Visit {
            index: usize,
            low: usize,
            on_stack: bool,
        }
        let mut visits: BTreeMap<Key, Visit> = BTreeMap::new();
        let mut stack: Vec<Key> = Vec::new();
        let mut frames: Vec<(Key, Vec<Key>, usize)> = Vec::new();
        let mut components: Vec<Vec<Key>> = Vec::new();

        let open = |key: Key, visits: &mut BTreeMap<Key, Visit>, stack: &mut Vec<Key>| {
            let index = visits.len();
            visits.insert(key, Visit { index, low: index, on_stack: true });
            stack.push(key);
        };
        let waits_of = |key: Key| -> Option<Vec<Key>> {
            self.visits.set(self.visits.get() + 1);
            let load = &self.state.loads[&key];
            if matches!(load.own, Own::Running) {
                return None;
            }
            let waits = self.waits(key);
            (!waits.is_empty()).then_some(waits)
        };

        open(root, &mut visits, &mut stack);
        let Some(waits) = waits_of(root) else { return Vec::new() };
        frames.push((root, waits, 0));
        while let Some((node, waits, next)) = frames.last_mut() {
            let node = *node;
            if let Some(&target) = waits.get(*next) {
                *next += 1;
                match visits.get(&target) {
                    None => {
                        open(target, &mut visits, &mut stack);
                        let Some(waits) = waits_of(target) else { return Vec::new() };
                        frames.push((target, waits, 0));
                    }
                    Some(visit) if visit.on_stack => {
                        let index = visit.index;
                        let low = &mut visits.get_mut(&node).expect("opened").low;
                        *low = (*low).min(index);
                    }
                    Some(_) => {}
                }
                continue;
            }
            frames.pop();
            let Visit { index, low, .. } = visits[&node];
            if let Some((parent, _, _)) = frames.last() {
                let parent_low = &mut visits.get_mut(parent).expect("opened").low;
                *parent_low = (*parent_low).min(low);
            }
            if low == index {
                let mut component = Vec::new();
                loop {
                    let member = stack.pop().expect("the component's members are stacked");
                    visits.get_mut(&member).expect("opened").on_stack = false;
                    component.push(member);
                    if member == node {
                        break;
                    }
                }
                components.push(component);
            }
        }

        // A component that waits only for its own members is decided now; one that waits
        // for another component is decided once that one has ended.
        let mut component_of = BTreeMap::new();
        for (number, component) in components.iter().enumerate() {
            for &member in component {
                component_of.insert(member, number);
            }
        }
        let mut sinks = Vec::new();
        for (number, component) in components.into_iter().enumerate() {
            let sink = component.iter().all(|&member| {
                self.waits(member).iter().all(|target| component_of[target] == number)
            });
            if sink {
                sinks.push(component);
            }
        }
        sinks
    }

    /// Decides one cycle that waits for nothing outside it: each member fails for its
    /// lowest failed dependency, the other members counting as failed, with that
    /// dependency's kind, a member's decided kind, or `InvalidData` where following lowest
    /// failed dependencies from member to member comes back round.
    fn decide(&mut self, cycle: &[Key]) {
        enum First {
            Outside(AssetId, ErrorKind),
            Member(AssetId, Key),
        }
        let members: BTreeSet<Key> = cycle.iter().copied().collect();
        let mut first = BTreeMap::new();
        for &member in &members {
            let load = &self.state.loads[&member];
            let lowest = load.dependencies().iter().find_map(|&dependency| {
                match self.dep_state(dependency) {
                    DepState::Current => None,
                    DepState::Failed(kind) => Some(First::Outside(dependency, kind)),
                    DepState::Waiting(target) => {
                        debug_assert!(members.contains(&target), "a cycle waits for no other");
                        Some(First::Member(dependency, target))
                    }
                }
            });
            first.insert(member, lowest.expect("a member waits for a member"));
        }

        let mut kinds: BTreeMap<Key, ErrorKind> = BTreeMap::new();
        for &member in &members {
            let mut path = Vec::new();
            let mut on_path = BTreeSet::new();
            let mut node = member;
            let kind = loop {
                if let Some(&kind) = kinds.get(&node) {
                    break kind;
                }
                if !on_path.insert(node) {
                    break ErrorKind::InvalidData;
                }
                path.push(node);
                match first[&node] {
                    First::Outside(_, kind) => break kind,
                    First::Member(_, target) => node = target,
                }
            };
            for node in path {
                kinds.insert(node, kind);
            }
        }

        for &member in &members {
            let (dependency, message): (AssetId, Cow<'static, str>) = match first[&member] {
                First::Outside(dependency, _) => (
                    dependency,
                    format!("its dependency {} failed", Hex(dependency.to_bits())).into(),
                ),
                First::Member(dependency, _) => (
                    dependency,
                    format!("it is in a dependency cycle, through {}", Hex(dependency.to_bits()))
                        .into(),
                ),
            };
            let kind = kinds[&member];
            self.end(member, Ending::Dependency { dependency, kind, message });
        }
    }

    /// Ends the load `key`: publishes it or records its failure, in one step with the move
    /// of the generation, and queues what waited for it. A load that reads again from a
    /// newer source does not end.
    fn end(&mut self, key: Key, ending: Ending) {
        if self.read_again(key, &ending) {
            return;
        }
        let load = self.state.loads.remove(&key).expect("an ending load is in flight");
        let Load { id, version, source, own, seq, task, trace, .. } = load;
        self.state.in_flight.remove(&seq);
        for dependency in source.entry().header.dependencies() {
            if let Some(waiting) = self.state.waiters.get_mut(dependency) {
                waiting.remove(&key);
                if waiting.is_empty() {
                    self.state.waiters.remove(dependency);
                }
            }
        }
        self.effects.tasks.extend(task);
        let named = self.state.named_version(id) == Some(version);
        let generation = self.move_generation();

        let (kind, cause, dependency, message) = match (ending, own) {
            (Ending::Dependency { dependency, kind, message }, own) => {
                self.effects.dropped.push(Box::new(own));
                (kind, Cause::Dependency(dependency), Some(dependency), message)
            }
            (Ending::Own, Own::Done(Ok(handle))) => {
                if named {
                    self.asset(id).current = Some(version);
                }
                self.record(key, Outcome::Published { handle, source });
                let current = named;
                let load = trace;
                self.effects.traces.push(Trace::Publish { load, id, version, current, generation });
                self.touch(id);
                self.wake_settled();
                return;
            }
            (Ending::Own, Own::Done(Err((error, cause)))) => {
                let (kind, message) = (error.kind(), error.message().to_owned());
                self.effects.dropped.push(Box::new(error));
                (kind, cause, None, Cow::Owned(message))
            }
            (Ending::Own, Own::Running) => unreachable!("a load ends after its steps 3 and 4"),
        };
        let failure = AssetFailure { id, version: Some(version), kind };
        self.record(key, Outcome::Failed { failure, cause });
        if let Some(dependency) = dependency {
            self.state.failed_for.entry(dependency).or_default().insert(key);
        }
        if named {
            self.asset(id).newest_failure = Some(failure);
        }
        self.effects.dropped.push(Box::new(source));
        self.effects.traces.push(Trace::Failure { load: trace, failure, dependency, message });
        self.touch(id);
        self.wake_settled();
    }

    /// Hands the wakers of the waits that are done to the effects, without cloning one.
    fn wake_settled(&mut self) {
        let floor = self.state.in_flight.first().copied().unwrap_or(u64::MAX);
        let ready = self.state.settling.extract_if(.., |_, (mark, _)| *mark <= floor);
        self.effects.wakers.extend(ready.map(|(_, (_, waker))| waker));
    }
}
