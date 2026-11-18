//! The renderer: what it is made with, and the steps a frame takes through it.

use std::sync::Arc;

use lactose_core::Result;
use lactose_ecs::WorldKind;
use lactose_render_graph::{BufferId, FrameDecl, ImageId, PassId};
use lactose_rhi_api::{Device, Extent, TimelinePoint};

use crate::camera::{CameraLatch, LatchedCamera};
use crate::feature::RenderFeature;
use crate::scatter::ScatterBatch;
use crate::shader::Shaders;
use crate::slots::SlotTable;
use crate::staging::{ExtractSink, RemoteWriter, Staging};

/// The target of the renderer's events: a frame's dropped rows and lost kinds at debug
/// level, and an error from a destroy in the renderer's drop.
pub const LOG_TARGET: &str = "cheddar";

/// What a renderer is made with.
#[derive(Clone, Copy, Debug)]
pub struct RendererDesc<'a> {
    /// The most instances held at once, at least one: the slots of the slot table and
    /// the records of the instance buffer.
    pub instances: u32,
    /// The rows one region of the staging ring holds, at least one: every row the writers
    /// write between two [`prepare`](Renderer::prepare)s, with the room each pass's
    /// `begin`, `end` and removed keys take (`README.md`, "Regions"). A region that holds
    /// one full pass of every world shown and the remote upload never drops a row.
    pub rows: u32,
    /// The regions of the ring, at least two: one open to the writers, and one for each
    /// frame the device may still be reading. Each has a camera buffer of its own.
    pub regions: u32,
    /// The cooked shaders, or `None` for a renderer that stages and prepares and declares
    /// nothing: what a cost test of the ring is made with, with no shader cooked.
    pub shaders: Option<Shaders<'a>>,
}

/// Whether a region is open to the writers.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Opened {
    /// A region is open: the writers of the next frame may run.
    Ready,
    /// No region is free. One comes free when this point is reached: the point of the
    /// held region whose frame was submitted longest ago. The caller waits for it,
    /// `Device::wait_timeline`, and calls [`Renderer::open`] again.
    Wait(TimelinePoint),
    /// No region is free and no point frees one yet: each is sealed for the frame that
    /// is prepared and not ended, or held by a torn frame that no submitted frame has
    /// followed. One comes free at the point of the next
    /// [`Renderer::submitted`]: the prepared frame's, or, with no frame prepared, that
    /// of a frame the caller submits with nothing of the renderer's in it. Returned only
    /// after a torn frame.
    Unsubmitted,
}

/// What a frame's [`prepare`](Renderer::prepare) took from the region and did to the
/// slot table. Counts, for a test, a trace and the frame's own decisions. None is a
/// timing.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct FrameStats {
    /// The rows extract's passes wrote into the region.
    pub pass_rows: u32,
    /// The rows the remote batch wrote into the region.
    pub remote_rows: u32,
    /// The rows a writer could not write: the region was full, no region was open, or
    /// the device refused the write.
    pub dropped: u32,
    /// The rows left out of the batch because a later row of their pass or batch named
    /// the same instance. Zero unless a writer broke its contract.
    pub duplicates: u32,
    /// The rows left out of the batch because their instance was new and every slot was
    /// held.
    pub unplaced: u32,
    /// The slots given to new instances.
    pub allocated: u32,
    /// The slots freed.
    pub freed: u32,
    /// The entries of the frame's batch.
    pub entries: u32,
    /// The instances that hold a slot after the frame's batch.
    pub live: u32,
    /// The kinds lost in the frame, as bits. Read through [`lost`](Self::lost).
    lost: u8,
}

impl FrameStats {
    /// Whether `kind` was *lost* in the frame: a pass of that kind could not leave its
    /// `begin`, its `end` or a removed key in the region, so the renderer applied nothing
    /// of the kind's passes and freed every slot the kind held. The kind shows nothing
    /// until a full pass, which the frame gets with `ExtractState::set_sink`; a client's
    /// replicated world is shown again by the next remote batch (`README.md`,
    /// "Writers").
    #[must_use]
    pub const fn lost(&self, kind: WorldKind) -> bool {
        self.lost & kind_bit(kind) != 0
    }

