//! One frame's declaration: its imports, its transients and its passes, in order.

use crate::exclusive::Exclusive;
use crate::id::{BufferId, ImageId, PassId};
use crate::pass::{ComputePassDecl, CopyPassDecl, RenderPassDecl};
use crate::recorder::{ComputeRecorder, CopyRecorder, RenderRecorder};
use crate::resource::{BufferImport, ImageImport, TransientImage};

/// A pass's recorder, boxed by the call that declared the pass. It is called at most once,
/// by the one recording job that owns it, so it is `FnOnce` and `Send` and need be neither
/// `Fn` nor `Sync`.
enum Recorder<'f> {
    /// A render pass's.
    Render(Box<dyn FnOnce(&mut RenderRecorder<'_>) + Send + 'f>),
    /// A compute pass's.
    Compute(Box<dyn FnOnce(&mut ComputeRecorder<'_>) + Send + 'f>),
    /// A copy pass's.
    Copy(Box<dyn FnOnce(&mut CopyRecorder<'_>) + Send + 'f>),
}

/// What one frame draws: built by the caller every frame, then given to
/// [`RenderGraph::compile`](crate::RenderGraph::compile).
///
/// A declaration is a value of its own. It borrows no graph, a call on it reaches no
/// device and checks nothing, and every breach of a declaration's rules is
/// [`compile`](crate::RenderGraph::compile)'s to report, naming the pass or the resource
/// by its label. It owns a copy of everything its calls are given and a box for each
/// recorder; `'f` is what the recorders borrow, shared or exclusively.
///
/// The order of the calls is the declaration: ids count in it, the key hashes in it, and
/// a pass sees what the passes declared before it wrote (`README.md`, "Uses and order").
/// A caller that wants one plan for its steady frames declares the same things in the
/// same order every frame.
///
/// `Send + Sync` whatever its recorders are, so that a job may build it for another to
/// compile and a `'static` one waits in a world's resource. What it copied is shared by
/// the recording jobs; each recorder is owned by the one job that calls it (`README.md`,
/// "Concurrency").
pub struct FrameDecl<'f> {
    /// The view count.
    views: u32,
    /// The recorders, by pass, reached through `&mut` or by value alone. The
    /// implementation keeps the copied declarations beside them.
    recorders: Exclusive<Vec<Recorder<'f>>>,
}

impl<'f> FrameDecl<'f> {
    /// An empty declaration for a frame of `views` views, at least one.
    ///
    /// The view count is hashed into the key by itself and read back from the plan
    /// ([`CompiledPlan::views`](crate::CompiledPlan::views)). The graph instances nothing
    /// by it: the caller declares each view's passes (`README.md`, "Not in this crate
    /// yet").
    #[must_use]
    pub fn new(views: u32) -> Self {
        todo!()
    }

    /// The view count the declaration was made with.
    #[must_use]
    pub fn views(&self) -> u32 {
        todo!()
    }

    /// Declares an image the caller made.
    pub fn import_image(&mut self, import: &ImageImport<'_>) -> ImageId {
        todo!()
    }

    /// Declares a buffer the caller made.
    pub fn import_buffer(&mut self, import: &BufferImport<'_>) -> BufferId {
        todo!()
    }

    /// Declares an image the graph makes, which lives from its first to its last kept use
    /// in this frame and is undefined when the frame begins.
    pub fn transient_image(&mut self, image: &TransientImage<'_>) -> ImageId {
        todo!()
    }

    /// Declares a render pass, after every pass declared so far.
    ///
    /// `record` is called by [`RenderGraph::record`](crate::RenderGraph::record), once if
    /// the pass is kept and never if it is culled, on a frame-critical worker, with the
    /// pass begun; the graph ends the pass when it returns. It may run while other
    /// passes' recorders run on other workers, in no promised order with them. Called or
    /// not, it is dropped before `RenderGraph::record` returns, or with the declaration.
    pub fn render_pass(
        &mut self,
        pass: &RenderPassDecl<'_>,
        record: impl FnOnce(&mut RenderRecorder<'_>) + Send + 'f,
    ) -> PassId {
        todo!()
    }

    /// Declares a compute pass, after every pass declared so far. `record` is called as a
    /// render pass's is ([`render_pass`](Self::render_pass)).
    pub fn compute_pass(
        &mut self,
        pass: &ComputePassDecl<'_>,
        record: impl FnOnce(&mut ComputeRecorder<'_>) + Send + 'f,
    ) -> PassId {
        todo!()
    }

    /// Declares a copy pass, after every pass declared so far. `record` is called as a
    /// render pass's is ([`render_pass`](Self::render_pass)), with no pass open on the
    /// encoder.
    pub fn copy_pass(
        &mut self,
        pass: &CopyPassDecl<'_>,
        record: impl FnOnce(&mut CopyRecorder<'_>) + Send + 'f,
    ) -> PassId {
        todo!()
    }
}
