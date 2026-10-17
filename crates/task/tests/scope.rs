//! Fork-join scopes: borrowing, the own-scope-only rule, slots, arenas and tags.

#![cfg(not(feature = "loom"))]

mod common;

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{
    LIMIT, Node, Rendezvous, executor, graph, let_workers_sleep, must_finish, sized, spin_for,
    wait_for,
};
use lactose_core::AllocTag;

#[test]
fn children_borrow_from_the_openers_stack() {
    let executor = executor(sized(4, 2), 1);
    let len = sized(512, 24) as usize;
    let checked = Arc::new(AtomicU32::new(0));
    let node = {
        let checked = checked.clone();
        Node::new("scope::borrow")
            .units(sized(8, 2))
            .run(move |units, ctx| {
                // Data on this job's stack and in its scratch arena, written by children.
                let mut on_stack = vec![0u32; len];
                let in_scratch = ctx.scratch().alloc_slice_fill(len, 0u32);
                let total = AtomicUsize::new(0);
                let label = String::from("borrowed, not moved");
                ctx.scope(|scope| {
                    for (index, (a, b)) in
                        on_stack.iter_mut().zip(in_scratch.iter_mut()).enumerate()
                    {
                        let (total, label) = (&total, &label);
                        scope.fork(move |_| {
                            *a = index as u32 + 1;
                            *b = index as u32 * 2;
                            total.fetch_add(label.len(), Ordering::Relaxed);
                        });
                    }
                });
                assert!(on_stack.iter().enumerate().all(|(index, &v)| v == index as u32 + 1));
                assert!(in_scratch.iter().enumerate().all(|(index, &v)| v == index as u32 * 2));
                assert_eq!(total.into_inner(), len * label.len());

                // A second scope in the same job, this time over a range.
                let cells: Vec<AtomicU32> = (0..len).map(|_| AtomicU32::new(0)).collect();
                ctx.scope(|scope| {
                    scope.fork_range(0..len as u32, |range, _| {
                        for index in range {
                            cells[index as usize].fetch_add(1, Ordering::Relaxed);
                        }
                    });
                    scope.fork_range(7..7, |_, _| unreachable!("an empty range has no units"));
                });
                assert!(cells.iter().all(|cell| cell.load(Ordering::Relaxed) == 1));
                checked.fetch_add(units.len() as u32, Ordering::Relaxed);
            })
            .build()
    };
    executor.block_on(executor.run_graph(graph(vec![node], &[]))).unwrap();
    assert_eq!(checked.load(Ordering::Relaxed), sized(8, 2));
    executor.shutdown();
}

#[test]
fn a_scope_returns_its_bodys_value_and_drops_range_closures() {
    let executor = executor(2, 1);
    let token = Arc::new(());
    let node = {
        let token = token.clone();
        Node::new("scope::drops")
            .run(move |_, ctx| {
                let sum = AtomicU32::new(0);
                let value = ctx.scope(|scope| {
                    // The closure owns a clone; it must be dropped when the scope closes,
                    // even though the storage it lives in never runs destructors itself.
                    let held = token.clone();
                    let sum = &sum;
                    scope.fork_range(0..sized(64, 6), move |range, _| {
                        let _held = &held;
                        sum.fetch_add(range.len() as u32, Ordering::Relaxed);
                    });
                    let held = token.clone();
                    scope.fork(move |_| drop(held));
                    "body value"
                });
                assert_eq!(value, "body value");
                assert_eq!(sum.into_inner(), sized(64, 6));
                assert_eq!(Arc::strong_count(&token), 2, "one in the node, one in the test");
            })
            .build()
    };
    executor.block_on(executor.run_graph(graph(vec![node], &[]))).unwrap();
    executor.shutdown();
    assert_eq!(Arc::strong_count(&token), 1);
}