    /// Whether any kind was lost in the frame.
    #[must_use]
    pub const fn any_lost(&self) -> bool {
        self.lost != 0
    }

    /// The counts with `kind` marked lost. The renderer's plumbing.
    pub(crate) const fn with_lost(mut self, kind: WorldKind) -> Self {
        self.lost |= kind_bit(kind);
        self
    }
}

/// The bit of [`FrameStats`]'s lost kinds that stands for `kind`.
const fn kind_bit(kind: WorldKind) -> u8 {
    match kind {
        WorldKind::Server => 1,
        WorldKind::Client => 2,
        WorldKind::Prediction => 4,
    }
}

/// What a frame's [`declare`](Renderer::declare) is given.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FrameView {
    /// The colour target, which the caller imported into the declaration: an image of
    /// the format [`Shaders::target`] names. The triangle pass clears it.
    pub target: ImageId,
    /// The target's extent, which is also the depth image's.
    pub extent: Extent,
    /// Where between its two placements each instance is drawn, from zero, the previous
    /// one, to one, the current one: the presentation frame's fraction.
    pub fraction: f32,
}

/// What a frame's [`declare`](Renderer::declare) added to the declaration, by its ids: for
/// a pass the caller declares after it, such as a test's copy of the instance buffer into
/// a readback.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DeclaredFrame {
    /// The persistent instance buffer, which the frame leaves in
    /// `BufferState::ShaderRead` on the graphics queue. Made with
    /// `BufferUsage::COPY_SOURCE`, so a copy pass may read it.
    pub instances: BufferId,
    /// The frame's camera buffer.
    pub camera: BufferId,
    /// The depth image the triangle pass wrote.
    pub depth: ImageId,
    /// The scatter pass, or `None` in a frame whose batch is empty, which declares none.
    pub scatter: Option<PassId>,
    /// The triangle pass.
    pub triangles: PassId,
}

/// The minimal renderer of one device (the milestone 2 plan, decision 5.4).
///
/// A frame goes through it in this order, each step one call: [`open`](Self::open), then
/// the writers ([`sink`](Self::sink)'s passes and [`remote`](Self::remote)'s batch), then
/// [`prepare`](Self::prepare), [`declare`](Self::declare), the caller's compile and
/// record, [`latch`](Self::latch), the caller's submit, and [`submitted`](Self::submitted)
/// or [`torn`](Self::torn). The next frame's `open` may come any time after `prepare`
/// (`README.md`, "A frame, by step").
///
/// `Send + Sync + 'static`, so that it sits in a world's resource or behind a `dyn`
/// object, and every call that changes it takes `&mut self`: one thread at a time by the
/// borrow, with no lock (`README.md`, "Concurrency").
///
/// Its drop waits, with no bound, for the point of the last frame it was told was
/// submitted, then destroys its buffers and pipelines. It is dropped off the
/// frame-critical workers, as a device's objects are, after the graph that submitted its
/// frames. A drop made while the thread unwinds a panic waits for nothing, and an object
/// a submit still uses is leaked, not destroyed (`README.md`, "Errors and drops").
pub struct Renderer {
    /// The device every call reaches.
    device: Arc<dyn Device>,
    /// What the renderer shares with the writers.
    staging: Arc<Staging>,
    /// The slot table.
    slots: SlotTable,
    /// The providers, in registration order.
    features: Vec<Box<dyn RenderFeature>>,
}

