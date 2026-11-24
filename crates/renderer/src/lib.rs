//! The minimal renderer (architecture sections 5.3, 12.4 and 12.5; the milestone 2 plan,
//! decision 5.4): the render slot table keyed by `StableKey`, the staging ring that extract
//! and the remote upload write and nothing reads on the host, the scatter that moves a
//! frame's rows into the persistent instance buffer, triangles drawn by instance from that
//! buffer, the orbit camera latched before the submit, and the trait through which another
//! crate adds passes to the frame. It reaches a device through `lactose-rhi-api` and a
//! frame through `lactose-render-graph`, and names no backend.
//!
//! **Status: implemented, proved on the null backend and on Vulkan devices.** `README.md` is
//! the contract, and every sentence under "Behaviour the signatures cannot express" is one a
//! dependent crate may rely on.
//!
//! # Where to start
//!
//! - [`Renderer`] owns the buffers, the pipelines and the [`SlotTable`]. A frame goes
//!   through [`open`](Renderer::open), the writers, [`prepare`](Renderer::prepare),
//!   [`declare`](Renderer::declare), [`latch`](Renderer::latch) and
//!   [`submitted`](Renderer::submitted).
//! - [`ExtractSink`] is the `InstanceSink` extract writes a world's instances through;
//!   [`RemoteWriter`] is what the presentation schedule writes remote instances through.
//!   Neither offers a read.
//! - [`ScatterBatch`], [`ScatterEntry`], [`StagingRow`] and [`InstanceRecord`] are what the
//!   scatter is given and what it leaves, with its rule as a function of the host.
//! - [`CameraData`] is the latched camera, and [`CameraData::orbit`] the one statement of
//!   how it is computed.
//! - [`RenderFeature`] is the pass-provider trait; [`graph`] re-exports what a provider
//!   names, for the crates `crate-tiers.toml` gives this crate alone.
//! - [`shader`] is the interface the three cooked shaders are written against.
//!
//! # One frame
//!
//! ```no_run
//! use lactose_core::Result;
//! use lactose_input_host::LatestSample;
//! use lactose_math::render::RenderPosition;
//! use lactose_render_graph::{FrameDecl, ImageImport, RenderGraph};
//! use lactose_renderer::{CameraLatch, FrameView, Opened, Projection, Renderer};
//! use lactose_rhi_api::{
//!     Extent, ImageHandle, ImageState, QueueRole, Sharing, StateOn, TimelinePoint,
//! };
//! use lactose_task::JobContext;
//!
//! /// Prepare, declare, compile, record, latch and submit: the frame's point.
//! fn submit_frame(
//!     renderer: &mut Renderer,
//!     graph: &mut RenderGraph,
//!     ctx: &mut JobContext<'_>,
//!     target: ImageHandle,
//!     extent: Extent,
//!     pivot: RenderPosition,
//!     input: &LatestSample,
//! ) -> Result<TimelinePoint> {
//!     // Extract's passes and the remote upload have written the open region. A kind
//!     // lost, or rows of a kind dropped or unplaced, the frame answers with a full pass
//!     // of that world (`README.md`, "What each consumer needs").
//!     let _counts = renderer.prepare()?;
//!
//!     let mut decl = FrameDecl::new(1);
//!     let on_graphics = |state| StateOn { state, queue: QueueRole::Graphics };
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
//!     renderer.declare(&mut decl, &FrameView { target, extent, fraction: 0.5 })?;
//!     let compiled = graph.compile(decl)?;
//!     let recorded = graph.record(compiled, ctx)?;
//!
//!     // The latch: the newest yaw and pitch, read inside the call, after every pass is
//!     // recorded and immediately before the submit, at the extent declared above.
//!     let projection = Projection { fov_y: 1.0, near: 0.1, far: 1000.0 };
//!     renderer.latch(&CameraLatch { pivot, input, boom: 4.0, projection })?;
//!     graph.submit(recorded)
//! }
//!
//! /// The frame's first step, before its writers run, made again after a frame that
//! /// failed: a region open, or the point the frame waits for, on its own thread and off
//! /// the frame-critical workers, before it calls this again.
//! fn open(
//!     renderer: &mut Renderer,
//!     graph: &mut RenderGraph,
//!     ctx: &mut JobContext<'_>,
//! ) -> Result<Option<TimelinePoint>> {
//!     loop {
//!         match renderer.open()? {
//!             Opened::Ready => return Ok(None),
//!             Opened::Wait(point) => return Ok(Some(point)),
//!             // After torn frames alone: a frame with no pass still signals a point.
//!             Opened::Unsubmitted => {
//!                 let empty = graph.compile(FrameDecl::new(1))?;
//!                 let recorded = graph.record(empty, ctx)?;
//!                 renderer.submitted(graph.submit(recorded)?);
//!             }
//!         }
//!     }
//! }
//!
//! /// The frame's last steps, once its writers have run: the renderer is told how the
//! /// frame ended, whichever step failed, and the next frame begins with `open` either
//! /// way. `torn` does nothing where no frame was prepared.
//! fn frame(
//!     renderer: &mut Renderer,
//!     graph: &mut RenderGraph,
//!     ctx: &mut JobContext<'_>,
//!     target: ImageHandle,
//!     extent: Extent,
//!     pivot: RenderPosition,
//!     input: &LatestSample,
//! ) -> Result<TimelinePoint> {
//!     match submit_frame(renderer, graph, ctx, target, extent, pivot, input) {
//!         Ok(point) => {
//!             renderer.submitted(point);
//!             Ok(point)
//!         }
//!         Err(error) => {
//!             renderer.torn();
//!             Err(error)
//!         }
//!     }
//! }
//! ```

