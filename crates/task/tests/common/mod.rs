//! Shared helpers for the integration tests.

#![allow(dead_code, reason = "each test binary uses a different subset")]

use core::ops::Range;
use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use lactose_task::{Executor, Graph, GraphBuilder, GraphNode, JobContext, NodeId};

/// Under Miri every test uses the small size, so the unsafe paths run without the volume.
pub const fn sized(native: u32, miri: u32) -> u32 {
    if cfg!(miri) { miri } else { native }
}

pub fn executor(frame: u32, background: u32) -> Executor {
    Executor::builder().frame_workers(frame).background_workers(background).build().unwrap()
}

type RunFn = Box<dyn Fn(Range<u32>, &mut JobContext<'_>) + Send + Sync>;
type FinalizeFn = Box<dyn Fn(&mut JobContext<'_>) + Send + Sync>;

/// A graph node assembled from closures.
pub struct Node {
    name: &'static str,
    units: Box<dyn Fn() -> u32 + Send + Sync>,
    run: RunFn,
    finalize: FinalizeFn,
}

impl Node {
    pub fn new(name: &'static str) -> Self {
        Self { name, units: Box::new(|| 1), run: Box::new(|_, _| {}), finalize: Box::new(|_| {}) }
    }

    pub fn units(mut self, units: u32) -> Self {
        self.units = Box::new(move || units);
        self
    }

    pub fn units_with(mut self, units: impl Fn() -> u32 + Send + Sync + 'static) -> Self {
        self.units = Box::new(units);
        self
    }

    pub fn run(
        mut self,
        run: impl Fn(Range<u32>, &mut JobContext<'_>) + Send + Sync + 'static,
    ) -> Self {
        self.run = Box::new(run);
        self
    }

    pub fn finalize(
        mut self,
        finalize: impl Fn(&mut JobContext<'_>) + Send + Sync + 'static,
    ) -> Self {
        self.finalize = Box::new(finalize);
        self
    }

    pub fn build(self) -> Arc<dyn GraphNode> {
        Arc::new(self)
    }
}

impl GraphNode for Node {
    fn name(&self) -> &'static str {
        self.name
    }

    fn units(&self) -> u32 {
        (self.units)()
    }

    fn run(&self, units: Range<u32>, ctx: &mut JobContext<'_>) {
        (self.run)(units, ctx);
    }

    fn finalize(&self, ctx: &mut JobContext<'_>) {
        (self.finalize)(ctx);
    }
}

/// Builds a graph from nodes and `(before, after)` index pairs.
pub fn graph(nodes: Vec<Arc<dyn GraphNode>>, edges: &[(u32, u32)]) -> Graph {
    let mut builder = GraphBuilder::new();
    for node in nodes {
        builder.add_node(node);
    }
    for &(before, after) in edges {
        builder.add_dependency(NodeId(before), NodeId(after));
    }
    builder.build().unwrap()
}

/// Spins until `flag` is set, failing the test if that takes implausibly long.
pub fn wait_for(flag: &AtomicBool) {
    let start = Instant::now();
    while !flag.load(Ordering::Acquire) {
        assert!(start.elapsed() < Duration::from_secs(60), "timed out waiting for a flag");
        std::thread::yield_now();
    }
}

/// Burns roughly `duration` of CPU without sleeping.
pub fn spin_for(duration: Duration) {
    let start = Instant::now();
    while start.elapsed() < duration {
        core::hint::spin_loop();
    }
}
