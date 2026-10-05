//! Continuation-counted graphs: the shape every schedule stage and render-graph record
//! phase runs in.

use core::ops::Range;
use core::pin::Pin;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use core::task::{Context, Poll, Waker};
use std::sync::{Arc, Mutex};

use lactose_core::{Error, ErrorKind, Result};

use crate::chaos::{JobKind, LogRecord};
use crate::guard::abort_on_unwind;
use crate::job::JobContext;
use crate::pool::{Arenas, Entry, Rest, Shared, WorkerLocal, drain_range};
use crate::span::{NodeSpans, spans_for};
use crate::{WorkerClass, lock};

/// One system, pass or other unit of a [`Graph`].
///
/// A node owns or shares its data (`'static`); the executor never borrows from the caller
/// across threads, and a run takes no per-run argument. `lactose-ecs` implements this for a
/// system bound to a shared world, and owns the argument that makes that sound: declared
/// access for concurrent nodes, and running exclusive systems between graph runs, when no
/// node is live.
///
/// A panic in any method aborts the process.
pub trait GraphNode: Send + Sync + 'static {
    /// The name of the tracing spans emitted once per run and once per range split off it.
    fn name(&self) -> &'static str;

    /// How many units of work this node has (chunks, for a system). Called once per run, at
    /// the moment the node is released, so it may depend on what earlier nodes did.
    ///
    /// It is called on the thread that releases the node: the worker that finalized its last
    /// dependency, or for a node without dependencies the caller of
    /// [`crate::Executor::run_graph`].
    fn units(&self) -> u32;

    /// Processes `units`. Called concurrently on disjoint ranges that together cover
    /// `0..self.units()` exactly once. The executor starts with the whole range and halves a
    /// range each time it is stolen; there is no target job duration, and a call may cover a
    /// single unit.
    fn run(&self, units: Range<u32>, ctx: &mut JobContext<'_>);

    /// Runs exactly once per run, after every unit has been processed and every scope opened
    /// by [`GraphNode::run`] has closed, and before any dependent node is released.
    /// Also called when `units()` is zero.
    ///
    /// Reductions merge their partial results here. `run` is called a scheduling-dependent
    /// number of times, so a node keeps one partial per [`JobContext::slot`] and merges
    /// them with a combiner whose result does not depend on order.
    fn finalize(&self, ctx: &mut JobContext<'_>) {
        let _ = ctx;
    }
}

/// Identifies a node within the graph that issued it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct NodeId(pub u32);

/// Why a graph could not be built.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum GraphError {
    /// The dependencies contain a cycle through these nodes, listed in dependency order.
    Cycle(Vec<NodeId>),
    /// An edge names a node this builder did not issue.
    UnknownNode(NodeId),
}

impl core::fmt::Display for GraphError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Cycle(nodes) => write!(f, "dependency cycle through nodes {nodes:?}"),
            Self::UnknownNode(node) => write!(f, "dependency names unknown node {node:?}"),
        }
    }
}

impl std::error::Error for GraphError {}

/// Collects nodes and dependencies into a [`Graph`].
#[derive(Default)]
pub struct GraphBuilder {
    nodes: Vec<Arc<dyn GraphNode>>,
    /// `(before, after)` pairs.
    edges: Vec<(u32, u32)>,
}

impl GraphBuilder {
    /// An empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a node. IDs are issued in call order starting at zero.
    pub fn add_node(&mut self, node: Arc<dyn GraphNode>) -> NodeId {
        let id = u32::try_from(self.nodes.len()).expect("a graph holds fewer than 2^32 nodes");
        self.nodes.push(node);
        NodeId(id)
    }

    /// Declares that `after` is released only once `before` has finalized.
    pub fn add_dependency(&mut self, before: NodeId, after: NodeId) {
        self.edges.push((before.0, after.0));
    }

