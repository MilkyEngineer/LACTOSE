//! The device: what every handle resolves to, the records and their locks, and the calls of
//! `Device`.

use core::ptr::NonNull;
use core::time::Duration;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use ash::vk;
use lactose_core::{Error, ErrorKind, Result};
use lactose_rhi_api::{
    BufferDesc, BufferHandle, BufferUsage, Capabilities, ComputePipelineDesc,
    ComputePipelineHandle, Device, DeviceInfo, DeviceRequest, Encoder, Extent, Format,
    GraphicsPipelineDesc, GraphicsPipelineHandle, HeapSlot, ImageDesc, ImageHandle, ImageMemory,
    ImageUsage, MemoryBlockDesc, MemoryBlockHandle, MemoryClass, PipelineStats, Placement,
    QueuePath, QueueRole, Rule, SamplerDesc, SamplerIndex, Sharing, Submit, Swapchain,
    SwapchainDesc, TimelineHandle, TimelinePoint,
};

use crate::convert::{self, aspect, buffer_usage, image_usage};
use crate::encoder::VulkanEncoder;
use crate::heap::Heap;
use crate::holders::Holders;
use crate::pipeline::{self, Cache};
use crate::raw::{self, Raw, Want, driver, image_info};
use crate::records::{
    BlockRec, BufferRec, ImageRec, PipelineRec, QUEUES, Records, Stamps, TimelineRec, queue_index,
    stale,
};
use crate::surface::Target;
use crate::swapchain::VulkanSwapchain;
use crate::table::Table;
use crate::{lock, submit};

/// Host memory a buffer's allocation is mapped to for as long as the buffer lives.
struct Mapping {
    at: NonNull<u8>,
    /// The allocation's size, which the mapping covers.
    size: u64,
    coherent: bool,
}

// SAFETY: the mapping is plain host memory the driver keeps mapped until the buffer's drop;
// every access copies through it under the records' host-range checks, which refuse two
// overlapping accesses, so the pointer carries no thread affinity and no shared `&mut`.
unsafe impl Send for Mapping {}
// SAFETY: as `Send`'s.
unsafe impl Sync for Mapping {}

/// A buffer as its handle resolves.
pub(crate) struct BufferObj {
    raw: Arc<Raw>,
    pub(crate) buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    pub(crate) size: u64,
    pub(crate) usage: BufferUsage,
    /// Its device address, where its usage holds `DEVICE_ADDRESS`; zero otherwise.
    pub(crate) address: u64,
    /// Made `Sharing::Exclusive`: a change between queues of two families transfers it.
    pub(crate) exclusive: bool,
    mapping: Option<Mapping>,
}

impl BufferObj {
    /// The range of the mapping a flush or an invalidate of `offset..end` covers: rounded
    /// out to the atom, and no further than the allocation.
    fn atoms(&self, mapping: &Mapping, offset: u64, end: u64) -> vk::MappedMemoryRange<'static> {
        let atom = self.raw.memory.non_coherent_atom;
        let start = offset / atom * atom;
        let end = end.div_ceil(atom).saturating_mul(atom).min(mapping.size);
        vk::MappedMemoryRange::default().memory(self.memory).offset(start).size(end - start)
    }

    /// Copies `bytes` into the mapping at `offset`, and flushes them where the memory is
    /// not coherent. The caller has bounded the range by the buffer's size and holds it as
    /// a host access, which no other access overlaps.
    fn write(&self, offset: u64, bytes: &[u8]) -> Result<()> {
        let Some(mapping) = &self.mapping else { return Ok(()) };
        // SAFETY: `offset + bytes.len()` is within the buffer, whose size is within the
        // mapping; no other access of the range is in progress (host-overlap), and a
        // pending submit that writes the buffer was refused (`pending`).
        unsafe {
            let at = mapping.at.as_ptr().add(usize::try_from(offset).unwrap_or(usize::MAX));
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), at, bytes.len());
        }
        if !mapping.coherent {
            let range = self.atoms(mapping, offset, offset + bytes.len() as u64);
            // SAFETY: the memory is mapped and the range within it, rounded to the atom.
            unsafe { self.raw.device.flush_mapped_memory_ranges(&[range]) }
                .map_err(|e| driver(e, "vkFlushMappedMemoryRanges"))?;
        }
        Ok(())
    }

    /// Copies `len` bytes out of the mapping from `offset`, invalidated first where the
    /// memory is not coherent. Bounded and held as `write`'s.
    fn read(&self, offset: u64, len: u64) -> Result<Vec<u8>> {
        let Some(mapping) = &self.mapping else { return Ok(Vec::new()) };
        if !mapping.coherent {
            let range = self.atoms(mapping, offset, offset + len);
            // SAFETY: as `write`'s.
            unsafe { self.raw.device.invalidate_mapped_memory_ranges(&[range]) }
                .map_err(|e| driver(e, "vkInvalidateMappedMemoryRanges"))?;
        }
        let len = usize::try_from(len).unwrap_or(usize::MAX);
        let mut bytes = vec![0; len];
        // SAFETY: as `write`'s; a pending submit that writes the buffer or changes its
        // state was refused, so nothing writes the range during the copy.
        unsafe {
            let at = mapping.at.as_ptr().add(usize::try_from(offset).unwrap_or(usize::MAX));
            core::ptr::copy_nonoverlapping(at, bytes.as_mut_ptr(), len);
        }
        Ok(bytes)
    }

    fn sync(&self, offset: u64, end: u64, flush: bool) -> Result<()> {
        let Some(mapping) = &self.mapping else { return Ok(()) };
        if mapping.coherent {
            return Ok(());
        }
        let range = self.atoms(mapping, offset, end);
        // SAFETY: the memory is mapped and the range within it, rounded to the atom.
        let done = unsafe {
            if flush {
                self.raw.device.flush_mapped_memory_ranges(&[range])
            } else {
                self.raw.device.invalidate_mapped_memory_ranges(&[range])
            }
        };
        let call =
            if flush { "vkFlushMappedMemoryRanges" } else { "vkInvalidateMappedMemoryRanges" };
        done.map_err(|e| driver(e, call))
    }
}

impl Drop for BufferObj {
    fn drop(&mut self) {
        if self.raw.leaking() {
            return;
        }
        // SAFETY: the last count of the buffer is gone: its handle is stale and no submit
        // that names it is pending (`pending`), so the GPU is done with it.
        unsafe {
            self.raw.device.destroy_buffer(self.buffer, None);
            self.raw.device.free_memory(self.memory, None);
        }
    }
}

/// A memory block as its handle resolves: one allocation images are placed in.
pub(crate) struct BlockObj {
    raw: Arc<Raw>,
    memory: vk::DeviceMemory,
    memory_type: u32,
}

impl Drop for BlockObj {
    fn drop(&mut self) {
        if self.raw.leaking() {
            return;
        }
        // SAFETY: every image placed in the block holds it, and is gone.
        unsafe { self.raw.device.free_memory(self.memory, None) };
    }
}

/// An image as its handle resolves.
pub(crate) struct ImageObj {
    raw: Arc<Raw>,
    pub(crate) image: vk::Image,
    /// Its own memory; `None` when placed in a block.
    memory: Option<vk::DeviceMemory>,
    /// The block it is placed in, kept alive while the image is.
    _block: Option<Arc<BlockObj>>,
    pub(crate) format: Format,
    pub(crate) extent: Extent,
    pub(crate) mips: u32,
    pub(crate) layers: u32,
    pub(crate) usage: ImageUsage,
    /// Made with a memoryless request, honoured or not.
    pub(crate) memoryless_request: bool,
    /// The request was honoured.
    pub(crate) memoryless: bool,
    pub(crate) placed: bool,
    /// Made `Sharing::Exclusive`.
    pub(crate) exclusive: bool,
    /// The view a heap slot holds: two-dimensional, layer zero, every mip.
    pub(crate) sampled: Option<vk::ImageView>,
    /// A view of each mip of each layer an attachment names, by `layer * mips + mip`.
    pub(crate) views: Vec<vk::ImageView>,
    /// One of a swapchain's images: the swapchain owns the image, and the object its views.
    pub(crate) swapchain: bool,
}

