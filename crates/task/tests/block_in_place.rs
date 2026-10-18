//! `JobContext::block_in_place`: the reservation that keeps a foreign pool and the executor
//! from oversubscribing the performance cores.
//!
//! These tests synchronise on events, not durations. A unit or child that begins before a
//! block holds its worker until the block is in place, so nothing counted can have started
//! early, and a block that is owed work lasts until that work has run. A slow or
//! oversubscribed machine makes them slower, never wrong: a regression these tests look for
//! shows as a bounded wait running out.

#![cfg(not(feature = "loom"))]

mod common;

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering, fence};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{LIMIT, Node, Rendezvous, executor, graph, must_finish, sized, spin_for, wait_for};
use lactose_task::{Executor, Graph};

/// Waits until `done` holds, or for [`LIMIT`], which only a regression reaches. Returns
/// whether it holds.
fn wait_until(done: impl Fn() -> bool) -> bool {
    let begun = Instant::now();
    while !done() && begun.elapsed() < LIMIT {
        std::thread::sleep(Duration::from_millis(1));
    }
    done()
}

/// Runs `graph` on `executor` to the end. A run that outlasts every wait inside it, laid end
/// to end, fails the test here: that is a gate that never reopened, or work it never let
/// start, and the test would otherwise hang until the test runner's timeout. The waits
/// inside report first.
fn run(executor: &Executor, graph: Graph) {
    let runner = executor.clone();
    must_finish("the graph run (a gate that never reopened?)", LIMIT * 4, move || {
        runner.block_on(runner.run_graph(graph)).unwrap();
    });
}

/// Marks the end of a block for counting, before the gate opens. A worker can be admitted to
/// a unit or child by a relaxed read of the reopened gate (`Gate::over_limit`), which orders
/// nothing, so without the two fences that unit may still read the flag as unset and be
/// counted as running during the block. With them, a load after the reader's fence sees a
/// store made before the writer's fence whenever the reader saw the gate open.
fn end_block(unblocked: &AtomicBool) {
    unblocked.store(true, Ordering::Release);
    fence(Ordering::SeqCst);
}