    /// Validates the dependencies and builds the graph.
    pub fn build(mut self) -> core::result::Result<Graph, GraphError> {
        let count = self.nodes.len();
        for &(before, after) in &self.edges {
            for node in [before, after] {
                if node as usize >= count {
                    return Err(GraphError::UnknownNode(NodeId(node)));
                }
            }
        }
        // Declaring a dependency twice is harmless but would be counted twice.
        self.edges.sort_unstable();
        self.edges.dedup();

        // Dependents of each node, laid out contiguously (the edges are sorted by `before`).
        let mut first = vec![0u32; count + 1];
        let mut dependencies = vec![0u32; count];
        for &(before, after) in &self.edges {
            first[before as usize + 1] += 1;
            dependencies[after as usize] += 1;
        }
        for node in 0..count {
            first[node + 1] += first[node];
        }
        let dependents: Box<[u32]> = self.edges.iter().map(|&(_, after)| after).collect();

        // Kahn's algorithm; whatever it cannot reach sits on or behind a cycle.
        let mut remaining = dependencies.clone();
        let mut ready: Vec<u32> =
            (0..count as u32).filter(|&node| remaining[node as usize] == 0).collect();
        let roots: Box<[u32]> = ready.as_slice().into();
        let mut ordered = 0;
        while let Some(node) = ready.pop() {
            ordered += 1;
            for &after in
                &dependents[first[node as usize] as usize..first[node as usize + 1] as usize]
            {
                remaining[after as usize] -= 1;
                if remaining[after as usize] == 0 {
                    ready.push(after);
                }
            }
        }
        if ordered != count {
            return Err(GraphError::Cycle(find_cycle(&self.edges, &remaining)));
        }

        let nodes = self
            .nodes
            .into_iter()
            .enumerate()
            .map(|(index, node)| NodeSlot {
                spans: spans_for(node.name()),
                node,
                dependencies: dependencies[index],
                dependents: first[index]..first[index + 1],
                pending_dependencies: AtomicU32::new(0),
                pending_units: AtomicU32::new(0),
            })
            .collect();
        Ok(Graph {
            state: Arc::new(RunState {
                nodes,
                dependents,
                roots,
                pending_nodes: AtomicU32::new(0),
                ordinal: AtomicU64::new(0),
                completion: Mutex::new(Completion {
                    finished: false,
                    cancelled: false,
                    waker: None,
                }),
            }),
        })
    }
}

/// Walks backwards from an unordered node through unordered nodes until one repeats. Every
/// node Kahn's algorithm left has at least one dependency it also left, so the walk cannot
/// get stuck.
fn find_cycle(edges: &[(u32, u32)], remaining: &[u32]) -> Vec<NodeId> {
    let stuck = |node: u32| remaining[node as usize] != 0;
    let start = (0..remaining.len() as u32).find(|&node| stuck(node)).expect("a node is stuck");
    let mut path = vec![start];
    loop {
        let node = *path.last().expect("path is never empty");
        let before = edges
            .iter()
            .find(|&&(before, after)| after == node && stuck(before))
            .expect("a stuck node has a stuck dependency")
            .0;
        if let Some(at) = path.iter().position(|&seen| seen == before) {
            let mut cycle: Vec<NodeId> = path[at..].iter().map(|&node| NodeId(node)).collect();
            cycle.reverse();
            return cycle;
        }
        path.push(before);
    }
}

struct NodeSlot {
    node: Arc<dyn GraphNode>,
    spans: &'static NodeSpans,
    dependencies: u32,
    /// This node's range in `RunState::dependents`.
    dependents: Range<u32>,
    /// Dependencies that have not finalized in the current run.
    pending_dependencies: AtomicU32,
    /// Units no job has finished yet in the current run. Meaningful once released.
    pending_units: AtomicU32,
}

struct Completion {
    finished: bool,
    /// The executor shut down before the run finished.
    cancelled: bool,
    waker: Option<Waker>,
}

