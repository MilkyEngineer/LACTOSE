//! What runs under the store's lock, and what a drop of the last handle stops (`README.md`,
//! "Lookups" and "Settling, cancellation and shutdown").
//!
//! Code that is not the store's (a waker's wake or drop, a task's drop, an asset's drop)
//! calls the store back here, which deadlocks if that code runs under the store's lock:
//! such a call runs on a thread of its own, and a call that does not return within
//! [`STUCK`] fails the test by exiting the process, since the stuck thread would also stop
//! the test's teardown.
//!
//! A hand-written global subscriber stops a load's task at the poll that resumes after
//! the load's one yield (the first entry of its `silo.load` span after `bytes` was
//! recorded), and counts the reads made and the loads published after the test dropped its
//! last handle.

#![forbid(unsafe_code)]

mod common;

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::task::{Context, Poll, Waker};

use common::*;
use lactose_asset::{Settled, Store};
use lactose_asset_api::{
    Asset, AssetCatalog, AssetId, AssetKind, CookedAsset, ErasedLoader, LoadPriority, Loader,
    Readiness, SimulationAsset,
};
use lactose_core::Result;
use lactose_task::WorkerClass;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

const NOW: LoadPriority = LoadPriority(0);

// ---- The subscriber ----

type AssetHook = Arc<dyn Fn(&str) + Send + Sync>;

struct SpanInfo {
    name: &'static str,
    asset: String,
    bytes_recorded: bool,
    resumed: bool,
}

#[derive(Default)]
struct Hook {
    next: AtomicU64,
    spans: Mutex<HashMap<u64, SpanInfo>>,
    /// Called at the poll of a `silo.load` that resumes after its yield, with its asset.
    on_resume: Mutex<Option<AssetHook>>,
    /// Called at every `silo.publish` event, with its asset.
    on_publish: Mutex<Option<AssetHook>>,
    /// Set by a test once its last handle to the store is dropped.
    dropped: AtomicBool,
    /// `io.read` spans created once `dropped` was set: reads submitted.
    reads_after_drop: AtomicU64,
    /// Entries of `io.read` spans once `dropped` was set: reads the pool ran.
    read_entries_after_drop: AtomicU64,
    /// `silo.publish` events once `dropped` was set.
    publishes_after_drop: AtomicU64,
}

/// Reads the `asset` field, and whether `bytes` is among the fields.
struct Fields<'a> {
    asset: &'a mut String,
    bytes: bool,
}

impl Visit for Fields<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        match field.name() {
            "asset" => *self.asset = format!("{value:?}"),
            "bytes" => self.bytes = true,
            _ => {}
        }
    }

    fn record_u64(&mut self, field: &Field, _: u64) {
        if field.name() == "bytes" {
            self.bytes = true;
        }
    }
}

impl Hook {
    fn after_drop(&self, counter: &AtomicU64) {
        if self.dropped.load(Ordering::SeqCst) {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn io_reads(&self) -> usize {
        let spans = self.spans.lock().unwrap();
        spans.values().filter(|span| span.name == "io.read").count()
    }
}

impl Subscriber for &'static Hook {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, attributes: &Attributes<'_>) -> Id {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let mut asset = String::new();
        attributes.record(&mut Fields { asset: &mut asset, bytes: false });
        let name = attributes.metadata().name();
        if name == "io.read" {
            self.after_drop(&self.reads_after_drop);
        }
        let info = SpanInfo { name, asset, bytes_recorded: false, resumed: false };
        self.spans.lock().unwrap().insert(id, info);
        Id::from_u64(id)
    }

