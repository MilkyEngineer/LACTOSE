//! Shared helpers for the integration tests.

#![allow(dead_code, reason = "each test binary uses a different subset")]

use core::ops::Range;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use lactose_task::{Executor, Graph, GraphBuilder, GraphNode, JobContext, NodeId};

/// How long a wait may last that only a regression runs out. Miri's clock runs with the
/// interpreter, hence its longer limit.
pub const LIMIT: Duration = Duration::from_secs(if cfg!(miri) { 600 } else { 60 });

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

/// Runs `f` on a helper thread and returns what it returns, failing the test if it takes
/// longer than `limit`: a regression that leaves `f` waiting for ever shows as a failure that
/// names `what`, not as a test-runner timeout. A panic in `f` is passed on as it is.
pub fn must_finish<R: Send + 'static>(
    what: &str,
    limit: Duration,
    f: impl FnOnce() -> R + Send + 'static,
) -> R {
    let (done, wait) = std::sync::mpsc::channel();
    let runner = std::thread::spawn(move || {
        let _ = done.send(f());
    });
    match wait.recv_timeout(limit) {
        Ok(result) => {
            runner.join().unwrap();
            result
        }
        Err(RecvTimeoutError::Disconnected) => {
            std::panic::resume_unwind(runner.join().expect_err("`f` returned without sending"))
        }
        Err(RecvTimeoutError::Timeout) => panic!("{what} did not finish in {limit:?}"),
    }
}

/// Gives a new executor's workers time to start and go to sleep, for a test of the wake-up
/// that work brings them. A worker that is still awake when the work arrives finds it
/// without one, so on a machine too slow for this pause the test proves less, never fails.
pub fn let_workers_sleep() {
    std::thread::sleep(Duration::from_millis(sized(50, 1).into()));
}

/// A meeting of every frame-critical worker: the first job each worker brings to it waits
/// until all of them have come. It is met only if every worker takes a job while the others
/// are held in theirs, which takes every worker awake and finding the work.
pub struct Rendezvous {
    /// Which slots have come.
    seen: Vec<AtomicBool>,
    arrived: AtomicU32,
    /// Set by a job that waited [`LIMIT`] for the others.
    gave_up: AtomicBool,
}

impl Rendezvous {
    pub fn new(executor: &Executor) -> Self {
        Self {
            seen: (0..executor.slot_count()).map(|_| AtomicBool::new(false)).collect(),
            arrived: AtomicU32::new(0),
            gave_up: AtomicBool::new(false),
        }
    }

    /// Called from a job or child running under `slot`. The first call for each slot waits
    /// until every worker has come, or for [`LIMIT`]; later calls return at once. The wait
    /// sleeps between looks: it holds its worker either way, and costs no CPU while a
    /// regression keeps it waiting.
    pub fn arrive(&self, slot: u32) {
        if self.seen[slot as usize].swap(true, Ordering::AcqRel) {
            return;
        }
        self.arrived.fetch_add(1, Ordering::AcqRel);
        let begun = Instant::now();
        while self.arrived() < self.seen.len() as u32 {
            if begun.elapsed() >= LIMIT {
                self.gave_up.store(true, Ordering::Release);
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// How many workers have come.
    pub fn arrived(&self) -> u32 {
        self.arrived.load(Ordering::Acquire)
    }

    /// Whether every worker came while the others were still waiting.
    pub fn met(&self) -> bool {
        self.arrived() == self.seen.len() as u32 && !self.gave_up.load(Ordering::Acquire)
    }
}
