//! Settling (`README.md`, "Settling, cancellation and shutdown"), and where a load runs:
//! the memory tag and the thread a loader sees, and the store's lock never held across
//! a loader's call.

#![forbid(unsafe_code)]

mod common;

use std::future::Future;
use std::pin::{Pin, pin};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::task::{Context, Poll, Wake, Waker};

use common::*;
use lactose_asset::Settled;
use lactose_asset_api::{AssetCatalog, AssetSet, LoadPriority, Readiness};
use lactose_core::AllocTag;

const NOW: LoadPriority = LoadPriority(0);

fn poll(settled: Pin<&mut Settled>) -> Poll<()> {
    settled.poll(&mut Context::from_waker(Waker::noop()))
}

#[test]
fn settled_waits_for_the_loads_accepted_before_its_first_poll_and_no_others() {
    let env = Env::new("settling_marks");
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let (first, later) = (id("first"), id("later"));
    env.provide(&store, &[(first, gated(b"first", [])), (later, gated(b"later", []))]);
    gate.hold(b"first");
    gate.hold(b"later");
    store.request_newest(first, NOW);

    let mut settled = pin!(store.settled());
    assert_eq!(poll(settled.as_mut()), Poll::Pending);
    store.request_newest(later, NOW);
    gate.wait_entered(b"first");
    assert_eq!(poll(settled.as_mut()), Poll::Pending, "the held load has not ended");
    gate.open(b"first");
    env.wait(settled);
    assert!(store.newest(first).ready().is_some());
    assert_eq!(store.newest(later).map(|_| ()), Readiness::Pending, "still held");
    gate.open(b"later");
    env.settle(&store);
    assert!(store.newest(later).ready().is_some());
    env.shutdown();
}

#[test]
fn settled_covers_the_dependencies_and_the_loads_a_mount_accepted() {
    let env = Env::new("settling_cover");
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let (dependant, dependency) = (id("dependant"), id("dependency"));
    env.provide(
        &store,
        &[(dependant, text(b"dependant", [dependency])), (dependency, gated(b"one", []))],
    );
    gate.hold(b"one");
    store.request_newest(dependant, NOW);
    let mut settled = pin!(store.settled());
    assert_eq!(poll(settled.as_mut()), Poll::Pending);
    gate.wait_entered(b"one");
    assert_eq!(poll(settled.as_mut()), Poll::Pending);
    gate.open(b"one");
    env.wait(settled);
    assert!(store.newest(dependant).ready().is_some());

    // A mount's loads are accepted when it resolves: a `Settled` polled after waits.
    gate.hold(b"two");
    env.provide(&store, &[(dependency, gated(b"two", []))]);
    let mut settled = pin!(store.settled());
    assert_eq!(poll(settled.as_mut()), Poll::Pending);
    gate.wait_entered(b"two");
    assert_eq!(poll(settled.as_mut()), Poll::Pending);
    gate.open(b"two");
    env.wait(settled);
    let reloaded = store.newest(dependency).ready().unwrap();
    assert_eq!(reloaded.downcast::<Gated>().unwrap().0, b"two");
    env.shutdown();
}

#[test]
fn settled_resolves_after_an_executor_shutdown() {
    let env = Env::new("settling_shutdown");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let x = id("x");
    env.provide(&store, &[(x, text(b"x", []))]);
    env.executor.clone().shutdown();
    store.request_newest(x, NOW);
    let mut settled = pin!(store.settled());
    assert_eq!(poll(settled.as_mut()), Poll::Ready(()));
    env.io.clone().shutdown();
}

#[test]
fn a_loader_runs_under_the_asset_tag_on_a_background_worker() {
    let env = Env::new("settling_tags");
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let x = id("x");
    env.provide(&store, &[(x, gated(b"tagged", []))]);
    {
        let _scope = AllocTag::Render.scope();
        store.request_newest(x, NOW);
    }
    env.settle(&store);
    let (tag, thread) = gate.seen(b"tagged").expect("the loader ran");
    assert_eq!(tag, AllocTag::Asset);
    let thread = thread.expect("a named worker");
    assert!(thread.starts_with("whisk-background"), "{thread}");
    env.shutdown();
}

#[test]
fn lookups_return_while_a_loader_is_held() {
    let env = Env::new("settling_lock");
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let (held, other) = (id("held"), id("other"));
    env.provide(&store, &[(held, gated(b"held", [])), (other, text(b"other", []))]);
    store.request_newest(other, NOW);
    env.settle(&store);
    gate.hold(b"held");
    store.request_newest(held, NOW);
    gate.wait_entered(b"held");

    let (answered, answers) = mpsc::channel();
    let (returned, returns) = mpsc::channel();
    let reader = store.clone();
    std::thread::spawn(move || {
        let mut set = AssetSet::new();
        set.insert(other);
        let newest = reader.newest(held).map(|_| ());
        let resolved = reader.resolve(&set).map(|table| table.len());
        let _ = reader.generation();
        reader.request_newest(held, NOW);
        returned.send(()).unwrap();
        answered.send((newest, resolved)).unwrap();
    });
    returns_or_exit(&returns, "a lookup while a loader's call is held");
    let (newest, resolved) = answers.recv().unwrap();
    assert_eq!(newest, Readiness::Pending);
    assert_eq!(resolved, Readiness::Ready(1));
    gate.open(b"held");
    env.settle(&store);
    env.shutdown();
}

/// A waker that counts its wakes.
#[derive(Default)]
struct Counting(AtomicU64);

impl Wake for Counting {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// A `Settled` first polled while `asset`'s held load is in flight is woken by the step
/// that ends it. The executor's shutdown joins the worker that ran the step, which wakes
/// before its poll returns, so the count is read after any wake: no wait bounds it.
fn woken_when(name: &str, bytes: &'static [u8]) -> (u64, bool) {
    let env = Env::new(name);
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let x = id("x");
    env.provide(&store, &[(x, gated(bytes, []))]);
    gate.hold(bytes);
    store.request_newest(x, NOW);
    let counting = Arc::new(Counting::default());
    let waker = Waker::from(Arc::clone(&counting));
    let mut settled = pin!(store.settled());
    assert_eq!(settled.as_mut().poll(&mut Context::from_waker(&waker)), Poll::Pending);
    gate.wait_entered(bytes);
    gate.open(bytes);
    eventually("the load ended", || store.newest(x).map(|_| ()) != Readiness::Pending);
    env.executor.clone().shutdown();
    let published = store.newest(x).ready().is_some();
    let woken = counting.0.load(Ordering::SeqCst);
    assert_eq!(poll(settled.as_mut()), Poll::Ready(()));
    env.io.clone().shutdown();
    (woken, published)
}

#[test]
fn a_wait_is_woken_by_the_publish_that_ends_it() {
    assert_eq!(woken_when("settling_woken_publish", b"published"), (1, true));
}

#[test]
fn a_wait_is_woken_by_the_failure_that_ends_it() {
    assert_eq!(woken_when("settling_woken_failure", b"refuse"), (1, false));
}
