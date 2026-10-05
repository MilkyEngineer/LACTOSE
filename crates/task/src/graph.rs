//! Continuation-counted graphs: the shape every schedule stage and render-graph record
//! phase runs in.

use core::ops::Range;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::sync::Arc;

use crate::job::JobContext;

/// One system, pass or other unit of a [`Graph`].
///
/// A node owns or shares its data (`'static`); the executor never borrows from the caller
/// across threads. `lactose-ecs` implements this for a system bound to a world and takes
/// responsibility there for the declared-access discipline that makes shared access sound.
pub trait GraphNode: Send + Sync + 'static {
    /// The name used for the tracing span emitted once per run and once per stolen range.
    fn name(&self) -> &'static str;

    /// How many units of work this node has (chunks, for a system). Called once per run, at
    /// the moment the node is released, so it may depend on what earlier nodes did.
    fn units(&self) -> u32;

    /// Processes `units`. Called concurrently on disjoint ranges that together cover
    /// `0..self.units()` exactly once. The executor starts with the whole range and halves a
    /// range each time it is stolen; there is no target job duration.
    fn run(&self, units: Range<u32>, ctx: &mut JobContext<'_>);

    /// Runs exactly once per run, after every unit has been processed and every scope opened
    /// by [`GraphNode::run`] has closed, and before any dependent node is released.
    /// Reductions merge their partial results here. Also called when `units()` is zero.
    fn finalize(&self, ctx: &mut JobContext<'_>) {}
}

/// Identifies a node within the graph that issued it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct NodeId(pub u32);

/// Why a graph could not be built.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum GraphError {
    /// The dependencies contain a cycle through these nodes.
    Cycle(Vec<NodeId>),
    /// An edge names a node this builder did not issue.
    UnknownNode(NodeId),
}

/// Collects nodes and dependencies into a [`Graph`].
#[derive(Default)]
pub struct GraphBuilder {
    _private: (),
}

impl GraphBuilder {
    /// An empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a node. IDs are issued in call order starting at zero.
    pub fn add_node(&mut self, node: Arc<dyn GraphNode>) -> NodeId {
        todo!()
    }

    /// Declares that `after` is released only once `before` has finalized.
    pub fn add_dependency(&mut self, before: NodeId, after: NodeId) {
        todo!()
    }

    /// Validates the dependencies and builds the graph.
    pub fn build(self) -> Result<Graph, GraphError> {
        todo!()
    }
}

/// An immutable dependency graph, built once and run every frame.
pub struct Graph {
    _private: (),
}

impl Graph {
    /// The number of nodes.
    #[must_use]
    pub fn len(&self) -> usize {
        todo!()
    }

    /// Whether the graph has no nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        todo!()
    }

    /// The longest dependency chain, in nodes. The benchmark harness divides makespan by
    /// the summed duration along this chain.
    #[must_use]
    pub fn critical_path_len(&self) -> usize {
        todo!()
    }
}

/// Resolves when a graph run has finished. See [`crate::Executor::run_graph`].
#[must_use = "futures do nothing unless awaited"]
pub struct GraphRun {
    _private: (),
}

impl Future for GraphRun {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        todo!()
    }
}
