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

    let executor = executor(sized(4, 2), 1);
    let units = sized(4096, 64);
    let node = |name| Node::new(name).units(units).run(|_, _| std::thread::yield_now()).build();
    let runs = 3;
    let mut graph = graph(vec![node("spans::wide"), node("spans::other")], &[(0, 1)]);
    for _ in 0..runs {
        graph = executor.block_on(executor.run_graph(graph));
    }
    executor.shutdown();

    let spans = recorder.spans.lock().unwrap();
    for name in ["spans::wide", "spans::other"] {
        let count = |module: &str| {
            spans.iter().filter(|(span, path)| span == name && path == module).count()
        };
        // Exactly one span per run of the node...
        assert_eq!(count("lactose_task::node"), runs);
        // ...and one per range split off it, which there are far fewer of than units: a
        // range is halved only when a worker's deque has run dry.
        let ranges = count("lactose_task::range");
        assert!(ranges >= 1, "{name}: the range was never split across {} workers", sized(4, 2));
        assert!(
            ranges < runs * units as usize / 2,
            "{name}: {ranges} range spans for {units} units"
        );
    }
    assert!(spans.iter().all(|(name, _)| name.starts_with("spans::")), "{spans:?}");
}
