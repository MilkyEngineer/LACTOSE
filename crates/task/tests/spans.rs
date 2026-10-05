//! Tracing: one span per node run and one per range split off it, named by the node, and
//! never one per unit.
//!
//! One test only: it installs a process-wide subscriber.

#![cfg(not(feature = "loom"))]

mod common;

use core::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use common::{Node, executor, graph, sized};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

/// Records the name and module path of every span created under the `whisk` target.
#[derive(Clone, Default)]
struct Recorder {
    spans: Arc<Mutex<Vec<(String, String)>>>,
    next: Arc<AtomicU64>,
}

impl Subscriber for Recorder {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "whisk"
    }

    fn new_span(&self, span: &Attributes<'_>) -> Id {
        let metadata = span.metadata();
        self.spans.lock().unwrap().push((
            metadata.name().to_owned(),
            metadata.module_path().unwrap_or_default().to_owned(),
        ));
        Id::from_u64(self.next.fetch_add(1, Ordering::Relaxed) + 1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}
    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}
    fn event(&self, _event: &Event<'_>) {}
    fn enter(&self, _span: &Id) {}
    fn exit(&self, _span: &Id) {}
}

#[test]
fn spans_are_per_node_and_per_split_range_never_per_unit() {
    let recorder = Recorder::default();
    tracing::subscriber::set_global_default(recorder.clone()).unwrap();
    let units = sized(4096, 64);
    let runs = 3;
    let count = |name: &str, module: &str| {
        let spans = recorder.spans.lock().unwrap();
        spans.iter().filter(|(span, path)| span == name && path == module).count()
    };

    // One worker makes the count exact. The worker halves what it holds each time its
    // deque runs dry: 4096 units leave the deque as ranges of 2048, 1024, ... 1, so a run
    // is one node span and log2(units) range spans, however many units there are.
    let single = executor(1, 1);
    let node = Node::new("spans::single").units(units).build();
    let mut graph_single = graph(vec![node], &[]);
    for _ in 0..runs {
        graph_single = single.block_on(single.run_graph(graph_single)).unwrap();
    }
    single.shutdown();
    assert_eq!(count("spans::single", "lactose_task::node"), runs);
    assert_eq!(count("spans::single", "lactose_task::range"), runs * units.ilog2() as usize);

    // With thieves the count depends on how often they come: one range span per steal or
    // dry deque, and never more than one per unit beyond the first, which is the bound
    // when every unit is stolen singly.
    let workers = sized(4, 2);
    let several = executor(workers, 1);
    let node = |name| Node::new(name).units(units).run(|_, _| std::thread::yield_now()).build();
    let mut graph_several = graph(vec![node("spans::wide"), node("spans::other")], &[(0, 1)]);
    for _ in 0..runs {
        graph_several = several.block_on(several.run_graph(graph_several)).unwrap();
    }
    several.shutdown();
    for name in ["spans::wide", "spans::other"] {
        assert_eq!(count(name, "lactose_task::node"), runs);
        let ranges = count(name, "lactose_task::range");
        assert!(ranges >= runs, "{name}: a range was never split across {workers} workers");
        assert!(ranges <= runs * (units as usize - 1), "{name}: {ranges} range spans");
    }
    let spans = recorder.spans.lock().unwrap();
    assert!(spans.iter().all(|(name, _)| name.starts_with("spans::")), "{spans:?}");
}
