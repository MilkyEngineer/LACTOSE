//! What the store's tests share: an executor and a pool, a store on a pool held before
//! its first request, a directory of packs under `CARGO_TARGET_TMPDIR`, waits bounded so
//! that a hang fails instead, and test asset kinds, among them one whose loader a test
//! holds and releases by name.

#![allow(dead_code, reason = "each test binary uses part of this")]

pub mod capture;
pub mod format;

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use lactose_asset::{PackBuilder, Storage, Store};
use lactose_asset_api::{
    Asset, AssetId, AssetKind, CookedAsset, CookedHeader, ErasedLoader, Loader, SimulationAsset,
};
use lactose_core::{AllocTag, Error, ErrorKind, Result};
use lactose_io::{Io, IoConfig};
use lactose_task::{
    CoreTopology, Executor, Platform, Sleep, ThreadHandle, ThreadSpec, WorkerClass,
};

/// The longest any test waits for something that should happen: past it, the wait fails.
pub const LIMIT: Duration = Duration::from_secs(60);

/// The limit of the 100,000-link tests, which take seconds unloaded and over a minute on a
/// machine loaded near 30: under nextest's 300 s kill, so that a hang still fails as one.
pub const LONG_LIMIT: Duration = Duration::from_secs(240);

/// How long a test waits for a call that must return at once, before it reports the call
/// stuck (on the store's lock, if the code under test ran something there that waits for
/// it) and exits, since the stuck thread would also stop the test's own teardown.
pub const STUCK: Duration = Duration::from_secs(10);

/// Waits for `returned` (sent by a thread once its call returned) up to [`STUCK`];
/// otherwise reports `what` as stuck and exits the test process as failed.
pub fn returns_or_exit(returned: &std::sync::mpsc::Receiver<()>, what: &str) {
    if returned.recv_timeout(STUCK).is_err() {
        eprintln!("{what} never returned within {STUCK:?}: it is stuck on the store's lock");
        std::process::exit(101);
    }
}

/// An executor, a pool and a directory of packs of the test's own.
pub struct Env {
    pub executor: Executor,
    pub io: Io,
    pub dir: PathBuf,
    packs: AtomicU64,
    limit: Duration,
}

impl Env {
    pub fn new(name: &str) -> Self {
        Self::build(name, Executor::builder(), None)
    }

    /// An executor with exactly `count` background workers.
    pub fn with_background_workers(name: &str, count: u32) -> Self {
        Self::build(name, Executor::builder().background_workers(count), None)
    }

    /// An env whose every wait is bounded by [`LONG_LIMIT`] rather than [`LIMIT`].
    pub fn long(name: &str) -> Self {
        Self { limit: LONG_LIMIT, ..Self::new(name) }
    }

    /// One background worker and one pool thread: every request and every load runs on
    /// one known thread.
    pub fn one_thread_each(name: &str) -> Self {
        let one = std::num::NonZero::new(1).unwrap();
        Self::build(name, Executor::builder().background_workers(1), Some(IoConfig::new(one)))
    }

    fn build(name: &str, builder: lactose_task::ExecutorBuilder, io: Option<IoConfig>) -> Self {
        let executor = builder.build().expect("an executor");
        let platform = executor.platform().as_ref();
        let config = io.unwrap_or_else(|| IoConfig::for_platform(platform));
        let io = Io::start(platform, config).expect("the pool");
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("asset").join(name);
        executor.block_on(io.create_dir_all(dir.clone())).expect("the test's directory");
        Self { executor, io, dir, packs: AtomicU64::new(0), limit: LIMIT }
    }

    pub fn store(&self, loaders: Vec<ErasedLoader>) -> Store {
        Store::new(self.executor.clone(), self.io.clone(), loaders).expect("one loader per kind")
    }

    /// A store on a pool of its own, of one thread that does not begin until the returned
    /// [`HeldPool`] is released or dropped: every request the store submits before that
    /// stays queued, so a first poll that submits one returns `Pending` however fast the
    /// pool would have been. The env's own pool still writes the packs.
    pub fn held_store(&self, loaders: Vec<ErasedLoader>) -> (Store, HeldPool) {
        let release = Arc::new(Release::default());
        let platform = HoldingPlatform {
            platform: Arc::clone(self.executor.platform()),
            release: Arc::clone(&release),
        };
        let one = std::num::NonZero::new(1).unwrap();
        let io = Io::start(&platform, IoConfig::new(one)).expect("the held pool");
        let store = Store::new(self.executor.clone(), io, loaders).expect("one loader per kind");
        (store, HeldPool(release))
    }

