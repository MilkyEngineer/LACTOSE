//! Rapier's parallel work runs on the pool the backend is handed and nowhere else (architecture
//! section 16). Off that pool, rayon starts its global pool, sized by the core count. The
//! boundary lint file bans the routes to it that it names, but not every one: Rapier's and
//! parry's public functions that run parallel work outside `PhysicsPipeline::step`, beyond
//! those listed, and stepping a world without a pool (the backend's own
//! `set_thread_pool(None)` among them) are not banned, so this test holds the backend at run
//! time.
//!
//! A backend's life (a pile above the narrow phase's broadcast threshold, stepped,
//! snapshotted and restored into a second backend that steps on) leaves rayon's global pool
//! unbuilt, so building it afterwards succeeds. The control shows the detector sees a started
//! pool. Each test needs a process of its own, which nextest gives it.

#![forbid(unsafe_code)]

mod support;

use support::*;

/// Steps, snapshots and restores two backends on pools of their own.
fn life() {
    let mut first = backend(2);
    Pile::new(150).create(&mut *first, 7);
    for tick in 1..=90 {
        step(&mut *first, tick, &[]);
    }
    let bytes = snapshot(&*first);
    let mut second = backend(3);
    second.restore(&bytes).unwrap();
    for tick in 91..=120 {
        step(&mut *second, tick, &[]);
    }
}

/// Builds rayon's global pool, which succeeds only if nothing in this process started it.
#[expect(
    clippy::disallowed_methods,
    reason = "the test builds the global pool to learn whether anything built it before"
)]
fn global_pool_was_unbuilt() -> bool {
    rayon::ThreadPoolBuilder::new().num_threads(1).build_global().is_ok()
}

#[test]
fn the_backend_never_starts_rayons_global_pool() {
    life();
    assert!(global_pool_was_unbuilt());
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the control asks rayon's global pool for its size, which starts it"
)]
fn the_detector_sees_a_started_global_pool() {
    life();
    assert!(rayon::current_num_threads() > 0);
    assert!(!global_pool_was_unbuilt());
}