    fn record(&self, span: &Id, values: &Record<'_>) {
        let mut ignored = String::new();
        let mut fields = Fields { asset: &mut ignored, bytes: false };
        values.record(&mut fields);
        if fields.bytes
            && let Some(info) = self.spans.lock().unwrap().get_mut(&span.into_u64())
        {
            info.bytes_recorded = true;
        }
    }

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        if event.metadata().name() != "silo.publish" {
            return;
        }
        self.after_drop(&self.publishes_after_drop);
        let mut asset = String::new();
        event.record(&mut Fields { asset: &mut asset, bytes: false });
        let hook = self.on_publish.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook(&asset);
        }
    }

    fn enter(&self, span: &Id) {
        let resumed = {
            let mut spans = self.spans.lock().unwrap();
            match spans.get_mut(&span.into_u64()) {
                Some(info) if info.name == "io.read" => {
                    self.after_drop(&self.read_entries_after_drop);
                    None
                }
                Some(info) if info.name == "silo.load" && info.bytes_recorded && !info.resumed => {
                    info.resumed = true;
                    Some(info.asset.clone())
                }
                _ => None,
            }
        };
        if let Some(asset) = resumed {
            let hook = self.on_resume.lock().unwrap().clone();
            if let Some(hook) = hook {
                hook(&asset);
            }
        }
    }

    fn exit(&self, _: &Id) {}
}

fn hook() -> &'static Hook {
    static HOOK: OnceLock<&'static Hook> = OnceLock::new();
    HOOK.get_or_init(|| {
        let hook: &'static Hook = Box::leak(Box::default());
        tracing::subscriber::set_global_default(hook)
            .expect("the first subscriber of this process");
        hook
    })
}

fn hex(id: AssetId) -> String {
    format!("{:#018x}", id.to_bits())
}

/// Runs `call` on a thread of its own and fails the test if it has not returned within
/// [`STUCK`].
fn returns(what: &str, call: impl FnOnce() + Send + 'static) {
    let (done, returned) = mpsc::channel();
    std::thread::spawn(move || {
        call();
        done.send(()).unwrap();
    });
    returns_or_exit(&returned, what);
}

/// A thread stopped by a hook until the test lets it go.
struct Stopped {
    arrived: mpsc::Receiver<()>,
    go: mpsc::Sender<()>,
}

impl Stopped {
    fn wait(&self, what: &str) {
        self.arrived.recv_timeout(LIMIT).expect(what);
    }

    fn release(&self) {
        self.go.send(()).unwrap();
    }
}

/// A hook that stops the thread it runs on, for `asset`, until the test lets it go.
fn stop_at(asset: AssetId) -> (AssetHook, Stopped) {
    let (arrived, arrival) = mpsc::channel();
    let (go, going) = mpsc::channel();
    let (arrived, going) = (Mutex::new(arrived), Mutex::new(going));
    let target = hex(asset);
    let hook: AssetHook = Arc::new(move |seen: &str| {
        if seen == target {
            arrived.lock().unwrap().send(()).unwrap();
            going.lock().unwrap().recv_timeout(LIMIT).expect("the test lets the thread go");
        }
    });
    (hook, Stopped { arrived: arrival, go })
}

/// Keeps the store's state alive past its last handle's drop, as a load that ends on
/// another worker at that moment does: a load is stopped at its `silo.publish` event,
/// which its task tells while it holds the store. It takes one background worker.
fn hold_a_publish(env: &Env, store: &Store) -> Stopped {
    let keeper = id("keeper");
    env.provide(store, &[(keeper, text(b"keeper", []))]);
    let (on_publish, stopped) = stop_at(keeper);
    *hook().on_publish.lock().unwrap() = Some(on_publish);
    store.request_newest(keeper, NOW);
    stopped.wait("the keeper's publish");
    stopped
}

/// Five gated assets mounted on `store`.
fn five(env: &Env, store: &Store) -> Vec<AssetId> {
    let names = ["one", "two", "three", "four", "five"];
    let assets: Vec<_> = names.iter().map(|name| (id(name), gated(name.as_bytes(), []))).collect();
    env.provide(store, &assets);
    assets.into_iter().map(|(asset, _)| asset).collect()
}

// ---- Code that is not the store's ----

/// A waker whose `wake` calls the store, and counts its wakes.
struct WakeCallsBack {
    store: Store,
    woken: mpsc::Sender<()>,
}

impl std::task::Wake for WakeCallsBack {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let _ = self.store.generation();
        let _ = self.woken.send(());
    }
}