/// Whether the block had ended when the calling unit or child began. See [`end_block`].
fn block_ended(unblocked: &AtomicBool) -> bool {
    fence(Ordering::SeqCst);
    unblocked.load(Ordering::Acquire)
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
    // How many workers hold a unit begun before the block. Each holds at most one.
    let held = Arc::new(AtomicU32::new(0));

    // Released first, so a worker takes it before any unit of the other node runs: those
    // wait for the block and must not wait for the worker that would begin it. That rests on
    // the graph releasing its roots in node order into the FIFO injector. With `others`
    // taken first every worker can be holding a unit and nobody left to begin the block.
    let physics = {
        let (held, blocked, unblocked, during) =
            (held.clone(), blocked.clone(), unblocked.clone(), during.clone());
        Node::new("block::physics")
            .run(move |_, ctx| {
                // The block begins with every other worker mid-range, so each has to give
                // way inside its range and not merely be refused its next entry by a gate
                // that closed before it took one.
                assert!(
                    wait_until(|| held.load(Ordering::Acquire) >= workers - 1),
                    "only {} of {} other workers took a unit before the block: a split woke no \
                     sleeping worker",
                    held.load(Ordering::Acquire),
                    workers - 1
                );
                let value = ctx.block_in_place(reserved, || {
                    blocked.store(true, Ordering::Release);
                    if reserved == 0 {
                        // Nothing may run, so nothing to wait for. However long this is, a
                        // correct pool lets no unit start in it.
                        std::thread::sleep(Duration::from_millis(sized(20, 1).into()));
                    } else {
                        let _ = wait_until(|| during.load(Ordering::Acquire) >= WANTED);
                    }
                    end_block(&unblocked);
                    "solver result"
                });
                assert_eq!(value, "solver result");
            })
            .build()
    };
    let others = {
        let (held, blocked, unblocked, seen, active, peak, during) = (
            held.clone(),
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
                    held.fetch_add(1, Ordering::AcqRel);
                    // Twice the physics node's wait for these, which reports a block
                    // that never began for want of workers.
                    let begun = Instant::now();
                    while !blocked.load(Ordering::Acquire) {
                        assert!(
                            begun.elapsed() < LIMIT * 2,
                            "no block began: every worker holds a unit of the other node, which \
                             can only happen if the physics node was not taken first (roots \
                             released in node order into the FIFO injector)"
                        );
                        std::thread::yield_now();
                    }
                    return;
                }
                if block_ended(&unblocked)
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
    run(&executor, graph(vec![physics, others], &[]));
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

/// After a block the gate returns to its full width. The other workers were idle through
/// it, with nothing queued; what follows needs every one of them at once.
#[test]
fn the_pool_recovers_its_full_width_after_a_block() {
    let workers = 3;
    let executor = executor(workers, 1);
    let rendezvous = Arc::new(Rendezvous::new(&executor));
    let blocker = Node::new("block::first").run(|_, ctx| ctx.block_in_place(0, || ())).build();
    let after = {
        let rendezvous = rendezvous.clone();
        Node::new("block::rendezvous")
            .units(workers)
            .run(move |_, ctx| rendezvous.arrive(ctx.slot()))
            .build()
    };
    run(&executor, graph(vec![blocker, after], &[(0, 1)]));
    executor.shutdown();
    assert!(
        rendezvous.met(),
        "only {} of {workers} workers took a unit after the block: a worker stayed parked",
        rendezvous.arrived()
    );
}

/// A worker held at the gate sleeps beside work it may not take, and the gate wakes it when
/// it opens (`gate::idle_park`). Nothing else would: no work is queued after the block.
///
/// Three workers: physics, an opener whose children are single forks, so that no range
/// split wakes anyone, and a thief. The opener and the thief are each held in a child until
/// `block_in_place(0)` is in place; then both give way, the thief with the opener's
/// children in sight. After the block the first child each worker runs waits for all three,
/// and the thief can only get there if the opening woke it.
#[test]
fn a_worker_held_at_the_gate_rejoins_when_it_opens() {
    let workers = 3;
    let executor = executor(workers, 1);
    let rendezvous = Arc::new(Rendezvous::new(&executor));
    let physics_started = Arc::new(AtomicBool::new(false));
    // How many workers are held in a child begun before the block, and how many have left it.
    let inside = Arc::new(AtomicU32::new(0));
    let left = Arc::new(AtomicU32::new(0));
    let blocked = Arc::new(AtomicBool::new(false));

    let physics = {
        let (physics_started, inside, left, blocked) =
            (physics_started.clone(), inside.clone(), left.clone(), blocked.clone());
        Node::new("block::held")
            .run(move |_, ctx| {
                physics_started.store(true, Ordering::Release);
                assert!(
                    wait_until(|| inside.load(Ordering::Acquire) >= 2),
                    "the opener and a thief were never both held in a child: a fork woke no \
                     sleeping worker"
                );
                ctx.block_in_place(0, || {
                    blocked.store(true, Ordering::Release);
                    let _ = wait_until(|| left.load(Ordering::Acquire) >= 2);
                    // From leaving its child to parking, a worker runs a few microseconds of
                    // the pool's code; the pause leaves time for them on a loaded machine. A
                    // correct pool passes however short it is; a pool that wakes nobody when
                    // the gate opens fails only if the thief has parked by then.
                    std::thread::sleep(Duration::from_millis(sized(20, 1).into()));
                });
            })
            .build()
    };
    let opener = {
        let (rendezvous, physics_started) = (rendezvous.clone(), physics_started.clone());
        Node::new("block::forks")
            .run(move |_, ctx| {
                // The block's worker is taken before any child exists, so no child waits for
                // the worker that would begin it.
                wait_for(&physics_started);
                let (rendezvous, inside, left, blocked) = (&rendezvous, &inside, &left, &blocked);
                ctx.scope(|scope| {
                    for _ in 0..sized(64, 8) {
                        scope.fork(move |child| {
                            if blocked.load(Ordering::Acquire) {
                                rendezvous.arrive(child.slot());
                            } else {
                                inside.fetch_add(1, Ordering::AcqRel);
                                // Unbounded and sleeping: if the block never comes,
                                // physics' wait for this one, begun first and bounded,
                                // fails the test. A child that gave up would let the
                                // opener take another and fake the premise.
                                while !blocked.load(Ordering::Acquire) {
                                    std::thread::sleep(Duration::from_millis(1));
                                }
                                left.fetch_add(1, Ordering::AcqRel);
                            }
                        });
                    }
                });
            })
            .build()
    };
    run(&executor, graph(vec![physics, opener], &[]));
    executor.shutdown();
    assert!(
        rendezvous.met(),
        "only {} of {workers} workers ran a child after the block: one held at the gate was \
         not woken when it opened",
        rendezvous.arrived()
    );
}

/// An opener that is parked on its scope latch runs nothing, so it must not count as one of
/// the reserved workers, whether it parked before the block began or after: both orders are
/// run. Otherwise `block_in_place(1)` leaves nobody working for the whole physics step.
///
/// The opener's own share is one child, which ends once a thief holds the range; then its
/// deque is empty and it parks. The block lasts until every child that did not begin before
/// it has run, which needs the one reserved permit. An opener that parked with its permit
/// would leave none for them.
#[test]
fn a_parked_opener_does_not_use_up_the_reservation() {
    parked_opener(false);
    parked_opener(true);
}

/// With `after_block`, the opener's share lasts until the block is in place and a while
/// longer, so the thieves give way first and the opener parks last, under the closed gate:
/// the permit it gives up there must reach an idle worker.
fn parked_opener(after_block: bool) {
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
        let (physics_started, stolen, share_done, blocked, unblocked, during) = (
            physics_started.clone(),
            stolen.clone(),
            share_done.clone(),
            blocked.clone(),
            unblocked.clone(),
            during.clone(),
        );
        Node::new("block::after-park")
            .run(move |_, ctx| {
                physics_started.store(true, Ordering::Release);
                if after_block {
                    wait_for(&stolen);
                } else {
                    wait_for(&share_done);
                    // Time for the opener to find its deque empty and park.
                    std::thread::sleep(Duration::from_millis(sized(10, 1).into()));
                }
                ctx.block_in_place(1, || {
                    blocked.store(true, Ordering::Release);
                    let _ = wait_until(|| during.load(Ordering::Acquire) >= wanted);
                    end_block(&unblocked);
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
                        } else if !block_ended(unblocked) {
                            during.fetch_add(1, Ordering::AcqRel);
                        }
                    });
                    // Forked last, so the opener pops it first while a thief takes the
                    // range from the other end of its deque.
                    scope.fork(move |_| {
                        wait_for(stolen);
                        share_done.store(true, Ordering::Release);
                        if after_block {
                            wait_for(blocked);
                            std::thread::sleep(Duration::from_millis(sized(20, 1).into()));
                        }
                    });
                });
            })
            .build()
    };
    run(&executor, graph(vec![physics, opener], &[]));
    executor.shutdown();
    let ran = during.load(Ordering::Acquire);
    assert!(
        ran >= wanted,
        "{ran} of {wanted} children ran during the block, the opener parked {} it: its permit \
         did not reach a worker that could run them",
        if after_block { "inside" } else { "before" }
    );
}