impl Renderer {
    /// A renderer for `device`: the ring's regions, a camera buffer for each, the
    /// instance buffer, the built-in triangle's vertices and indices, and, with
    /// `desc.shaders`, the scatter's pipeline and the triangle pass's. Region zero is
    /// open when it returns, so the writers may run before the first
    /// [`open`](Self::open).
    ///
    /// Every pipeline the renderer uses is made here and none in a frame (architecture
    /// section 12.5, "All are created at load"), so that once it has returned
    /// `Device::pipeline_cache` holds them and `Device::pipeline_stats` counts them; the
    /// client binary writes the cache file, and the renderer opens none.
    ///
    /// # Errors
    ///
    /// `ErrorKind::InvalidData` where `desc.instances` or `desc.rows` is zero or
    /// `desc.regions` is less than two, and for a cooked shader whose reflection is not
    /// the interface's (its stage, the bytes of its push block, its heap, its constants,
    /// its inputs and outputs), naming the shader and the field (`README.md`, "The
    /// shader interface"); what the device returns for a buffer or a pipeline it cannot
    /// make. After an error nothing it made is left on the device.
    pub fn new(device: Arc<dyn Device>, desc: &RendererDesc<'_>) -> Result<Self> {
        todo!()
    }

    /// The device the renderer was made for.
    #[must_use]
    pub fn device(&self) -> &Arc<dyn Device> {
        &self.device
    }

    /// A sink for one world's extract: `ExtractState::set_sink` takes it in a box. Each
    /// world shown holds one of its own; all of them write this renderer's open region.
    #[must_use]
    pub fn sink(&self) -> ExtractSink {
        ExtractSink::new(Arc::clone(&self.staging))
    }

