//! `ExtractSink::write` from several jobs (`README.md`, "Writers", "Concurrency"): the units
//! of one graph node write disjoint rows of one pass through one sink at once, as extract's
//! forked jobs do, and the pass is read back with every row once.

use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::Arc;

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
                self.sink.write(batch);
            }
        }
    }
}

#[test]
fn rows_written_from_several_jobs_are_each_read_once() {
    let desc = RendererDesc { instances: ROWS, rows: ROWS + 8, regions: 2, shaders: None };
    let mut renderer = ok(Renderer::new(null(), &desc));
    let executor = executor(4);
    let rows: Vec<InstanceRow> = (1..=u64::from(ROWS)).map(|n| row(n, n as f32)).collect();
    let mut sink = renderer.sink();
    sink.begin(ExtractPass { kind: WorldKind::Server, tick: Tick(1), full: true });
    let sink = Arc::new(sink);
    let mut builder = GraphBuilder::new();
    builder.add_node(Arc::new(Writers { sink: Arc::clone(&sink), rows: rows.clone() }));
    executor.block_on(executor.run_graph(builder.build().expect("one node"))).expect("the run");
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
}