/// The second review's case: every frame-critical worker other than the blocked one is
/// inside a scope, running its own children, when `block_in_place(1)` begins. The block
/// waits until both openers are held inside one of their children, so both are over the
/// limit mid-range: each finishes that child, and at most one keeps running children while
/// the block lasts. One of them must end up with the reserved permit and keep running
/// children; before the fix both parked on their latches beside their own deques and
/// nothing ran until the block ended.
#[test]
fn a_reservation_is_used_when_every_other_worker_is_an_opener() {
    let executor = executor(3, 1);
    let started = Arc::new(AtomicU32::new(0));
    // How many openers are inside one of their own children, held there until the block.
    let inside = Arc::new(AtomicU32::new(0));
    let blocked = Arc::new(AtomicBool::new(false));
    let unblocked = Arc::new(AtomicBool::new(false));
    let during = Arc::new(AtomicU32::new(0));
    let active = Arc::new(AtomicU32::new(0));
    let peak = Arc::new(AtomicU32::new(0));
    let seen: Arc<Vec<AtomicBool>> =
        Arc::new((0..executor.slot_count()).map(|_| AtomicBool::new(false)).collect());
    // What the block waits for: this many child units run while it is in place. It cannot
    // be reached unless a reserved worker runs children during the block. The longer the
    // block, the surer an opener that ignores the gate is caught running beside the other.
    let wanted = sized(400, 4);
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
        let (all_started, inside, blocked, unblocked, during) = (
            all_started.clone(),
            inside.clone(),
            blocked.clone(),
            unblocked.clone(),
            during.clone(),
        );
        Node::new("block::all-openers")
            .run(move |_, ctx| {
                all_started();
                // The block begins with both openers running their own children, not before
                // they reach them: otherwise they meet a closed gate before their first child
                // and the give-way inside a range is never exercised.
                let _ = wait_until(|| inside.load(Ordering::Acquire) >= 2);
                ctx.block_in_place(1, || {
                    blocked.store(true, Ordering::Release);
                    let _ = wait_until(|| during.load(Ordering::Acquire) >= wanted);
                    end_block(&unblocked);
                });
            })
            .build()
    };
    let opener = |name| {
        let (all_started, inside, blocked, unblocked, during, active, peak, seen) = (
            all_started.clone(),
            inside.clone(),
            blocked.clone(),
            unblocked.clone(),
            during.clone(),
            active.clone(),
            peak.clone(),
            seen.clone(),
        );
        Node::new(name)
            .run(move |_, ctx| {
                all_started();
                let (inside, blocked, unblocked, during, active, peak, seen) =
                    (&inside, &blocked, &unblocked, &during, &active, &peak, &seen);
                ctx.scope(|scope| {
                    scope.fork_range(0..children, move |range, child| {
                        for _ in range {
                            if !blocked.load(Ordering::Acquire) {
                                // Holds the opener inside its children until the block
                                // begins, so they cannot all have run before it. With no
                                // idle worker to steal, only an opener gets here, once.
                                inside.fetch_add(1, Ordering::AcqRel);
                                wait_for(blocked);
                                continue;
                            }
                            // As in `measure`: each worker's first child after the block
                            // may have been admitted before it, so it is not counted.
                            if block_ended(unblocked)
                                || !seen[child.slot() as usize].swap(true, Ordering::AcqRel)
                            {
                                continue;
                            }
                            let now = active.fetch_add(1, Ordering::AcqRel) + 1;
                            peak.fetch_max(now, Ordering::AcqRel);
                            spin_for(Duration::from_micros(50));
                            during.fetch_add(1, Ordering::AcqRel);
                            active.fetch_sub(1, Ordering::AcqRel);
                        }
                    });
                });
            })
            .build()
    };
    let nodes = vec![physics, opener("block::opener-a"), opener("block::opener-b")];
    run(&executor, graph(nodes, &[]));
    executor.shutdown();
    let ran = during.load(Ordering::Acquire);
    assert!(ran >= wanted, "{ran} of {wanted} children ran during the block with one reserved");
    let peak = peak.load(Ordering::Acquire);
    assert!(
        peak <= 1,
        "{peak} children ran at once with one worker reserved: an opener over the limit kept running its own"
    );
}