impl ImageObj {
    /// The object of one of a swapchain's images, with its attachment view: a colour
    /// attachment of one mip and one layer, owned by the present queue's family.
    pub(crate) fn swapchain_image(
        raw: &Arc<Raw>,
        image: vk::Image,
        format: Format,
        extent: Extent,
    ) -> Result<Self> {
        let mut object = Self {
            raw: Arc::clone(raw),
            image,
            memory: None,
            _block: None,
            format,
            extent,
            mips: 1,
            layers: 1,
            usage: ImageUsage::COLOUR,
            memoryless_request: false,
            memoryless: false,
            placed: false,
            exclusive: true,
            sampled: None,
            views: Vec::new(),
            swapchain: true,
        };
        make_views(&raw.device, &mut object)?;
        Ok(object)
    }

    pub(crate) fn mip_extent(&self, mip: u32) -> Extent {
        Extent {
            width: (self.extent.width >> mip.min(31)).max(1),
            height: (self.extent.height >> mip.min(31)).max(1),
        }
    }

    pub(crate) fn view(&self, mip: u32, layer: u32) -> vk::ImageView {
        self.views.get((layer * self.mips + mip) as usize).copied().unwrap_or_default()
    }
}

impl Drop for ImageObj {
    fn drop(&mut self) {
        if self.raw.leaking() {
            return;
        }
        // SAFETY: as `BufferObj`'s; no slot holds the image (`pending`), and its block, if
        // any, is freed after it. A swapchain's image is the swapchain's to destroy; a view
        // may be destroyed after its image, as nothing reads it then.
        unsafe {
            for view in self.sampled.iter().chain(&self.views) {
                self.raw.device.destroy_image_view(*view, None);
            }
            if self.swapchain {
                return;
            }
            self.raw.device.destroy_image(self.image, None);
            if let Some(memory) = self.memory {
                self.raw.device.free_memory(memory, None);
            }
        }
    }
}

/// A graphics pipeline as its handle resolves: what a pass and a draw check against.
pub(crate) struct GraphicsObj {
    pub(crate) raw: Arc<Raw>,
    pub(crate) pipeline: vk::Pipeline,
    pub(crate) colour: Vec<Format>,
    pub(crate) depth: Option<Format>,
    /// Bit `i`: the fragment stage reads input `i`.
    pub(crate) inputs: u32,
}

impl Drop for GraphicsObj {
    fn drop(&mut self) {
        if !self.raw.leaking() {
            // SAFETY: as `BufferObj`'s.
            unsafe { self.raw.device.destroy_pipeline(self.pipeline, None) };
        }
    }
}

pub(crate) struct ComputeObj {
    pub(crate) raw: Arc<Raw>,
    pub(crate) pipeline: vk::Pipeline,
}

impl Drop for ComputeObj {
    fn drop(&mut self) {
        if !self.raw.leaking() {
            // SAFETY: as `BufferObj`'s.
            unsafe { self.raw.device.destroy_pipeline(self.pipeline, None) };
        }
    }
}

/// A timeline as its handle resolves: its semaphore, which a wait in progress keeps alive
/// past a destroy.
pub(crate) struct TimelineObj {
    raw: Arc<Raw>,
    pub(crate) semaphore: vk::Semaphore,
}

impl Drop for TimelineObj {
    fn drop(&mut self) {
        if !self.raw.leaking() {
            // SAFETY: no pending submit waits for or signals it (`pending`), and every host
            // wait on it held this count.
            unsafe { self.raw.device.destroy_semaphore(self.semaphore, None) };
        }
    }
}

/// A heap slot's entry: the image written to it, if any.
pub(crate) type SlotEntry = Option<(ImageHandle, Arc<ImageObj>)>;

/// What the heap's lock guards besides the order of descriptor writes: the sampler table's
/// driver objects, and the samplers destroyed that a submit made before the destroy may
/// still read.
struct HeapState {
    samplers: Vec<Option<vk::Sampler>>,
    retired: Vec<(Stamps, vk::Sampler)>,
    /// The command pool and input set pools of each encoder dropped while its thread
    /// unwound with its submit pending, which the last holder destroys once the device is
    /// idle.
    orphans: Vec<(vk::CommandPool, Vec<vk::DescriptorPool>)>,
}

/// What a device, its encoders and its submits share.
///
/// The locks, in the one order any call takes them: a queue's (`queues`) or the heap's
/// (`heap_state`), never both and never two queues'; then the records' (`records`); then
/// one entry's, a handle's in a table or a heap slot's (`slots`). Recording takes an entry's
/// lock alone. No lock but a queue's or the heap's is held across a driver call, and none
/// across a wait or the caller's closure. A destroying call is one: the last count of a
/// driver object is dropped, and a reaped sampler destroyed, once the records', entry and
/// slot locks are given up.
pub(crate) struct Shared {
    pub(crate) raw: Arc<Raw>,
    pub(crate) caps: Capabilities,
    info: DeviceInfo,
    /// The index in `Raw::queues` of the queue each records queue (graphics, compute,
    /// present) is.
    roles: [usize; 3],
    /// Taken by a submit through its checks, its driver call and its hold after.
    pub(crate) queues: [Mutex<()>; QUEUES],
    heap_state: Mutex<HeapState>,
    pub(crate) heap: Heap,
    pub(crate) records: Mutex<Records>,
    lost: AtomicBool,
    /// The device objects and encoders that hold the device (`hold`).
    holders: Holders,
    pub(crate) buffers: Table<BufferObj>,
    pub(crate) images: Table<ImageObj>,
    blocks: Table<BlockObj>,
    pub(crate) graphics: Table<GraphicsObj>,
    pub(crate) compute: Table<ComputeObj>,
    pub(crate) timelines: Table<TimelineObj>,
    /// Each heap slot's image, behind the slot's entry lock, which a pass that declares the
    /// slot resolves as it begins.
    pub(crate) slots: Box<[Mutex<SlotEntry>]>,
    pub(crate) cache: Cache,
}