#[test]
fn a_wait_is_woken_outside_the_lock() {
    let env = Env::new("locking_wake");
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let x = id("x");
    env.provide(&store, &[(x, gated(b"held", []))]);
    gate.hold(b"held");
    store.request_newest(x, NOW);
    let (woken, wakes) = mpsc::channel();
    let waker = Waker::from(Arc::new(WakeCallsBack { store: store.clone(), woken }));
    let mut settled = Box::pin(store.settled());
    assert!(settled.as_mut().poll(&mut Context::from_waker(&waker)).is_pending());
    gate.open(b"held");
    returns_or_exit(&wakes, "the wake that calls the store");
    assert!(settled.as_mut().poll(&mut Context::from_waker(&waker)).is_ready());
    env.shutdown();
}

/// A waker whose last drop calls the store.
struct DropCallsBack {
    store: Store,
}

#[expect(clippy::manual_noop_waker, reason = "its drop is what the test is about")]
impl std::task::Wake for DropCallsBack {
    fn wake(self: Arc<Self>) {}
}

impl Drop for DropCallsBack {
    fn drop(&mut self) {
        let _ = self.store.generation();
    }
}

#[test]
fn a_replaced_waker_is_dropped_outside_the_lock() {
    let env = Env::new("locking_replaced_waker");
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let x = id("x");
    env.provide(&store, &[(x, gated(b"held", []))]);
    gate.hold(b"held");
    store.request_newest(x, NOW);

    let first = Waker::from(Arc::new(DropCallsBack { store: store.clone() }));
    let mut settled: Pin<Box<Settled>> = Box::pin(store.settled());
    assert!(settled.as_mut().poll(&mut Context::from_waker(&first)).is_pending());
    drop(first);
    // The store holds the only `Waker` of the first now: the second poll replaces it.
    returns("a poll that replaces a waker whose drop calls the store", move || {
        let _ = settled.as_mut().poll(&mut Context::from_waker(Waker::noop()));
    });
    gate.open(b"held");
    env.settle(&store);
    env.shutdown();
}

#[test]
fn a_task_dropped_with_its_replaced_waker_is_dropped_outside_the_lock() {
    // One worker held by the gated loader, one for the task below.
    let env = Env::with_background_workers("locking_replaced_task", 2);
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let x = id("x");
    env.provide(&store, &[(x, gated(b"held", []))]);
    gate.hold(b"held");
    store.request_newest(x, NOW);
    gate.wait_entered(b"held");

    // The task polls one `Settled` with its own waker, hands it here and suspends for
    // good holding another, whose drop takes the store's lock. Then nothing but the
    // store's map holds the task's waker, so the poll here that replaces it drops the
    // task, and with it the second `Settled`.
    let (hand, handed) = mpsc::channel::<Pin<Box<Settled>>>();
    let waiter = store.clone();
    let task = env.executor.spawn(WorkerClass::Background, async move {
        let mut second = Box::pin(waiter.settled());
        let _ = second.as_mut().poll(&mut Context::from_waker(Waker::noop()));
        let mut first = Box::pin(waiter.settled());
        std::future::poll_fn(|cx| {
            let _ = first.as_mut().poll(cx);
            Poll::Ready(())
        })
        .await;
        drop(waiter);
        hand.send(first).unwrap();
        std::future::pending::<()>().await;
        drop(second);
    });
    task.detach();
    let mut first = handed.recv_timeout(LIMIT).expect("the task handed its Settled over");
    // The task's poll has returned once the one worker it leaves free has run a task.
    env.drain(1);
    returns("a poll whose replaced waker is a task's last", move || {
        let _ = first.as_mut().poll(&mut Context::from_waker(Waker::noop()));
    });
    gate.open(b"held");
    env.settle(&store);
    env.shutdown();
}

static CALLS_ON_DROP: OnceLock<Store> = OnceLock::new();

/// An asset whose destructor reads the store's generation, as a GPU resource's might call
/// into a renderer that reads the store.
struct CallsOnDrop;

impl Asset for CallsOnDrop {
    const KIND: AssetKind = AssetKind::from_name("test.calls_on_drop");
    const FORMAT_VERSION: u32 = 1;
}

impl SimulationAsset for CallsOnDrop {}