/// The case a recorded deque length gets wrong. The opener's deque holds one unrelated
/// entry when the scope opens, with the children above it. A thief takes the unrelated
/// entry from the other end, so the deque is now shorter than it was at scope open while
/// every child is still on it. The thief is then kept busy until the scope has closed, so
/// the opener has to recognise and run all of its children itself.
#[test]
fn the_opener_runs_its_children_after_thieves_drain_the_deque_below_them() {
    let executor = executor(2, 1);
    let children = sized(16, 4);
    let other_started = Arc::new(AtomicBool::new(false));
    let scope_closed = Arc::new(AtomicBool::new(false));
    let ran = Arc::new(Mutex::new(Vec::new()));

    let node = {
        let (other_started, scope_closed, ran) =
            (other_started.clone(), scope_closed.clone(), ran.clone());
        Node::new("scope::drained")
            // Two units: whichever worker takes the node splits it, leaves unit 1 on its
            // deque as the unrelated entry and runs unit 0, which opens the scope.
            .units(2)
            .run(move |range, ctx| {
                if range.start == 1 {
                    // The unrelated entry, on the thief.
                    other_started.store(true, Ordering::Release);
                    wait_for(&scope_closed);
                    return;
                }
                let slot = ctx.slot();
                ctx.scope(|scope| {
                    for child in 0..children {
                        let ran = &ran;
                        scope.fork(move |ctx| ran.lock().unwrap().push((child, ctx.slot())));
                    }
                    // Hold the scope open until the thief has taken the unrelated entry.
                    wait_for(&other_started);
                });
                scope_closed.store(true, Ordering::Release);
                let mut ran = ran.lock().unwrap();
                ran.sort_unstable();
                let expected: Vec<_> = (0..children).map(|child| (child, slot)).collect();
                assert_eq!(*ran, expected, "every child ran once, on the opener");
            })
            .build()
    };
    executor.block_on(executor.run_graph(graph(vec![node], &[]))).unwrap();
    assert!(scope_closed.load(Ordering::Acquire));
    executor.shutdown();
}

/// With one worker nothing can steal, so when the opener has run its children the next
/// entry on its deque is unrelated work. It must go back on the deque, not run inside the
/// wait.
#[test]
fn the_opener_never_runs_a_foreign_job_while_waiting() {
    let executor = executor(1, 1);
    let units = sized(32, 6);
    let in_scope = Arc::new(AtomicBool::new(false));
    let ran = Arc::new(AtomicU32::new(0));
    let unrelated_ran = Arc::new(AtomicU32::new(0));

    let unrelated = {
        let (in_scope, unrelated_ran) = (in_scope.clone(), unrelated_ran.clone());
        Node::new("scope::unrelated")
            .units(units)
            .run(move |range, _| {
                assert!(
                    !in_scope.load(Ordering::Acquire),
                    "unrelated node ran inside a scope wait"
                );
                unrelated_ran.fetch_add(range.len() as u32, Ordering::Relaxed);
            })
            .build()
    };
    let opener = {
        let (in_scope, ran) = (in_scope.clone(), ran.clone());
        Node::new("scope::opener")
            .units(units)
            .run(move |range, ctx| {
                // Other units of this node, and the unrelated node, sit on the deque below
                // the children forked here.
                assert!(
                    !in_scope.swap(true, Ordering::AcqRel),
                    "another unit ran inside a scope wait"
                );
                let children = AtomicU32::new(0);
                ctx.scope(|scope| {
                    scope.fork(|_| {
                        children.fetch_add(1, Ordering::Relaxed);
                    });
                    scope.fork_range(0..5, |range, _| {
                        children.fetch_add(range.len() as u32, Ordering::Relaxed);
                    });
                });
                assert_eq!(children.into_inner(), 6);
                in_scope.store(false, Ordering::Release);
                ran.fetch_add(range.len() as u32, Ordering::Relaxed);
            })
            .build()
    };
    // Both nodes are released together, so each is on the deque while the other runs.
    let root = Node::new("scope::root").build();
    executor
        .block_on(executor.run_graph(graph(vec![root, opener, unrelated], &[(0, 1), (0, 2)])))
        .unwrap();
    assert_eq!(ran.load(Ordering::Relaxed), units);
    assert_eq!(unrelated_ran.load(Ordering::Relaxed), units);
    executor.shutdown();
}