    /// Writes `bytes` whole at a new path of the directory.
    pub fn write(&self, bytes: Vec<u8>) -> PathBuf {
        let number = self.packs.fetch_add(1, Ordering::Relaxed);
        let path = self.dir.join(format!("{number:04}.pack"));
        self.write_at(path.clone(), bytes);
        path
    }

    /// Replaces the file at `path` by rename.
    pub fn write_at(&self, path: PathBuf, bytes: Vec<u8>) {
        self.wait(self.io.replace(path, bytes)).expect("the file written");
    }

    /// A pack of `assets`, each stored as `storage` says, written at a new path.
    pub fn pack(&self, assets: &[(AssetId, CookedAsset)], storage: Storage) -> PathBuf {
        self.write(pack(assets, storage))
    }

    pub fn mount(&self, store: &Store, path: PathBuf) -> Result<()> {
        self.wait(store.mount(path))
    }

    /// Mounts a pack of `assets`, stored raw.
    pub fn provide(&self, store: &Store, assets: &[(AssetId, CookedAsset)]) {
        let path = self.pack(assets, Storage::Raw);
        self.mount(store, path).expect("the pack mounted");
    }

    pub fn settle(&self, store: &Store) {
        self.wait(store.settled());
    }

    /// Drives `future` to its end on this thread, failing the test if that takes longer than
    /// the env's limit, [`LIMIT`] unless it was made [`Env::long`].
    pub fn wait<F: Future>(&self, future: F) -> F::Output {
        wait_on(&self.executor, future, self.limit)
    }

    /// Returns once every run of a load's steps 3 and 4 that is on a background worker now
    /// has returned from its poll, `free` being the number of background workers not held
    /// inside a loader: it spawns `free` tasks that each hold a worker until all of them
    /// do. A run that ends a load's steps ends them in the poll its loader returns in, so
    /// after a held loader call returned, this is the point its own steps ended.
    pub fn drain(&self, free: usize) {
        let barrier = Arc::new(std::sync::Barrier::new(free));
        let tasks: Vec<_> = (0..free)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                self.executor.spawn(WorkerClass::Background, async move {
                    barrier.wait();
                })
            })
            .collect();
        for task in tasks {
            self.wait(task).expect("a draining task");
        }
    }

    pub fn shutdown(self) {
        self.io.shutdown();
        self.executor.shutdown();
    }
}

/// Whether a held pool's threads may begin.
#[derive(Default)]
struct Release {
    released: Mutex<bool>,
    changed: Condvar,
}

impl Release {
    fn released(&self) -> MutexGuard<'_, bool> {
        self.released.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn release(&self) {
        *self.released() = true;
        self.changed.notify_all();
    }

