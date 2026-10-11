//! `JobContext::block_in_place`: the reservation that keeps a foreign pool and the executor
//! from oversubscribing the performance cores.
//!
//! These tests synchronise on events, not durations. A unit or child that begins before a
//! block holds its worker until the block is in place, so nothing counted can have started
//! early, and a block that is owed work lasts until that work has run. A slow or
//! oversubscribed machine makes them slower, never wrong: a regression shows as a bounded
//! wait running out.

#![cfg(not(feature = "loom"))]

mod common;

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Node, executor, graph, sized, spin_for, wait_for};

/// Waits until `done` holds, or for a limit only a regression reaches. Miri's clock runs
/// with the interpreter, hence its longer limit.
fn wait_until(done: impl Fn() -> bool) {
    let limit = Duration::from_secs(if cfg!(miri) { 600 } else { 60 });
    let begun = Instant::now();
    while !done() && begun.elapsed() < limit {
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// How many units a block with workers reserved waits for: far fewer than are left to run.
const WANTED: u32 = sized(200, 4);

/// Runs a blocking "physics" node next to a node with many short units. Returns the most
/// units that ran at once among those the block let start, and how many it let start.
///
/// A unit a worker begins after the block is in place may still be one it was committed to
/// before: the check that admits a unit and the unit itself are not one step. So the first
/// such unit of each worker is not counted. Every later one passed its check with the gate
/// closed, and of those at most `reserved` may run at once.
fn measure(workers: u32, reserved: u32) -> (u32, u32) {
    let executor = executor(workers, 1);
    let blocked = Arc::new(AtomicBool::new(false));
    let unblocked = Arc::new(AtomicBool::new(false));
    let seen: Arc<Vec<AtomicBool>> =
        Arc::new((0..executor.slot_count()).map(|_| AtomicBool::new(false)).collect());
    let active = Arc::new(AtomicU32::new(0));
    let peak = Arc::new(AtomicU32::new(0));
    let during = Arc::new(AtomicU32::new(0));

    // Released first, so a worker takes it before any unit of the other node runs: those
    // wait for the block and must not wait for the worker that would begin it.
    let physics = {
        let (blocked, unblocked, during) = (blocked.clone(), unblocked.clone(), during.clone());
        Node::new("block::physics")
            .run(move |_, ctx| {
                let value = ctx.block_in_place(reserved, || {
                    blocked.store(true, Ordering::Release);
                    if reserved == 0 {
                        // Nothing may run, so nothing to wait for. However long this is, a
                        // correct pool lets no unit start in it.
                        std::thread::sleep(Duration::from_millis(sized(20, 1).into()));
                    } else {
                        wait_until(|| during.load(Ordering::Acquire) >= WANTED);
                    }
                    unblocked.store(true, Ordering::Release);
                    "solver result"
                });
                assert_eq!(value, "solver result");
            })
            .build()
    };
    let others = {
        let (blocked, unblocked, seen, active, peak, during) = (
            blocked.clone(),
            unblocked.clone(),
            seen.clone(),
            active.clone(),
            peak.clone(),
            during.clone(),
        );
        Node::new("block::others")
            .units(workers * sized(400, 6))
            .run(move |_, ctx| {
                if !blocked.load(Ordering::Acquire) {
                    // Holds the worker mid-job until the block is in place.
                    wait_for(&blocked);
                    return;
                }
                if unblocked.load(Ordering::Acquire)
                    || !seen[ctx.slot() as usize].swap(true, Ordering::AcqRel)
                {
                    return;
                }
                let now = active.fetch_add(1, Ordering::AcqRel) + 1;
                peak.fetch_max(now, Ordering::AcqRel);
                during.fetch_add(1, Ordering::AcqRel);
                spin_for(Duration::from_micros(50));
                active.fetch_sub(1, Ordering::AcqRel);
            })
            .build()
    };
    executor.block_on(executor.run_graph(graph(vec![physics, others], &[]))).unwrap();
    executor.shutdown();
    (peak.load(Ordering::Acquire), during.load(Ordering::Acquire))
}

#[test]
fn only_the_reserved_workers_take_jobs_during_a_block() {
    let workers = 4;
    for reserved in [0, 1, 2] {
        let (peak, during) = measure(workers, reserved);
        assert!(peak <= reserved, "{peak} jobs ran at once with {reserved} workers reserved");
        if reserved == 0 {
            assert_eq!(during, 0, "no worker may take a job with nothing reserved");
        } else {
            assert!(during >= WANTED, "{during} of {WANTED} units ran with {reserved} reserved");
        }
    }
}

#[test]
fn the_pool_recovers_its_full_width_after_a_block() {
    let workers = 3;
    let executor = executor(workers, 1);
    let blocker = Node::new("block::first").run(|_, ctx| ctx.block_in_place(0, || ())).build();
    // After the block, a node that needs every worker at once to finish: each unit waits
    // until all of them have started.
    let started = Arc::new(AtomicU32::new(0));
    let rendezvous = {
        let started = started.clone();
        Node::new("block::rendezvous")
            .units(workers)
            .run(move |_, _| {
                started.fetch_add(1, Ordering::AcqRel);
                let begin = std::time::Instant::now();
                while started.load(Ordering::Acquire) < workers {
                    assert!(begin.elapsed() < Duration::from_secs(60), "a worker stayed parked");
                    std::thread::yield_now();
                }
            })
            .build()
    };
    executor.block_on(executor.run_graph(graph(vec![blocker, rendezvous], &[(0, 1)]))).unwrap();
    executor.shutdown();
}

/// An opener that is parked on its scope latch runs nothing, so it must not count as one of
/// the reserved workers, whether it parked before the block began (as here, nearly always)
/// or after. Otherwise `block_in_place(1)` leaves nobody working for the whole physics step.
///
/// The opener's own share is one child, which ends once a thief holds the range; then its
/// deque is empty and it parks. The block lasts until every child that did not begin before
/// it has run, which needs the one reserved permit. An opener that parked with its permit
/// would leave none for them.
#[test]
fn a_parked_opener_does_not_use_up_the_reservation() {
    let executor = executor(4, 1);
    let physics_started = Arc::new(AtomicBool::new(false));
    let stolen = Arc::new(AtomicBool::new(false));
    let share_done = Arc::new(AtomicBool::new(false));
    let blocked = Arc::new(AtomicBool::new(false));
    let unblocked = Arc::new(AtomicBool::new(false));
    let during = Arc::new(AtomicU32::new(0));
    let children = sized(400, 8);
    // A child that begins before the block holds its worker until the block begins, so at
    // most one per worker other than the blocked one is not counted.
    let wanted = children - 3;

    let physics = {
        let (physics_started, share_done, blocked, unblocked, during) = (
            physics_started.clone(),
            share_done.clone(),
            blocked.clone(),
            unblocked.clone(),
            during.clone(),
        );
        Node::new("block::after-park")
            .run(move |_, ctx| {
                physics_started.store(true, Ordering::Release);
                wait_for(&share_done);
                // Time for the opener to find its deque empty and park. If it is later than
                // this, it gives its permit up inside the block instead.
                std::thread::sleep(Duration::from_millis(sized(10, 1).into()));
                ctx.block_in_place(1, || {
                    blocked.store(true, Ordering::Release);
                    wait_until(|| during.load(Ordering::Acquire) >= wanted);
                    unblocked.store(true, Ordering::Release);
                });
            })
            .build()
    };
    let opener = {
        let (physics_started, stolen, share_done, blocked, unblocked, during) = (
            physics_started.clone(),
            stolen.clone(),
            share_done.clone(),
            blocked.clone(),
            unblocked.clone(),
            during.clone(),
        );
        Node::new("block::opener")
            .run(move |_, ctx| {
                // The block's worker is taken before any child exists, so children that
                // wait for the block never wait for the worker that would begin it.
                wait_for(&physics_started);
                let (stolen, share_done, blocked, unblocked, during) =
                    (&stolen, &share_done, &blocked, &unblocked, &during);
                ctx.scope(|scope| {
                    scope.fork_range(0..children, move |_, _| {
                        if !blocked.load(Ordering::Acquire) {
                            stolen.store(true, Ordering::Release);
                            wait_for(blocked);
                        } else if !unblocked.load(Ordering::Acquire) {
                            during.fetch_add(1, Ordering::AcqRel);
                        }
                    });
                    // Forked last, so the opener pops it first while a thief takes the
                    // range from the other end of its deque.
                    scope.fork(move |_| {
                        wait_for(stolen);
                        share_done.store(true, Ordering::Release);
                    });
                });
            })
            .build()
    };
    executor.block_on(executor.run_graph(graph(vec![physics, opener], &[]))).unwrap();
    executor.shutdown();
    let ran = during.load(Ordering::Acquire);
    assert!(
        ran >= wanted,
        "{ran} of {wanted} children ran during the block: the parked opener held the one \
         reserved permit"
    );
}

/// The second review's case: every frame-critical worker other than the blocked one is
/// inside a scope, running its own children, when `block_in_place(1)` begins. Both openers
/// are over the limit and both give way. One of them must end up with the reserved permit
/// and keep running children; before the fix both parked on their latches beside their own
/// deques and nothing ran until the block ended.
#[test]
fn a_reservation_is_used_when_every_other_worker_is_an_opener() {
    let executor = executor(3, 1);
    let started = Arc::new(AtomicU32::new(0));
    let blocked = Arc::new(AtomicBool::new(false));
    let unblocked = Arc::new(AtomicBool::new(false));
    let during = Arc::new(AtomicU32::new(0));
    // What the block waits for: this many child units run while it is in place. It cannot
    // be reached unless a reserved worker runs children during the block.
    let wanted = sized(200, 4);
    let children = sized(40_000, 400);

    let all_started = {
        let started = started.clone();
        move || {
            started.fetch_add(1, Ordering::AcqRel);
            while started.load(Ordering::Acquire) < 3 {
                core::hint::spin_loop();
            }
        }
    };
    let physics = {
        let (all_started, blocked, unblocked, during) =
            (all_started.clone(), blocked.clone(), unblocked.clone(), during.clone());
        Node::new("block::all-openers")
            .run(move |_, ctx| {
                all_started();
                ctx.block_in_place(1, || {
                    blocked.store(true, Ordering::Release);
                    wait_until(|| during.load(Ordering::Acquire) >= wanted);
                    unblocked.store(true, Ordering::Release);
                });
            })
            .build()
    };
    let opener = |name| {
        let (all_started, blocked, unblocked, during) =
            (all_started.clone(), blocked.clone(), unblocked.clone(), during.clone());
        Node::new(name)
            .run(move |_, ctx| {
                all_started();
                let (blocked, unblocked, during) = (&blocked, &unblocked, &during);
                ctx.scope(|scope| {
                    scope.fork_range(0..children, move |range, _| {
                        for _ in range {
                            if !blocked.load(Ordering::Acquire) {
                                // Holds the opener inside its children until the block
                                // begins, so they cannot all have run before it.
                                wait_for(blocked);
                                continue;
                            }
                            spin_for(Duration::from_micros(10));
                            if !unblocked.load(Ordering::Acquire) {
                                during.fetch_add(1, Ordering::AcqRel);
                            }
                        }
                    });
                });
            })
            .build()
    };
    let nodes = vec![physics, opener("block::opener-a"), opener("block::opener-b")];
    executor.block_on(executor.run_graph(graph(nodes, &[]))).unwrap();
    executor.shutdown();
    let ran = during.load(Ordering::Acquire);
    assert!(ran >= wanted, "{ran} of {wanted} children ran during the block with one reserved");
}
