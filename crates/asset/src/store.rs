//! The store: mounted packs, loads in flight and published versions, behind the two store
//! traits of `lactose-asset-api`.

use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::task::{Context, Poll};

use lactose_asset_api::{
    AssetCatalog, AssetFailure, AssetId, AssetSet, AssetVersion, ErasedLoader, LoadPriority,
    PublishedAssets, Readiness, UntypedHandle, VersionTable,
};
use lactose_core::{AllocTag, Error, ErrorKind, Result};
use lactose_io::{Encoding, Io};
use lactose_task::{Executor, WorkerClass, yield_now};
use tracing::Instrument;
use tracing::field::{Empty, debug};

use crate::state::{Cause, Hex, Key, Loaders, Pack, Spawn, State, Step, key};
use crate::{LOG_TARGET, RegionBaselineLoader, pack};

/// A handle to the asset store: `Clone`, one store per process, every clone the same store.
///
/// `app` builds it once from the executor, the file IO pool and every asset kind's
/// loader, mounts the cooked packs, and hands it on as the two `dyn` resources of
/// `lactose-asset-api`: a clone coerces to `Arc<dyn AssetCatalog>` for code outside the
/// determinism boundary and, from there, to `Arc<dyn PublishedAssets>` for a boundary
/// world. `README.md` is the contract; the words it defines are used here as it defines
/// them:
///
/// - the **named** version of an asset is the one in the mounted pack with the latest place
///   among those that hold the asset ([`Store::mount`]);
/// - a version is **published** once its load has ended well, and from then on never
///   changes and is never unloaded;
/// - the **current** version of an asset is the one `newest` answers with. A version
///   becomes current when it is published while named, or when `request_newest` or a mount
///   finds the named version already published; a current version is replaced only by
///   another and never withdrawn;
/// - an asset is **followed** once its newest version has been asked for, by
///   `request_newest` or as a declared dependency of a load. Each mount asks again for the
///   followed assets it renamed, for those it is a new source for after a failed read, and
///   for those that had failed for one of either, which is the store's hot reload.
///
/// No method blocks, waits for IO or runs a loader: loads run as tasks on the executor's
/// background workers, and a lookup holds the store's lock for its own map work only,
/// never across a read, a decode or a loader.
///
/// Dropping the last handle cancels every load in flight; [`Settled`] and [`Mount`] each
/// hold one. The store owns no thread and has no shutdown of its own: once the executor
/// or the pool has stopped, a load fails with `ErrorKind::Cancelled`.
#[derive(Clone)]
pub struct Store {
    inner: Arc<Inner>,
}

/// What the handles share. Hidden and changeable: the packs, the loads and the published
/// versions the implementation keeps beside these are its own.
///
/// Load tasks hold a `Weak` of it and never a handle, so dropping the last handle drops
/// `state`, and with it every load's `Task`, which cancels the load.
struct Inner {
    executor: Executor,
    io: Io,
    /// Shared with the load tasks, which call a loader without holding the store.
    loaders: Arc<Loaders>,
    state: Mutex<State>,
}

