//! The render graph (architecture section 12.3): a frame's passes, declared by the images
//! and buffers they use, compiled into the passes that are kept, the lifetimes and aliased
//! memory of the transients, the state transitions between passes and the queue each pass
//! runs on, then recorded on the frame-critical workers and submitted in graph order. It
//! reaches a device through `lactose-rhi-api` and names no backend.
//!
//! **Status: implemented, proved on the null backend.** `README.md` is the contract, and
//! every sentence under "Behaviour the signatures cannot express" is one a dependent crate
//! may rely on. The device tests come with the Vulkan backend.
//!
//! # Where to start
//!
//! - [`FrameDecl`] is one frame's declaration: imports ([`ImageImport`], [`BufferImport`]),
//!   transients ([`TransientImage`]) and passes ([`RenderPassDecl`], [`ComputePassDecl`],
//!   [`CopyPassDecl`]), each pass with the recorder that draws it.
//! - [`RenderGraph`] takes a frame through [`compile`](RenderGraph::compile),
//!   [`record`](RenderGraph::record) and [`submit`](RenderGraph::submit).
//! - [`CompiledPlan`] is what a declaration compiled to, for a test or a trace to read.
//! - [`RenderRecorder`], [`ComputeRecorder`] and [`CopyRecorder`] are what a recorder is
//!   lent.
//! - [`History`] is the ping-pong for a resource a frame reads from the frame before.
//!
//! # One frame
//!
//! ```no_run
//! use lactose_core::Result;
//! use lactose_render_graph::{
//!     BufferAccess, BufferImport, BufferUse, ColourUse, FrameDecl, ImageImport, RenderGraph,
//!     RenderPassDecl,
//! };
//! use lactose_rhi_api::{
//!     BufferHandle, BufferState, GraphicsPipelineHandle, ImageHandle, ImageState, Load,
//!     QueueRole, Sharing, StateOn, TimelinePoint,
//! };
//! use lactose_task::JobContext;
//!
//! fn on_graphics<S>(state: S) -> StateOn<S> {
//!     StateOn { state, queue: QueueRole::Graphics }
//! }
//!
//! fn frame(
//!     graph: &mut RenderGraph,
//!     ctx: &mut JobContext<'_>,
//!     target: ImageHandle,
//!     camera: BufferHandle,
//!     pipeline: GraphicsPipelineHandle,
//! ) -> Result<TimelinePoint> {
//!     let mut decl = FrameDecl::new(1);
//!     let target = decl.import_image(&ImageImport {
//!         label: "target",
//!         image: target,
//!         mips: 1,
//!         layers: 1,
//!         sharing: Sharing::Exclusive,
//!         initial: on_graphics(ImageState::Undefined),
//!         last: on_graphics(ImageState::ColourAttachment),
//!         handoff: None,
//!         ready: None,
//!     });
//!     let camera_id = decl.import_buffer(&BufferImport {
//!         label: "camera",
//!         buffer: camera,
//!         sharing: Sharing::Exclusive,
//!         initial: on_graphics(BufferState::Undefined),
//!         last: on_graphics(BufferState::ShaderRead),
//!         ready: None,
//!     });
//!     decl.render_pass(
//!         &RenderPassDecl {
//!             label: "triangle",
//!             colour: &[ColourUse {
//!                 image: target,
//!                 mip: 0,
//!                 layer: 0,
//!                 load: Load::Clear([0.0, 0.0, 0.0, 1.0]),
//!                 local_read: false,
//!             }],
//!             depth: None,
//!             sampled: &[],
//!             buffers: &[BufferUse { buffer: camera_id, access: BufferAccess::Read }],
//!             slots: &[],
//!         },
//!         move |pass| {
//!             pass.bind_pipeline(pipeline);
//!             pass.push_address(0, camera_id, 0);
//!             pass.draw(0..3, 0..1);
//!         },
//!     );
//!
//!     let compiled = graph.compile(decl)?;
//!     let recorded = graph.record(compiled, ctx)?;
//!     // The latch: the camera is written here, after every pass is recorded.
//!     graph.device().write_buffer(camera, 0, 64, &mut |bytes| bytes.fill(0))?;
//!     graph.device().flush_buffer(camera, 0, 64)?;
//!     graph.submit(recorded)
//! }
//! ```

#![forbid(unsafe_code)]

mod compile;
mod decl;
mod graph;
mod history;
mod id;
mod job;
mod pass;
mod plan;
mod recorder;
mod resource;
mod sync_view;

pub use decl::FrameDecl;
pub use graph::{GraphDesc, GraphStats, LOG_TARGET, RecordedFrame, RenderGraph};
pub use history::History;
pub use id::{BufferId, ImageId, PassId};
pub use pass::{
    BufferAccess, BufferUse, ColourUse, ComputePassDecl, CopyPassDecl, DepthUse, RenderPassDecl,
};
pub use plan::{CompiledFrame, CompiledPlan, GraphKey, PlannedPass};
pub use recorder::{ComputeRecorder, CopyRecorder, RenderRecorder};
pub use resource::{BufferImport, ImageImport, TransientImage, TransientMemory};

// What the contract promises at the type level, so that the implementation cannot lose it
// silently (`README.md`, "Concurrency").
const _: () = {
    const fn send_sync<T: ?Sized + Send + Sync>() {}
    const fn owned<T: Send + Sync + 'static>() {}
    const fn plain<T: Copy + Send + Sync + 'static>() {}

    // The graph and a recorded frame sit in a world's resource, which is
    // `Any + Send + Sync`, or behind a `dyn` object a frame is driven through.
    owned::<RenderGraph>();
    owned::<RecordedFrame>();
    // A plan is shared by the recording jobs and outlives the frame that compiled it.
    owned::<CompiledPlan>();

    // A declaration is shared by the recording jobs, whatever its recorders borrow, and
    // may be built by one job and compiled by another. The parameter is there to name the
    // lifetime: the assertion holds for every `'f`, not for `'static` alone.
    const fn declaration<'f>(_: std::marker::PhantomData<&'f ()>) {
        send_sync::<FrameDecl<'f>>();
        send_sync::<CompiledFrame<'f>>();
    }
    declaration(std::marker::PhantomData);

    // What a declaration is given, and what a plan reports, is plain data.
    plain::<ImageId>();
    plain::<BufferId>();
    plain::<PassId>();
    plain::<GraphKey>();
    plain::<PlannedPass>();
    plain::<TransientMemory>();
    plain::<GraphDesc>();
    plain::<GraphStats>();
    plain::<BufferAccess>();
    plain::<BufferUse>();
    plain::<ColourUse>();
    plain::<DepthUse>();
    plain::<ImageImport<'static>>();
    plain::<BufferImport<'static>>();
    plain::<TransientImage<'static>>();
    plain::<RenderPassDecl<'static>>();
    plain::<ComputePassDecl<'static>>();
    plain::<CopyPassDecl<'static>>();
    plain::<History<u64>>();

    // An id is four bytes and a key eight: a declaration copies them freely.
    assert!(size_of::<ImageId>() == 4);
    assert!(size_of::<BufferId>() == 4);
    assert!(size_of::<PassId>() == 4);
    assert!(size_of::<GraphKey>() == 8);
};
