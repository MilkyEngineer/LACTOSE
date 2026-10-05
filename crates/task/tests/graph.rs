//! Graph behaviour: counters, finalize, release order, ranges and run ownership.

#![cfg(not(feature = "loom"))]

mod common;

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use common::{Node, executor, graph, sized};
use lactose_task::{GraphBuilder, GraphError, NodeId};

#[test]
fn every_unit_runs_exactly_once_and_finalize_follows_them() {
    let executor = executor(sized(4, 2), 1);
    let units = sized(2000, 40);
    let hits: Arc<Vec<AtomicU32>> = Arc::new((0..units).map(|_| AtomicU32::new(0)).collect());
    let finalized = Arc::new(AtomicU32::new(0));

    let node = {
        let (hits, seen, finalized) = (hits.clone(), hits.clone(), finalized.clone());
        Node::new("graph::cover")
            .units(units)
            .run(move |range, _| {
                assert!(range.start < range.end && range.end <= units);
                for unit in range {
                    hits[unit as usize].fetch_add(1, Ordering::Relaxed);
                }
            })
            .finalize(move |_| {
                // Every unit is done before finalize, whichever worker ran it.
                assert!(seen.iter().all(|hit| hit.load(Ordering::Relaxed) == 1));
                finalized.fetch_add(1, Ordering::Relaxed);
            })
            .build()
    };
    let mut graph = graph(vec![node], &[]);
    for run in 1..=3 {
        graph = executor.block_on(executor.run_graph(graph)).unwrap();
        assert_eq!(finalized.load(Ordering::Relaxed), run);
        assert!(hits.iter().all(|hit| hit.swap(0, Ordering::Relaxed) == 1));
    }
    assert_eq!(graph.len(), 1);
    assert!(!graph.is_empty());
    executor.shutdown();
}

#[test]
fn finalize_runs_exactly_once_for_a_node_without_units() {
    let executor = executor(2, 1);
    let order = Arc::new(Mutex::new(Vec::new()));
    let node = |name: &'static str, units| {
        let (ran, finalized) = (order.clone(), order.clone());
        Node::new(name)
            .units(units)
            .run(move |_, _| ran.lock().unwrap().push((name, "run")))
            .finalize(move |_| finalized.lock().unwrap().push((name, "finalize")))
            .build()
    };
    // A chain whose middle and last nodes have nothing to do.
    let chain = graph(
        vec![node("first", 1), node("empty", 0), node("also empty", 0), node("last", 1)],
        &[(0, 1), (1, 2), (2, 3)],
    );
    executor.block_on(executor.run_graph(chain)).unwrap();
    assert_eq!(
        *order.lock().unwrap(),
        [
            ("first", "run"),
            ("first", "finalize"),
            ("empty", "finalize"),
            ("also empty", "finalize"),
            ("last", "run"),
            ("last", "finalize"),
        ]
    );

    // A graph that is nothing but an empty node still completes.
    order.lock().unwrap().clear();
    executor.block_on(executor.run_graph(graph(vec![node("only", 0)], &[]))).unwrap();
    assert_eq!(*order.lock().unwrap(), [("only", "finalize")]);
    executor.shutdown();
}

#[test]
fn dependents_are_released_only_after_every_dependency_finalized() {
    let executor = executor(sized(4, 2), 1);
    let width = sized(12, 3) as usize;
    let rounds = sized(50, 2);
    // Layers of `width` nodes; every node depends on every node of the layer before, and a
    // node's finalize is slow enough for an early release to be caught.
    let layers = 4;
    let finalized: Arc<Vec<AtomicBool>> =
        Arc::new((0..layers * width).map(|_| AtomicBool::new(false)).collect());
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for index in 0..layers * width {
        let layer = index / width;
        let (in_units, in_run, in_finalize) =
            (finalized.clone(), finalized.clone(), finalized.clone());
        let dependencies = move |state: &[AtomicBool]| {
            layer == 0
                || state[(layer - 1) * width..layer * width]
                    .iter()
                    .all(|done| done.load(Ordering::Acquire))
        };
        nodes.push(
            Node::new("graph::layered")
                .units_with(move || {
                    assert!(
                        dependencies(&in_units),
                        "units() called before a dependency finalized"
                    );
                    3
                })
                .run(move |_, _| {
                    assert!(dependencies(&in_run), "released before a dependency finalized");
                })
                .finalize(move |_| {
                    std::thread::yield_now();
                    assert!(!in_finalize[index].swap(true, Ordering::AcqRel), "finalized twice");
                })
                .build(),
        );
        if layer > 0 {
            for before in (layer - 1) * width..layer * width {
                edges.push((before as u32, index as u32));
            }
        }
    }
    let mut graph = graph(nodes, &edges);
    for _ in 0..rounds {
        graph = executor.block_on(executor.run_graph(graph)).unwrap();
        assert!(finalized.iter().all(|done| done.swap(false, Ordering::AcqRel)));
    }
    executor.shutdown();
}