#[test]
fn a_slot_is_held_by_one_running_job_or_child_at_a_time() {
    let workers = sized(4, 2);
    let executor = executor(workers, 1);
    assert_eq!(executor.slot_count(), workers);
    assert_eq!(executor.frame_worker_count(), workers);
    let busy: Arc<Vec<AtomicBool>> =
        Arc::new((0..workers).map(|_| AtomicBool::new(false)).collect());
    let jobs = Arc::new(AtomicU32::new(0));

    let node = || {
        let (busy, in_finalize, jobs) = (busy.clone(), busy.clone(), jobs.clone());
        let slots = executor.slot_count();
        Node::new("scope::slots")
            .units(sized(400, 12))
            .run(move |range, ctx| {
                let slot = ctx.slot();
                assert!(slot < slots);
                let hold = |slot: u32| {
                    assert!(slot < slots);
                    assert!(!busy[slot as usize].swap(true, Ordering::AcqRel), "slot shared");
                };
                let release = |slot: u32| busy[slot as usize].store(false, Ordering::Release);

                hold(slot);
                std::thread::yield_now();
                // The opener is suspended while it waits and lends its slot to the children
                // it runs itself.
                release(slot);
                ctx.scope(|scope| {
                    scope.fork_range(0..sized(6, 3), |_, ctx| {
                        hold(ctx.slot());
                        std::thread::yield_now();
                        release(ctx.slot());
                    });
                });
                assert_eq!(ctx.slot(), slot, "a job keeps its slot across a scope");
                hold(slot);
                release(slot);
                jobs.fetch_add(range.len() as u32, Ordering::Relaxed);
            })
            .finalize(move |ctx| {
                let slot = &in_finalize[ctx.slot() as usize];
                assert!(!slot.swap(true, Ordering::AcqRel), "slot shared with finalize");
                slot.store(false, Ordering::Release);
            })
            .build()
    };
    executor.block_on(executor.run_graph(graph(vec![node(), node(), node()], &[]))).unwrap();
    assert_eq!(jobs.load(Ordering::Relaxed), 3 * sized(400, 12));
    executor.shutdown();
}

#[test]
fn job_and_child_scratch_are_separate_and_reset_between_uses() {
    let executor = executor(sized(3, 2), 1);
    let rounds = sized(200, 6);
    let node = Node::new("scope::scratch")
        .units(rounds)
        .run(|range, ctx| {
            let job_scratch = ctx.scratch();
            let job_scratch_address = core::ptr::from_ref(job_scratch).addr();
            let mine = job_scratch.alloc_slice_fill(64, range.start);
            let children = AtomicU32::new(0);
            ctx.scope(|scope| {
                for child in 0..4u32 {
                    let children = &children;
                    scope.fork(move |ctx| {
                        // A child's scratch is not the opener's, even on the opener's own
                        // worker, and filling it must not disturb the opener's data.
                        let scratch = ctx.scratch();
                        assert_ne!(core::ptr::from_ref(scratch).addr(), job_scratch_address);
                        let buffer = scratch.alloc_slice_fill(256, child);
                        buffer.iter_mut().for_each(|value| *value += 1);
                        assert!(buffer.iter().all(|&value| value == child + 1));
                        children.fetch_add(1, Ordering::Relaxed);
                    });
                }
            });
            assert_eq!(children.into_inner(), 4);
            assert!(mine.iter().all(|&value| value == range.start), "child scratch overlapped");
        })
        .finalize(|ctx| {
            // Finalize is a job too, with scratch of its own.
            assert_eq!(*ctx.scratch().alloc(7u8), 7);
        })
        .build();
    executor.block_on(executor.run_graph(graph(vec![node], &[]))).unwrap();
    executor.shutdown();
}

#[test]
fn children_inherit_the_openers_allocation_tag() {
    let executor = executor(sized(4, 2), 1);
    let tagged = Arc::new(AtomicU32::new(0));
    let node = {
        let tagged = tagged.clone();
        Node::new("scope::tags")
            .units(sized(16, 3))
            .run(move |_, ctx| {
                // A graph job starts untagged, also on a worker that just ran a tagged child.
                assert_eq!(AllocTag::current(), AllocTag::Untagged);
                let _physics = AllocTag::Physics.scope();
                ctx.scope(|scope| {
                    let tagged = &tagged;
                    scope.fork_range(0..sized(24, 4), move |range, _| {
                        assert_eq!(AllocTag::current(), AllocTag::Physics);
                        spin_for(Duration::from_micros(20));
                        tagged.fetch_add(range.len() as u32, Ordering::Relaxed);
                    });
                });
            })
            .finalize(|_| assert_eq!(AllocTag::current(), AllocTag::Untagged))
            .build()
    };
    executor.block_on(executor.run_graph(graph(vec![node], &[]))).unwrap();
    assert_eq!(tagged.load(Ordering::Relaxed), sized(16, 3) * sized(24, 4));
    executor.shutdown();
}

#[test]
fn finalize_may_open_a_scope() {
    let executor = executor(2, 1);
    let merged = Arc::new(AtomicU32::new(0));
    let node = {
        let merged = merged.clone();
        Node::new("scope::finalize")
            .units(0)
            .finalize(move |ctx| {
                let parts = [1u32, 2, 3, 4];
                let sum = AtomicU32::new(0);
                ctx.scope(|scope| {
                    for part in &parts {
                        let sum = &sum;
                        scope.fork(move |_| {
                            sum.fetch_add(*part, Ordering::Relaxed);
                        });
                    }
                });
                merged.store(sum.into_inner(), Ordering::Release);
            })
            .build()
    };
    executor.block_on(executor.run_graph(graph(vec![node], &[]))).unwrap();
    assert_eq!(merged.load(Ordering::Acquire), 10);
    executor.shutdown();
}