impl Shared {
    pub(crate) fn records(&self) -> MutexGuard<'_, Records> {
        lock(&self.records)
    }

    /// A new holder of the device: an encoder.
    pub(crate) fn hold(self: &Arc<Self>) -> Arc<Self> {
        self.holders.hold();
        Arc::clone(self)
    }

    /// Takes an unwinding encoder's pools, its submit pending, for the last holder to
    /// destroy once the device is idle.
    pub(crate) fn orphan(&self, pool: vk::CommandPool, inputs: Vec<vk::DescriptorPool>) {
        lock(&self.heap_state).orphans.push((pool, inputs));
    }

    /// A holder's drop. The last waits for the device to idle, as the contract's last drop
    /// waits for every submit, and destroys the samplers and the orphaned pools; one made
    /// while the thread unwinds a panic with a submit pending leaks the device instead, and
    /// one made so with none pending tears down as any other.
    pub(crate) fn let_go(&self) {
        // Every queue call another holder made before its drop happens before the last
        // holder's wait for idle, which needs every queue of the device externally
        // synchronised (`Holders::let_go`).
        if !self.holders.let_go() {
            return;
        }
        if std::thread::panicking() {
            let read = self.read_queues();
            let pending = {
                let mut records = self.records();
                records.fold(&read, &[]);
                records.any_pending() && !records.lost
            };
            if pending {
                self.raw.leak();
                return;
            }
        }
        // SAFETY: the last holder: no other call of the device is in progress.
        let _ = unsafe { self.raw.device.device_wait_idle() };
        let mut guard = lock(&self.heap_state);
        let state = &mut *guard;
        let device = &self.raw.device;
        let retired = state.retired.drain(..).map(|(_, sampler)| sampler);
        for sampler in retired.chain(state.samplers.iter_mut().filter_map(Option::take)) {
            // SAFETY: the device is idle.
            unsafe { device.destroy_sampler(sampler, None) };
        }
        for (pool, inputs) in state.orphans.drain(..) {
            // SAFETY: as above; the pools are an encoder's that is gone.
            unsafe {
                for inputs in inputs {
                    device.destroy_descriptor_pool(inputs, None);
                }
                device.destroy_command_pool(pool, None);
            }
        }
    }

    /// The families a resource of `sharing` is made concurrent across: none for an
    /// exclusive one, and none where every queue is of one family, as Vulkan's concurrent
    /// sharing names two families or more.
    pub(crate) fn families(&self, sharing: Sharing) -> Vec<u32> {
        if sharing == Sharing::Exclusive {
            return Vec::new();
        }
        let mut families: Vec<u32> = self.raw.queues.iter().map(|queue| queue.family).collect();
        families.sort_unstable();
        families.dedup();
        if families.len() < 2 { Vec::new() } else { families }
    }

    /// The raw queue a records queue is.
    pub(crate) fn queue(&self, queue: usize) -> &raw::Queue {
        &self.raw.queues[self.roles[queue]]
    }

    /// The records queue a role's submits go to.
    pub(crate) fn queue_of(&self, role: QueueRole) -> usize {
        queue_index(self.caps.queues.serving(role))
    }

    /// The queue as a barrier sees it.
    pub(crate) fn side(&self, queue: usize) -> convert::Side {
        let raw = self.queue(queue);
        convert::Side { queue: self.roles[queue], family: raw.family, graphics: raw.graphics }
    }

    /// The greatest of `values`, ascending, that `semaphore` has reached, each tried by a
    /// wait of no time; none if none has. A value is known reached only through a wait that
    /// succeeds, never a read of the counter: synchronisation validation (layer 1.4.363)
    /// takes a counter read as the host learning that every batch the queue was given has
    /// completed, whatever value it returns, and then reports no hazard across submits.
    ///
    /// # Safety
    ///
    /// `semaphore` is a timeline semaphore of this device, live through the call.
    unsafe fn reached(
        &self,
        semaphore: vk::Semaphore,
        values: &[u64],
    ) -> std::result::Result<Option<u64>, vk::Result> {
        // A timeline's value never falls, so the values reached are a prefix of `values`.
        let (mut low, mut high, mut best) = (0, values.len(), None);
        while low < high {
            let mid = low + (high - low) / 2;
            let semaphores = [semaphore];
            let value = [values[mid]];
            let info = vk::SemaphoreWaitInfo::default().semaphores(&semaphores).values(&value);
            // SAFETY: the caller keeps the semaphore alive; a wait of no time blocks nothing.
            match unsafe { self.raw.device.wait_semaphores(&info, 0) } {
                Ok(()) => {
                    best = Some(values[mid]);
                    low = mid + 1;
                }
                Err(vk::Result::TIMEOUT) => high = mid,
                Err(error) => return Err(error),
            }
        }
        Ok(best)
    }

    /// Reads from the driver, with no lock held, the greatest stamp of a queue's pending
    /// submits that has completed (`reached`). A loss is marked; any other failure, or no
    /// pending submit completed, reads nothing.
    pub(crate) fn read_queue(&self, queue: usize) -> Option<u64> {
        let pending: Vec<u64> = {
            let records = self.records();
            let rec = &records.queues[queue];
            (rec.known + 1..=rec.received).collect()
        };
        // SAFETY: the queue's semaphore lives as long as the device.
        match unsafe { self.reached(self.queue(queue).timeline, &pending) } {
            Ok(value) => value,
            Err(error) => {
                self.lost_on(error);
                None
            }
        }
    }

    /// Reads every queue's completed value, each queue the layout gives its own once.
    pub(crate) fn read_queues(&self) -> [Option<u64>; QUEUES] {
        let mut read = [None; QUEUES];
        read[0] = self.read_queue(0);
        if self.caps.queues.compute == QueuePath::Own {
            read[1] = self.read_queue(1);
        }
        if self.caps.queues.present == QueuePath::Own {
            read[2] = self.read_queue(2);
        }
        read
    }

    /// Reads from the driver, with no lock held, the greatest value a timeline is awaited
    /// to reach that it has reached (`reached`), if its handle resolves.
    pub(crate) fn read_timeline(&self, timeline: TimelineHandle) -> Option<(TimelineHandle, u64)> {
        let object = self.timelines.resolve(timeline)?;
        let awaited = self.records().awaited(timeline);
        // SAFETY: the count keeps the semaphore alive through the call.
        match unsafe { self.reached(object.semaphore, &awaited) } {
            Ok(value) => value.map(|value| (timeline, value)),
            Err(error) => {
                self.lost_on(error);
                None
            }
        }
    }

    /// Marks the device lost where `result` says so.
    pub(crate) fn lost_on(&self, result: vk::Result) {
        if result == vk::Result::ERROR_DEVICE_LOST {
            let mut records = self.records();
            records.lost = true;
            // Release: `is_lost` promises nothing about what else it sees; every call that
            // acts on a loss reads the records'.
            self.lost.store(true, Ordering::Release);
        }
    }

    /// The records after the queues' completed values are read and folded in.
    pub(crate) fn settled(&self) -> MutexGuard<'_, Records> {
        let read = self.read_queues();
        let mut records = self.records();
        records.fold(&read, &[]);
        records
    }

    /// Takes out the retired samplers no pending submit may read, for `destroy_samplers`
    /// once the records' lock is given up.
    fn reap(state: &mut HeapState, records: &Records) -> Vec<vk::Sampler> {
        let mut reaped = Vec::new();
        state.retired.retain(|&(stamps, sampler)| {
            let kept = records.pending(&stamps) && !records.lost;
            if !kept {
                reaped.push(sampler);
            }
            kept
        });
        reaped
    }

    /// Destroys the samplers `reap` took out, under the heap's lock and no other.
    fn destroy_samplers(&self, samplers: Vec<vk::Sampler>) {
        for sampler in samplers {
            // SAFETY: every submit stamped before the sampler's entry held the placeholder
            // has completed; later submits read the placeholder.
            unsafe { self.raw.device.destroy_sampler(sampler, None) };
        }
    }
}

/// The error of every call that reaches a lost device.
pub(crate) fn lost() -> Error {
    Error::new(ErrorKind::DeviceLost, "the Vulkan device is lost")
}

/// Refuses a call on a lost device.
pub(crate) fn alive(records: &Records) -> Result<()> {
    if records.lost { Err(lost()) } else { Ok(()) }
}

/// The end of `len` bytes from `offset` in a resource of `size` bytes, or the refusal of a
/// range that leaves it.
pub(crate) fn range(offset: u64, len: u64, size: u64, what: &str) -> Result<u64> {
    match offset.checked_add(len) {
        Some(end) if end <= size => Ok(end),
        _ => {
            Err(Rule::Bounds.refuse(format!("{what}: {len} bytes from {offset}, in {size} bytes")))
        }
    }
}

/// The Vulkan backend's device: a driver's adapter, made against the required set and
/// reporting what it offers of the optional one, every rule of the contract's "Every
/// backend" column checked before the driver call it guards.
pub struct VulkanDevice {
    shared: Arc<Shared>,
}