impl Drop for CallsOnDrop {
    fn drop(&mut self) {
        if let Some(store) = CALLS_ON_DROP.get() {
            let _ = store.generation();
        }
    }
}

struct CallsOnDropLoader;

impl Loader for CallsOnDropLoader {
    type Asset = CallsOnDrop;

    fn load(&self, _: &[u8], _: &[AssetId]) -> Result<CallsOnDrop> {
        Ok(CallsOnDrop)
    }
}

/// `a` loads, but its dependency is of a kind with no loader: `a` fails for it, and the
/// asset its loader made is dropped by the step that ends it.
#[test]
fn an_asset_a_failure_lets_go_of_is_dropped_outside_the_lock() {
    let env = Env::new("locking_asset_drop");
    let loaders = vec![ErasedLoader::new(CallsOnDropLoader)];
    let store = Store::new(env.executor.clone(), env.io.clone(), loaders).unwrap();
    let (a, b) = (id("a"), id("b"));
    let cooked = CookedAsset::new::<CallsOnDrop>(b"a".to_vec(), [b]);
    env.provide(&store, &[(a, cooked), (b, unloadable(b"b", []))]);
    let _ = CALLS_ON_DROP.set(store.clone());
    let (executor, requester) = (env.executor.clone(), store.clone());
    returns("a request whose load drops an asset that calls the store", move || {
        requester.request_newest(a, NOW);
        wait_on(&executor, requester.settled(), LIMIT);
        let _ = requester.generation();
    });
    assert!(matches!(store.newest(a), Readiness::Failed(_)));
    env.shutdown();
}

// ---- The last handle's drop ----

/// A load's task is stopped at the poll that resumes after its yield, before its check,
/// while the last handle is dropped, and the store's state is kept alive by a publish
/// stopped on the other worker: no loader call begins.
#[test]
fn no_loader_call_begins_after_the_drop() {
    let hook = hook();
    let env = Env::with_background_workers("locking_check", 2);
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let x = id("x");
    env.provide(&store, &[(x, gated(b"x", []))]);
    let keeper = hold_a_publish(&env, &store);
    let (on_resume, resumed) = stop_at(x);
    *hook.on_resume.lock().unwrap() = Some(on_resume);
    store.request_newest(x, NOW);
    resumed.wait("the load resumed after its yield");
    drop(store);
    resumed.release();
    keeper.release();
    // Every worker joined: the stopped polls have returned.
    env.executor.clone().shutdown();
    assert!(gate.entered().is_empty(), "a loader call began after the drop");
    env.io.clone().shutdown();
}

/// One background worker, held by a task of the test's own while five loads are
/// requested, so that none of their tasks has been polled when the last handle is
/// dropped: none of them reads.
#[test]
fn no_unpolled_load_reads_after_the_drop() {
    let hook = hook();
    let env = Env::one_thread_each("locking_unpolled");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let assets = five(&env, &store);
    let (started, running) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let blocker = env.executor.spawn(WorkerClass::Background, async move {
        started.send(()).unwrap();
        released.recv_timeout(LIMIT).expect("the test releases the worker");
    });
    running.recv_timeout(LIMIT).expect("the blocker holds the background worker");
    let before = hook.io_reads();
    for asset in &assets {
        store.request_newest(*asset, NOW);
    }
    assert_eq!(hook.io_reads(), before, "a load's task ran while the worker was held");
    drop(store);
    hook.dropped.store(true, Ordering::SeqCst);
    release.send(()).unwrap();
    env.wait(blocker).unwrap();
    env.executor.clone().shutdown();
    assert_eq!(hook.reads_after_drop.load(Ordering::SeqCst), 0, "a read submitted after the drop");
    env.io.clone().shutdown();
}