/// Work submitted from inside a scope must not hide the scope's children. A graph started
/// by the scope body, or by a child the opener runs itself, used to land on the opener's
/// deque above children that had not run yet; the opener then met a foreign entry first,
/// concluded none of its children were left, and parked with them underneath. With one
/// worker nobody else could ever take them.
#[test]
fn work_submitted_inside_a_scope_does_not_strand_its_children() {
    for workers in [1, 2, sized(4, 2)] {
        let executor = executor(workers, 1);
        let rounds = sized(200, 3);
        let children_ran = Arc::new(AtomicU32::new(0));
        let inner_ran = Arc::new(AtomicU32::new(0));
        let inner_node = {
            let inner_ran = inner_ran.clone();
            move || {
                let inner_ran = inner_ran.clone();
                Node::new("scope::inner")
                    .units(3)
                    .run(move |range, _| {
                        inner_ran.fetch_add(range.len() as u32, Ordering::Relaxed);
                    })
                    .build()
            }
        };
        let opener = || {
            let (handle, children_ran, inner_node) =
                (executor.clone(), children_ran.clone(), inner_node.clone());
            Node::new("scope::submits")
                .units(2)
                .run(move |_, ctx| {
                    let (handle, children_ran, inner_node) = (&handle, &children_ran, &inner_node);
                    ctx.scope(|scope| {
                        scope.fork(move |_| {
                            children_ran.fetch_add(1, Ordering::Relaxed);
                        });
                        // From the scope body: legal, and dropping the run does not
                        // cancel it.
                        drop(handle.run_graph(graph(vec![inner_node()], &[])));
                        scope.fork_range(0..4, move |range, _| {
                            // From a child, which the opener may be running itself.
                            if range.start == 0 {
                                drop(handle.run_graph(graph(vec![inner_node()], &[])));
                            }
                            children_ran.fetch_add(range.len() as u32, Ordering::Relaxed);
                        });
                        // A task woken from inside the scope goes to the injector too.
                        handle.spawn(lactose_task::WorkerClass::FrameCritical, async {}).detach();
                    });
                })
                .build()
        };
        let runner = executor.clone();
        let mut outer = graph(vec![opener(), opener()], &[]);
        must_finish("a scope whose body starts a graph (the opener is stuck)", LIMIT, move || {
            for _ in 0..rounds {
                outer = runner.block_on(runner.run_graph(outer)).unwrap();
            }
        });
        // Two openers of two units each per round, five children per scope, and two inner
        // graphs of three units per scope.
        assert_eq!(children_ran.load(Ordering::Relaxed), rounds * 4 * 5);
        let expected_inner = rounds * 4 * 2 * 3;
        let start = std::time::Instant::now();
        while inner_ran.load(Ordering::Relaxed) != expected_inner {
            assert!(start.elapsed() < Duration::from_secs(60), "an inner graph was lost");
            std::thread::yield_now();
        }
        executor.shutdown();
    }
}

/// Each child a scope forks wakes a sleeper for itself (`WorkerLocal::push`). A node released
/// while every worker sleeps opens a scope of single children, which no split can spread, and
/// the first child each worker runs waits until every worker is running one. The node wakes
/// one sleeper, its opener; the others can only be woken by the forks.
#[test]
fn forked_children_reach_every_sleeping_worker() {
    let workers = 3;
    let executor = executor(workers, 1);
    let rendezvous = Arc::new(Rendezvous::new(&executor));
    let node = {
        let rendezvous = rendezvous.clone();
        Node::new("scope::wakes")
            .run(move |_, ctx| {
                let rendezvous = &rendezvous;
                ctx.scope(|scope| {
                    for _ in 0..workers {
                        scope.fork(move |child| rendezvous.arrive(child.slot()));
                    }
                });
            })
            .build()
    };
    let_workers_sleep();
    executor.block_on(executor.run_graph(graph(vec![node], &[]))).unwrap();
    executor.shutdown();
    assert!(
        rendezvous.met(),
        "only {} of {workers} workers ran a child: a fork woke no sleeping worker",
        rendezvous.arrived()
    );
}