    /// Unbounded: nothing joins a thread held here before the release, and the
    /// [`HeldPool`]'s drop releases it.
    fn wait(&self) {
        let mut released = self.released();
        while !*released {
            released = self.changed.wait(released).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// The executor's platform, with every thread it starts waiting for the release before
/// its body runs.
struct HoldingPlatform {
    platform: Arc<dyn Platform>,
    release: Arc<Release>,
}

impl Platform for HoldingPlatform {
    fn topology(&self) -> CoreTopology {
        self.platform.topology()
    }

    fn hardware_threads(&self) -> Option<u32> {
        self.platform.hardware_threads()
    }

    fn spawn_thread(
        &self,
        spec: ThreadSpec,
        body: Box<dyn FnOnce() + Send + 'static>,
    ) -> Result<ThreadHandle> {
        let release = Arc::clone(&self.release);
        self.platform.spawn_thread(
            spec,
            Box::new(move || {
                release.wait();
                body();
            }),
        )
    }
}

/// The test's handle to the pool of an [`Env::held_store`]. Dropping it releases the
/// pool, so a test that fails or never releases leaves no pool thread waiting.
pub struct HeldPool(Arc<Release>);

impl HeldPool {
    /// Lets the pool's thread begin: it runs what was queued, in order, skipping what was
    /// cancelled.
    pub fn release(&self) {
        self.0.release();
    }
}

impl Drop for HeldPool {
    fn drop(&mut self) {
        self.0.release();
    }
}

/// Drives `future` to its end on this thread, failing the test if that takes longer than
/// `limit`.
pub fn wait_on<F: Future>(executor: &Executor, future: F, limit: Duration) -> F::Output {
    let deadline = executor.sleep_until(Instant::now() + limit);
    executor.block_on(Bounded { future: Box::pin(future), deadline, limit })
}

/// A wait that fails once its deadline passes. The deadline is polled first: a wait whose
/// wake was lost is polled again only by the deadline's own wake, and must fail then
/// rather than find its future ready and pass slowly.
struct Bounded<F> {
    future: Pin<Box<F>>,
    deadline: Sleep,
    limit: Duration,
}

impl<F: Future> Future for Bounded<F> {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        if Pin::new(&mut self.deadline).poll(cx).is_ready() {
            panic!("a wait took longer than {:?}", self.limit);
        }
        self.future.as_mut().poll(cx)
    }
}

/// Polls `condition` until it holds, failing the test past [`LIMIT`].
pub fn eventually(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + LIMIT;
    while !condition() {
        assert!(Instant::now() < deadline, "{what} never happened");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// The bytes of a pack of `assets`, each stored as `storage` says.
pub fn pack(assets: &[(AssetId, CookedAsset)], storage: Storage) -> Vec<u8> {
    let mut pack = PackBuilder::new();
    for (id, cooked) in assets {
        pack.add(*id, cooked, storage).expect("an asset of the pack");
    }
    pack.finish().expect("a pack")
}

pub fn id(name: &str) -> AssetId {
    AssetId::from_name(name)
}

/// Text: an asset whose bytes are its content, loaded at once.
#[derive(Debug, PartialEq, Eq)]
pub struct Text(pub Vec<u8>);

impl Asset for Text {
    const KIND: AssetKind = AssetKind::from_name("test.text");
    const FORMAT_VERSION: u32 = 1;
}

impl SimulationAsset for Text {}

#[derive(Clone, Copy)]
pub struct TextLoader;

impl Loader for TextLoader {
    type Asset = Text;

    fn load(&self, bytes: &[u8], _: &[AssetId]) -> Result<Text> {
        Ok(Text(bytes.to_vec()))
    }
}

pub fn text(bytes: &[u8], dependencies: impl IntoIterator<Item = AssetId>) -> CookedAsset {
    CookedAsset::new::<Text>(bytes.to_vec(), dependencies)
}

/// A cooked asset of a kind no store here has a loader for.
pub fn unloadable(bytes: &[u8], dependencies: impl IntoIterator<Item = AssetId>) -> CookedAsset {
    let header = CookedHeader::new(AssetKind::from_name("test.unloadable"), 1, dependencies);
    CookedAsset { header, bytes: bytes.to_vec() }
}

/// A text asset in a cooked format its loader does not read.
pub fn stale_text(bytes: &[u8]) -> CookedAsset {
    let header = CookedHeader::new(Text::KIND, Text::FORMAT_VERSION + 1, []);
    CookedAsset { header, bytes: bytes.to_vec() }
}

/// Gated: an asset whose loader a test holds and releases by the asset's bytes, and which
/// counts its calls. Bytes that begin with `refuse` are refused as `InvalidData`, intact.
#[derive(Debug, PartialEq, Eq)]
pub struct Gated(pub Vec<u8>);

impl Asset for Gated {
    const KIND: AssetKind = AssetKind::from_name("test.gated");
    const FORMAT_VERSION: u32 = 1;
}

impl SimulationAsset for Gated {}

/// The `Gated` assets alive in this process.
pub static GATED_LIVE: AtomicU64 = AtomicU64::new(0);

impl Drop for Gated {
    fn drop(&mut self) {
        GATED_LIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

pub fn gated(bytes: &[u8], dependencies: impl IntoIterator<Item = AssetId>) -> CookedAsset {
    CookedAsset::new::<Gated>(bytes.to_vec(), dependencies)
}

#[derive(Default)]
struct GateState {
    held: BTreeSet<Vec<u8>>,
    hold_all: bool,
    /// The bytes of every call that began, in order.
    entered: Vec<Vec<u8>>,
    /// The bytes of every call that returned, in order.
    returned: Vec<Vec<u8>>,
    /// The tag and the thread's name each call ran under.
    seen: BTreeMap<Vec<u8>, (AllocTag, Option<String>)>,
}

/// The gate a [`GatedLoader`] passes through.
#[derive(Default)]
pub struct Gate {
    state: Mutex<GateState>,
    changed: Condvar,
}

impl Gate {
    fn state(&self) -> MutexGuard<'_, GateState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn update(&self, change: impl FnOnce(&mut GateState)) {
        change(&mut self.state());
        self.changed.notify_all();
    }

    pub fn hold(&self, bytes: &[u8]) {
        self.update(|state| {
            state.held.insert(bytes.to_vec());
        });
    }

    pub fn open(&self, bytes: &[u8]) {
        self.update(|state| {
            state.held.remove(bytes);
        });
    }

    pub fn hold_all(&self) {
        self.update(|state| state.hold_all = true);
    }

    pub fn open_all(&self) {
        self.update(|state| {
            state.hold_all = false;
            state.held.clear();
        });
    }

    pub fn entered(&self) -> Vec<Vec<u8>> {
        self.state().entered.clone()
    }

    pub fn returned(&self) -> Vec<Vec<u8>> {
        self.state().returned.clone()
    }

    pub fn seen(&self, bytes: &[u8]) -> Option<(AllocTag, Option<String>)> {
        self.state().seen.get(bytes).cloned()
    }

    /// Waits until a call for `bytes` has begun.
    pub fn wait_entered(&self, bytes: &[u8]) {
        self.wait_for(|state| state.entered.iter().any(|entered| entered == bytes));
    }

    /// Waits until a call for `bytes` has returned.
    pub fn wait_returned(&self, bytes: &[u8]) {
        self.wait_for(|state| state.returned.iter().any(|returned| returned == bytes));
    }

    fn wait_for(&self, mut done: impl FnMut(&GateState) -> bool) {
        let deadline = Instant::now() + LIMIT;
        let mut state = self.state();
        while !done(&state) {
            let left =
                deadline.checked_duration_since(Instant::now()).expect("a gate wait timed out");
            state =
                self.changed.wait_timeout(state, left).unwrap_or_else(PoisonError::into_inner).0;
        }
    }

    fn pass(&self, bytes: &[u8]) {
        let thread = std::thread::current().name().map(str::to_owned);
        self.update(|state| {
            state.entered.push(bytes.to_vec());
            state.seen.insert(bytes.to_vec(), (AllocTag::current(), thread));
        });
        self.wait_for(|state| !state.hold_all && !state.held.contains(bytes));
        self.update(|state| state.returned.push(bytes.to_vec()));
    }
}

#[derive(Clone)]
pub struct GatedLoader(pub Arc<Gate>);

impl Loader for GatedLoader {
    type Asset = Gated;

    fn load(&self, bytes: &[u8], _: &[AssetId]) -> Result<Gated> {
        self.0.pass(bytes);
        if bytes.starts_with(b"refuse") {
            return Err(Error::new(ErrorKind::InvalidData, "the test loader refuses these bytes"));
        }
        GATED_LIVE.fetch_add(1, Ordering::SeqCst);
        Ok(Gated(bytes.to_vec()))
    }
}

/// The loaders most tests build a store with, and the gate of the gated one.
pub fn test_loaders() -> (Vec<ErasedLoader>, Opener) {
    let gate = Arc::new(Gate::default());
    let loaders =
        vec![ErasedLoader::new(TextLoader), ErasedLoader::new(GatedLoader(Arc::clone(&gate)))];
    (loaders, Opener(gate))
}

/// The test's handle to the gate: a test that panics opens every gate as it unwinds, so
/// that its held loader calls return and the failure ends the process at once rather
/// than at a held call's bounded wait.
pub struct Opener(Arc<Gate>);

impl std::ops::Deref for Opener {
    type Target = Gate;

    fn deref(&self) -> &Gate {
        &self.0
    }
}

impl Drop for Opener {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.open_all();
        }
    }
}
