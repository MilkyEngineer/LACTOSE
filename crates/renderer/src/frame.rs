//! The renderer: what it is made with, and the steps a frame takes through it.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use lactose_core::{Error, ErrorKind, Result};
use lactose_ecs::WorldKind;
use lactose_render_graph::{
    BufferAccess, BufferId, BufferImport, BufferUse, ColourUse, ComputePassDecl, DepthUse,
    FrameDecl, ImageId, PassId, RenderPassDecl, TransientImage,
};
use lactose_rhi_api::{
    BufferDesc, BufferHandle, BufferState, BufferUsage, Device, Extent, IndexedIndirectArgs, Load,
    MemoryClass, QueueRole, Rect, Sharing, StateOn, TimelinePoint, Viewport,
};

use crate::batch::Builder;
use crate::camera::{CameraData, CameraLatch, LatchedCamera};
use crate::feature::{FeatureView, RenderFeature};
use crate::pipeline::{self, DEPTH, Pipelines};
use crate::scatter::{BatchRange, InstanceRecord, ScatterBatch, ScatterEntry, StagingRow};
use crate::shader::{CLEAR, SCATTER_GROUP, Shaders, TRIANGLE};
use crate::slots::SlotTable;
use crate::staging::{
    ExtractSink, REGIONS_MAX, RemoteWriter, Staging, Unsealed, Use, kind_index, next_generation,
};

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
    /// `Device::wait_timeline`, off the frame-critical workers, and calls
    /// [`Renderer::open`] again.
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
/// timing, and each saturates at `u32::MAX`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct FrameStats {
    /// The rows extract's passes wrote into the region.
    pub pass_rows: u32,
    /// The rows the remote batches wrote into the region, those of every batch that was
    /// not applied included: such a batch, whichever the reason, is counted here alone
    /// (`README.md`, "Slots").
    pub remote_rows: u32,
    /// The rows a writer could not write: the region was full, no region was open, or
    /// the device refused the write; never a written row. Those of each kind's extract
    /// passes are also counted by [`dropped_in`](Self::dropped_in); the remote batch's are
    /// counted here alone, since the next frame's batch writes every remote instance
    /// again. Below saturation it is at least the sum of `dropped_in` over the kinds,
    /// in every frame. A pass lost with no region open can still be reported lost a frame
    /// before some of its dropped rows are counted, the early report "Writers" allows, and
    /// never a frame after: `models::a_kinds_rows_are_never_taken_before_its_loss`.
    pub dropped: u32,
    /// The rows left out of the batch because a later row of their pass or batch named
    /// the same instance. Zero unless a writer broke its contract.
    pub duplicates: u32,
    /// The rows left out of the batch because their instance was new and every slot was
    /// held. Those of each kind's extract passes are also counted by
    /// [`unplaced_in`](Self::unplaced_in); the remote batch's are counted here alone.
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
    /// The dropped rows of each kind's extract passes, by [`kind_index`]. Read through
    /// [`dropped_in`](Self::dropped_in).
    dropped_by_kind: [u32; 3],
    /// The unplaced rows of each kind's extract passes, by [`kind_index`]. Read through
    /// [`unplaced_in`](Self::unplaced_in).
    unplaced_by_kind: [u32; 3],
}

impl FrameStats {
    /// Whether `kind` was *lost* in the frame: a pass of that kind could not leave its
    /// `begin`, its `end` or a removed key in the region, or began while no region was
    /// open, so the renderer applied nothing of the kind's passes and freed every slot
    /// the kind held. The kind shows nothing until a full pass, which the frame gets with
    /// `ExtractState::set_sink`; a client's replicated world is shown again by the next
    /// remote batch (`README.md`, "Writers").
    #[must_use]
    pub const fn lost(&self, kind: WorldKind) -> bool {
        self.lost & kind_bit(kind) != 0
    }

    /// Whether any kind was lost in the frame.
    #[must_use]
    pub const fn any_lost(&self) -> bool {
        self.lost != 0
    }

    /// The rows of `kind`'s extract passes that were dropped. Extract writes a dropped
    /// row again only at its settling write, when it changes, or at a full pass, so a
    /// frame that finds this above zero brings a full pass of the world of `kind` about
    /// with `ExtractState::set_sink` (`README.md`, "Writers").
    #[must_use]
    pub const fn dropped_in(&self, kind: WorldKind) -> u32 {
        self.dropped_by_kind[kind_index(kind)]
    }

