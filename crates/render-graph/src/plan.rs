//! What a declaration compiles to, and the key it is cached under.

use std::ops::Range;
use std::sync::Arc;

use lactose_rhi_api::{
    BufferState, ImageHandle, ImageRange, ImageState, MemoryBlockHandle, QueueLayout, QueueRole,
    StateOn, Store,
};

use crate::decl::FrameDecl;
use crate::id::{ImageId, PassId};
use crate::resource::TransientMemory;

/// The hash a plan is cached under (architecture section 12.3): of the view count, the
/// device's queue layout, every resource's declaration but its handle and its `ready`
/// point, and every pass's declaration but its clear values, its heap slots and its
/// recorder, each in declaration order, with each use under the list it is declared in
/// (`README.md`, "Compile and its cache").
///
/// Equal declarations under equal layouts have equal keys, in every process of one build.
/// Two declarations with equal keys may differ: the cache compares what it hashed before
/// it calls a hit, so a collision compiles again and
/// [`CompiledFrame::fresh`](crate::CompiledFrame::fresh) is exact where a key is not.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GraphKey(u64);

impl GraphKey {
    /// The key whose hash is `value`.
    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The hash, for a log line or a trace.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// A kept pass, as a plan orders it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PlannedPass {
    /// The pass.
    pub pass: PassId,
    /// The role whose encoder records the pass and whose queue runs it: `Graphics`, or
    /// `Compute` for a compute pass the graph moved to the compute queue. Never `Present`,
    /// and never `Compute` on a device whose compute role shares the graphics queue.
    pub queue: QueueRole,
    /// Which of the frame's submits carries the pass, counted from zero in the order the
    /// submits are made.
    pub submit: u32,
}

/// The mark every plan a graph compiles carries, compared by address: how `record` and
/// `submit` tell a frame of another graph from one whose plan this graph evicted
/// (`README.md`, "What each consumer needs").
pub(crate) struct Owner;

/// What a declaration compiles to: the kept passes in graph order, each with its queue and
/// its submit, where each transient lives, and every transition, alias call, store action
/// and wait the frame records. It holds no handle of an import, so every frame whose
/// declaration matches is recorded from it.
///
/// Immutable once compiled, `Send + Sync`, and shared by the recording jobs. A plan is a
/// function of the declaration, the device's queue layout and the device's capabilities
/// and placements alone: nothing in it depends on a thread, an address or the order a
/// hash map iterates in.
pub struct CompiledPlan {
    /// The key the plan is cached under.
    key: GraphKey,
    /// The graph that compiled it.
    pub(crate) owner: Arc<Owner>,
    /// The declaration's view count.
    views: u32,
    /// The device's layout.
    queues: QueueLayout,
    /// The kept passes in graph order.
    passes: Vec<PlannedPass>,
    /// For each declared pass, whether it was culled.
    culled: Vec<bool>,
    /// For each declared image, the transient the plan made for it.
    pub(crate) transients: Vec<Option<Transient>>,
    /// The memory blocks the placed transients are in, by number.
    pub(crate) blocks: Vec<MemoryBlockHandle>,
    /// The frame's submits, in the order they are made.
    pub(crate) submits: Vec<SubmitPlan>,
    /// The frame's recording jobs, in submission order: each submit's are a run of them.
    pub(crate) jobs: Vec<JobPlan>,
}

/// A transient the plan made.
#[derive(Clone, Copy)]
pub(crate) struct Transient {
    /// The image.
    pub(crate) image: ImageHandle,
    /// Where it lives.
    pub(crate) memory: TransientMemory,
}

/// One submit of a frame.
pub(crate) struct SubmitPlan {
    /// The role it is made to: `Graphics` or `Compute`.
    pub(crate) queue: QueueRole,
    /// Its jobs, as indices into the plan's.
    pub(crate) jobs: Range<usize>,
    /// The earlier submits of the frame, on another queue, whose points it waits for.
    pub(crate) waits: Vec<u32>,
    /// The imports its commands name, whose `ready` points it waits for.
    pub(crate) names: Vec<Resource>,
}

/// An image or a buffer of a declaration, by index.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Resource {
    Image(u32),
    Buffer(u32),
}

/// One recording job: the steps it records into its one encoder, in order.
pub(crate) struct JobPlan {
    /// The role of its encoder.
    pub(crate) queue: QueueRole,
    /// The declared passes from its first kept pass to its last, whose recorders it
    /// owns; empty for a job of transitions alone.
    pub(crate) passes: Range<usize>,
    /// What it records.
    pub(crate) steps: Vec<Step>,
}

/// What a job records, in order.
pub(crate) enum Step {
    /// A kept pass, inside its span.
    Pass(PassStep),
    /// Transitions outside every pass's span: a closing transition, a release, or an
    /// acquire at the frame's edge.
    Transitions(Transitions),
}

/// A kept pass: the alias calls and transitions before it, its beginning, its recorder and
/// its end.
pub(crate) struct PassStep {
    /// The declared pass.
    pub(crate) pass: usize,
    /// The transients that begin before the pass by an alias call.
    pub(crate) alias: Vec<u32>,
    /// The transitions before the pass.
    pub(crate) before: Transitions,
    /// What the pass is begun with, beside what its declaration gives.
    pub(crate) begin: Begin,
}

