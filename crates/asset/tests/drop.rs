//! Cancellation by drop (`README.md`, "Settling, cancellation and shutdown"): the last
//! handle dropped while a loader's call is held. The process's allocator counts by tag,
//! which assumes one process per test, as nextest runs them.

#![forbid(unsafe_code)]

mod common;

use std::future::Future;
use std::pin::pin;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::task::{Context, Poll, Waker};

use common::*;
use lactose_asset_api::{AssetCatalog, LoadPriority, Readiness};
use lactose_core::{AllocTag, ErrorKind, TaggedAlloc};

#[global_allocator]
static ALLOCATOR: TaggedAlloc = TaggedAlloc::system();

#[test]
fn dropping_the_last_handle_cancels_every_load_and_never_blocks() {
    let env = Env::one_thread_each("drop_last_handle");
    // The baseline of `AllocTag::Io` is the pool's swept-directory set: one entry per
    // directory replaced into, kept under `AllocTag::Io` until the pool stops. Every pack
    // of this test is written into one directory, so one pack written before the
    // measurement adds that entry. A replace completes after it frees what else it
    // allocated, so nothing of its own is in flight.
    env.pack(&[(id("warm"), text(b"warm", []))], lactose_asset::Storage::Raw);
    let idle = ALLOCATOR.stats(AllocTag::Io).live_bytes;
    let (loaders, gate) = test_loaders();
    let store = env.store(loaders);
    let kept = id("kept");
    env.provide(&store, &[(kept, text(b"read before the drop", []))]);
    store.request_newest(kept, LoadPriority::default());
    env.settle(&store);
    let handle = store.newest(kept).ready().expect("published");

    let names = ["one", "two", "three", "four"];
    let assets: Vec<_> = names.iter().map(|name| (id(name), gated(name.as_bytes(), []))).collect();
    env.provide(&store, &assets);
    gate.hold_all();
    for (asset, _) in &assets {
        store.request_newest(*asset, LoadPriority::default());
    }
    eventually("the first loader call began", || gate.entered().len() == 1);

    let (dropped, returned) = mpsc::channel();
    std::thread::spawn(move || {
        drop(store);
        dropped.send(()).unwrap();
    });
    returned.recv_timeout(LIMIT).expect("the drop returned while a loader was held");

    let held = gate.entered()[0].clone();
    gate.open_all();
    gate.wait_returned(&held);
    eventually("the held call's asset dropped", || GATED_LIVE.load(Ordering::SeqCst) == 0);
    // The read buffers and the packs' files, the last of them closed with the store. At
    // most `idle`, not exactly it: the baseline may include a block still in flight then.
    // On macOS a `std::sync::Mutex` allocates at its first lock, under the locking thread's
    // tag, so a request's completion first locked by the pool thread is charged to `Io`
    // until the pool drops the request, which may be after `idle` was read.
    let deadline = std::time::Instant::now() + LIMIT;
    while ALLOCATOR.stats(AllocTag::Io).live_bytes > idle {
        let stats = ALLOCATOR.stats(AllocTag::Io);
        assert!(std::time::Instant::now() < deadline, "Io holds {stats:?}, {idle} bytes idle");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    // Every worker joined: whatever was queued has been dropped or run.
    env.executor.clone().shutdown();
    assert_eq!(gate.entered(), [held], "no other loader call began");
    assert_eq!(GATED_LIVE.load(Ordering::SeqCst), 0);
    let text = handle.downcast::<Text>().expect("a text asset");
    assert_eq!(text.0, b"read before the drop");
    env.io.clone().shutdown();
}

/// The store's pool is held, so the first poll leaves the open queued: a pool fast enough
/// to finish the open and both reads within that poll would resolve the mount there.
#[test]
fn a_mount_polled_once_and_dropped_mounts_nothing() {
    let env = Env::new("drop_mount");
    let (loaders, _) = test_loaders();
    let store = env.held_store(loaders);
    let (x, y) = (id("x"), id("y"));
    let path = env.pack(&[(x, text(b"x", []))], lactose_asset::Storage::Raw);
    {
        let mut mount = pin!(store.mount(path));
        let poll = mount.as_mut().poll(&mut Context::from_waker(Waker::noop()));
        assert!(poll.is_pending(), "the open is queued on the held pool");
    }
    store.release();
    // The pool's one thread runs its queue in the order the futures were first polled
    // (`crates/io/README.md`, "Requests and futures" and "The pool"): once this mount has
    // resolved, it has passed the dropped mount's open, which it skips as cancelled.
    env.provide(&store, &[(y, text(b"y", []))]);
    store.request_newest(x, LoadPriority::default());
    store.request_newest(y, LoadPriority::default());
    env.settle(&store);
    assert!(
        matches!(store.newest(x), Readiness::Failed(failure) if failure.kind == ErrorKind::NotFound)
    );
    assert!(store.newest(y).ready().is_some());
    env.shutdown();
}

/// On a pool left to run, a mount's first poll resolves it if the open and both reads end
/// within that poll; the pack is then mounted, as by any mount that resolved `Ok`.
/// Otherwise the drop mounts nothing, then or later: the check waits for another mount
/// resolved after it on the pool's one thread.
#[test]
fn a_mount_dropped_after_one_poll_mounts_its_pack_only_if_that_poll_resolved_it() {
    let env = Env::one_thread_each("drop_mount_running_pool");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let (x, y) = (id("x"), id("y"));
    let path = env.pack(&[(x, text(b"x", []))], lactose_asset::Storage::Raw);
    let resolved = {
        let mut mount = pin!(store.mount(path));
        match mount.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(result) => {
                result.expect("the pack mounted");
                true
            }
            Poll::Pending => false,
        }
    };
    env.provide(&store, &[(y, text(b"y", []))]);
    store.request_newest(x, LoadPriority::default());
    store.request_newest(y, LoadPriority::default());
    env.settle(&store);
    assert!(store.newest(y).ready().is_some());
    if resolved {
        let handle = store.newest(x).ready().expect("the mounted pack's version");
        assert_eq!(handle.downcast::<Text>().expect("a text asset").0, b"x");
    } else {
        assert!(
            matches!(store.newest(x), Readiness::Failed(failure) if failure.kind == ErrorKind::NotFound)
        );
    }
    env.shutdown();
}