    /// The rows of `kind`'s extract passes that were left out because every slot was
    /// held. Their instances are not drawn until extract writes them again, so a frame
    /// that finds this above zero brings a full pass of the world of `kind` about, as for
    /// [`dropped_in`](Self::dropped_in).
    #[must_use]
    pub const fn unplaced_in(&self, kind: WorldKind) -> u32 {
        self.unplaced_by_kind[kind_index(kind)]
    }
}

/// The bit of [`FrameStats`]'s lost kinds that stands for `kind`.
const fn kind_bit(kind: WorldKind) -> u8 {
    1 << kind_index(kind)
}

/// What a frame's [`declare`](Renderer::declare) is given.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FrameView {
    /// The colour target, which the caller imported into the declaration: an image of
    /// the format [`Shaders::target`] names. The triangle pass clears it.
    pub target: ImageId,
    /// The target's extent: the frame's one extent, which is also the viewport's, the
    /// depth image's and, at the latch, the camera's aspect ratio. Each side is above
    /// zero and at most `Capabilities::viewport_extent`.
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
    /// `BufferUsage::COPY_SOURCE`, so a copy pass may read it once a scatter has written
    /// it: in a frame whose `SlotTable::extent` is above zero.
    pub instances: BufferId,
    /// The frame's camera buffer, which the latch writes.
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
/// object. Every call that changes it takes `&mut self`, one thread at a time by the
/// borrow, with no lock, but [`declare`](Self::declare): that takes `&self`, so that its
/// recorders may borrow the renderer, and stores the frame's extent in one atomic word,
/// which is also the mark that the prepared frame is declared (`README.md`,
/// "Concurrency").
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
    /// The prepared frame's declared extent, `width << 32 | height`, or zero while it is
    /// not declared: `declare` refuses a side of zero, so zero is free to mean that.
    /// `declare` sets it with one `Relaxed` compare-exchange from zero, and `prepare`
    /// clears it; every call but `declare` reaches it through `&mut self`, which orders it
    /// after every `declare`.
    declared: AtomicU64,
    /// The providers, in registration order.
    features: Vec<Box<dyn RenderFeature>>,
    /// The regions' buffers but their rows, which `staging` holds, and their states.
    regions: Vec<Region>,
    /// The open generation of the region opened last.
    generation: u32,
    /// The frame prepared and not yet ended.
    frame: Option<Prepared>,
    /// The batch of the frame last prepared.
    entries: Vec<ScatterEntry>,
    /// Its dispatches.
    dispatches: Vec<BatchRange>,
    /// What applies a sealed region to the slot table.
    builder: Builder,
    /// The persistent instance buffer: `instances` records in `MemoryClass::Device`.
    instances: BufferHandle,
    /// The built-in triangle's vertices.
    vertices: BufferHandle,
    /// The built-in triangle's indices.
    indices: BufferHandle,
    /// The pipelines, where the renderer was made with shaders.
    pipelines: Option<Pipelines>,
    /// Whether the next frame declared imports the instance buffer from `Undefined`: the
    /// renderer's first, and the first after a torn frame.
    fresh: bool,
    /// The point of the last frame the renderer was told was submitted.
    last: Option<TimelinePoint>,
    /// How many frames were submitted, which orders the held regions.
    submitted: u64,
}

/// One region of the ring, as the renderer holds it.
struct Region {
    /// The frame's batch: room for `instances + 2 * rows` entries.
    entries: BufferHandle,
    /// The draw's arguments.
    args: BufferHandle,
    /// The camera.
    camera: BufferHandle,
    state: RegionState,
}

/// Where a region is in its cycle (`README.md`, "Regions").
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RegionState {
    Free,
    Open,
    Sealed,
    /// Submitted with `point`, the `order`th frame submitted.
    Held {
        point: TimelinePoint,
        order: u64,
    },
    /// Left by a torn frame, until the next `submitted`.
    Torn,
}

/// The frame prepared and not yet ended.
#[derive(Clone, Copy, Debug)]
struct Prepared {
    /// Its region.
    region: u32,
    /// Whether its batch and arguments reached the device: a frame whose `prepare` failed
    /// is not declared, and is ended with `torn`.
    uploaded: bool,
    /// Whether it was latched.
    latched: bool,
}