    /// The writer of the frame's remote batch, with the batch opened. The caller makes
    /// it after the frame's extract passes and drops it before
    /// [`prepare`](Self::prepare), which the borrow holds it to.
    pub fn remote(&mut self) -> RemoteWriter<'_> {
        todo!()
    }

    /// Registers a provider. Its passes are declared in every frame from the next
    /// [`declare`](Self::declare) on, after the renderer's own and after those of the
    /// providers registered before it. Milestone 2 registers none.
    pub fn add_feature(&mut self, feature: Box<dyn RenderFeature>) {
        todo!()
    }

    /// Opens a free region to the writers, or says which point frees one.
    ///
    /// A region is free when it was never sealed, or when the point its frame was
    /// [`submitted`](Self::submitted) with is reached, which the call learns by reading
    /// the timeline's value and never by waiting. With a region already open it returns
    /// [`Opened::Ready`] and changes nothing. The caller makes it before the writers of a
    /// frame run, so that the region a pass writes is free before the pass begins
    /// (`crates/present-api/README.md`, "What a pass gives the sink"), and waits for the
    /// point of an [`Opened::Wait`] outside any pass it measures. It may come any time
    /// after the frame's [`prepare`](Self::prepare), the frame ended or not.
    ///
    /// # Errors
    ///
    /// What `Device::timeline_value` returns.
    pub fn open(&mut self) -> Result<Opened> {
        todo!()
    }

    /// Seals the open region and builds the frame's batch from what the writers left in
    /// it: applies each pass's and the remote batch's frees and rows to the slot table,
    /// in the order they began, writes the batch's entries and the draw's arguments to
    /// the region's buffers, and flushes what the frame's submit will read. After it no
    /// region is open until the next [`open`](Self::open).
    ///
    /// # Errors
    ///
    /// `ErrorKind::InvalidData` when no region is open, when a sink's pass is open, or
    /// when the frame prepared before this one has not been ended by
    /// [`submitted`](Self::submitted) or [`torn`](Self::torn): nothing is sealed and
    /// nothing changes. What the device returns for a write or a flush, after which the
    /// frame is ended with [`torn`](Self::torn).
    pub fn prepare(&mut self) -> Result<FrameStats> {
        todo!()
    }

    /// The slot table, as the last [`prepare`](Self::prepare) left it.
    #[must_use]
    pub fn slots(&self) -> &SlotTable {
        &self.slots
    }

    /// The batch of the frame last prepared: empty before the first, and after
    /// [`torn`](Self::torn).
    #[must_use]
    pub fn batch(&self) -> ScatterBatch<'_> {
        todo!()
    }

    /// Declares the prepared frame's passes into `frame`: the scatter, where the batch
    /// has an entry, then the triangle pass, then each provider's passes in registration
    /// order. The caller imported `view.target` and may declare passes of its own before
    /// and after.
    ///
    /// The recorders borrow the renderer for `'f`, which ends when the frame is
    /// recorded, so nothing changes the renderer between this call and the end of
    /// `RenderGraph::record`. A prepared frame is declared once.
    ///
    /// # Errors
    ///
    /// `ErrorKind::InvalidData` when no frame is prepared, when the prepared frame was
    /// already declared, and for a renderer made without shaders. `frame` is then as it
    /// was.
    pub fn declare<'f>(
        &'f self,
        frame: &mut FrameDecl<'f>,
        view: &FrameView,
    ) -> Result<DeclaredFrame> {
        todo!()
    }

    /// The late latch (architecture section 5.3): reads `latch.input` once, computes
    /// [`CameraData::orbit`](crate::CameraData::orbit) from the pivot, the yaw and pitch
    /// it read, the boom and the projection, writes it to the frame's camera buffer and
    /// flushes it. It writes that one buffer and nothing else. It takes no lock but the
    /// device backend's on its records and waits for no device and no signal; its one loop is
    /// `LatestSample::read`'s, which retries only while a publish is in flight. The
    /// caller makes it after `RenderGraph::record` has returned and immediately before
    /// `RenderGraph::submit`; nothing recorded depends on the camera's bytes.
    ///
    /// # Errors
    ///
    /// `ErrorKind::InvalidData` when no frame is prepared or the frame was already
    /// latched; what the device returns for the write or the flush. After an error the
    /// frame is not submitted, and is ended with [`torn`](Self::torn).
    pub fn latch(&mut self, latch: &CameraLatch<'_>) -> Result<LatchedCamera> {
        todo!()
    }

    /// Ends the prepared frame: `RenderGraph::submit` returned `point` for it, or, in a
    /// cost test that declares nothing, a submit of the caller's signals it. The frame's
    /// region, and any region a torn frame before it left held, is free once `point` is
    /// reached. `point` is one whose reaching means every submit made before it that
    /// read a region has completed, as a frame point of the graph is
    /// (`crates/render-graph/README.md`, "Submission").
    ///
    /// With no frame prepared it gives `point` to the regions torn frames left held and
    /// does nothing else: the way out of [`Opened::Unsubmitted`].
    ///
    /// A frame that was prepared and not declared never ran its batch. On a renderer
    /// made with shaders the call then ends it as [`torn`](Self::torn) does, but for the
    /// region, which is free at `point`; on one made without, the instance buffer is
    /// read by nothing and the slot table stands.
    pub fn submitted(&mut self, point: TimelinePoint) {
        todo!()
    }

    /// Ends the prepared frame as torn: it was not submitted, or its submit returned an
    /// error (`crates/render-graph/README.md`, "Submission"), or the frame was given up
    /// after [`prepare`](Self::prepare).
    ///
    /// Nothing the instance buffer holds is relied on again: every slot is freed, every
    /// base and the tick of every kind's last pass are forgotten, the batch is emptied,
    /// and the batch of the next frame prepared begins by freeing every record a slot
    /// was ever given, so that none the torn frame left is drawn (`README.md`, "Errors
    /// and drops"). The instances come back as extract writes them, which the caller
    /// brings about with `ExtractState::set_sink` for every world shown, since a pass
    /// after it is full; the remote batch writes every remote instance each frame
    /// without being asked. The frame's region stays held until the point of the next
    /// [`submitted`](Self::submitted), because a submit of the torn frame may still read
    /// it. Does nothing when no frame is prepared.
    pub fn torn(&mut self) {
        todo!()
    }
}

impl Drop for Renderer {
    /// Waits for the point of the last frame the renderer was told was submitted, unless
    /// the thread is unwinding a panic, then destroys what the renderer made. The device
    /// refuses a destroy under a submit that is still pending: one of a frame the
    /// renderer was told no point for, or any while unwinding. Such an error is logged
    /// under [`LOG_TARGET`] and the object is leaked. On a lost device the wait returns
    /// at once.
    fn drop(&mut self) {
        todo!()
    }
}