impl Store {
    /// A store with nothing mounted, which will load with `loaders`, one per asset kind,
    /// and with [`RegionBaselineLoader`](crate::RegionBaselineLoader), which it adds
    /// itself.
    ///
    /// The store keeps `executor` and `io` for its life: it spawns one
    /// `WorkerClass::Background` task per load on the first and reads every pack through
    /// the second. Nothing is spawned, opened or read here.
    ///
    /// Fails with `ErrorKind::InvalidData`, naming the kind, if two loaders produce one
    /// `AssetKind` (the collision check of kind names, `Asset::KIND`), a loader of the
    /// region-baseline kind among `loaders` included.
    pub fn new(
        executor: Executor,
        io: Io,
        loaders: impl IntoIterator<Item = ErasedLoader>,
    ) -> Result<Self> {
        let mut indexed = Loaders::new();
        for loader in loaders.into_iter().chain([ErasedLoader::new(RegionBaselineLoader)]) {
            let kind = loader.kind();
            if indexed.insert(kind, loader).is_some() {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!("two loaders produce asset kind {}", Hex(kind.0)),
                ));
            }
        }
        let state = Mutex::new(State::default());
        Ok(Self { inner: Arc::new(Inner { executor, io, loaders: Arc::new(indexed), state }) })
    }

    /// Mounts the pack at `path`: opens it through the pool, reads its trailer and its
    /// index, checks them, and makes every version the pack holds requestable.
    ///
    /// **Place.** Each call takes the next place in the store's mount order when it is
    /// called, not when its future resolves, and the pack holds that place from the moment
    /// the future resolves `Ok`. A mount that fails, or whose future is dropped, leaves its
    /// place empty. Packs mounted by the same calls in the same order therefore name the
    /// same versions once every mount has resolved, whatever order they resolved in.
    ///
    /// **What a mount changes.** Atomically with respect to every other method: every
    /// version in the pack becomes requestable by exact version; the pack's version of an
    /// asset becomes the asset's named version unless a pack with a later place holds the
    /// asset; and the store does what `AssetCatalog::request_newest` does, at the lowest
    /// priority, for every followed asset whose named version the mount changed (unless
    /// that version is current or loading); for every followed asset whose named version
    /// failed for a reason of the source (the read, or content that did not hash to its
    /// version) when this pack holds that version and is now where it is read from; and,
    /// transitively, for every followed asset whose named version last failed for a
    /// dependency the mount asks for again. A followed asset the mount did not rename and
    /// whose named version failed for a reason of the version (no loader, another cooked
    /// format, a loader refusing intact bytes) is not retried.
    ///
    /// Renaming changes no answer of `get`, `newest` or `resolve`: a mount changes one,
    /// and moves the generation, only through those requests, at the mount itself where
    /// one makes an already published version current or accepts a load that ends at
    /// once, and otherwise when the loads it accepted end.
    ///
    /// **Cost.** One open and two reads, the trailer and the index, whatever the pack's
    /// size; no payload is read or checked until its asset is loaded. The pack stays open,
    /// on one descriptor, until the last handle to the store is dropped. The file must not
    /// be written in place afterwards; replacing it by rename (`Io::replace`) is safe,
    /// since the store keeps reading the file it opened, and a later `mount` of the same
    /// path mounts the new file as another pack.
    ///
    /// Nothing is opened until the future is first polled. It resolves to
    /// `ErrorKind::NotFound` if there is no such file; `InvalidData` if the file is not a
    /// well-formed pack (`README.md`, "The pack format", lists every rule), an index
    /// length that runs past the file included, whatever its size; `VersionMismatch` if it
    /// is a pack of another format version; `ResourceExhausted` if its index lies within
    /// the file and is longer than `lactose_io::MAX_READ_BYTES`, or a buffer cannot be
    /// allocated; `Io` for anything else the open or a read reports; `Cancelled` if the
    /// pool has stopped. A failed mount mounts nothing.
    #[must_use = "a mount does nothing until polled"]
    pub fn mount(&self, path: impl Into<PathBuf>) -> Mount {
        let path = path.into();
        let place = self.inner.lock().take_place();
        Mount { run: Box::pin(Arc::clone(&self.inner).mount(place, path)) }
    }

    /// A future that resolves once every load the store accepted before the future's first
    /// poll has ended, each published or failed.
    ///
    /// A load is accepted inside the call that starts it: a `request` or `request_newest`
    /// that finds something to load, the loads of its dependencies with it, and the
    /// requests a mount makes when it resolves. The future waits for no load accepted
    /// after its first poll, except as a load accepted before waits for one (a dependency
    /// that a later mount renamed). Every accepted load ends, also when the executor or
    /// the pool has stopped (it then fails with `Cancelled`), so the future always
    /// resolves. It is the store's one wait, for tools, tests and a start-up that must not
    /// step before its content is in: a frame loop watches `AssetCatalog::generation`
    /// instead.
    #[must_use = "futures do nothing unless awaited"]
    pub fn settled(&self) -> Settled {
        Settled { run: Box::pin(Wait { store: Some(Arc::clone(&self.inner)), slot: None }) }
    }
}

impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Store").field("loaders", &self.inner.loaders.len()).finish_non_exhaustive()
    }
}

/// The exact-version half of the store: what a boundary world is handed. The trait states
/// what every store upholds; `README.md` states what this one adds.
impl PublishedAssets for Store {
    /// Accepts a load of exactly `version` of `id` from the mounted pack with the latest
    /// place that holds it, unless that version is published or already loading, in which
    /// case the call does nothing, its priority included, also when the index names
    /// `version`. A recorded failure is retried. The asset is not followed for it, and the
    /// end of the load is an answer for `newest` (the current version, or the newest
    /// failure) only if the index names `version` when the load ends.
    ///
    /// If no mounted pack holds `(id, version)` the load ends inside this call, failed with
    /// `ErrorKind::NotFound`. `priority` is recorded on the load's span and orders nothing
    /// at milestone 1. A request for `AssetId::NONE` does nothing.
    fn request(&self, id: AssetId, version: AssetVersion, priority: LoadPriority) {
        if !id.is_none() {
            self.inner.step(|step| step.exact(id, version, priority));
        }
    }

    /// `Ready` for a published version; `Failed`, with the failure recorded when the last
    /// load of it ended, for a version that is not published and whose last load failed;
    /// `Pending` otherwise. A retry in flight leaves a recorded failure the answer until
    /// the retry ends.
    fn get(&self, id: AssetId, version: AssetVersion) -> Readiness<UntypedHandle> {
        if id.is_none() {
            return Readiness::Failed(no_asset());
        }
        self.inner.lock().get(id, version)
    }
}

/// The catalogue half of the store: what code outside the boundary is handed. The trait
/// states what every store upholds; `README.md` states what this one adds.
impl AssetCatalog for Store {
    /// Marks `id` followed, then: if no mounted pack holds `id`, the load ends inside this
    /// call, failed with `ErrorKind::NotFound` and no version, which becomes the asset's
    /// newest failure; if the named version is published, makes it current if it is not;
    /// if it is loading, does nothing; otherwise accepts a load of it, retrying a recorded
    /// failure. `priority` is recorded on the load's span and orders nothing at milestone
    /// 1. A request for `AssetId::NONE` does nothing.
    fn request_newest(&self, id: AssetId, priority: LoadPriority) {
        if !id.is_none() {
            self.inner.step(|step| step.newest(id, priority, "newest"));
        }
    }

    /// `Ready` with the current version if the asset has one; otherwise `Failed` with its
    /// newest failure if one is recorded; otherwise `Pending`. The newest failure of an
    /// asset is the later of: the failure of a load that ended while the index named its
    /// version, whoever asked for the load; and `{ id, version: None, kind: NotFound }`,
    /// recorded when the asset's newest version was asked for and no mounted pack held
    /// the asset. A request and a load in flight leave the answer as it was, and so does a
    /// mount but for the published version it may make current.
    fn newest(&self, id: AssetId) -> Readiness<UntypedHandle> {
        if id.is_none() {
            return Readiness::Failed(no_asset());
        }
        self.inner.lock().newest(id)
    }

    /// The table of the trait, taken in one critical section: `Failed` with the failure of
    /// the lowest [`AssetId`] in `set` whose `newest` is `Failed`; else `Pending` if any
    /// member's is `Pending`; else the current version of every member and of every asset
    /// those versions declare as a dependency, transitively.
    fn resolve(&self, set: &AssetSet) -> Readiness<VersionTable> {
        self.inner.lock().resolve(set)
    }