/// `state` on the graphics queue, the one queue every buffer of the renderer is used on.
const fn on_graphics<S>(state: S) -> StateOn<S> {
    StateOn { state, queue: QueueRole::Graphics }
}

fn invalid(message: &'static str) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

/// The most entries a batch of a renderer of `instances` instances and `rows` rows a region
/// holds, `instances + 2 * rows`: a free of every slot after a torn frame, and a row and a
/// free for each row. A [`BatchRange`] names an entry by a `u32`, so a description past
/// `u32::MAX` is refused, before any buffer is made.
fn batch_bound(instances: u32, rows: u32) -> Result<u32> {
    let batch = u64::from(instances) + 2 * u64::from(rows);
    u32::try_from(batch).map_err(|_| {
        invalid("a renderer's batch, `instances + 2 * rows` entries, is at most `u32::MAX`")
    })
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
    /// `desc.regions` is less than two or more than 65,535, the most the open word's
    /// region index holds (`README.md`, "Concurrency"), where the batch's bound,
    /// `desc.instances + 2 * desc.rows` entries, is past `u32::MAX`, the most a
    /// [`BatchRange`] names (`README.md`, "Regions"), checked before any buffer is made,
    /// and for a cooked shader whose
    /// reflection is not the interface's (its stage, the bytes of its push block, its
    /// heap, its constants, its inputs and outputs), naming the shader and the field
    /// (`README.md`, "Pipelines and shaders"); what the device returns for a buffer or a
    /// pipeline it cannot make.
    /// After an error nothing it made is left on the device. The scatter's workgroup, which
    /// the cooked format carries, is held to
    /// [`SCATTER_GROUP`](crate::shader::SCATTER_GROUP): `new` refuses any other.
    pub fn new(device: Arc<dyn Device>, desc: &RendererDesc<'_>) -> Result<Self> {
        if desc.instances == 0 || desc.rows == 0 {
            return Err(invalid("a renderer holds at least one instance and one row"));
        }
        if !(2..=REGIONS_MAX).contains(&desc.regions) {
            return Err(invalid("a renderer has from 2 to 65,535 regions"));
        }
        let batch = u64::from(batch_bound(desc.instances, desc.rows)?);
        if let Some(shaders) = &desc.shaders {
            pipeline::check(shaders)?;
        }
        let mut made = Made { device: &*device, buffers: Vec::new() };
        let address = BufferUsage::DEVICE_ADDRESS;
        let upload = MemoryClass::Upload;
        let mut rows = Vec::new();
        let mut regions = Vec::new();
        for _ in 0..desc.regions {
            let size = u64::from(desc.rows) * u64::from(StagingRow::BYTES);
            rows.push(made.buffer("cheddar::rows", size, address, upload)?);
            let size = batch * u64::from(ScatterEntry::BYTES);
            let entries = made.buffer("cheddar::entries", size, address, upload)?;
            let size = u64::from(IndexedIndirectArgs::BYTES);
            let args = made.buffer("cheddar::args", size, BufferUsage::INDIRECT, upload)?;
            let size = u64::from(CameraData::BYTES);
            let camera = made.buffer("cheddar::camera", size, address, upload)?;
            regions.push(Region { entries, args, camera, state: RegionState::Free });
        }
        regions[0].state = RegionState::Open;
        let size = u64::from(desc.instances) * u64::from(InstanceRecord::BYTES);
        let usage = address.union(BufferUsage::COPY_SOURCE);
        let instances = made.buffer("cheddar::instances", size, usage, MemoryClass::Device)?;
        let bytes: Vec<u8> = TRIANGLE.as_flattened().iter().flat_map(|v| v.to_le_bytes()).collect();
        let vertices = made.buffer("cheddar::vertices", bytes.len() as u64, address, upload)?;
        made.fill(vertices, &bytes)?;
        let bytes: Vec<u8> = [0_u32, 1, 2].iter().flat_map(|i| i.to_le_bytes()).collect();
        let indices =
            made.buffer("cheddar::indices", bytes.len() as u64, BufferUsage::INDEX, upload)?;
        made.fill(indices, &bytes)?;
        let pipelines = match &desc.shaders {
            Some(shaders) => Some(pipeline::make(&*device, shaders)?),
            None => None,
        };
        made.buffers.clear();
        drop(made);

        let groups = device.capabilities().compute_groups[0].max(1);
        let per_dispatch = groups.min(u32::MAX / SCATTER_GROUP) * SCATTER_GROUP;
        let staging = Arc::new(Staging::new(Arc::clone(&device), desc.rows, &rows));
        Ok(Self {
            device,
            staging,
            slots: SlotTable::new(desc.instances),
            declared: AtomicU64::new(0),
            features: Vec::new(),
            regions,
            generation: 1,
            frame: None,
            entries: Vec::new(),
            dispatches: Vec::new(),
            builder: Builder::new(per_dispatch),
            instances,
            vertices,
            indices,
            pipelines,
            fresh: true,
            last: None,
            submitted: 0,
        })
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
    /// it after the frame's `Client` pass has ended and drops it before
    /// [`prepare`](Self::prepare), which the borrow holds it to. The batch takes no count
    /// of the open word, so a writer passed to `mem::forget` leaves the renderer
    /// preparable: its batch has no end note and is left out whole, its rows counted in
    /// [`FrameStats::remote_rows`] alone (`README.md`, "Slots").
    pub fn remote(&mut self) -> RemoteWriter<'_> {
        RemoteWriter::new(&self.staging)
    }

    /// Registers a provider. Its passes are declared in every frame from the next
    /// [`declare`](Self::declare) on, after the renderer's own and after those of the
    /// providers registered before it. Milestone 2 registers none.
    pub fn add_feature(&mut self, feature: Box<dyn RenderFeature>) {
        self.features.push(feature);
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
        if self.regions.iter().any(|region| region.state == RegionState::Open) {
            return Ok(Opened::Ready);
        }
        for region in &mut self.regions {
            if let RegionState::Held { point, .. } = region.state
                && self.device.timeline_value(point.timeline)? >= point.value
            {
                region.state = RegionState::Free;
            }
        }
        if let Some(index) = self.regions.iter().position(|r| r.state == RegionState::Free) {
            self.generation = next_generation(self.generation);
            self.regions[index].state = RegionState::Open;
            // Below `REGIONS_MAX`, which `new` holds the ring to.
            let region = index as u32;
            self.staging.open(Use { region, generation: self.generation });
            return Ok(Opened::Ready);
        }
        let oldest = self
            .regions
            .iter()
            .filter_map(|region| match region.state {
                RegionState::Held { point, order } => Some((order, point)),
                _ => None,
            })
            .min_by_key(|&(order, _)| order);
        Ok(oldest.map_or(Opened::Unsubmitted, |(_, point)| Opened::Wait(point)))
    }

    /// Seals the open region and builds the frame's batch from what the writers left in
    /// it: applies each pass's frees and rows to the slot table, kind by kind in the order
    /// of `WorldKind`'s variants and in the order they began within a kind, and then the
    /// region's last remote batch, where it is whole, no note of a `Client` pass lies after
    /// its opening and the `Client` kind is not lost; writes the batch's entries and the
    /// draw's arguments to the region's buffers, and flushes what the frame's submit will
    /// read. After it no region is open until the next [`open`](Self::open), and the frame
    /// is not declared (`README.md`, "Slots").
    ///
    /// # Errors
    ///
    /// `ErrorKind::InvalidData` when no region is open, when a sink's pass is open, or
    /// when the frame prepared before this one has not been ended by
    /// [`submitted`](Self::submitted) or [`torn`](Self::torn): nothing is sealed and
    /// nothing changes. What the device returns for a write or a flush, after which the
    /// frame is ended with [`torn`](Self::torn).
    pub fn prepare(&mut self) -> Result<FrameStats> {
        if self.frame.is_some() {
            return Err(invalid("the frame prepared before this one is not ended"));
        }
        let sealed = self.staging.seal().map_err(|why| match why {
            Unsealed::NotOpen => invalid("no region is open"),
            Unsealed::PassOpen => invalid("a sink's pass is open"),
        })?;
        let at = sealed.at.region;
        self.regions[at as usize].state = RegionState::Sealed;
        *self.declared.get_mut() = 0;
        self.frame = Some(Prepared { region: at, uploaded: false, latched: false });
        let built = self.builder.build(
            &mut self.slots,
            self.staging.entries(&sealed),
            &sealed.counts,
            &mut self.entries,
            &mut self.dispatches,
        );
        let saturate = |count: u64| u32::try_from(count).unwrap_or(u32::MAX);
        let stats = FrameStats {
            pass_rows: saturate(built.pass_rows),
            remote_rows: saturate(built.remote_rows),
            dropped: saturate(sealed.counts.dropped),
            duplicates: saturate(built.duplicates),
            unplaced: saturate(built.unplaced),
            allocated: saturate(built.allocated),
            freed: saturate(built.freed),
            entries: saturate(self.entries.len() as u64),
            live: self.slots.live(),
            lost: built.lost,
            dropped_by_kind: sealed.counts.dropped_by_kind.map(saturate),
            unplaced_by_kind: built.unplaced_by_kind.map(saturate),
        };
        if stats.dropped != 0 || stats.unplaced != 0 || stats.any_lost() {
            tracing::debug!(
                target: LOG_TARGET,
                dropped = stats.dropped,
                unplaced = stats.unplaced,
                lost = stats.lost,
                "a frame's region dropped or left out rows, or lost a kind"
            );
        }
        self.upload(at, sealed.len)?;
        if let Some(frame) = &mut self.frame {
            frame.uploaded = true;
        }
        Ok(stats)
    }

    /// Writes the batch's entries and the draw's arguments to region `at`, and flushes them
    /// and the region's first `rows` rows.
    fn upload(&self, at: u32, rows: u32) -> Result<()> {
        let region = &self.regions[at as usize];
        let device = &*self.device;
        if !self.entries.is_empty() {
            let len = self.entries.len() as u64 * u64::from(ScatterEntry::BYTES);
            let mut fill = |bytes: &mut [u8]| {
                let (chunks, _) = bytes.as_chunks_mut::<{ ScatterEntry::BYTES as usize }>();
                for (out, entry) in chunks.iter_mut().zip(&self.entries) {
                    *out = entry.to_bytes();
                }
            };
            device.write_buffer(region.entries, 0, len, &mut fill)?;
            device.flush_buffer(region.entries, 0, len)?;
        }
        let args = IndexedIndirectArgs {
            index_count: 3,
            instance_count: self.slots.extent(),
            first_index: 0,
            base_vertex: 0,
            first_instance: 0,
        }
        .to_bytes();
        let len = u64::from(IndexedIndirectArgs::BYTES);
        device.write_buffer(region.args, 0, len, &mut |bytes| bytes.copy_from_slice(&args))?;
        device.flush_buffer(region.args, 0, len)?;
        if rows != 0 {
            let len = u64::from(rows) * u64::from(StagingRow::BYTES);
            device.flush_buffer(self.staging.buffer(at), 0, len)?;
        }
        Ok(())
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
        ScatterBatch::new(&self.entries, &self.dispatches)
    }

    /// Declares the prepared frame's passes into `frame`: the scatter, where the batch
    /// has an entry, then the triangle pass, which clears the target and draws only
    /// where `SlotTable::extent` is above zero, then each provider's passes in
    /// registration order. The caller imported `view.target` and may declare passes of
    /// its own before and after (`README.md`, "The scatter" and "The triangle pass").
    ///
    /// The recorders borrow the renderer for `'f`, which ends when the frame is
    /// recorded, so nothing changes the renderer between this call and the end of
    /// `RenderGraph::record` but this call's own store: once its arguments are checked,
    /// it stores `view.extent` in the renderer's declared word with one compare-exchange
    /// from zero, which is also the mark that the frame is declared, and only then adds
    /// its passes. So a prepared frame is declared once, and of two calls at once exactly
    /// one succeeds (`README.md`, "Concurrency").
    ///
    /// # Errors
    ///
    /// `ErrorKind::InvalidData` when no frame is prepared, when the prepared frame was
    /// already declared, for a renderer made without shaders, and for a `view.extent`
    /// with a side of zero or beyond the device's `Capabilities::viewport_extent`.
    /// `frame` and the declared word are then as they were.
    pub fn declare<'f>(
        &'f self,
        frame: &mut FrameDecl<'f>,
        view: &FrameView,
    ) -> Result<DeclaredFrame> {
        let Some(prepared) = self.frame else { return Err(invalid("no frame is prepared")) };
        if !prepared.uploaded {
            return Err(invalid(
                "the prepared frame's `prepare` failed, so it is ended with `torn`",
            ));
        }
        let Some(pipelines) = self.pipelines else {
            return Err(invalid("a renderer made without shaders declares nothing"));
        };
        let [width, height] = self.device.capabilities().viewport_extent;
        let extent = view.extent;
        if extent.width == 0 || extent.height == 0 || extent.width > width || extent.height > height
        {
            return Err(invalid("the frame's extent has a side of zero or beyond the viewport's"));
        }
        let word = u64::from(extent.width) << 32 | u64::from(extent.height);
        if self.declared.compare_exchange(0, word, Ordering::Relaxed, Ordering::Relaxed).is_err() {
            return Err(invalid("the prepared frame is already declared"));
        }

        let region = &self.regions[prepared.region as usize];
        let mut import = |label, buffer, initial, last| {
            frame.import_buffer(&BufferImport {
                label,
                buffer,
                sharing: Sharing::Exclusive,
                initial: on_graphics(initial),
                last: on_graphics(last),
                ready: None,
            })
        };
        use BufferState::{Index, Indirect, ShaderRead, Undefined};
        let initial = if self.fresh { Undefined } else { ShaderRead };
        let instances = import("cheddar::instances", self.instances, initial, ShaderRead);
        let camera = import("cheddar::camera", region.camera, Undefined, ShaderRead);
        let scattered = !self.entries.is_empty();
        let reads_rows = self.entries.iter().any(|entry| entry.flags & ScatterEntry::FREE == 0);
        let entries =
            scattered.then(|| import("cheddar::entries", region.entries, Undefined, ShaderRead));
        let rows = reads_rows.then(|| {
            import("cheddar::rows", self.staging.buffer(prepared.region), Undefined, ShaderRead)
        });
        let drawn = (self.slots.extent() > 0).then(|| {
            let vertices = import("cheddar::vertices", self.vertices, Undefined, ShaderRead);
            let indices = import("cheddar::indices", self.indices, Undefined, Index);
            let args = import("cheddar::args", region.args, Undefined, Indirect);
            (vertices, indices, args)
        });
        let depth = frame.transient_image(&TransientImage {
            label: "cheddar::depth",
            format: DEPTH,
            extent,
            mips: 1,
            layers: 1,
        });

        let read = |buffer| BufferUse { buffer, access: BufferAccess::Read };
        let scatter = entries.map(|entries| {
            let mut buffers =
                vec![read(entries), BufferUse { buffer: instances, access: BufferAccess::Write }];
            buffers.extend(rows.map(read));
            let pass = ComputePassDecl {
                label: "cheddar::scatter",
                sampled: &[],
                buffers: &buffers,
                slots: &[],
            };
            let dispatches = &self.dispatches;
            frame.compute_pass(&pass, move |recorder| {
                recorder.bind_pipeline(pipelines.scatter);
                for range in dispatches {
                    match rows {
                        Some(rows) => recorder.push_address(0, rows, 0),
                        None => recorder.push_bytes(0, &[0; 8]),
                    }
                    let first = u64::from(range.first) * u64::from(ScatterEntry::BYTES);
                    recorder.push_address(8, entries, first);
                    recorder.push_address(16, instances, 0);
                    let mut tail = [0; 8];
                    tail[..4].copy_from_slice(&range.count.to_le_bytes());
                    recorder.push_bytes(24, &tail);
                    recorder.dispatch([range.count.div_ceil(SCATTER_GROUP), 1, 1]);
                }
            })
        });

        let mut buffers = Vec::new();
        match drawn {
            Some((vertices, indices, args)) => buffers.extend([
                read(instances),
                read(camera),
                read(vertices),
                BufferUse { buffer: indices, access: BufferAccess::Index },
                BufferUse { buffer: args, access: BufferAccess::Indirect },
            ]),
            None if scatter.is_some() => buffers.push(read(instances)),
            None => {}
        }
        let colour = [ColourUse {
            image: view.target,
            mip: 0,
            layer: 0,
            load: Load::Clear(CLEAR),
            local_read: false,
        }];
        let pass = RenderPassDecl {
            label: "cheddar::triangles",
            colour: &colour,
            depth: Some(DepthUse { image: depth, mip: 0, layer: 0, load: Load::Clear(1.0) }),
            sampled: &[],
            buffers: &buffers,
            slots: &[],
        };
        let fraction = view.fraction;
        let triangles = frame.render_pass(&pass, move |recorder| {
            recorder.set_viewport(Viewport {
                x: 0.0,
                y: 0.0,
                width: extent.width as f32,
                height: extent.height as f32,
                near: 0.0,
                far: 1.0,
            });
            recorder.set_scissor(Rect { x: 0, y: 0, extent });
            let Some((vertices, indices, args)) = drawn else { return };
            recorder.bind_pipeline(pipelines.triangles);
            recorder.bind_index_buffer(indices, 0);
            recorder.push_address(0, instances, 0);
            recorder.push_address(8, camera, 0);
            recorder.push_address(16, vertices, 0);
            let mut tail = [0; 8];
            tail[..4].copy_from_slice(&fraction.to_le_bytes());
            recorder.push_bytes(24, &tail);
            recorder.draw_indexed_indirect(args, 0, 1, IndexedIndirectArgs::BYTES);
        });

        let feature_view = FeatureView::new(view.target, depth, camera, extent);
        for feature in &self.features {
            feature.declare(frame, &feature_view);
        }
        Ok(DeclaredFrame { instances, camera, depth, scatter, triangles })
    }

    /// The late latch (architecture section 5.3): reads `latch.input` once, computes
    /// [`CameraData::orbit`](crate::CameraData::orbit) from the pivot, the yaw and pitch
    /// it read, the boom, the projection and the extent the frame was declared with,
    /// writes it to the frame's camera buffer and flushes it. It writes that one buffer
    /// and nothing else. It takes no lock but the device backend's on its records and
    /// waits for no device and no signal; its one loop is `LatestSample::read`'s, which
    /// retries only while a publish is in flight. The
    /// caller makes it after `RenderGraph::record` has returned and immediately before
    /// `RenderGraph::submit`; nothing recorded depends on the camera's bytes.
    ///
    /// # Errors
    ///
    /// `ErrorKind::InvalidData` when no frame is prepared, when the prepared frame was not
    /// declared, so that it has no extent, and when it was already latched; what the
    /// device returns for the write or the flush. After an error the frame is not
    /// submitted, and is ended with [`torn`](Self::torn).
    pub fn latch(&mut self, latch: &CameraLatch<'_>) -> Result<LatchedCamera> {
        let Some(prepared) = self.frame else { return Err(invalid("no frame is prepared")) };
        let word = *self.declared.get_mut();
        if word == 0 {
            return Err(invalid("the prepared frame was not declared, so it has no extent"));
        }
        if prepared.latched {
            return Err(invalid("the prepared frame was already latched"));
        }
        // The declared word's halves: the width above, the height below.
        let extent = Extent { width: (word >> 32) as u32, height: word as u32 };
        let sample = latch.input.read();
        let data = CameraData::orbit(
            latch.pivot,
            sample.yaw,
            sample.pitch,
            latch.boom,
            latch.projection,
            extent,
        );
        let camera = self.regions[prepared.region as usize].camera;
        let len = u64::from(CameraData::BYTES);
        let bytes = data.to_bytes();
        self.device.write_buffer(camera, 0, len, &mut |out| out.copy_from_slice(&bytes))?;
        self.device.flush_buffer(camera, 0, len)?;
        if let Some(frame) = &mut self.frame {
            frame.latched = true;
        }
        Ok(LatchedCamera { data, yaw: sample.yaw, pitch: sample.pitch })
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
        self.submitted += 1;
        let held = RegionState::Held { point, order: self.submitted };
        for region in &mut self.regions {
            if region.state == RegionState::Torn {
                region.state = held;
            }
        }
        self.last = Some(point);
        let Some(frame) = self.frame.take() else { return };
        self.regions[frame.region as usize].state = held;
        let declared = core::mem::take(self.declared.get_mut()) != 0;
        if declared {
            self.fresh = false;
        } else if self.pipelines.is_some() {
            // The batch never ran, so nothing the instance buffer holds is the table's.
            self.forget();
        }
    }

    /// What [`torn`](Self::torn) does but for the region: every slot freed, every base
    /// and tick forgotten, the batch emptied, the next batch's first run freeing every
    /// record a slot was ever given, and the next frame's import of the instance buffer
    /// from `Undefined`.
    fn forget(&mut self) {
        let held: Vec<u32> = self.slots.iter().map(|(slot, _)| slot.index()).collect();
        for slot in held {
            self.slots.release(slot);
        }
        self.builder.torn();
        self.entries.clear();
        self.dispatches.clear();
        self.fresh = true;
    }

    /// Ends the prepared frame as torn: it was not submitted, or its submit returned an
    /// error (`crates/render-graph/README.md`, "Submission"), or the frame was given up
    /// after [`prepare`](Self::prepare).
    ///
    /// Nothing the instance buffer holds is relied on again: every slot is freed, every
    /// base and the tick of every kind's last pass are forgotten, the batch is emptied,
    /// and the batch of the next frame prepared begins by freeing every record a slot
    /// was ever given, so that none the torn frame left is drawn, and the next frame
    /// declared imports the instance buffer from `Undefined` (`README.md`, "Errors and
    /// drops"). The instances come back as extract writes them, which the caller
    /// brings about with `ExtractState::set_sink` for every world shown, since a pass
    /// after it is full; the remote batch writes every remote instance each frame
    /// without being asked. The frame's region stays held until the point of the next
    /// [`submitted`](Self::submitted), because a submit of the torn frame may still read
    /// it. Does nothing when no frame is prepared.
    pub fn torn(&mut self) {
        let Some(frame) = self.frame.take() else { return };
        self.regions[frame.region as usize].state = RegionState::Torn;
        *self.declared.get_mut() = 0;
        self.forget();
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
        let device = &*self.device;
        if !std::thread::panicking()
            && let Some(point) = self.last
            && let Err(error) = device.wait_timeline(point, Duration::MAX)
        {
            log("the wait for the last frame submitted", &error);
        }
        let mut buffers = vec![self.instances, self.vertices, self.indices];
        for (index, region) in (0..).zip(&self.regions) {
            let rows = self.staging.buffer(index);
            buffers.extend([rows, region.entries, region.args, region.camera]);
        }
        for buffer in buffers {
            if let Err(error) = device.destroy_buffer(buffer) {
                log("a buffer's destroy", &error);
            }
        }
        if let Some(pipelines) = self.pipelines {
            if let Err(error) = device.destroy_compute_pipeline(pipelines.scatter) {
                log("the scatter pipeline's destroy", &error);
            }
            if let Err(error) = device.destroy_graphics_pipeline(pipelines.triangles) {
                log("the triangle pipeline's destroy", &error);
            }
        }
    }
}