impl VulkanDevice {
    /// Makes a device on the adapter `request` names, or on the loader's first, with a heap
    /// and sampler table of the sizes it asks for and its pipeline cache started from its
    /// bytes. Its sink takes every warning and error the Khronos validation layer, the
    /// loader and the driver report through the debug messenger.
    ///
    /// # Errors
    ///
    /// `ErrorKind::Unsupported` where no loader is found, the adapter lacks a feature of the
    /// required set (named in the message), its limits are below
    /// [`Capabilities::LEAST`]'s or below the heap and sampler table asked for, or the request
    /// has a surface: this device presents to no window yet. `ErrorKind::NotFound` for an
    /// adapter no name of the loader's contains. A driver's failure as its kind. A
    /// pipeline cache that fails a check starts the cache empty and is no error.
    pub fn new(request: DeviceRequest) -> Result<Self> {
        let target = request.surface.as_ref().map(|source| Target::Window(Arc::clone(source)));
        Self::open(&request, target)
    }

    /// Makes a device as [`new`](Self::new) does that presents to a surface no window shows
    /// (`VK_EXT_headless_surface`), for tests that need a swapchain and no window. Its
    /// swapchain's images reach no display, and it reports no display times.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new)'s; `ErrorKind::Unsupported` too for a request that names a
    /// window, and where the driver offers no headless surface.
    pub fn headless(request: DeviceRequest) -> Result<Self> {
        if request.surface.is_some() {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "a headless device for a request that names a window",
            ));
        }
        Self::open(&request, Some(Target::Headless))
    }

    fn open(request: &DeviceRequest, target: Option<Target>) -> Result<Self> {
        let opened = raw::open(request, target)?;
        let caps = opened.caps;
        let heap = Heap::new(&opened.raw, caps.heap_slots, caps.samplers, caps.colour_attachments)?;
        let (cache, start) =
            Cache::open(&opened.raw, opened.cache_identity, &request.pipeline_cache)?;
        let samplers = vec![None; caps.samplers as usize];
        let shared = Shared {
            raw: opened.raw,
            caps,
            info: opened.info,
            roles: opened.roles,
            queues: [Mutex::new(()), Mutex::new(()), Mutex::new(())],
            heap_state: Mutex::new(HeapState {
                samplers,
                retired: Vec::new(),
                orphans: Vec::new(),
            }),
            heap,
            records: Mutex::new(Records::new(caps.heap_slots, caps.samplers, start)),
            lost: AtomicBool::new(false),
            holders: Holders::new(),
            buffers: Table::new(),
            images: Table::new(),
            blocks: Table::new(),
            graphics: Table::new(),
            compute: Table::new(),
            timelines: Table::new(),
            slots: (0..caps.heap_slots).map(|_| Mutex::new(None)).collect(),
            cache,
        };
        Ok(Self { shared: Arc::new(shared) })
    }

    /// What the device holds, for the unit tests' calls below the contract: the
    /// thread-safety canary's, and the swapchain's injected reuse.
    #[cfg(test)]
    pub(crate) fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    fn records(&self) -> MutexGuard<'_, Records> {
        self.shared.records()
    }

    /// Writes a heap slot, or empties it, in the two holds the contract names, under the
    /// heap's lock.
    fn heap_write(&self, slot: HeapSlot, image: Option<ImageHandle>) -> Result<()> {
        let shared = &*self.shared;
        let _heap = lock(&shared.heap_state);
        let index = slot.0 as usize;
        let read = shared.read_queues();
        let resolved = {
            let mut records = self.records();
            alive(&records)?;
            records.fold(&read, &[]);
            if index >= records.slots.len() {
                return Err(Rule::Bounds.refuse(format!(
                    "slot {} of a heap of {}",
                    slot.0,
                    records.slots.len()
                )));
            }
            let resolved = match image {
                None => None,
                Some(image) => {
                    records.images.live(image, "an image")?;
                    let object = shared.images.resolve(image).ok_or_else(|| stale("an image"))?;
                    if !object.usage.contains(ImageUsage::SAMPLED) {
                        return Err(Rule::Usage.refuse("a slot written with an image not SAMPLED"));
                    }
                    Some((image, object))
                }
            };
            records.begin_slot_write(index, image)?;
            resolved
        };
        let view = resolved.as_ref().and_then(|(_, object)| object.sampled);
        // SAFETY: the heap's lock is held; the view's image is counted by the slot from the
        // first hold, so no destroy frees it while the slot holds it (`pending`).
        unsafe { shared.heap.write_slot(slot.0, view) };
        // The image the slot held may be its last count: it is dropped after the slot's lock.
        let replaced = core::mem::replace(&mut *lock(&shared.slots[index]), resolved);
        drop(replaced);
        self.records().end_slot_write(index, image);
        Ok(())
    }

    /// Allocates memory of `requirements` and of `want`, with a device address where asked.
    fn allocate(
        &self,
        requirements: vk::MemoryRequirements,
        memory_type: u32,
        address: bool,
    ) -> Result<vk::DeviceMemory> {
        let largest = self.shared.raw.memory.largest[memory_type as usize];
        if requirements.size > largest {
            return Err(Error::new(
                ErrorKind::ResourceExhausted,
                format!(
                    "an allocation of {} bytes, above the {largest} its memory type allows",
                    requirements.size
                ),
            ));
        }
        let mut flags =
            vk::MemoryAllocateFlagsInfo::default().flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
        let mut info = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type);
        if address {
            info = info.push_next(&mut flags);
        }
        // SAFETY: the device is live; the type is one the resource accepts.
        unsafe { self.shared.raw.device.allocate_memory(&info, None) }
            .map_err(|e| driver(e, "vkAllocateMemory"))
    }

    /// Makes the driver's buffer of `desc`, its memory bound and mapped as its class asks.
    fn make_buffer(&self, desc: &BufferDesc<'_>) -> Result<BufferObj> {
        let raw = &self.shared.raw;
        let device = &raw.device;
        let families = self.shared.families(desc.sharing);
        let mut info = vk::BufferCreateInfo::default()
            .size(desc.size)
            .usage(buffer_usage(desc.usage))
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        if !families.is_empty() {
            info = info.sharing_mode(vk::SharingMode::CONCURRENT).queue_family_indices(&families);
        }
        // SAFETY: the device is live and the info valid: a size above zero.
        let buffer = unsafe { device.create_buffer(&info, None) }
            .map_err(|e| driver(e, "vkCreateBuffer"))?;
        let mut object = BufferObj {
            raw: Arc::clone(raw),
            buffer,
            memory: vk::DeviceMemory::null(),
            size: desc.size,
            usage: desc.usage,
            address: 0,
            exclusive: desc.sharing == Sharing::Exclusive,
            mapping: None,
        };
        // SAFETY: the buffer is live.
        let requirements = unsafe { device.get_buffer_memory_requirements(buffer) };
        let want = match desc.memory {
            MemoryClass::Device => Want::Device,
            MemoryClass::Upload => Want::Upload,
            MemoryClass::Readback => Want::Readback,
        };
        let memory_type =
            raw.memory.choose(requirements.memory_type_bits, want).ok_or_else(|| {
                Error::new(
                    ErrorKind::Unsupported,
                    format!("no memory type for a {:?} buffer", desc.memory),
                )
            })?;
        let address = desc.usage.contains(BufferUsage::DEVICE_ADDRESS);
        object.memory = self.allocate(requirements, memory_type, address)?;
        // SAFETY: both live; the memory unbound, of a type and size the buffer accepts.
        unsafe { device.bind_buffer_memory(buffer, object.memory, 0) }
            .map_err(|e| driver(e, "vkBindBufferMemory"))?;
        if address {
            let info = vk::BufferDeviceAddressInfo::default().buffer(buffer);
            // SAFETY: the buffer has SHADER_DEVICE_ADDRESS and memory with the address flag.
            object.address = unsafe { device.get_buffer_device_address(&info) };
        }
        if raw.memory.host_visible(memory_type) {
            // SAFETY: host-visible memory, unmapped, mapped whole for the buffer's life.
            let at = unsafe {
                device.map_memory(object.memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())
            }
            .map_err(|e| driver(e, "vkMapMemory"))?;
            let at = NonNull::new(at.cast::<u8>())
                .ok_or_else(|| Error::new(ErrorKind::Platform, "vkMapMemory gave no address"))?;
            object.mapping = Some(Mapping {
                at,
                size: requirements.size,
                coherent: raw.memory.coherent(memory_type),
            });
        }
        Ok(object)
    }

    /// Refuses an image description no device honours, and one this device cannot make.
    fn check_image(&self, desc: &ImageDesc<'_>) -> Result<()> {
        check_image(desc)?;
        let raw = &self.shared.raw;
        let limits = &raw.properties.limits;
        let extent = desc.extent;
        if extent.width > limits.max_image_dimension2_d
            || extent.height > limits.max_image_dimension2_d
            || desc.layers > limits.max_image_array_layers
        {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!(
                    "an image past {} texels a side or {} layers",
                    limits.max_image_dimension2_d, limits.max_image_array_layers
                ),
            ));
        }
        // SAFETY: a query of the instance's adapter.
        let offered = unsafe {
            raw.instance.get_physical_device_image_format_properties(
                raw.physical,
                convert::format(desc.format),
                vk::ImageType::TYPE_2D,
                vk::ImageTiling::OPTIMAL,
                image_usage(desc.usage),
                vk::ImageCreateFlags::empty(),
            )
        };
        match offered {
            Ok(properties)
                if properties.max_mip_levels >= desc.mips
                    && properties.max_array_layers >= desc.layers =>
            {
                Ok(())
            }
            _ => Err(Error::new(
                ErrorKind::Unsupported,
                format!(
                    "an image of {:?} with {:?}, which the adapter does not offer",
                    desc.format, desc.usage
                ),
            )),
        }
    }

    /// The driver's description of an image of `desc`, concurrent across `families` when
    /// they are given.
    fn image_info<'a>(&self, desc: &ImageDesc<'_>, families: &'a [u32]) -> vk::ImageCreateInfo<'a> {
        let extent = vk::Extent2D { width: desc.extent.width, height: desc.extent.height };
        let info = image_info(desc.format, extent, desc.mips, desc.layers, image_usage(desc.usage));
        if families.is_empty() {
            info
        } else {
            info.sharing_mode(vk::SharingMode::CONCURRENT).queue_family_indices(families)
        }
    }

    /// The memory requirements of an image of `desc`, made or not.
    fn requirements(&self, desc: &ImageDesc<'_>) -> vk::MemoryRequirements {
        let families = self.shared.families(desc.sharing);
        let info = self.image_info(desc, &families);
        let request = vk::DeviceImageMemoryRequirements::default().create_info(&info);
        let mut out = vk::MemoryRequirements2::default();
        // SAFETY: a valid description, checked by `check_image`.
        unsafe { self.shared.raw.device.get_device_image_memory_requirements(&request, &mut out) };
        out.memory_requirements
    }
}