    /// Moves, by at least one and in one step with the change under the store's lock, when
    /// a version is published, when an already published version becomes current, and when
    /// a failure is recorded (a retry that fails again records it again). It never moves
    /// otherwise, and while it is unchanged `get`, `newest` and `resolve` answer as they
    /// did: no lookup sees a change this does not show yet, nor the reverse.
    fn generation(&self) -> u64 {
        self.inner.lock().generation()
    }
}

/// A mount in flight: [`Store::mount`]. Resolves once the pack is mounted, or to the error
/// that mounted nothing. Dropping it cancels the mount: nothing is mounted, and the file,
/// if it was opened, is closed.
#[must_use = "a mount does nothing until polled"]
pub struct Mount {
    run: Pin<Box<dyn Future<Output = Result<()>> + Send>>,
}

impl Future for Mount {
    type Output = Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.run.as_mut().poll(cx)
    }
}

/// The wait for the loads accepted so far: [`Store::settled`].
#[must_use = "futures do nothing unless awaited"]
pub struct Settled {
    run: Pin<Box<dyn Future<Output = ()> + Send>>,
}

impl Future for Settled {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.run.as_mut().poll(cx)
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `change` as one critical section and ends what can end, then, with the lock
    /// released, wakes the waits that ended and spawns the loads that were accepted.
    fn step<R>(self: &Arc<Self>, change: impl FnOnce(&mut Step<'_>) -> R) -> R {
        let (result, effects) = {
            let mut state = self.lock();
            let mut step = Step::new(&mut state, &self.loaders);
            let result = change(&mut step);
            step.settle();
            (result, step.effects)
        };
        for waker in effects.wakers {
            waker.wake();
        }
        for task in effects.tasks {
            task.detach();
        }
        for spawn in effects.spawns {
            self.spawn(spawn);
        }
        result
    }

    /// Spawns steps 3 and 4 of an accepted load. Outside the lock: a shut-down executor
    /// drops the future inside `spawn`, and its guard then takes the lock to record the
    /// load as cancelled.
    fn spawn(self: &Arc<Self>, spawn: Spawn) {
        let (key, seq) = (key(spawn.id, spawn.version), spawn.seq);
        let span = spawn.span.clone();
        let guard = Cancelled { store: Arc::downgrade(self), key, seq, armed: true };
        let run = run_load(Arc::downgrade(self), Arc::clone(&self.loaders), spawn, guard);
        let task = {
            // A task re-enters its spawner's tag for every poll and for its drop.
            let _tag = AllocTag::Asset.scope();
            self.executor.spawn(WorkerClass::Background, run.instrument(span))
        };
        let unused = self.lock().give_task(key, seq, task);
        if let Some(task) = unused {
            task.detach();
        }
    }

    /// The future of [`Store::mount`], for the place the call took.
    async fn mount(self: Arc<Self>, place: usize, path: PathBuf) -> Result<()> {
        let span = tracing::info_span!(
            target: LOG_TARGET,
            "silo.mount",
            path = %path.display(),
            place = place as u64,
            assets = Empty,
            requested = Empty,
            error = Empty,
        );
        match read_pack(&self.io, &path).instrument(span.clone()).await {
            Ok(pack) => {
                span.record("assets", pack.entries.len() as u64);
                let requested = span.in_scope(|| self.step(|step| step.mounted(place, &pack)));
                span.record("requested", requested);
                Ok(())
            }
            Err(error) => {
                span.record("error", debug(error.kind()));
                Err(error)
            }
        }
    }
}

/// Opens the pack at `path` and reads and checks its trailer and its index: one open and
/// two reads, and no read of a file shorter than a trailer.
async fn read_pack(io: &Io, path: &Path) -> Result<Arc<Pack>> {
    let file = io.open(path).await?;
    let file_len = file.size();
    if file_len < pack::TRAILER_LEN {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("a file of {file_len} bytes is shorter than a pack's trailer"),
        ));
    }
    let trailer = file.read_at(file_len - pack::TRAILER_LEN, pack::TRAILER_LEN).await?;
    let trailer = pack::Trailer::parse(&trailer, file_len)?;
    let payload_len = file_len - pack::TRAILER_LEN - trailer.index_len;
    let index = file.read_at(payload_len, trailer.index_len).await?;
    let _tag = AllocTag::Asset.scope();
    let entries = pack::decode_index(&index, trailer.index_hash, payload_len)?;
    Ok(Arc::new(Pack { file, entries }))
}