fn log(what: &str, error: &Error) {
    tracing::warn!(target: LOG_TARGET, "{what} failed: {error}");
}

/// The buffers `Renderer::new` has made so far, destroyed if it returns early.
struct Made<'d> {
    device: &'d dyn Device,
    buffers: Vec<BufferHandle>,
}

impl Made<'_> {
    fn buffer(
        &mut self,
        label: &str,
        size: u64,
        usage: BufferUsage,
        memory: MemoryClass,
    ) -> Result<BufferHandle> {
        let desc = BufferDesc { label, size, usage, memory, sharing: Sharing::Exclusive };
        let buffer = self.device.create_buffer(&desc)?;
        self.buffers.push(buffer);
        Ok(buffer)
    }

    /// Writes `bytes` to the start of an `Upload` buffer and flushes them.
    fn fill(&self, buffer: BufferHandle, bytes: &[u8]) -> Result<()> {
        let len = bytes.len() as u64;
        self.device.write_buffer(buffer, 0, len, &mut |out| out.copy_from_slice(bytes))?;
        self.device.flush_buffer(buffer, 0, len)
    }
}

impl Drop for Made<'_> {
    fn drop(&mut self) {
        for &buffer in &self.buffers {
            // No submit has used them, so the device refuses none.
            let _ = self.device.destroy_buffer(buffer);
        }
    }
}

#[cfg(all(test, not(feature = "loom")))]
mod tests {
    use super::*;

    #[test]
    fn the_batch_bound_is_refused_one_past_u32_max_and_accepted_at_it() {
        assert_eq!(batch_bound(u32::MAX - 2, 1).ok(), Some(u32::MAX));
        assert_eq!(batch_bound(1, u32::MAX / 2).ok(), Some(u32::MAX));
        for (instances, rows) in [(u32::MAX - 1, 1), (u32::MAX, 4), (2, u32::MAX / 2)] {
            let refused = batch_bound(instances, rows).expect_err("past `u32::MAX`");
            assert_eq!(refused.kind(), ErrorKind::InvalidData, "{instances} {rows}");
        }
    }
}
