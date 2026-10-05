//! `JobContext::block_in_place`: the reservation that keeps a foreign pool and the executor
//! from oversubscribing the performance cores.

#![cfg(not(feature = "loom"))]

mod common;

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use common::{Node, executor, graph, sized, spin_for};

/// Runs a blocking "physics" node next to a node with many short units and returns the most
/// units that ran at once inside the measured part of the block, and how many ran in it.
fn measure(workers: u32, reserved: u32) -> (u32, u32) {
    let executor = executor(workers, 1);
    let measuring = Arc::new(AtomicBool::new(false));
    let blocked = Arc::new(AtomicBool::new(false));
    let active = Arc::new(AtomicU32::new(0));
    let peak = Arc::new(AtomicU32::new(0));
    let during = Arc::new(AtomicU32::new(0));
    // Under Miri time is simulated and slow; the windows shrink with the job length.
    let unit = Duration::from_micros(sized(200, 20).into());
    let settle = unit * 25;
    let window = unit * sized(100, 30);

    let physics = {
        let (measuring, blocked) = (measuring.clone(), blocked.clone());
        Node::new("block::physics")
            .run(move |_, ctx| {
                let value = ctx.block_in_place(reserved, || {
                    blocked.store(true, Ordering::Release);
                    // Jobs that were already running when the block began finish first.
                    std::thread::sleep(settle);
                    measuring.store(true, Ordering::Release);
                    std::thread::sleep(window);
                    measuring.store(false, Ordering::Release);
                    "solver result"
                });
                assert_eq!(value, "solver result");
            })
            .build()
    };
    let others = {
        let (measuring, blocked, active, peak, during) =
            (measuring.clone(), blocked.clone(), active.clone(), peak.clone(), during.clone());
        Node::new("block::others")
            // Enough units to outlast the block on every worker.
            .units(workers * sized(400, 120))
            .run(move |_, _| {
                // Keep the pool busy until the block is in place, so workers are mid-job
                // when it starts.
                while !blocked.load(Ordering::Acquire) {
                    core::hint::spin_loop();
                }
                let now = active.fetch_add(1, Ordering::AcqRel) + 1;
                let counted = measuring.load(Ordering::Acquire);
                spin_for(unit);
                if counted && measuring.load(Ordering::Acquire) {
                    peak.fetch_max(now, Ordering::AcqRel);
                    during.fetch_add(1, Ordering::Relaxed);
                }
                active.fetch_sub(1, Ordering::AcqRel);
            })
            .build()
    };
    executor.block_on(executor.run_graph(graph(vec![physics, others], &[]))).unwrap();
    executor.shutdown();
    (peak.load(Ordering::Acquire), during.load(Ordering::Relaxed))
}

#[test]
fn only_the_reserved_workers_take_jobs_during_a_block() {
    let workers = 4;
    for reserved in [0, 1, 2] {
        let (peak, during) = measure(workers, reserved);
        if cfg!(miri) {
            // Miri's clock is simulated and its scheduler may hold a thread between taking
            // a job and running it for the whole settling time, so the counts mean nothing
            // there. The run above still takes every gate path under Miri.
            continue;
        }
        assert!(peak <= reserved, "{peak} jobs ran at once with {reserved} workers reserved");
        if reserved == 0 {
            assert_eq!(during, 0, "no worker may take a job with nothing reserved");
        } else {
            assert!(during > 0, "the reserved workers must keep working");
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
/// the reserved workers, whether it parked before the block began (as here) or after.
/// Otherwise `block_in_place(1)` leaves nobody working for the whole physics step.
#[test]
fn a_parked_opener_does_not_use_up_the_reservation() {
    let executor = executor(4, 1);
    let started = Arc::new(AtomicBool::new(false));
    let measuring = Arc::new(AtomicBool::new(false));
    let during = Arc::new(AtomicU32::new(0));
    let unit = Duration::from_micros(sized(1000, 50).into());

    let opener = {
        let (started, measuring, during) = (started.clone(), measuring.clone(), during.clone());
        Node::new("block::opener")
            .run(move |_, ctx| {
                let me = ctx.slot();
                let (started, measuring, during) = (&started, &measuring, &during);
                ctx.scope(|scope| {
                    scope.fork_range(0..sized(400, 40), move |_, ctx| {
                        if ctx.slot() == me {
                            // The opener waits until a thief has part of the range, then
                            // races through its own share and parks.
                            common::wait_for(started);
                            return;
                        }
                        started.store(true, Ordering::Release);
                        spin_for(unit);
                        if measuring.load(Ordering::Acquire) {
                            during.fetch_add(1, Ordering::Relaxed);
                        }
                    });
                });
            })
            .build()
    };
    let physics = {
        let (started, measuring) = (started.clone(), measuring.clone());
        Node::new("block::after-park")
            .run(move |_, ctx| {
                common::wait_for(&started);
                // Long enough for the opener to finish its share and park on the latch.
                spin_for(unit * 5);
                ctx.block_in_place(1, || {
                    std::thread::sleep(unit * 10);
                    measuring.store(true, Ordering::Release);
                    std::thread::sleep(unit * 100);
                    measuring.store(false, Ordering::Release);
                });
            })
            .build()
    };
    executor.block_on(executor.run_graph(graph(vec![opener, physics], &[]))).unwrap();
    executor.shutdown();
    // Under Miri the clock is simulated and the count means nothing; the paths still run.
    assert!(
        cfg!(miri) || during.load(Ordering::Relaxed) > 0,
        "nothing ran during the block: the parked opener held the one reserved permit"
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
    // What the block waits for: this many child units finished while it is in place. It
    // cannot be reached unless a reserved worker runs children during the block, so a
    // regression shows as the bounded wait below running out, on any machine.
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
                let progressed = ctx.block_in_place(1, || {
                    blocked.store(true, Ordering::Release);
                    let begun = std::time::Instant::now();
                    while during.load(Ordering::Acquire) < wanted
                        && begun.elapsed() < Duration::from_secs(if cfg!(miri) { 600 } else { 20 })
                    {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    during.load(Ordering::Acquire) >= wanted
                });
                unblocked.store(true, Ordering::Release);
                assert!(progressed, "no child ran during the block with one worker reserved");
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
                            spin_for(Duration::from_micros(10));
                            let counted = blocked.load(Ordering::Acquire)
                                && !unblocked.load(Ordering::Acquire);
                            if counted {
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
}