/// The pool's one thread is held opening a FIFO (an open for reading waits for a writer),
/// five loads' reads queue behind it, and the last handle is dropped: none of them runs.
#[cfg(unix)]
#[test]
fn no_queued_read_runs_after_the_drop() {
    let hook = hook();
    let env = Env::one_thread_each("locking_queued");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let assets = five(&env, &store);
    let fifo = env.dir.join("held.fifo");
    let _ = std::fs::remove_file(&fifo);
    let made = std::process::Command::new("mkfifo").arg(&fifo).status().unwrap();
    assert!(made.success(), "mkfifo");
    // Submitted before any load's read: the pool runs its requests in order.
    let mut held = Box::pin(env.io.read_file(fifo.clone()));
    assert!(held.as_mut().poll(&mut Context::from_waker(Waker::noop())).is_pending());
    let before = hook.io_reads();
    for asset in &assets {
        store.request_newest(*asset, NOW);
    }
    // Every load's task has submitted its read once the one worker has run another task.
    env.drain(1);
    assert_eq!(hook.io_reads() - before, assets.len(), "every load's read submitted");
    drop(store);
    hook.dropped.store(true, Ordering::SeqCst);
    drop(std::fs::OpenOptions::new().write(true).open(&fifo).unwrap());
    env.wait(held).expect("the held open returned");
    // A read submitted now runs after every queued one was run or skipped.
    let after = env.dir.join("after");
    std::fs::write(&after, b"after").unwrap();
    env.wait(env.io.read_file(after)).expect("a read after the queued ones");
    let entries = hook.read_entries_after_drop.load(Ordering::SeqCst);
    assert_eq!(entries, 0, "a queued read ran after the drop");
    env.shutdown();
}

/// A loader call that passed its check before the last handle's drop may still run; its
/// result is dropped, and nothing is published, also while the store's state is kept
/// alive by a publish stopped on another worker.
#[test]
fn a_loader_call_past_its_check_at_the_drop_publishes_nothing() {
    let hook = hook();
    // One worker for the held call, one for the stopped publish, one free.
    let env = Env::with_background_workers("locking_past_check", 3);
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let x = id("x");
    env.provide(&store, &[(x, gated(b"held", []))]);
    gate.hold(b"held");
    store.request_newest(x, NOW);
    gate.wait_entered(b"held");
    let keeper = hold_a_publish(&env, &store);
    let live = GATED_LIVE.load(Ordering::SeqCst);
    drop(store);
    hook.dropped.store(true, Ordering::SeqCst);
    gate.open(b"held");
    gate.wait_returned(b"held");
    // The call's poll has ended once the two workers the stopped publish leaves free have
    // each run a task.
    env.drain(2);
    assert_eq!(hook.publishes_after_drop.load(Ordering::SeqCst), 0, "published after the drop");
    assert_eq!(GATED_LIVE.load(Ordering::SeqCst), live, "the call's asset is kept");
    keeper.release();
    env.executor.clone().shutdown();
    env.io.clone().shutdown();
}

#[test]
fn a_dropped_wait_drops_its_waker_outside_the_lock() {
    let env = Env::new("locking_dropped_wait");
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let x = id("x");
    env.provide(&store, &[(x, gated(b"held", []))]);
    gate.hold(b"held");
    store.request_newest(x, NOW);

    let first = Waker::from(Arc::new(DropCallsBack { store: store.clone() }));
    let mut settled: Pin<Box<Settled>> = Box::pin(store.settled());
    assert!(settled.as_mut().poll(&mut Context::from_waker(&first)).is_pending());
    drop(first);
    // The store holds the only `Waker` of the first now: dropping the wait drops it.
    returns("a drop of a wait whose waker's drop calls the store", move || drop(settled));
    gate.open(b"held");
    env.settle(&store);
    env.shutdown();
}

/// The calls of [`LoadsAfterTheDrop`]: each loader call says it began and waits to be let
/// go; each asset's drop says it began, and the first drop waits to be let go too.
struct AfterTheDrop {
    began: Mutex<mpsc::Sender<u8>>,
    go: Mutex<mpsc::Receiver<()>>,
    dropping: Mutex<mpsc::Sender<u8>>,
    drop_go: Mutex<mpsc::Receiver<()>>,
    first_held: AtomicBool,
}

static AFTER_THE_DROP: OnceLock<AfterTheDrop> = OnceLock::new();

/// An asset whose drop the test sees, and can hold.
struct DropSeen(u8);

