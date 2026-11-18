//! The graph: the plan cache, the transients and the encoders, and the three steps a frame
//! takes through them.

use std::sync::Arc;

use lactose_core::Result;
use lactose_rhi_api::{Device, Encoder, TimelinePoint};
use lactose_task::JobContext;

use crate::decl::FrameDecl;
use crate::plan::{CompiledFrame, CompiledPlan};
use crate::sync_view::SyncView;

/// The target of the graph's spans and events. Every kept pass is recorded inside one
/// span of this target named `pass`, at trace level, with the pass's label as its `label`
/// field.
pub const LOG_TARGET: &str = "gouda";

/// What a graph is made with.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GraphDesc {
    /// The most recording jobs the passes of one submit are split into, at least one. A
    /// submit that carries `n` passes is recorded by the smaller of `n` and `jobs` jobs,
    /// each a run of consecutive passes with an encoder of its own.
    pub jobs: u32,
    /// The most plans the cache keeps, at least one. A plan compiled into a full cache
    /// evicts the plan least recently returned by
    /// [`compile`](RenderGraph::compile).
    pub plans: u32,
}

/// What a graph has done so far: counts a test or a trace reads. None is a timing.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GraphStats {
    /// The plans compiled: the calls to [`compile`](RenderGraph::compile) that missed the
    /// cache and succeeded.
    pub compiles: u64,
    /// The calls to [`compile`](RenderGraph::compile) that the cache answered.
    pub hits: u64,
    /// The plans evicted from the cache.
    pub evictions: u64,
    /// The plans the cache holds now.
    pub plans: u32,
    /// The encoders the graph holds: free, and pending in a submitted frame. A recorded
    /// frame's encoders are its own from `record`'s return to `submit`, and are not
    /// counted between the two.
    pub encoders: u32,
}

/// The render graph of one device (architecture section 12.3).
///
/// A frame goes through it in three calls, each consuming what the one before returned:
/// [`compile`](Self::compile), [`record`](Self::record) and [`submit`](Self::submit).
/// Between `record` and `submit` the caller latches: it writes the buffers the frame reads
/// last, such as the camera's, and nothing recorded depends on their bytes.
///
/// `Send + Sync + 'static`, so that it sits in a world's resource or behind a `dyn`
/// object, and every call that changes it takes `&mut self`: one thread at a time by the
/// borrow, with no lock (`README.md`, "Concurrency").
///
/// Its drop waits, with no bound, until every submit it made has completed, then
/// destroys the transients, the memory blocks, the timelines and the encoders it made. It
/// is dropped off the frame-critical workers, as a device's objects are, and after the
/// host has signalled every point a pending frame waits for. A drop made while the thread
/// unwinds a panic does not wait: it destroys what no pending submit uses and leaks the
/// rest, as the device's own drops do (`README.md`, "Errors and drops").
pub struct RenderGraph {
    /// The device every call reaches.
    device: Arc<dyn Device>,
    /// The encoders no frame holds.
    encoders: SyncView<Vec<Box<dyn Encoder>>>,
}

impl RenderGraph {
    /// A graph for `device`, with an empty cache and no encoder. Reads the device's
    /// capabilities, which are fixed, and makes the timelines its submits signal.
    ///
    /// # Errors
    ///
    /// `ErrorKind::InvalidData` where `desc.jobs` or `desc.plans` is zero; what
    /// `Device::create_timeline` returns.
    pub fn new(device: Arc<dyn Device>, desc: &GraphDesc) -> Result<Self> {
        todo!()
    }

    /// The device the graph was made for.
    #[must_use]
    pub fn device(&self) -> &Arc<dyn Device> {
        todo!()
    }

