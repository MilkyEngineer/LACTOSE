//! The store: mounted packs, loads in flight and published versions, behind the two store
//! traits of `lactose-asset-api`.

use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use lactose_asset_api::{
    AssetCatalog, AssetId, AssetSet, AssetVersion, ErasedLoader, LoadPriority, PublishedAssets,
    Readiness, UntypedHandle, VersionTable,
};
use lactose_core::Result;
use lactose_io::Io;
use lactose_task::Executor;

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
struct Inner {
    executor: Executor,
    io: Io,
    loaders: Vec<ErasedLoader>,
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
        todo!("index the loaders by kind")
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
        todo!("take the next place, then open, read and check the index in the future")
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
        todo!("wait for the loads accepted so far")
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
        todo!("accept a load of the exact version")
    }

    /// `Ready` for a published version; `Failed`, with the failure recorded when the last
    /// load of it ended, for a version that is not published and whose last load failed;
    /// `Pending` otherwise. A retry in flight leaves a recorded failure the answer until
    /// the retry ends.
    fn get(&self, id: AssetId, version: AssetVersion) -> Readiness<UntypedHandle> {
        todo!("look the version up")
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
        todo!("follow the asset and accept a load of its named version")
    }

    /// `Ready` with the current version if the asset has one; otherwise `Failed` with its
    /// newest failure if one is recorded; otherwise `Pending`. The newest failure of an
    /// asset is the later of: the failure of a load that ended while the index named its
    /// version, whoever asked for the load; and `{ id, version: None, kind: NotFound }`,
    /// recorded when the asset's newest version was asked for and no mounted pack held
    /// the asset. A request and a load in flight leave the answer as it was, and so does a
    /// mount but for the published version it may make current.
    fn newest(&self, id: AssetId) -> Readiness<UntypedHandle> {
        todo!("look the current version up")
    }

    /// The table of the trait, taken in one critical section: `Failed` with the failure of
    /// the lowest [`AssetId`] in `set` whose `newest` is `Failed`; else `Pending` if any
    /// member's is `Pending`; else the current version of every member and of every asset
    /// those versions declare as a dependency, transitively.
    fn resolve(&self, set: &AssetSet) -> Readiness<VersionTable> {
        todo!("walk the current versions and their declared dependencies")
    }

    /// Moves, by at least one and in one step with the change under the store's lock, when
    /// a version is published, when an already published version becomes current, and when
    /// a failure is recorded (a retry that fails again records it again). It never moves
    /// otherwise, and while it is unchanged `get`, `newest` and `resolve` answer as they
    /// did: no lookup sees a change this does not show yet, nor the reverse.
    fn generation(&self) -> u64 {
        todo!("read the counter")
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