#![forbid(unsafe_code)]

mod batch;
mod camera;
mod feature;
mod frame;
mod key;
#[cfg(all(test, feature = "loom"))]
mod models;
mod pipeline;
mod scatter;
pub mod shader;
mod slots;
mod staging;
mod sync;

/// What a pass provider names from `lactose-render-graph`, re-exported so that a crate
/// `crate-tiers.toml` allows this crate and not that one (`lactose-terrain`,
/// `lactose-debug-ui`) can implement [`RenderFeature`](crate::RenderFeature). Each item
/// is the render graph's own, under its own contract (`crates/render-graph/README.md`).
pub mod graph {
    pub use lactose_render_graph::{
        BufferAccess, BufferId, BufferImport, BufferUse, ColourUse, ComputePassDecl,
        ComputeRecorder, CopyPassDecl, CopyRecorder, DepthUse, FrameDecl, ImageId, ImageImport,
        PassId, RenderPassDecl, RenderRecorder, TransientImage,
    };
}

pub use camera::{CameraData, CameraLatch, LatchedCamera, Projection};
pub use feature::{FeatureView, RenderFeature};
pub use frame::{DeclaredFrame, FrameStats, FrameView, LOG_TARGET, Opened, Renderer, RendererDesc};
pub use scatter::{
    BatchRange, CellTransform, InstanceRecord, ScatterBatch, ScatterEntry, StagingRow,
};
pub use shader::Shaders;
pub use slots::{Slot, SlotHolder, SlotIter, SlotTable};
pub use staging::{ExtractSink, RemoteWriter};

// What the contract promises at the type level, so that the implementation cannot lose it
// silently (`README.md`, "Concurrency" and "Formats").
const _: () = {
    use lactose_present_api::{InstanceSink, RenderTransform};
    use lactose_rhi_api::PUSH_BYTES_MAX;

    const fn send_sync<T: ?Sized + Send + Sync>() {}
    const fn owned<T: Send + Sync + 'static>() {}
    const fn plain<T: Copy + Send + Sync + 'static>() {}
    // What `ExtractState::set_sink` takes in a box, and `World::insert_service` holds.
    const fn sink<T: InstanceSink + 'static>() {}

    // The renderer sits in a world's resource or behind a `dyn` object the frame is
    // driven through, and is handed from the job that prepares to the job that latches.
    owned::<Renderer>();
    // A sink is boxed into a world's extract state, and its `write` is called from
    // extract's forked jobs through `&self`.
    owned::<ExtractSink>();
    sink::<ExtractSink>();
    // The slot table is read by reference from any job of the render side.
    owned::<SlotTable>();
    // A provider is boxed into the renderer, and its recorders borrow it on the
    // recording jobs. Naming `dyn RenderFeature` is also what holds the trait to being
    // usable as a trait object.
    send_sync::<dyn RenderFeature>();
    owned::<Box<dyn RenderFeature>>();

    // The remote writer is shared by the presentation schedule's jobs, whatever it
    // borrows. The parameter is there to name the lifetime: the assertion holds for every
    // `'r`, not for `'static` alone.
    const fn borrowed<'r>(_: std::marker::PhantomData<&'r ()>) {
        send_sync::<RemoteWriter<'r>>();
        send_sync::<ScatterBatch<'r>>();
        send_sync::<SlotIter<'r>>();
    }
    borrowed(std::marker::PhantomData);

    // What a frame is given and what it reports is plain data.
    plain::<Slot>();
    plain::<SlotHolder>();
    plain::<StagingRow>();
    plain::<ScatterEntry>();
    plain::<InstanceRecord>();
    plain::<CellTransform>();
    plain::<BatchRange>();
    plain::<ScatterBatch<'static>>();
    plain::<CameraData>();
    plain::<LatchedCamera>();
    plain::<Projection>();
    plain::<CameraLatch<'static>>();
    plain::<FrameView>();
    plain::<FeatureView>();
    plain::<DeclaredFrame>();
    plain::<FrameStats>();
    plain::<Opened>();
    plain::<RendererDesc<'static>>();
    plain::<Shaders<'static>>();

    // The formats of `README.md`, "Formats": a placement is forty bytes, and each record
    // is the size the shaders step by.
    assert!(size_of::<RenderTransform>() == 40);
    assert!(size_of::<Slot>() == 4);
    assert!(StagingRow::BYTES == 40 + 8);
    assert!(ScatterEntry::BYTES == 12);
    assert!(InstanceRecord::BYTES == 3 * 40 + 8);
    assert!(CameraData::BYTES == 64 + 16);
    // Both push blocks fit every backend's.
    assert!(shader::SCATTER_PUSH_BYTES <= PUSH_BYTES_MAX);
    assert!(shader::TRIANGLE_PUSH_BYTES <= PUSH_BYTES_MAX);
    // An entry's flags are distinct bits.
    assert!(ScatterEntry::FREE & ScatterEntry::TAKE_BASE == 0);
    assert!(ScatterEntry::FREE & ScatterEntry::HAS_BASE == 0);
    assert!(ScatterEntry::TAKE_BASE & ScatterEntry::HAS_BASE == 0);
};