impl Asset for DropSeen {
    const KIND: AssetKind = AssetKind::from_name("test.drop_seen");
    const FORMAT_VERSION: u32 = 1;
}

impl SimulationAsset for DropSeen {}

impl Drop for DropSeen {
    fn drop(&mut self) {
        let calls = AFTER_THE_DROP.get().expect("the test's channels");
        calls.dropping.lock().unwrap().send(self.0).unwrap();
        if calls.first_held.swap(false, Ordering::SeqCst) {
            calls.drop_go.lock().unwrap().recv_timeout(LIMIT).expect("the drop let go");
        }
    }
}

struct LoadsAfterTheDrop;

impl Loader for LoadsAfterTheDrop {
    type Asset = DropSeen;

    fn load(&self, bytes: &[u8], _: &[AssetId]) -> Result<DropSeen> {
        let calls = AFTER_THE_DROP.get().expect("the test's channels");
        calls.began.lock().unwrap().send(bytes[0]).unwrap();
        calls.go.lock().unwrap().recv_timeout(LIMIT).expect("the call let go");
        Ok(DropSeen(bytes[0]))
    }
}

/// Two loader calls past their check at the last handle's drop, while the store's state
/// is kept alive: the first call's asset is dropped unpublished and its drop is held, and
/// the second call's end still takes the store's lock, so that drop ran after it.
#[test]
fn a_result_dropped_after_the_drop_is_dropped_outside_the_lock() {
    // One worker for each held call, one for the stopped publish.
    let env = Env::with_background_workers("locking_result_after_drop", 3);
    let (began, beginnings) = mpsc::channel();
    let (go, going) = mpsc::channel();
    let (dropping, drops) = mpsc::channel();
    let (drop_go, drop_going) = mpsc::channel();
    let calls = AfterTheDrop {
        began: Mutex::new(began),
        go: Mutex::new(going),
        dropping: Mutex::new(dropping),
        drop_go: Mutex::new(drop_going),
        first_held: AtomicBool::new(true),
    };
    assert!(AFTER_THE_DROP.set(calls).is_ok(), "one test of this loader per process");
    let (loaders, _) = test_loaders();
    let loaders = loaders.into_iter().chain([ErasedLoader::new(LoadsAfterTheDrop)]).collect();
    let store = env.store(loaders);
    let (x, y) = (id("x"), id("y"));
    let seen = |bytes: &[u8]| CookedAsset::new::<DropSeen>(bytes.to_vec(), []);
    env.provide(&store, &[(x, seen(b"x")), (y, seen(b"y"))]);
    store.request_newest(x, NOW);
    store.request_newest(y, NOW);
    let mut entered = [0; 2].map(|_| beginnings.recv_timeout(LIMIT).expect("a call began"));
    entered.sort_unstable();
    assert_eq!(entered, [b'x', b'y']);
    let keeper = hold_a_publish(&env, &store);
    drop(store);

    // The call let go first ends first, and its asset's drop is held.
    go.send(()).unwrap();
    let first = drops.recv_timeout(LIMIT).expect("the first result dropped");
    go.send(()).unwrap();
    let second = drops.recv_timeout(STUCK);
    drop_go.send(()).unwrap();
    let Ok(second) = second else {
        eprintln!(
            "the second call's end never returned within {STUCK:?}: it is stuck on the store's lock"
        );
        std::process::exit(101);
    };
    let mut dropped = [first, second];
    dropped.sort_unstable();
    assert_eq!(dropped, [b'x', b'y'], "both results dropped, neither published");
    keeper.release();
    env.executor.clone().shutdown();
    env.io.clone().shutdown();
}

/// A subscriber is told of a publish after the step that publishes released the lock: a
/// lookup returns while the subscriber's callback is held.
#[test]
fn a_lookup_returns_while_the_subscriber_is_told_of_a_publish() {
    let env = Env::new("locking_subscriber");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let keeper = hold_a_publish(&env, &store);
    let reader = store.clone();
    returns("a lookup while the subscriber is told of a publish", move || {
        let _ = reader.generation();
    });
    keeper.release();
    env.settle(&store);
    env.shutdown();
}