/// Makes the views of an image: the heap's and one for each attachment subresource.
fn make_views(device: &ash::Device, object: &mut ImageObj) -> Result<()> {
    let view = |mip: u32, mips: u32, layer: u32| {
        let range = vk::ImageSubresourceRange::default()
            .aspect_mask(aspect(object.format))
            .base_mip_level(mip)
            .level_count(mips)
            .base_array_layer(layer)
            .layer_count(1);
        let info = vk::ImageViewCreateInfo::default()
            .image(object.image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(convert::format(object.format))
            .subresource_range(range);
        // SAFETY: the image is live and bound, and the range within it.
        unsafe { device.create_image_view(&info, None) }.map_err(|e| driver(e, "vkCreateImageView"))
    };
    if object.usage.contains(ImageUsage::SAMPLED) {
        object.sampled = Some(view(0, object.mips, 0)?);
    }
    if object.usage.contains(ImageUsage::COLOUR) || object.usage.contains(ImageUsage::DEPTH) {
        for layer in 0..object.layers {
            for mip in 0..object.mips {
                let made = view(mip, 1, layer)?;
                object.views.push(made);
            }
        }
    }
    Ok(())
}

impl Drop for VulkanDevice {
    fn drop(&mut self) {
        self.shared.let_go();
    }
}

/// Refuses an image description no device honours.
fn check_image(desc: &ImageDesc<'_>) -> Result<()> {
    let usage = desc.usage;
    let never = |what: &str| Err(Rule::Usage.refuse(format!("an image {what}")));
    let extent = desc.extent;
    if extent.width == 0 || extent.height == 0 || desc.mips == 0 || desc.layers == 0 {
        return never("with a zero extent, mip count or layer count");
    }
    if desc.mips > u32::BITS - extent.width.max(extent.height).leading_zeros() {
        return never("with more mips than its extent has");
    }
    if usage == ImageUsage::EMPTY {
        return never("with no usage");
    }
    let colour = usage.contains(ImageUsage::COLOUR);
    let depth = usage.contains(ImageUsage::DEPTH);
    if colour && depth {
        return never("both a colour and a depth attachment");
    }
    if colour && desc.format.is_depth() {
        return never("a colour attachment of a depth format");
    }
    if depth && !desc.format.is_depth() {
        return never("a depth attachment of a colour format");
    }
    if usage.contains(ImageUsage::INPUT) && !colour {
        return never("an input that is no colour attachment");
    }
    if usage.contains(ImageUsage::TRANSIENT) {
        let attachment = ImageUsage::COLOUR.union(ImageUsage::DEPTH).union(ImageUsage::INPUT);
        if usage.bits() & !attachment.union(ImageUsage::TRANSIENT).bits() != 0 {
            return never("TRANSIENT with a use other than an attachment's");
        }
        if !colour && !depth {
            return never("TRANSIENT and no attachment");
        }
        if desc.mips != 1 || desc.layers != 1 {
            return never("TRANSIENT with more than one mip or layer");
        }
    }
    if desc.memory == ImageMemory::Memoryless && !usage.contains(ImageUsage::TRANSIENT) {
        return never("with a memoryless request and no TRANSIENT");
    }
    Ok(())
}

impl Device for VulkanDevice {
    fn info(&self) -> &DeviceInfo {
        &self.shared.info
    }

    fn capabilities(&self) -> &Capabilities {
        &self.shared.caps
    }

    fn is_lost(&self) -> bool {
        // Pairs with `lost_on`'s store, though nothing is read through it.
        self.shared.lost.load(Ordering::Acquire)
    }

    fn create_buffer(&self, desc: &BufferDesc<'_>) -> Result<BufferHandle> {
        alive(&self.records())?;
        if desc.size == 0 {
            return Err(Rule::Usage.refuse("a buffer of no bytes"));
        }
        let max = self.shared.raw.memory.buffer_max;
        if desc.size > max {
            return Err(Error::new(
                ErrorKind::ResourceExhausted,
                format!("a buffer of more than the device's {max} bytes"),
            ));
        }
        let object = self.make_buffer(desc)?;
        let handle = {
            let mut records = self.records();
            alive(&records)?;
            records.buffers.insert(BufferRec::new(desc.size, desc.memory))?
        };
        self.shared.buffers.insert(handle, Arc::new(object));
        Ok(handle)
    }

    fn destroy_buffer(&self, buffer: BufferHandle) -> Result<()> {
        let mut records = self.shared.settled();
        let rec = records.buffers.live(buffer, "a buffer")?;
        if !records.lost {
            if !rec.host.is_empty() {
                return Err(Rule::HostOverlap.refuse("a buffer destroyed during a host access"));
            }
            if records.pending(&rec.stamps) {
                return Err(Rule::Pending.refuse("a buffer destroyed under a pending submit"));
            }
        }
        records.buffers.remove(buffer);
        drop(records);
        self.shared.buffers.remove(buffer);
        Ok(())
    }

    fn buffer_address(&self, buffer: BufferHandle) -> Result<u64> {
        let records = self.records();
        alive(&records)?;
        records.buffers.live(buffer, "a buffer")?;
        let object = self.shared.buffers.resolve(buffer).ok_or_else(|| stale("a buffer"))?;
        if !object.usage.contains(BufferUsage::DEVICE_ADDRESS) {
            return Err(Rule::Usage.refuse("the address of a buffer without DEVICE_ADDRESS"));
        }
        Ok(object.address)
    }

    fn write_buffer(
        &self,
        buffer: BufferHandle,
        offset: u64,
        len: u64,
        fill: &mut dyn FnMut(&mut [u8]),
    ) -> Result<()> {
        let (object, end) = self.host_access(buffer, offset, len, Access::Write)?;
        let lend = Lend { shared: &self.shared, buffer, start: offset, end };
        let mut bytes = vec![0; usize::try_from(len).unwrap_or(usize::MAX)];
        fill(&mut bytes);
        let written = object.write(offset, &bytes);
        drop(lend);
        written
    }

    fn read_buffer(
        &self,
        buffer: BufferHandle,
        offset: u64,
        len: u64,
        read: &mut dyn FnMut(&[u8]),
    ) -> Result<()> {
        let (object, end) = self.host_access(buffer, offset, len, Access::Read)?;
        let lend = Lend { shared: &self.shared, buffer, start: offset, end };
        let bytes = object.read(offset, len);
        if let Ok(bytes) = &bytes {
            read(bytes);
        }
        drop(lend);
        bytes.map(drop)
    }

    fn flush_buffer(&self, buffer: BufferHandle, offset: u64, len: u64) -> Result<()> {
        // Checked under the records' lock, as null checks it, and no range held for the
        // driver's call: the host only writes an `Upload` buffer, and the resolved count
        // keeps the memory through a destroy made meanwhile.
        let (object, end) = self.host_access(buffer, offset, len, Access::Flush)?;
        object.sync(offset, end, true)
    }

    fn invalidate_buffer(&self, buffer: BufferHandle, offset: u64, len: u64) -> Result<()> {
        // As a flush's: the host only reads a `Readback` buffer.
        let (object, end) = self.host_access(buffer, offset, len, Access::Invalidate)?;
        object.sync(offset, end, false)
    }

    fn create_image(&self, desc: &ImageDesc<'_>) -> Result<ImageHandle> {
        alive(&self.records())?;
        self.check_image(desc)?;
        let raw = &self.shared.raw;
        let device = &raw.device;
        let usage = desc.usage;
        let memoryless_request = desc.memory == ImageMemory::Memoryless;
        let requirements = self.requirements(desc);
        let lazy = if usage.contains(ImageUsage::COLOUR) && usage.contains(ImageUsage::INPUT) {
            raw.memory.lazy_colour
        } else if usage.contains(ImageUsage::DEPTH) {
            raw.memory.lazy_depth
        } else {
            None
        }
        .filter(|&at| memoryless_request && requirements.memory_type_bits & (1 << at) != 0);
        let block = match desc.memory {
            ImageMemory::Placed { block, offset } => {
                let size = {
                    let records = self.records();
                    records.blocks.live(block, "a memory block")?.size
                };
                let object =
                    self.shared.blocks.resolve(block).ok_or_else(|| stale("a memory block"))?;
                if requirements.memory_type_bits & (1 << object.memory_type) == 0 {
                    return Err(Error::new(
                        ErrorKind::Unsupported,
                        "an image no memory block can hold",
                    ));
                }
                if offset % requirements.alignment != 0 {
                    return Err(Rule::Bounds.refuse(format!(
                        "a placement at {offset}, not a multiple of {}",
                        requirements.alignment
                    )));
                }
                range(offset, requirements.size, size, "a placement")?;
                Some((block, object, offset))
            }
            _ => None,
        };
        let families = self.shared.families(desc.sharing);
        let info = self.image_info(desc, &families);
        // SAFETY: the device is live and the description checked against the adapter.
        let image =
            unsafe { device.create_image(&info, None) }.map_err(|e| driver(e, "vkCreateImage"))?;
        let mut object = ImageObj {
            raw: Arc::clone(raw),
            image,
            memory: None,
            _block: block.as_ref().map(|(_, object, _)| Arc::clone(object)),
            format: desc.format,
            extent: desc.extent,
            mips: desc.mips,
            layers: desc.layers,
            usage,
            memoryless_request,
            memoryless: lazy.is_some(),
            placed: block.is_some(),
            exclusive: desc.sharing == Sharing::Exclusive,
            sampled: None,
            views: Vec::new(),
            swapchain: false,
        };
        // SAFETY: the image is live.
        let actual = unsafe { device.get_image_memory_requirements(image) };
        match &block {
            Some((_, block, offset)) => {
                // SAFETY: the block's type is among the image's, and the range inside it at
                // the image's alignment, as checked above.
                unsafe { device.bind_image_memory(image, block.memory, *offset) }
                    .map_err(|e| driver(e, "vkBindImageMemory"))?;
            }
            None => {
                let memory_type = match lazy {
                    Some(at) => at,
                    None => raw.memory.choose(actual.memory_type_bits, Want::Device).ok_or_else(
                        || Error::new(ErrorKind::Unsupported, "no memory type for the image"),
                    )?,
                };
                let memory = self.allocate(actual, memory_type, false)?;
                object.memory = Some(memory);
                // SAFETY: fresh memory of a type and size the image accepts.
                unsafe { device.bind_image_memory(image, memory, 0) }
                    .map_err(|e| driver(e, "vkBindImageMemory"))?;
            }
        }
        make_views(device, &mut object)?;
        let handle = {
            let mut records = self.records();
            alive(&records)?;
            let placed = block.as_ref().map(|(handle, ..)| *handle);
            if let Some(handle) = placed {
                records.blocks.live_mut(handle, "a memory block")?.images += 1;
            }
            records.images.insert(ImageRec {
                stamps: [0; QUEUES],
                slots: 0,
                placed,
                swapchain: false,
            })?
        };
        self.shared.images.insert(handle, Arc::new(object));
        Ok(handle)
    }

    fn destroy_image(&self, image: ImageHandle) -> Result<()> {
        let mut records = self.shared.settled();
        let rec = records.images.live(image, "an image")?;
        if rec.swapchain {
            return Err(Rule::Swapchain.refuse("a destroy of a swapchain's image"));
        }
        if !records.lost {
            if rec.slots > 0 {
                return Err(Rule::Pending.refuse("an image a heap slot holds, destroyed"));
            }
            if records.pending(&rec.stamps) {
                return Err(Rule::Pending.refuse("an image destroyed under a pending submit"));
            }
        }
        let placed = rec.placed;
        records.images.remove(image);
        if let Some(block) = placed.and_then(|block| records.blocks.get_mut(block)) {
            block.images -= 1;
        }
        drop(records);
        self.shared.images.remove(image);
        Ok(())
    }

    fn image_memoryless(&self, image: ImageHandle) -> Result<bool> {
        let records = self.records();
        alive(&records)?;
        records.images.live(image, "an image")?;
        let object = self.shared.images.resolve(image).ok_or_else(|| stale("an image"))?;
        Ok(object.memoryless)
    }

    fn image_placement(&self, desc: &ImageDesc<'_>) -> Result<Placement> {
        alive(&self.records())?;
        self.check_image(desc)?;
        let requirements = self.requirements(desc);
        match self.shared.raw.memory.block {
            Some(at) if requirements.memory_type_bits & (1 << at) != 0 => {
                Ok(Placement { size: requirements.size, alignment: requirements.alignment })
            }
            _ => Err(Error::new(ErrorKind::Unsupported, "an image no memory block can hold")),
        }
    }

    fn create_memory_block(&self, desc: &MemoryBlockDesc<'_>) -> Result<MemoryBlockHandle> {
        alive(&self.records())?;
        if desc.size == 0 {
            return Err(Rule::Usage.refuse("a memory block of no bytes"));
        }
        let raw = &self.shared.raw;
        let Some(memory_type) = raw.memory.block else {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "no memory type both colour and depth images take",
            ));
        };
        let requirements = vk::MemoryRequirements {
            size: desc.size,
            alignment: 1,
            memory_type_bits: 1 << memory_type,
        };
        let memory = self.allocate(requirements, memory_type, false)?;
        let object = BlockObj { raw: Arc::clone(raw), memory, memory_type };
        let handle = {
            let mut records = self.records();
            alive(&records)?;
            records.blocks.insert(BlockRec { size: desc.size, images: 0 })?
        };
        self.shared.blocks.insert(handle, Arc::new(object));
        Ok(handle)
    }

    fn destroy_memory_block(&self, block: MemoryBlockHandle) -> Result<()> {
        let mut records = self.records();
        let rec = records.blocks.live(block, "a memory block")?;
        if !records.lost && rec.images > 0 {
            return Err(
                Rule::Pending.refuse("a memory block destroyed while an image is placed in it")
            );
        }
        records.blocks.remove(block);
        drop(records);
        self.shared.blocks.remove(block);
        Ok(())
    }

    fn write_heap_slot(&self, slot: HeapSlot, image: ImageHandle) -> Result<()> {
        self.heap_write(slot, Some(image))
    }

    fn clear_heap_slot(&self, slot: HeapSlot) -> Result<()> {
        self.heap_write(slot, None)
    }

    fn create_sampler(&self, desc: &SamplerDesc) -> Result<SamplerIndex> {
        let shared = &*self.shared;
        let mut state = lock(&shared.heap_state);
        let (index, reaped) = {
            let records = shared.settled();
            alive(&records)?;
            let reaped = Shared::reap(&mut state, &records);
            (records.samplers.iter().position(|live| !live), reaped)
        };
        shared.destroy_samplers(reaped);
        let Some(index) = index else {
            return Err(Error::new(ErrorKind::ResourceExhausted, "the sampler table is full"));
        };
        // The table's samplers, those retired that a pending submit may still read, and the
        // placeholder are all live: the device's count bounds them together.
        let live = state.samplers.iter().flatten().count() + state.retired.len() + 1;
        let max = shared.raw.properties.limits.max_sampler_allocation_count;
        if live >= max as usize {
            return Err(Error::new(
                ErrorKind::ResourceExhausted,
                format!(
                    "{live} samplers live, the device's {max} at most, until a retired one is reaped"
                ),
            ));
        }
        let info = vk::SamplerCreateInfo::default()
            .mag_filter(convert::filter(desc.magnify))
            .min_filter(convert::filter(desc.minify))
            .mipmap_mode(convert::mip_filter(desc.mip))
            .address_mode_u(convert::address(desc.address_u))
            .address_mode_v(convert::address(desc.address_v))
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .max_lod(vk::LOD_CLAMP_NONE);
        // SAFETY: the device is live and the info valid.
        let sampler = unsafe { shared.raw.device.create_sampler(&info, None) }
            .map_err(|e| driver(e, "vkCreateSampler"))?;
        let at = u32::try_from(index).unwrap_or(u32::MAX);
        // SAFETY: the heap's lock is held, and the sampler lives until its entry is written
        // again and no submit that may read it is pending (`retired`).
        unsafe { shared.heap.write_sampler(at, Some(sampler)) };
        state.samplers[index] = Some(sampler);
        self.records().samplers[index] = true;
        Ok(SamplerIndex(at))
    }

    fn destroy_sampler(&self, sampler: SamplerIndex) -> Result<()> {
        let shared = &*self.shared;
        let mut state = lock(&shared.heap_state);
        let index = sampler.0 as usize;
        {
            let mut records = shared.settled();
            if !records.samplers.get(index).copied().unwrap_or(false) {
                return Err(Rule::Destroyed.refuse(format!("sampler {} is not live", sampler.0)));
            }
            if !records.lost && records.any_pending() {
                return Err(Rule::Pending.refuse("a sampler destroyed while a submit is pending"));
            }
            records.samplers[index] = false;
        }
        // SAFETY: the heap's lock is held; the placeholder lives as long as the device.
        unsafe { shared.heap.write_sampler(sampler.0, None) };
        if let Some(object) = state.samplers[index].take() {
            // A submit stamped before the entry held the placeholder may read it: it waits
            // for those, past which every submit reads the placeholder.
            let reaped = {
                let records = self.records();
                let stamps = records.next_stamps().map(|next| next - 1);
                state.retired.push((stamps, object));
                Shared::reap(&mut state, &records)
            };
            shared.destroy_samplers(reaped);
        }
        Ok(())
    }

    fn create_graphics_pipeline(
        &self,
        desc: &GraphicsPipelineDesc<'_>,
    ) -> Result<GraphicsPipelineHandle> {
        alive(&self.records())?;
        let start = Instant::now();
        let (object, hits, misses) = pipeline::graphics(&self.shared, desc)?;
        let handle = {
            let mut records = self.records();
            alive(&records)?;
            pipeline::count(&mut records.stats, hits, misses, start);
            records.graphics.insert(PipelineRec::default())?
        };
        self.shared.graphics.insert(handle, Arc::new(object));
        Ok(handle)
    }

    fn create_compute_pipeline(
        &self,
        desc: &ComputePipelineDesc<'_>,
    ) -> Result<ComputePipelineHandle> {
        alive(&self.records())?;
        let start = Instant::now();
        let (object, hits, misses) = pipeline::compute(&self.shared, desc)?;
        let handle = {
            let mut records = self.records();
            alive(&records)?;
            pipeline::count(&mut records.stats, hits, misses, start);
            records.compute.insert(PipelineRec::default())?
        };
        self.shared.compute.insert(handle, Arc::new(object));
        Ok(handle)
    }

    fn destroy_graphics_pipeline(&self, pipeline: GraphicsPipelineHandle) -> Result<()> {
        let mut records = self.shared.settled();
        let rec = records.graphics.live(pipeline, "a graphics pipeline")?;
        if !records.lost && records.pending(&rec.stamps) {
            return Err(Rule::Pending.refuse("a pipeline destroyed under a pending submit"));
        }
        records.graphics.remove(pipeline);
        drop(records);
        self.shared.graphics.remove(pipeline);
        Ok(())
    }

    fn destroy_compute_pipeline(&self, pipeline: ComputePipelineHandle) -> Result<()> {
        let mut records = self.shared.settled();
        let rec = records.compute.live(pipeline, "a compute pipeline")?;
        if !records.lost && records.pending(&rec.stamps) {
            return Err(Rule::Pending.refuse("a pipeline destroyed under a pending submit"));
        }
        records.compute.remove(pipeline);
        drop(records);
        self.shared.compute.remove(pipeline);
        Ok(())
    }

    fn pipeline_cache(&self) -> Result<Vec<u8>> {
        alive(&self.records())?;
        self.shared.cache.seal()
    }

    fn pipeline_stats(&self) -> PipelineStats {
        self.records().stats
    }

    fn create_timeline(&self, initial: u64) -> Result<TimelineHandle> {
        alive(&self.records())?;
        let mut kind = vk::SemaphoreTypeCreateInfo::default()
            .semaphore_type(vk::SemaphoreType::TIMELINE)
            .initial_value(initial);
        let info = vk::SemaphoreCreateInfo::default().push_next(&mut kind);
        let raw = &self.shared.raw;
        // SAFETY: the device is live and the info valid.
        let semaphore = unsafe { raw.device.create_semaphore(&info, None) }
            .map_err(|e| driver(e, "vkCreateSemaphore"))?;
        let object = TimelineObj { raw: Arc::clone(raw), semaphore };
        let handle = {
            let mut records = self.records();
            alive(&records)?;
            records.timelines.insert(TimelineRec { known: initial, mark: None })?
        };
        self.shared.timelines.insert(handle, Arc::new(object));
        Ok(handle)
    }

    fn destroy_timeline(&self, timeline: TimelineHandle) -> Result<()> {
        let read = self.shared.read_queues();
        let value = self.shared.read_timeline(timeline);
        let mut records = self.records();
        records.fold(&read, value.as_slice());
        records.timeline(timeline)?;
        if !records.lost {
            let named = records.queues.iter().any(|queue| {
                queue.submits.iter().any(|submit| {
                    let waits = submit.seq > queue.known
                        && submit.waits.iter().any(|p| p.timeline == timeline);
                    let known = records.timelines.get(timeline).map_or(0, |rec| rec.known);
                    let promises =
                        submit.signals.iter().any(|p| p.timeline == timeline && p.value > known);
                    waits || promises
                })
            });
            if named {
                return Err(Rule::Pending.refuse(
                    "a timeline destroyed while a pending submit waits for or signals it",
                ));
            }
        }
        records.timelines.remove(timeline);
        drop(records);
        self.shared.timelines.remove(timeline);
        Ok(())
    }

    /// The value the device knows the timeline has reached, after a wait of no time for
    /// each value it is awaited to reach (`Shared::reached`). Every signal of a timeline is
    /// the device's, so no other value can be reached.
    fn timeline_value(&self, timeline: TimelineHandle) -> Result<u64> {
        let (object, known, awaited) = {
            let records = self.records();
            alive(&records)?;
            let known = records.timeline(timeline)?.known;
            let object =
                self.shared.timelines.resolve(timeline).ok_or_else(|| stale("a timeline"))?;
            (object, known, records.awaited(timeline))
        };
        // SAFETY: the count keeps the semaphore alive through the call.
        let reached = unsafe { self.shared.reached(object.semaphore, &awaited) };
        let reached = reached.map_err(|e| {
            self.shared.lost_on(e);
            driver(e, "vkWaitSemaphores")
        })?;
        let Some(value) = reached else {
            return Ok(known);
        };
        let mut records = self.records();
        if self.shared.timelines.resolve(timeline).is_some_and(|now| Arc::ptr_eq(&now, &object)) {
            records.fold_timeline(timeline, value);
            records.settle();
        }
        Ok(value)
    }

    fn signal_timeline(&self, point: TimelinePoint) -> Result<()> {
        let object = {
            let read = self.shared.read_timeline(point.timeline);
            let mut records = self.records();
            alive(&records)?;
            records.fold(&[None; QUEUES], read.as_slice());
            records.begin_host_signal(point)?;
            self.shared.timelines.resolve(point.timeline).ok_or_else(|| stale("a timeline"))?
        };
        let info =
            vk::SemaphoreSignalInfo::default().semaphore(object.semaphore).value(point.value);
        // SAFETY: the count keeps the semaphore alive; the value grows past every value the
        // timeline has, has been given or been promised, as the first hold checked.
        let signalled = unsafe { self.shared.raw.device.signal_semaphore(&info) };
        if let Err(error) = signalled {
            self.shared.lost_on(error);
        }
        let mut records = self.records();
        if self
            .shared
            .timelines
            .resolve(point.timeline)
            .is_some_and(|now| Arc::ptr_eq(&now, &object))
        {
            records.end_host_signal(point, signalled.is_ok());
        }
        signalled.map_err(|e| driver(e, "vkSignalSemaphore"))
    }

    fn wait_timeline(&self, point: TimelinePoint, timeout: Duration) -> Result<bool> {
        let object = {
            let records = self.records();
            records.timeline(point.timeline)?;
            alive(&records)?;
            self.shared.timelines.resolve(point.timeline).ok_or_else(|| stale("a timeline"))?
        };
        let semaphores = [object.semaphore];
        let values = [point.value];
        let info = vk::SemaphoreWaitInfo::default().semaphores(&semaphores).values(&values);
        let nanos = u64::try_from(timeout.as_nanos()).unwrap_or(u64::MAX);
        // SAFETY: the count keeps the semaphore alive through the wait.
        match unsafe { self.shared.raw.device.wait_semaphores(&info, nanos) } {
            Ok(()) => {
                let mut records = self.records();
                if self
                    .shared
                    .timelines
                    .resolve(point.timeline)
                    .is_some_and(|now| Arc::ptr_eq(&now, &object))
                {
                    records.fold_timeline(point.timeline, point.value);
                    records.settle();
                }
                Ok(true)
            }
            Err(vk::Result::TIMEOUT) => Ok(false),
            Err(error) => {
                self.shared.lost_on(error);
                Err(driver(error, "vkWaitSemaphores"))
            }
        }
    }

    fn create_encoder(&self, queue: QueueRole, label: &str) -> Result<Box<dyn Encoder>> {
        alive(&self.records())?;
        let _ = label;
        Ok(Box::new(VulkanEncoder::new(&self.shared, queue)?))
    }

    fn submit(&self, queue: QueueRole, submit: Submit<'_>) -> Result<()> {
        submit::submit(&self.shared, queue, submit)
    }

    fn create_swapchain(&self, desc: &SwapchainDesc) -> Result<Box<dyn Swapchain>> {
        Ok(Box::new(VulkanSwapchain::new(&self.shared, desc.extent)?))
    }
}

