//! Stress: steal, split, scope and wake paths hammered on real hardware.
//!
//! loom checks the latch and the sleep protocol but does not model weakly ordered loads, and
//! does not model the deque at all. This test runs the whole executor under contention on
//! the host, which matters most on aarch64, where stores and loads really do reorder. Any
//! lost unit, double run, lost wake-up or early finalize fails an exact count or hangs the
//! test.
//!
//! It runs for two seconds by default. Set `WHISK_STRESS_SECONDS` for a soak:
//!
//! ```text
//! WHISK_STRESS_SECONDS=600 cargo nextest run -p lactose-task --release --test stress
//! ```

#![cfg(not(feature = "loom"))]

mod common;

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Node, graph, sized};
use lactose_task::{Executor, Graph, WorkerClass, yield_now};

fn duration() -> Duration {
    let seconds = std::env::var("WHISK_STRESS_SECONDS")
        .ok()
        .map(|value| value.parse().expect("WHISK_STRESS_SECONDS must be a number of seconds"));
    match seconds {
        Some(seconds) => Duration::from_secs(seconds),
        // Under Miri one pass through every path is the point; the loops below run at
        // least once.
        None if cfg!(miri) => Duration::ZERO,
        None => Duration::from_secs(2),
    }
}

/// One graph of `width` fan-out nodes between a root and a join. Every unit adds to a
/// per-run sum and some open a scope; the join's finalize checks the sum exactly.
fn frame_graph(width: u32, units: u32, runs: &Arc<AtomicU64>) -> Graph {
    let sum = Arc::new(AtomicU64::new(0));
    let finalized = Arc::new(AtomicU32::new(0));
    let mut nodes = vec![Node::new("stress::root").units(0).build()];
    let mut edges = Vec::new();
    for index in 0..width {
        let (sum, finalized) = (sum.clone(), finalized.clone());
        nodes.push(
            Node::new("stress::fan")
                // Mixed sizes, including empty and single-unit nodes.
                .units((index * 7919) % (units + 1))
                .run(move |range, ctx| {
                    for unit in range {
                        if unit % 5 == 0 {
                            // Fork-join over a stack buffer: steal, split and latch.
                            let mut parts = [0u64; 12];
                            let extra = AtomicU64::new(0);
                            ctx.scope(|scope| {
                                for (child, part) in parts.iter_mut().enumerate() {
                                    scope.fork(move |_| *part = child as u64);
                                }
                                scope.fork_range(0..9, |range, _| {
                                    extra.fetch_add(range.len() as u64, Ordering::Relaxed);
                                });
                            });
                            assert_eq!(parts.iter().sum::<u64>() + extra.into_inner(), 66 + 9);
                        }
                        sum.fetch_add(u64::from(unit) + 1, Ordering::Relaxed);
                    }
                })
                .finalize(move |_| {
                    finalized.fetch_add(1, Ordering::Relaxed);
                })
                .build(),
        );
        edges.push((0, index + 1));
        edges.push((index + 1, width + 1));
    }
    let expected: u64 = (0..width)
        .map(|index| {
            let units = u64::from((index * 7919) % (units + 1));
            units * (units + 1) / 2
        })
        .sum();
    let runs = runs.clone();
    nodes.push(
        Node::new("stress::join")
            .finalize(move |_| {
                assert_eq!(sum.swap(0, Ordering::AcqRel), expected, "a unit was lost or run twice");
                assert_eq!(finalized.swap(0, Ordering::AcqRel), width, "a finalize was lost");
                runs.fetch_add(1, Ordering::Relaxed);
            })
            .build(),
    );
    graph(nodes, &edges)
}

#[test]
fn steal_split_scope_and_wake_paths_hold_under_contention() {
    // The machine's own topology: on a hybrid CPU that is every performance core.
    let executor = if cfg!(miri) {
        Executor::builder().frame_workers(3).background_workers(1).build().unwrap()
    } else {
        Executor::builder().build().unwrap()
    };
    let deadline = Instant::now() + duration();
    let stop = Arc::new(AtomicBool::new(false));
    let runs = Arc::new(AtomicU64::new(0));

    // Several tick loops at once, each running its own graph back to back, so nodes of
    // different runs interleave on the same deques.
    let loops: Vec<_> = (0..sized(3, 2))
        .map(|index| {
            let (runner, stop, runs) = (executor.clone(), stop.clone(), runs.clone());
            let mut graph = frame_graph(sized(24, 4) + index, sized(96, 7), &runs);
            executor.spawn(WorkerClass::FrameCritical, async move {
                loop {
                    graph = runner.run_graph(graph).await;
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                }
            })
        })
        .collect();

    // Wake-path traffic on both classes: tasks that yield, sleep on short timers and
    // finish, so workers keep going to sleep and being woken while graphs run.
    let pings = Arc::new(AtomicU64::new(0));
    let chatter: Vec<_> = [WorkerClass::FrameCritical, WorkerClass::Background]
        .into_iter()
        .map(|class| {
            let (runner, stop, pings) = (executor.clone(), stop.clone(), pings.clone());
            executor.spawn(class, async move {
                loop {
                    let burst: Vec<_> = (0..sized(8, 2))
                        .map(|_| {
                            let pings = pings.clone();
                            runner.spawn(class, async move {
                                yield_now().await;
                                pings.fetch_add(1, Ordering::Relaxed);
                            })
                        })
                        .collect();
                    for task in burst {
                        task.await;
                    }
                    runner.sleep_until(Instant::now() + Duration::from_micros(200)).await;
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                }
            })
        })
        .collect();

    // The main thread is the watchdog: no progress for a long time means a lost wake-up.
    let mut last = (0, Instant::now());
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        let now = runs.load(Ordering::Relaxed);
        if now != last.0 {
            last = (now, Instant::now());
        }
        assert!(last.1.elapsed() < Duration::from_secs(30), "no graph run finished in 30 s");
    }
    stop.store(true, Ordering::Release);
    for task in loops.into_iter().chain(chatter) {
        executor.block_on(task);
    }
    assert!(runs.load(Ordering::Relaxed) >= u64::from(sized(3, 2)));
    assert!(pings.load(Ordering::Relaxed) > 0);
    executor.shutdown();
}