    /// Compiles `decl`, or finds the plan an equal declaration compiled to.
    ///
    /// On a miss it derives the plan and makes the transients a kept pass uses and the
    /// blocks they are placed in; on a hit it makes nothing. Either way it may destroy
    /// the transients of evicted plans that no frame holds and whose last submitted
    /// frame has completed, which it learns by reading a timeline's value and never by
    /// waiting.
    ///
    /// # Errors
    ///
    /// `ErrorKind::InvalidData`, naming the pass or the resource by its label, for a
    /// declaration that breaks a rule of `README.md`, "Declaring"; what the device
    /// returns for a transient it cannot make (`Unsupported`, `ResourceExhausted`,
    /// `DeviceLost`). After an error the cache holds no plan for the declaration and the
    /// graph is as it was.
    pub fn compile<'f>(&mut self, decl: FrameDecl<'f>) -> Result<CompiledFrame<'f>> {
        todo!()
    }

    /// Records `frame`: forks its recording jobs in one scope of `ctx` and returns when
    /// every one has finished.
    ///
    /// Each job owns one encoder and records a run of consecutive passes into it, in
    /// graph order: for each pass, inside one span, the alias calls and transitions the
    /// plan derived, the pass's beginning, its recorder, and its end. The jobs run on the
    /// frame-critical workers, the calling worker among them, and share nothing they
    /// write: each owns its encoder and the recorders of its passes (`README.md`,
    /// "Concurrency"). A recorder does not block or wait: not for another recorder, which
    /// on one worker runs after it, not for a lock and not for a timeline point. A panic
    /// in a recorder aborts the process, as any scope child's does.
    ///
    /// An encoder is taken from the graph's free ones, or made. A pending encoder becomes
    /// free, and is reset, once the point of the frame it was submitted in is seen
    /// reached, which the graph learns by reading the timeline's value and never by
    /// waiting.
    ///
    /// # Errors
    ///
    /// `ErrorKind::InvalidData` for a frame another graph compiled, for a recorder that
    /// named an id its pass did not declare for that use, and for a copy pass's recorder
    /// that wrote one destination in two calls, naming the pass and the call, the first
    /// such pass in graph order where there are several; what
    /// `Device::create_encoder` returns. After an error nothing of the frame is
    /// submitted or can be, and its encoders are free again.
    pub fn record(
        &mut self,
        frame: CompiledFrame<'_>,
        ctx: &mut JobContext<'_>,
    ) -> Result<RecordedFrame> {
        todo!()
    }

    /// Submits `frame`: one `Device::submit` for each submit of its plan, in order, and
    /// returns the frame's point, which is reached when every one of them has completed.
    ///
    /// Frame points are values of one timeline of the graph's and grow with every frame
    /// submitted, so a point reached means that frame's submits and every earlier
    /// frame's have completed. The call waits for nothing.
    ///
    /// # Errors
    ///
    /// `ErrorKind::InvalidData` for a frame another graph recorded; otherwise what
    /// `Device::submit` returned, unchanged, so that `Rule::of` names the rule a refusal
    /// broke. An error from the frame's first submit leaves every import as the
    /// declaration found it. An error from a later one leaves the frame torn: the
    /// submits before it were made, the frame's point is never signalled for it, and the
    /// states of the frame's imports are no longer the graph's word. The error does not
    /// say which submit failed, so the caller takes every error as a torn frame: it
    /// imports the frame's imports next from `Undefined`, relies on no buffer's bytes, and
    /// calls [`History::reset`](crate::History::reset) on every history the frame
    /// imported. The next frame submitted whole waits for what the torn frame submitted,
    /// and its point covers it (`README.md`, "Submission").
    pub fn submit(&mut self, frame: RecordedFrame) -> Result<TimelinePoint> {
        todo!()
    }

    /// The point of the last frame [`submit`](Self::submit) returned, or `None` before
    /// the first.
    #[must_use]
    pub fn last_frame(&self) -> Option<TimelinePoint> {
        todo!()
    }

    /// The graph's counts.
    #[must_use]
    pub fn stats(&self) -> GraphStats {
        todo!()
    }
}

impl Drop for RenderGraph {
    /// Waits until every submit the graph made has completed, then destroys what the
    /// graph made. An error from a destroy is logged under [`LOG_TARGET`] and dropped. On
    /// a lost device the wait returns at once. While the thread unwinds a panic it waits
    /// for nothing: it makes no destroy a pending submit would refuse, and its encoders'
    /// drops leak what is pending.
    fn drop(&mut self) {
        todo!()
    }
}

/// A frame recorded and not yet submitted: its encoders, each holding the commands of one
/// recording job, and its plan.
///
/// `Send + Sync + 'static` and borrowing nothing, so that it waits in a world's resource
/// while the frame latches. It holds no recorder: each was called, or dropped, before
/// `record` returned. Dropped unsubmitted, it drops its encoders, none of which is
/// pending, and the graph makes others.
pub struct RecordedFrame {
    /// The plan the frame was recorded from.
    plan: Arc<CompiledPlan>,
    /// The frame's encoders, in the order its submits take them.
    encoders: SyncView<Vec<Box<dyn Encoder>>>,
}