/// Steps 3 and 4 of a load, on a background worker: the read, one yield, the loader's
/// call. It holds no handle to the store, so that dropping the last one cancels it.
///
/// `guard` is an argument, not a local, so that it lives in the future from the call on:
/// a future dropped before its first poll (inside `spawn`, at an executor's shutdown)
/// drops it too, and the load still ends.
async fn run_load(store: Weak<Inner>, loaders: Arc<Loaders>, spawn: Spawn, mut guard: Cancelled) {
    let Spawn { id, version, seq, source, span } = spawn;
    let key = key(id, version);
    let entry = source.entry();
    let read = if entry.compressed {
        let encoding = Encoding::Zstd { decoded_len: entry.decoded_len };
        source.pack.file.read_encoded_at(entry.offset, entry.stored_len, encoding)
    } else {
        source.pack.file.read_at(entry.offset, entry.stored_len)
    };
    let result = match read.await {
        Err(error) => Err((error, Cause::Source)),
        Ok(bytes) => {
            span.record("bytes", bytes.len() as u64);
            yield_now().await;
            if store.strong_count() == 0 {
                // No loader's call begins once the last handle is gone.
                guard.armed = false;
                return;
            }
            let loader = &loaders[&entry.header.kind()];
            loader.load(id, version, &entry.header, &bytes).map_err(|error| {
                // `InvalidData` is the version check's or the loader's refusal of intact
                // bytes: only the content's hash tells the two apart.
                let damaged = error.kind() == ErrorKind::InvalidData
                    && entry.header.version_of(&bytes) != version;
                (error, if damaged { Cause::Source } else { Cause::Version })
            })
        }
    };
    guard.armed = false;
    if let Some(store) = store.upgrade() {
        store.step(|step| step.own_done(key, seq, result));
    }
}

/// Records a load whose task was dropped before its steps 3 and 4 ended (at an executor's
/// shutdown, or spawned after it) as `Cancelled`, so that every accepted load ends.
struct Cancelled {
    store: Weak<Inner>,
    key: Key,
    seq: u64,
    armed: bool,
}

impl Drop for Cancelled {
    fn drop(&mut self) {
        if self.armed
            && let Some(store) = self.store.upgrade()
        {
            let error = Error::new(ErrorKind::Cancelled, "the load's task was dropped");
            let (key, seq) = (self.key, self.seq);
            store.step(|step| step.own_done(key, seq, Err((error, Cause::Source))));
        }
    }
}

/// The failure of `AssetId::NONE`, which names no asset.
fn no_asset() -> AssetFailure {
    AssetFailure { id: AssetId::NONE, version: None, kind: ErrorKind::NotFound }
}

/// The future inside [`Settled`]: a handle to the store until it resolves, and from its
/// first poll its number among the waits and its mark.
struct Wait {
    store: Option<Arc<Inner>>,
    slot: Option<(u64, u64)>,
}

impl Future for Wait {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = &mut *self;
        let Some(store) = &this.store else { return Poll::Ready(()) };
        if !store.lock().poll_settled(&mut this.slot, cx.waker()) {
            return Poll::Pending;
        }
        this.store = None;
        Poll::Ready(())
    }
}

impl Drop for Wait {
    fn drop(&mut self) {
        if let (Some(store), Some((number, _))) = (self.store.take(), self.slot) {
            store.lock().forget_settled(number);
        }
    }
}