/// A graph and the counters of its current run. The counters live with the graph, so a run
/// allocates nothing; that is sound because a run owns its [`Graph`].
pub(crate) struct RunState {
    nodes: Box<[NodeSlot]>,
    dependents: Box<[u32]>,
    roots: Box<[u32]>,
    /// Nodes that have not finalized in the current run.
    pending_nodes: AtomicU32,
    ordinal: AtomicU64,
    completion: Mutex<Completion>,
}

impl RunState {
    /// Which run of its executor this is. Identifies the run in an execution log.
    pub(crate) fn ordinal(&self) -> u64 {
        self.ordinal.load(Ordering::Relaxed)
    }

    /// Resolves the run's future to `Cancelled`. Called at shutdown, possibly while a
    /// worker is still finishing a job of the run.
    pub(crate) fn cancel(&self) {
        let waker = {
            let mut completion = lock(&self.completion);
            completion.cancelled = true;
            completion.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

/// A run state that only exists to give benchmark entries something to point at.
pub(crate) fn placeholder_run() -> Arc<RunState> {
    struct Nothing;

    impl GraphNode for Nothing {
        fn name(&self) -> &'static str {
            "bench::placeholder"
        }

        fn units(&self) -> u32 {
            0
        }

        fn run(&self, _units: Range<u32>, _ctx: &mut JobContext<'_>) {}
    }

    let mut builder = GraphBuilder::new();
    builder.add_node(Arc::new(Nothing));
    builder.build().expect("one node has no cycle").state
}

/// An immutable dependency graph, built once and run every frame. A run takes the graph by
/// value and hands it back, so runs of one graph cannot overlap.
pub struct Graph {
    state: Arc<RunState>,
}

impl Graph {
    /// The number of nodes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.state.nodes.len()
    }

    /// Whether the graph has no nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.state.nodes.is_empty()
    }
}

/// Resolves to the graph when its run has finished, or to an error of kind `Cancelled` if
/// the executor shut down first. See [`crate::Executor::run_graph`].
#[must_use = "futures do nothing unless awaited"]
pub struct GraphRun {
    graph: Option<Graph>,
}

impl Future for GraphRun {
    type Output = Result<Graph>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<Graph>> {
        let graph = self.graph.as_ref().expect("a graph run is not polled after it resolved");
        let mut completion = lock(&graph.state.completion);
        if completion.finished {
            drop(completion);
            return Poll::Ready(Ok(self.graph.take().expect("checked above")));
        }
        if completion.cancelled {
            drop(completion);
            // The graph goes with the run: its counters are in an unknown state.
            self.graph = None;
            return Poll::Ready(Err(Error::new(
                ErrorKind::Cancelled,
                "the executor shut down before the graph run finished",
            )));
        }
        completion.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

/// Starts a run of `graph` on `shared`.
pub(crate) fn start(shared: &Shared, graph: Graph) -> GraphRun {
    let state = &graph.state;
    // The previous run, if any, has resolved, and a run resolves only after the last access
    // any worker makes to these counters (`finish` is that access).
    for slot in &*state.nodes {
        slot.pending_dependencies.store(slot.dependencies, Ordering::Relaxed);
    }
    state.pending_nodes.store(state.nodes.len() as u32, Ordering::Relaxed);
    {
        let mut completion = lock(&state.completion);
        completion.finished = state.nodes.is_empty();
        completion.cancelled = false;
        if completion.finished {
            drop(completion);
            return GraphRun { graph: Some(graph) };
        }
    }
    state.ordinal.store(shared.begin_run(state), Ordering::Relaxed);
    if shared.is_shut_down() {
        // Registered first, checked second: either shutdown's sweep finds the run, or this
        // sees the flag. Cancelling twice is harmless.
        state.cancel();
        return GraphRun { graph: Some(graph) };
    }
    // Submitting an entry publishes the stores above to the worker that takes it.
    // `units()` of the roots runs here, on the caller's thread, under the same rule as on a
    // worker: a run that is half released has no state to unwind to.
    abort_on_unwind(|| {
        for &root in &*state.roots {
            release(shared, state, root, false);
        }
    });
    GraphRun { graph: Some(graph) }
}

/// Queues `node`, whose dependencies have all finalized, as one range over all its units.
fn release(shared: &Shared, state: &Arc<RunState>, node: u32, keeps_one: bool) {
    let slot = &state.nodes[node as usize];
    let units = slot.node.units();
    slot.pending_units.store(units, Ordering::Relaxed);
    let entry = Entry::Range { run: state.clone(), node, units: 0..units, root: true };
    shared.submit_range(entry, keeps_one);
}

/// Runs `units` of `node` on this worker and, if that completes the node, finalizes it and
/// releases its dependents.
pub(crate) fn run_range(
    local: &WorkerLocal,
    arenas: &mut Arenas,
    state: &Arc<RunState>,
    node: u32,
    units: Range<u32>,
    root: bool,
    position: u64,
) {
    debug_assert_eq!(local.class, WorkerClass::FrameCritical, "graph work on a background worker");
    let shared = &*local.shared;
    let slot = &state.nodes[node as usize];
    let span = if root { slot.spans.node() } else { slot.spans.range() };
    let _entered = span.enter();

    let record = |kind, units: Range<u32>| match &shared.chaos {
        Some(chaos) if chaos.recording() && !chaos.replaying => chaos.record(LogRecord {
            worker: local.index as u32,
            kind,
            key: state.ordinal(),
            index: node,
            units,
        }),
        // In replay the position came with the entry, and the take recorded it.
        _ => position,
    };

    let replaying = shared.replay().is_some();
    let done = if replaying {
        // The log named exactly these units as one call.
        if !units.is_empty() {
            let mut ctx = JobContext::new(local, arenas, position);
            slot.node.run(units.clone(), &mut ctx);
            arenas.job.reset();
        }
        units.len() as u32
    } else {
        drain_range(
            local,
            units,
            |upper| {
                local.push(Entry::Range { run: state.clone(), node, units: upper, root: false });
            },
            |unit| {
                let position = record(JobKind::Units, unit.clone());
                let mut ctx = JobContext::new(local, arenas, position);
                slot.node.run(unit, &mut ctx);
                arenas.job.reset();
            },
            || {
                if shared.is_shut_down() {
                    // Queued graph work is dropped at shutdown; this run cannot finish.
                    Rest::Abandon
                } else if local.pool().over_limit() {
                    Rest::Requeue
                } else {
                    Rest::Run
                }
            },
        )
    };

    // Every scope a job opens closes before `run` returns, so the counter falls only after
    // the scopes of the units it accounts for have closed.
    if slot.pending_units.fetch_sub(done, Ordering::AcqRel) != done {
        return;
    }
    if replaying && done != 0 {
        // Finalize is a job of its own in the log, possibly on another worker.
        shared.submit_range(
            Entry::Range { run: state.clone(), node, units: 0..0, root: false },
            false,
        );
        return;
    }

    let position = record(JobKind::Finalize, 0..0);
    let mut ctx = JobContext::new(local, arenas, position);
    slot.node.finalize(&mut ctx);
    arenas.job.reset();

    let mut kept_one = false;
    for &dependent in
        &state.dependents[slot.dependents.start as usize..slot.dependents.end as usize]
    {
        let pending = &state.nodes[dependent as usize].pending_dependencies;
        if pending.fetch_sub(1, Ordering::AcqRel) == 1 {
            // This worker is about to look for its next entry and takes one of the nodes
            // it releases here itself, so the first one wakes nobody.
            release(shared, state, dependent, !core::mem::replace(&mut kept_one, true));
        }
    }
    if state.pending_nodes.fetch_sub(1, Ordering::AcqRel) == 1 {
        // The run's last access to its counters. Marking it finished and taking the waker
        // happen under one lock, so the future cannot resolve, and the graph cannot be run
        // again, while this worker is still in here.
        let waker = {
            let mut completion = lock(&state.completion);
            completion.finished = true;
            completion.waker.take()
        };
        let ordinal = state.ordinal();
        shared.end_run(ordinal);
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}
