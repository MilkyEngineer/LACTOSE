//! `ExtractSink::write` from several jobs (`README.md`, "Writers", "Concurrency"): the units
//! of one graph node write disjoint rows of one pass through one sink at once, as extract's
//! forked jobs do, and the pass is read back with every row once. A run in which no two
//! writes overlapped shows nothing of that, so the test counts the writes in flight and runs
//! again when it never saw two; it fails if no run of several did.

use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use lactose_core::Tick;
use lactose_ecs::WorldKind;
use lactose_present_api::{ExtractPass, InstanceRow, InstanceSink};
use lactose_renderer::{ExtractSink, Renderer, RendererDesc};
use lactose_task::{GraphBuilder, GraphNode, JobContext};

use crate::common::{executor, key, null, ok, row};

const ROWS: u32 = 4096;
const UNITS: u32 = 64;

/// Each unit writes its share of the rows, in batches of a few rows.
struct Writers {
    sink: Arc<ExtractSink>,
    rows: Vec<InstanceRow>,
    /// The writes in flight, and the most seen at once.
    in_flight: AtomicU32,
    most: AtomicU32,
}

impl GraphNode for Writers {
    fn name(&self) -> &'static str {
        "test::writers"
    }

    fn units(&self) -> u32 {
        UNITS
    }

    fn run(&self, units: Range<u32>, _ctx: &mut JobContext<'_>) {
        let share = (ROWS / UNITS) as usize;
        for unit in units {
            let mine = &self.rows[unit as usize * share..(unit as usize + 1) * share];
            for batch in mine.chunks(7) {
                let now = self.in_flight.fetch_add(1, Ordering::Relaxed) + 1;
                self.most.fetch_max(now, Ordering::Relaxed);
                self.sink.write(batch);
                self.in_flight.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }
}

/// The runs the test makes before it fails for want of two writes at once.
const RUNS: u32 = 20;

#[test]
fn rows_written_from_several_jobs_are_each_read_once() {
    for run in 1..=RUNS {
        if writes_from_several_jobs_overlapped() {
            eprintln!("two writes overlapped in run {run}");
            return;
        }
    }
    panic!("no two writes overlapped in {RUNS} runs");
}

/// One run: whether two writes overlapped. Every row is read once whichever.
fn writes_from_several_jobs_overlapped() -> bool {
    let desc = RendererDesc { instances: ROWS, rows: ROWS + 8, regions: 2, shaders: None };
    let mut renderer = ok(Renderer::new(null(), &desc));
    let executor = executor(4);
    let rows: Vec<InstanceRow> = (1..=u64::from(ROWS)).map(|n| row(n, n as f32)).collect();
    let mut sink = renderer.sink();
    sink.begin(ExtractPass { kind: WorldKind::Server, tick: Tick(1), full: true });
    let sink = Arc::new(sink);
    let mut builder = GraphBuilder::new();
    let writers = Arc::new(Writers {
        sink: Arc::clone(&sink),
        rows: rows.clone(),
        in_flight: AtomicU32::new(0),
        most: AtomicU32::new(0),
    });
    builder.add_node(Arc::clone(&writers) as Arc<dyn GraphNode>);
    executor.block_on(executor.run_graph(builder.build().expect("one node"))).expect("the run");
    let overlapped = writers.most.load(Ordering::Relaxed) > 1;
    drop(writers);
    let mut sink = Arc::into_inner(sink).expect("the writers are done");
    sink.end();
    executor.shutdown();

    let stats = ok(renderer.prepare());
    assert!(!stats.any_lost());
    assert_eq!((stats.pass_rows, stats.dropped, stats.duplicates), (ROWS, 0, 0));
    assert_eq!((stats.allocated, stats.live, stats.entries), (ROWS, ROWS, ROWS));
    let batch = renderer.batch();
    let read: BTreeSet<u32> = batch.entries().iter().map(|entry| entry.row).collect();
    assert_eq!(read.len(), ROWS as usize, "every entry reads a row of its own");
    let slots = renderer.slots();
    let held: BTreeSet<_> = (1..=u64::from(ROWS))
        .map(|n| slots.get(WorldKind::Server, key(n)).expect("every row is held"))
        .collect();
    assert_eq!(held.len(), ROWS as usize);
    overlapped
}

/// A sink holds the ring's state and the device, so it outlives its renderer: its writes
/// go on to buffers the renderer destroyed, which null refuses and the sink counts as
/// dropped, and the device lives until the sink is dropped (`README.md`, "Writers").
#[test]
fn a_sink_outlives_its_renderer() {
    let device = null();
    let weak = Arc::downgrade(&device);
    let desc = RendererDesc { instances: 2, rows: 16, regions: 2, shaders: None };
    let renderer = ok(Renderer::new(device, &desc));
    let mut sink = renderer.sink();
    drop(renderer);
    sink.begin(ExtractPass { kind: WorldKind::Server, tick: Tick(1), full: true });
    sink.write(&[row(1, 1.0)]);
    sink.end();
    assert!(weak.upgrade().is_some(), "the sink holds the device after its renderer's drop");
    drop(sink);
    assert!(weak.upgrade().is_none(), "the device is dropped with the last sink");
}
