//! What the store's tests share: an executor and a pool, a directory of packs under
//! `CARGO_TARGET_TMPDIR`, waits bounded so that a hang fails instead, and test asset kinds,
//! among them one whose loader a test holds and releases by name.

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
use lactose_task::{Executor, Sleep};

/// The longest any test waits for something that should happen: past it, the wait fails.
pub const LIMIT: Duration = Duration::from_secs(60);

/// An executor, a pool and a directory of packs of the test's own.
pub struct Env {
    pub executor: Executor,
    pub io: Io,
    pub dir: PathBuf,
    packs: AtomicU64,
}

impl Env {
    pub fn new(name: &str) -> Self {
        Self::build(name, Executor::builder(), None)
    }

    /// An executor with exactly `count` background workers.
    pub fn with_background_workers(name: &str, count: u32) -> Self {
        Self::build(name, Executor::builder().background_workers(count), None)
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
        Self { executor, io, dir, packs: AtomicU64::new(0) }
    }

    pub fn store(&self, loaders: Vec<ErasedLoader>) -> Store {
        Store::new(self.executor.clone(), self.io.clone(), loaders).expect("one loader per kind")
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

    /// Drives `future` to its end on this thread, failing the test if that takes longer
    /// than [`LIMIT`].
    pub fn wait<F: Future>(&self, future: F) -> F::Output {
        let deadline = self.executor.sleep_until(Instant::now() + LIMIT);
        self.executor.block_on(Bounded { future: Box::pin(future), deadline })
    }

    pub fn shutdown(self) {
        self.io.shutdown();
        self.executor.shutdown();
    }
}

struct Bounded<F> {
    future: Pin<Box<F>>,
    deadline: Sleep,
}

impl<F: Future> Future for Bounded<F> {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        if let Poll::Ready(output) = self.future.as_mut().poll(cx) {
            return Poll::Ready(output);
        }
        if Pin::new(&mut self.deadline).poll(cx).is_ready() {
            panic!("a wait took longer than {LIMIT:?}");
        }
        Poll::Pending
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
pub fn test_loaders() -> (Vec<ErasedLoader>, Arc<Gate>) {
    let gate = Arc::new(Gate::default());
    let loaders =
        vec![ErasedLoader::new(TextLoader), ErasedLoader::new(GatedLoader(Arc::clone(&gate)))];
    (loaders, gate)
}