impl VulkanDevice {
    /// The first hold of a host access: its checks, and for a write or a read its range
    /// held, which a [`Lend`] clears. A flush or an invalidate holds none.
    fn host_access(
        &self,
        buffer: BufferHandle,
        offset: u64,
        len: u64,
        access: Access,
    ) -> Result<(Arc<BufferObj>, u64)> {
        let mut records = self.shared.settled();
        alive(&records)?;
        let rec = records.buffers.live(buffer, "a buffer")?;
        let (class, what) = match access {
            Access::Write => (MemoryClass::Upload, "a host write"),
            Access::Read => (MemoryClass::Readback, "a host read"),
            Access::Flush => (MemoryClass::Upload, "a flush"),
            Access::Invalidate => (MemoryClass::Readback, "an invalidate"),
        };
        if rec.memory != class {
            return Err(Rule::Usage.refuse(format!("{what} of a buffer that is not {class:?}")));
        }
        let end = range(offset, len, rec.size, what)?;
        if rec.host.overlaps(offset, end) {
            return Err(Rule::HostOverlap.refuse(format!("{what} over a host access in progress")));
        }
        if access == Access::Write && records.pending(&rec.writers) {
            return Err(Rule::Pending.refuse("a host write of a buffer a pending submit writes"));
        }
        if matches!(access, Access::Read | Access::Invalidate) && records.pending(&rec.changed) {
            return Err(Rule::Pending.refuse(format!(
                "{what} of a buffer a pending submit writes or changes the state of"
            )));
        }
        let object = self.shared.buffers.resolve(buffer).ok_or_else(|| stale("a buffer"))?;
        if matches!(access, Access::Write | Access::Read) {
            records.buffers.live_mut(buffer, "a buffer")?.host.add(offset, end);
        }
        Ok((object, end))
    }
}

/// What a host access is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Access {
    /// `write_buffer`, of an `Upload` buffer.
    Write,
    /// `read_buffer`, of a `Readback` buffer.
    Read,
    /// `flush_buffer`, of an `Upload` buffer.
    Flush,
    /// `invalidate_buffer`, of a `Readback` buffer.
    Invalidate,
}

/// A host lend in progress: its range, removed by the second hold when the lend ends,
/// however the caller's closure ends.
struct Lend<'a> {
    shared: &'a Shared,
    buffer: BufferHandle,
    start: u64,
    end: u64,
}

impl Drop for Lend<'_> {
    fn drop(&mut self) {
        let mut records = self.shared.records();
        // After a destroy the handle resolves to nothing, and the hold changes nothing.
        if let Some(rec) = records.buffers.get_mut(self.buffer) {
            rec.host.remove_one(self.start, self.end);
        }
    }
}