/// What a pass is begun with that the plan derived: the store actions of its attachments
/// and the buffers of its reach. The targets, the loads and the slots are the
/// declaration's.
pub(crate) enum Begin {
    /// A render pass: a store action for each colour attachment and for the depth one.
    Render { colour: Vec<Store>, depth: Option<Store>, reach: Vec<(u32, BufferState)> },
    /// A compute pass.
    Compute { reach: Vec<(u32, BufferState)> },
    /// A copy pass, which begins nothing on the encoder.
    Copy,
}

/// Transitions over ids, resolved to the frame's handles when recorded.
#[derive(Default)]
pub(crate) struct Transitions {
    pub(crate) images: Vec<PlannedImageTransition>,
    pub(crate) buffers: Vec<PlannedBufferTransition>,
}

impl Transitions {
    pub(crate) fn is_empty(&self) -> bool {
        self.images.is_empty() && self.buffers.is_empty()
    }

    pub(crate) fn append(&mut self, other: &mut Self) {
        self.images.append(&mut other.images);
        self.buffers.append(&mut other.buffers);
    }
}

/// An image's transition, over every mip and layer it was declared with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct PlannedImageTransition {
    pub(crate) image: u32,
    pub(crate) range: ImageRange,
    pub(crate) from: StateOn<ImageState>,
    pub(crate) to: StateOn<ImageState>,
}

/// A buffer's transition.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct PlannedBufferTransition {
    pub(crate) buffer: u32,
    pub(crate) from: StateOn<BufferState>,
    pub(crate) to: StateOn<BufferState>,
}

/// What `compile` derived, before the device objects are made.
pub(crate) struct PlanParts {
    pub(crate) views: u32,
    pub(crate) queues: QueueLayout,
    pub(crate) passes: Vec<PlannedPass>,
    pub(crate) culled: Vec<bool>,
    pub(crate) submits: Vec<SubmitPlan>,
    pub(crate) jobs: Vec<JobPlan>,
}

impl CompiledPlan {
    /// The plan of `parts`, whose transients and blocks are made.
    pub(crate) fn new(
        key: GraphKey,
        owner: Arc<Owner>,
        parts: PlanParts,
        transients: Vec<Option<Transient>>,
        blocks: Vec<MemoryBlockHandle>,
    ) -> Self {
        let PlanParts { views, queues, passes, culled, submits, jobs } = parts;
        Self { key, owner, views, queues, passes, culled, transients, blocks, submits, jobs }
    }

    /// The key the plan is cached under.
    #[must_use]
    pub fn key(&self) -> GraphKey {
        self.key
    }

    /// The view count of the declaration.
    #[must_use]
    pub fn views(&self) -> u32 {
        self.views
    }

    /// The queue layout the plan was compiled for: the device's.
    #[must_use]
    pub fn queues(&self) -> QueueLayout {
        self.queues
    }

    /// The kept passes in graph order: declaration order, with the culled passes left
    /// out. Their `submit` numbers never decrease along it.
    #[must_use]
    pub fn passes(&self) -> &[PlannedPass] {
        &self.passes
    }

    /// Whether `pass` was culled: no import is written by it, and no transient it writes
    /// is used by a kept pass declared after it. `false` for an id the declaration did
    /// not issue.
    #[must_use]
    pub fn is_culled(&self, pass: PassId) -> bool {
        self.culled.get(pass.index() as usize).copied().unwrap_or(false)
    }

    /// How many submits a frame of this plan makes, the ones that carry no pass counted:
    /// one where every command of the frame is for one queue.
    #[must_use]
    pub fn submits(&self) -> u32 {
        count(self.submits.len())
    }

    /// How many recording jobs, and so encoders, a frame of this plan takes: at most
    /// [`GraphDesc::jobs`](crate::GraphDesc::jobs) for each submit that carries a pass,
    /// and one for each submit that carries transitions alone.
    #[must_use]
    pub fn jobs(&self) -> u32 {
        count(self.jobs.len())
    }

    /// Where the plan put the transient `image`. `None` for an import, for a transient no
    /// kept pass uses, which is never made, and for an id the declaration did not issue.
    #[must_use]
    pub fn transient(&self, image: ImageId) -> Option<TransientMemory> {
        self.transients
            .get(image.index() as usize)
            .copied()
            .flatten()
            .map(|transient| transient.memory)
    }
}

/// A count of a plan's lists, which `compile` keeps below 2^32 by the ids it counts in.
pub(crate) fn count(len: usize) -> u32 {
    u32::try_from(len).expect("a plan's lists are counted in the declaration's ids")
}

/// A declaration and the plan it compiled to: what
/// [`RenderGraph::record`](crate::RenderGraph::record) takes.
///
/// It borrows no graph. It holds its plan, so the plan's transients outlive it whatever
/// the cache evicts. Dropped, it leaves the graph as it was.
pub struct CompiledFrame<'f> {
    /// The declaration: the frame's handles, clear values, slots and recorders.
    pub(crate) decl: FrameDecl<'f>,
    /// The plan.
    pub(crate) plan: Arc<CompiledPlan>,
    /// Whether this compile made the plan.
    pub(crate) fresh: bool,
}

impl CompiledFrame<'_> {
    /// The plan the frame is recorded from.
    #[must_use]
    pub fn plan(&self) -> &CompiledPlan {
        &self.plan
    }

    /// Whether the call that returned this frame compiled its plan: `true` on a miss,
    /// `false` when the cache held a plan of an equal declaration.
    #[must_use]
    pub fn fresh(&self) -> bool {
        self.fresh
    }
}
