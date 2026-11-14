//! Cancellation by drop (`README.md`, "Settling, cancellation and shutdown"): the last
//! handle dropped while a loader's call is held. The process's allocator counts by tag,
//! which assumes one process per test, as nextest runs them.

#![forbid(unsafe_code)]

mod common;

use std::sync::atomic::Ordering;
use std::sync::mpsc;

use common::*;
use lactose_asset_api::{AssetCatalog, LoadPriority, Readiness};
use lactose_core::{AllocTag, TaggedAlloc};

#[global_allocator]
static ALLOCATOR: TaggedAlloc = TaggedAlloc::system();

#[test]
fn dropping_the_last_handle_cancels_every_load_and_never_blocks() {
    let env = Env::one_thread_each("drop_last_handle");
    // The pool keeps what its thread allocates for itself under `AllocTag::Io` at start
    // and at its first replace (the thread's own state, the directories it has swept of
    // stale temporaries) until it stops; one pack written measures that. A replace
    // completes after it frees what it allocated, so nothing of its own is in flight.
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
    // The read buffers and the packs' files, the last of them closed with the store.
    let deadline = std::time::Instant::now() + LIMIT;
    while ALLOCATOR.stats(AllocTag::Io).live_bytes != idle {
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

#[test]
fn a_mount_polled_once_and_dropped_mounts_nothing() {
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Waker};

    let env = Env::new("drop_mount");
    let (loaders, _) = test_loaders();
    let store = env.store(loaders);
    let x = id("x");
    let path = env.pack(&[(x, text(b"x", []))], lactose_asset::Storage::Raw);
    {
        let mut mount = pin!(store.mount(path));
        let poll = mount.as_mut().poll(&mut Context::from_waker(Waker::noop()));
        assert!(poll.is_pending(), "a mount resolves no sooner than its reads");
    }
    store.request_newest(x, LoadPriority::default());
    env.settle(&store);
    assert!(
        matches!(store.newest(x), Readiness::Failed(failure) if failure.kind == lactose_core::ErrorKind::NotFound)
    );
    env.shutdown();
}
