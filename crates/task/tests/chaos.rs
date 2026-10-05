//! Chaos scheduling: perturbed runs stay correct, and a recorded log replays the same
//! job-to-worker order.

#![cfg(not(feature = "loom"))]

mod common;

use core::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use common::{Node, graph, sized};
use lactose_core::ErrorKind;
use lactose_task::{ChaosConfig, ExecutionLog, Executor, Graph};

const WORKERS: u32 = 3;

/// What ran where: `(node, first unit or child, kind)` to the slot it ran under.
type Placement = Arc<Mutex<Vec<(u32, u32, &'static str, u32)>>>;

/// A graph with a fan-out, a join, an empty node and a node whose units fork children.
fn workload(placement: &Placement) -> Graph {
    let node = |index: u32, units: u32, children: u32| {
        let (ran, finalized) = (placement.clone(), placement.clone());
        Node::new("chaos::node")
            .units(units)
            .run(move |range, ctx| {
                assert_eq!(range.len(), 1, "a recorded job is one unit");
                ran.lock().unwrap().push((index, range.start, "unit", ctx.slot()));
                if children != 0 {
                    let unit = range.start;
                    let ran = &ran;
                    ctx.scope(|scope| {
                        scope.fork(move |ctx| {
                            ran.lock().unwrap().push((index, unit * 100, "fork", ctx.slot()));
                        });
                        scope.fork_range(1..children, move |range, ctx| {
                            let key = unit * 100 + range.start;
                            ran.lock().unwrap().push((index, key, "range", ctx.slot()));
                        });
                    });
                }
            })
            .finalize(move |ctx| finalized.lock().unwrap().push((index, 0, "finalize", ctx.slot())))
            .build()
    };
    let units = sized(24, 5);
    graph(
        vec![
            node(0, 1, 0),
            node(1, units, 0),
            node(2, sized(6, 2), sized(8, 3)),
            node(3, 0, 0),
            node(4, units, 0),
        ],
        &[(0, 1), (0, 2), (0, 3), (1, 4), (2, 4), (3, 4)],
    )
}

/// Runs the workload twice on `executor` and returns the sorted placement and the log.
fn run_twice(executor: Executor) -> (Vec<(u32, u32, &'static str, u32)>, ExecutionLog) {
    let placement = Placement::default();
    let mut graph = workload(&placement);
    for _ in 0..2 {
        graph = executor.block_on(executor.run_graph(graph)).unwrap();
    }
    let log = executor.execution_log().expect("recording was requested");
    executor.shutdown();
    let mut placement = core::mem::take(&mut *placement.lock().unwrap());
    // Two runs give two entries per key; a stable sort keeps them in run order only by
    // accident, so compare as a multiset.
    placement.sort_unstable();
    (placement, log)
}

fn builder() -> lactose_task::ExecutorBuilder {
    Executor::builder().frame_workers(WORKERS).background_workers(1)
}

#[test]
fn a_recorded_log_replays_the_same_job_to_worker_order() {
    for seed in 0..u64::from(sized(6, 1)) {
        let chaos = ChaosConfig {
            seed,
            shuffle_steals: true,
            inject_yields: true,
            force_split: seed % 2 == 0,
            record: true,
        };
        let (recorded_placement, log) = run_twice(builder().chaos(chaos).build().unwrap());

        // The log survives a trip through bytes, as a CI artifact would.
        let log = ExecutionLog::from_bytes(log.as_bytes().to_vec());
        let replaying = builder()
            .replay(log.clone())
            .chaos(ChaosConfig { record: true, ..ChaosConfig::default() })
            .build()
            .unwrap();
        let (replayed_placement, replayed_log) = run_twice(replaying);

        // The same jobs started in the same order on the same workers...
        assert_eq!(replayed_log, log, "seed {seed}");
        // ...which the jobs themselves observed as the same slots.
        assert_eq!(replayed_placement, recorded_placement, "seed {seed}");
    }
}

#[test]
fn an_exhausted_log_hands_over_to_free_scheduling() {
    let chaos = ChaosConfig { seed: 9, force_split: true, record: true, ..ChaosConfig::default() };
    let recorder = builder().chaos(chaos).build().unwrap();
    let placement = Placement::default();
    let graph = recorder.block_on(recorder.run_graph(workload(&placement))).unwrap();
    let log = recorder.execution_log().unwrap();
    recorder.shutdown();
    let one_run = placement.lock().unwrap().len();
    drop(graph);

    // Replay one recorded run, then keep going past the end of the log.
    let replaying = builder().replay(log).build().unwrap();
    assert!(replaying.execution_log().is_none(), "replay alone does not record");
    let placement = Placement::default();
    let mut graph = workload(&placement);
    for _ in 0..3 {
        graph = replaying.block_on(replaying.run_graph(graph)).unwrap();
    }
    assert_eq!(placement.lock().unwrap().len(), one_run * 3);
    replaying.shutdown();
}

#[test]
fn every_chaos_option_keeps_runs_correct() {
    let options: [fn(&mut ChaosConfig); 4] = [
        |config| config.shuffle_steals = true,
        |config| config.inject_yields = true,
        |config| config.force_split = true,
        |config| config.record = true,
    ];
    for (seed, option) in options.into_iter().enumerate() {
        let mut chaos = ChaosConfig { seed: seed as u64, ..ChaosConfig::default() };
        option(&mut chaos);
        let executor = builder().chaos(chaos).build().unwrap();
        let units = sized(300, 12);
        let hits: Arc<Vec<AtomicU32>> = Arc::new((0..units).map(|_| AtomicU32::new(0)).collect());
        let node = {
            let hits = hits.clone();
            Node::new("chaos::cover")
                .units(units)
                .run(move |range, _| {
                    if chaos.force_split {
                        assert_eq!(range.len(), 1);
                    }
                    for unit in range {
                        hits[unit as usize].fetch_add(1, Ordering::Relaxed);
                    }
                })
                .build()
        };
        executor.block_on(executor.run_graph(graph(vec![node], &[]))).unwrap();
        assert!(hits.iter().all(|hit| hit.load(Ordering::Relaxed) == 1));
        assert_eq!(executor.execution_log().is_some(), chaos.record);
        executor.shutdown();
    }
}

#[test]
fn without_chaos_nothing_is_recorded() {
    let executor = builder().build().unwrap();
    executor.block_on(executor.run_graph(workload(&Placement::default()))).unwrap();
    assert!(executor.execution_log().is_none());
    executor.shutdown();
}

#[test]
fn replay_rejects_a_log_it_cannot_follow() {
    let kind = |log: ExecutionLog, workers| {
        Executor::builder()
            .frame_workers(workers)
            .background_workers(1)
            .replay(log)
            .build()
            .err()
            .map(|error| error.kind())
    };
    assert_eq!(
        kind(ExecutionLog::from_bytes(b"not a log".to_vec()), WORKERS),
        Some(ErrorKind::InvalidData)
    );

    let recorder =
        builder().chaos(ChaosConfig { record: true, ..ChaosConfig::default() }).build().unwrap();
    recorder.block_on(recorder.run_graph(workload(&Placement::default()))).unwrap();
    let log = recorder.execution_log().unwrap();
    recorder.shutdown();
    // A log is tied to the worker count it was recorded with.
    assert_eq!(kind(log, WORKERS + 1), Some(ErrorKind::InvalidData));
}