#[test]
fn units_is_read_at_release_and_may_depend_on_earlier_nodes() {
    let executor = executor(2, 1);
    let produced = Arc::new(AtomicU32::new(0));
    let consumed = Arc::new(AtomicU32::new(0));
    let producer = {
        let produced = produced.clone();
        Node::new("graph::producer").finalize(move |_| produced.store(7, Ordering::Release)).build()
    };
    let consumer = {
        let (produced, consumed) = (produced.clone(), consumed.clone());
        Node::new("graph::consumer")
            .units_with(move || produced.load(Ordering::Acquire))
            .run(move |range, _| {
                consumed.fetch_add(range.len() as u32, Ordering::Relaxed);
            })
            .build()
    };
    executor.block_on(executor.run_graph(graph(vec![producer, consumer], &[(0, 1)]))).unwrap();
    assert_eq!(consumed.load(Ordering::Relaxed), 7);
    executor.shutdown();
}

#[test]
fn an_empty_graph_resolves_and_a_dropped_run_still_finishes() {
    let executor = executor(2, 1);
    let empty =
        executor.block_on(executor.run_graph(GraphBuilder::new().build().unwrap())).unwrap();
    assert!(empty.is_empty());

    let finished = Arc::new(AtomicBool::new(false));
    let token = Arc::new(());
    let node = {
        let (finished, token) = (finished.clone(), token.clone());
        Node::new("graph::detached")
            .units(sized(64, 8))
            .run(|_, _| std::thread::yield_now())
            .finalize(move |_| {
                let _held = &token;
                finished.store(true, Ordering::Release);
            })
            .build()
    };
    // Dropping the future does not cancel the run, and the graph is dropped afterwards.
    drop(executor.run_graph(graph(vec![node], &[])));
    common::wait_for(&finished);
    let start = std::time::Instant::now();
    while Arc::strong_count(&token) != 1 {
        assert!(start.elapsed() < std::time::Duration::from_secs(60), "the graph was not dropped");
        std::thread::yield_now();
    }
    executor.shutdown();
}

#[test]
fn concurrent_runs_of_different_graphs_do_not_interfere() {
    let executor = executor(sized(4, 2), 1);
    let rounds = sized(40, 2);
    let total = Arc::new(AtomicUsize::new(0));
    let make = |units: u32| {
        let total = total.clone();
        let node = Node::new("graph::concurrent")
            .units(units)
            .run(move |range, _| {
                total.fetch_add(range.len(), Ordering::Relaxed);
            })
            .build();
        graph(vec![node.clone(), node.clone(), node], &[(0, 2), (1, 2)])
    };
    let tasks: Vec<_> = [5u32, 17, 33]
        .into_iter()
        .map(|units| {
            let (runner, mut graph) = (executor.clone(), make(units));
            executor.spawn(lactose_task::WorkerClass::FrameCritical, async move {
                for _ in 0..rounds {
                    graph = runner.run_graph(graph).await.unwrap();
                }
            })
        })
        .collect();
    for task in tasks {
        executor.block_on(task).unwrap();
    }
    assert_eq!(total.load(Ordering::Relaxed), (5 + 17 + 33) * 3 * rounds as usize);
    executor.shutdown();
}

#[test]
fn builder_rejects_unknown_nodes_and_cycles() {
    let node = || Node::new("graph::invalid").build();

    let mut builder = GraphBuilder::new();
    let a = builder.add_node(node());
    assert_eq!(a, NodeId(0));
    builder.add_dependency(a, NodeId(5));
    assert_eq!(builder.build().err(), Some(GraphError::UnknownNode(NodeId(5))));

    // 0 -> 1 -> 2 -> 3 -> 1, with 4 hanging off the cycle.
    let mut builder = GraphBuilder::new();
    let ids: Vec<NodeId> = (0..5).map(|_| builder.add_node(node())).collect();
    for (before, after) in [(0, 1), (1, 2), (2, 3), (3, 1), (3, 4)] {
        builder.add_dependency(ids[before], ids[after]);
    }
    let Err(GraphError::Cycle(mut cycle)) = builder.build() else { panic!("expected a cycle") };
    cycle.sort();
    assert_eq!(cycle, [NodeId(1), NodeId(2), NodeId(3)]);

    let mut builder = GraphBuilder::new();
    let only = builder.add_node(node());
    builder.add_dependency(only, only);
    assert_eq!(builder.build().err(), Some(GraphError::Cycle(vec![only])));

    // Declaring an edge twice is one dependency.
    let executor = executor(1, 1);
    let ran = Arc::new(AtomicU32::new(0));
    let counted = {
        let ran = ran.clone();
        Node::new("graph::duplicate")
            .run(move |_, _| {
                ran.fetch_add(1, Ordering::Relaxed);
            })
            .build()
    };
    executor.block_on(executor.run_graph(graph(vec![node(), counted], &[(0, 1), (0, 1)]))).unwrap();
    assert_eq!(ran.load(Ordering::Relaxed), 1);
    executor.shutdown();
}
