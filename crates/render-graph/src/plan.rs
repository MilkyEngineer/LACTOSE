//! What a declaration compiles to, and the key it is cached under.

use std::sync::Arc;

use lactose_rhi_api::{QueueLayout, QueueRole};

use crate::decl::FrameDecl;
use crate::id::{ImageId, PassId};
use crate::resource::TransientMemory;

/// The hash a plan is cached under (architecture section 12.3): of the view count, the
/// device's queue layout, every resource's declaration but its handle and its `ready`
/// point, and every pass's declaration but its clear values, its heap slots and its
/// recorder, each in declaration order (`README.md`, "Compile and its cache").
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
}

impl CompiledPlan {
    /// The key the plan is cached under.
    #[must_use]
    pub fn key(&self) -> GraphKey {
        todo!()
    }

    /// The view count of the declaration.
    #[must_use]
    pub fn views(&self) -> u32 {
        todo!()
    }

    /// The queue layout the plan was compiled for: the device's.
    #[must_use]
    pub fn queues(&self) -> QueueLayout {
        todo!()
    }

    /// The kept passes in graph order: declaration order, with the culled passes left
    /// out. Their `submit` numbers never decrease along it.
    #[must_use]
    pub fn passes(&self) -> &[PlannedPass] {
        todo!()
    }

    /// Whether `pass` was culled: no import is written by it, and no transient it writes
    /// is used by a kept pass declared after it. `false` for an id the declaration did
    /// not issue.
    #[must_use]
    pub fn is_culled(&self, pass: PassId) -> bool {
        todo!()
    }

    /// How many submits a frame of this plan makes, the ones that carry no pass counted:
    /// one where every command of the frame is for one queue.
    #[must_use]
    pub fn submits(&self) -> u32 {
        todo!()
    }

    /// How many recording jobs, and so encoders, a frame of this plan takes: at most
    /// [`GraphDesc::jobs`](crate::GraphDesc::jobs) for each submit that carries a pass,
    /// and one for each submit that carries transitions alone.
    #[must_use]
    pub fn jobs(&self) -> u32 {
        todo!()
    }

    /// Where the plan put the transient `image`. `None` for an import, for a transient no
    /// kept pass uses, which is never made, and for an id the declaration did not issue.
    #[must_use]
    pub fn transient(&self, image: ImageId) -> Option<TransientMemory> {
        todo!()
    }
}

/// A declaration and the plan it compiled to: what
/// [`RenderGraph::record`](crate::RenderGraph::record) takes.
///
/// It borrows no graph. It holds its plan, so the plan's transients outlive it whatever
/// the cache evicts. Dropped, it leaves the graph as it was.
pub struct CompiledFrame<'f> {
    /// The declaration: the frame's handles, clear values, slots and recorders.
    decl: FrameDecl<'f>,
    /// The plan.
    plan: Arc<CompiledPlan>,
    /// Whether this compile made the plan.
    fresh: bool,
}

impl CompiledFrame<'_> {
    /// The plan the frame is recorded from.
    #[must_use]
    pub fn plan(&self) -> &CompiledPlan {
        todo!()
    }

    /// Whether the call that returned this frame compiled its plan: `true` on a miss,
    /// `false` when the cache held a plan of an equal declaration.
    #[must_use]
    pub fn fresh(&self) -> bool {
        todo!()
    }
}
