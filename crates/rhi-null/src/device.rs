//! The device: what every handle resolves to, the records and their locks, and the calls of
//! `Device`.

use core::time::Duration;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::Instant;

use lactose_core::{Error, ErrorKind, Result};
use lactose_rhi_api::{
    BufferDesc, BufferHandle, BufferState, BufferUsage, Capabilities, ComputePipelineDesc,
    ComputePipelineHandle, Device, DeviceInfo, DeviceRequest, Encoder, Extent, Format,
    GraphicsPipelineDesc, GraphicsPipelineHandle, HeapSlot, ImageDesc, ImageHandle, ImageMemory,
    ImageUsage, MemoryBlockDesc, MemoryBlockHandle, MemoryClass, MessageSink, PipelineStats,
    Placement, QueueRole, Rule, SamplerDesc, SamplerIndex, Sharing, Submit, SurfaceSource,
    Swapchain, SwapchainDesc, TimelineHandle, TimelinePoint,
};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};

use crate::encoder::NullEncoder;
use crate::records::{
    BlockRec, BufferRec, ImageRec, PipelineRec, Placed, QUEUES, Records, Sub, TimelineRec,
    queue_index, stale,
};
use crate::swapchain::NullSwapchain;
use crate::table::Table;
use crate::{lock, pipeline, submit};

/// How long `acquire`, `present` and `resize` hold before they give up: about a second, as
/// the contract asks of every backend.
pub(crate) const BOUND: Duration = Duration::from_secs(1);

/// The largest buffer null makes. Its bytes are kept in host memory, made on first use.
const BUFFER_MAX: u64 = 1 << 32;

/// The largest width or height of an image null makes.
const EXTENT_MAX: u32 = 16384;

/// What a placed image's offset is a multiple of, and what its size is rounded up to.
const PLACEMENT_ALIGNMENT: u64 = 256;

/// A buffer as its handle resolves: its description, its address and its bytes.
pub(crate) struct BufferObj {
    pub(crate) size: u64,
    pub(crate) usage: BufferUsage,
    pub(crate) address: u64,
    /// The bytes, made on first use. A leaf lock: taken last, for a copy alone.
    store: Mutex<Vec<u8>>,
}

impl BufferObj {
    fn bytes(&self) -> MutexGuard<'_, Vec<u8>> {
        let mut store = lock(&self.store);
        if store.is_empty() {
            // `size` is at most `BUFFER_MAX`, which a 64-bit host addresses.
            store.resize(usize::try_from(self.size).unwrap_or(usize::MAX), 0);
        }
        store
    }

    /// `len` bytes from `at`, which the caller has bounded by the buffer's size.
    pub(crate) fn read(&self, at: usize, len: usize) -> Vec<u8> {
        self.bytes()[at..at + len].to_vec()
    }

    /// Writes `bytes` from `at`, which the caller has bounded by the buffer's size.
    pub(crate) fn write(&self, at: usize, bytes: &[u8]) {
        self.bytes()[at..at + bytes.len()].copy_from_slice(bytes);
    }
}

/// An image as its handle resolves: what recording checks against.
pub(crate) struct ImageObj {
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
    pub(crate) swapchain: bool,
}

impl ImageObj {
    /// The extent of one mip.
    pub(crate) fn mip_extent(&self, mip: u32) -> Extent {
        Extent {
            width: (self.extent.width >> mip.min(31)).max(1),
            height: (self.extent.height >> mip.min(31)).max(1),
        }
    }
}

/// A graphics pipeline as its handle resolves: what a pass and a draw check against.
pub(crate) struct GraphicsObj {
    pub(crate) colour: Vec<Format>,
    pub(crate) depth: Option<Format>,
    /// Bit `i`: the fragment stage reads input `i`.
    pub(crate) inputs: u32,
    /// Bit `i`: the pipeline writes colour attachment `i`, its write mask not empty.
    pub(crate) writes: u32,
}

/// A heap slot's entry: the image written to it, if any.
pub(crate) type SlotEntry = Option<(ImageHandle, Arc<ImageObj>)>;

/// What a unit test runs between a call's two holds.
#[cfg(test)]
type Hook = Box<dyn FnMut(Between) + Send>;

/// A compute pipeline as its handle resolves: nothing recording checks.
pub(crate) struct ComputeObj;

/// A point between a call's two holds, where a unit test runs another call.
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Between {
    HostSignal,
    /// Between a heap write's first hold and its slot's write. The closure runs under the
    /// heap's lock: it may submit, but may not write the heap, which would wait on itself.
    HeapWrite,
    HostLend,
    /// A present's wait for an undecided point, with no lock held.
    PresentWait,
}

/// What a device, its encoders and its swapchain share.
///
/// The locks, in the one order any call takes them: a queue's (`queues`) or the heap's
/// (`heap`), never both; then the records' (`records`); then one entry's, a handle's in a
/// table or a heap slot's (`slots`); then a buffer's bytes. Recording takes an entry's lock
/// alone. No lock is held across a wait or the caller's closure: a wait on `progress`
/// gives the records' lock up while it waits.
///
/// One test-only exception: a unit test's [`Between::HeapWrite`] closure submits under the
/// heap's lock, so holds the heap's and then a queue's. No cycle can form: nothing takes
/// the heap's lock while it holds a queue's, since a submit and a present write no slot.
pub(crate) struct Shared {
    pub(crate) caps: Capabilities,
    info: DeviceInfo,
    /// Taken by a submit or present to the queue, through its walk of the records, so that
    /// a queue's submits are numbered in the order they are taken.
    pub(crate) queues: [Mutex<()>; QUEUES],
    /// Taken by a heap write, through both its holds and the slot's write between them.
    heap: Mutex<()>,
    pub(crate) records: Mutex<Records>,
    /// Signalled whenever a timeline grows, a submit runs, a present completes or the
    /// device is lost.
    pub(crate) progress: Condvar,
    lost: AtomicBool,
    /// The device objects, encoders and swapchains that hold the device. Counted apart from
    /// the `Arc`'s count, which a call's own clone raises too, so that exactly one drop finds
    /// itself the last.
    holders: AtomicUsize,
    pub(crate) buffers: Table<BufferObj>,
    pub(crate) images: Table<ImageObj>,
    pub(crate) graphics: Table<GraphicsObj>,
    pub(crate) compute: Table<ComputeObj>,
    /// Each heap slot's image, behind the slot's entry lock, which a pass that declares the
    /// slot resolves as it begins.
    pub(crate) slots: Box<[Mutex<SlotEntry>]>,
    /// The window, held for as long as the device is, and read nowhere: null presents to
    /// none.
    _surface: Option<Arc<dyn SurfaceSource>>,
    /// The sink: null has no validator and no driver, and sends nothing.
    _messages: Arc<dyn MessageSink>,
    #[cfg(test)]
    pub(crate) between: Mutex<Option<Hook>>,
}

impl Shared {
    pub(crate) fn records(&self) -> MutexGuard<'_, Records> {
        lock(&self.records)
    }

    /// A new holder of the device: an encoder, the swapchain, or a second device object.
    pub(crate) fn hold(self: &Arc<Self>) -> Arc<Self> {
        // Relaxed: the count decides only which drop is the last, which its RMW alone
        // settles; what that drop waits for is read under the records' lock.
        self.holders.fetch_add(1, Ordering::Relaxed);
        Arc::clone(self)
    }

    /// A holder's drop. The last waits, as the contract's last drop does, until every
    /// submit the device has taken has run or the device is lost; not while the thread
    /// unwinds a panic.
    pub(crate) fn let_go(&self) {
        if self.holders.fetch_sub(1, Ordering::Relaxed) != 1 || std::thread::panicking() {
            return;
        }
        let mut records = self.records();
        while !records.lost && records.queues.iter().any(|queue| queue.ran + 1 < queue.next) {
            records = self.wait(records, None);
        }
    }

    /// Wakes every call that waits on the records.
    pub(crate) fn progressed(&self) {
        self.progress.notify_all();
    }

    /// Waits on the records for at most `until`, or with no limit for `None`; the lock is
    /// given up while it waits.
    pub(crate) fn wait<'a>(
        &self,
        records: MutexGuard<'a, Records>,
        until: Option<Instant>,
    ) -> MutexGuard<'a, Records> {
        match until {
            None => self.progress.wait(records).unwrap_or_else(std::sync::PoisonError::into_inner),
            Some(until) => {
                let left = until.saturating_duration_since(Instant::now());
                self.progress
                    .wait_timeout(records, left)
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .0
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn between(&self, at: Between) {
        let taken = lock(&self.between).take();
        if let Some(mut hook) = taken {
            hook(at);
            *lock(&self.between) = Some(hook);
        }
    }
}

/// The error of every call that reaches a lost device.
pub(crate) fn lost() -> Error {
    Error::new(ErrorKind::DeviceLost, "the null device is lost")
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

/// The null backend's device: every rule of `lactose-rhi-api`'s contract checked against
/// the records it keeps, under the [`Capabilities`] it is created with, and nothing drawn.
///
/// A submit's buffer-to-buffer copies move bytes once its waits are reached; draws,
/// dispatches and copies that involve an image move nothing.
pub struct NullDevice {
    shared: Arc<Shared>,
}

impl NullDevice {
    /// Makes a device that reports `capabilities`, its heap and sampler table of the sizes
    /// `request` asks for. With a surface in `request` it can make a swapchain, whose
    /// images no window shows; [`NullSurface`] is a surface that needs no window.
    ///
    /// # Errors
    ///
    /// `ErrorKind::Unsupported` for a request whose heap or sampler table is larger than
    /// `capabilities` holds, or for `capabilities` below what a device must offer
    /// ([`Capabilities::LEAST`]'s limits); `ErrorKind::NotFound` for an adapter other than
    /// `null`. A pipeline cache that fails a check starts the cache empty and is no error.
    pub fn new(request: DeviceRequest, capabilities: Capabilities) -> Result<Self> {
        let unsupported = |what: &str| Error::new(ErrorKind::Unsupported, what.to_owned());
        if request.heap_slots > capabilities.heap_slots {
            return Err(unsupported("a heap larger than the capability set holds"));
        }
        if request.samplers > capabilities.samplers {
            return Err(unsupported("a sampler table larger than the capability set holds"));
        }
        let least = Capabilities::LEAST;
        let below = capabilities.colour_attachments < least.colour_attachments
            || (0..3).any(|at| capabilities.compute_groups[at] < least.compute_groups[at])
            || (0..2).any(|at| capabilities.viewport_extent[at] < least.viewport_extent[at])
            || capabilities.viewport_bounds[0] > least.viewport_bounds[0]
            || capabilities.viewport_bounds[1] < least.viewport_bounds[1];
        if below {
            return Err(unsupported("a capability set below the least a device offers"));
        }
        if let Some(adapter) = &request.adapter
            && !"null".contains(adapter.as_str())
        {
            return Err(Error::new(
                ErrorKind::NotFound,
                format!("no adapter named like `{adapter}`: null has one, `null`"),
            ));
        }
        if let Some(surface) = &request.surface {
            // Read once, as the contract says a backend does; null presents to none, so
            // what the handles are, or that they are unavailable, changes nothing.
            let _ = surface.window_handle();
            let _ = surface.display_handle();
        }
        let (cache, stages) = pipeline::open(&request.pipeline_cache);
        let caps = Capabilities {
            heap_slots: request.heap_slots,
            samplers: request.samplers,
            ..capabilities
        };
        let shared = Shared {
            caps,
            info: DeviceInfo { backend: "null", adapter: "null".to_owned(), driver: String::new() },
            queues: [Mutex::new(()), Mutex::new(()), Mutex::new(())],
            heap: Mutex::new(()),
            records: Mutex::new(Records::new(caps.heap_slots, caps.samplers, cache, stages)),
            progress: Condvar::new(),
            lost: AtomicBool::new(false),
            holders: AtomicUsize::new(1),
            buffers: Table::new(),
            images: Table::new(),
            graphics: Table::new(),
            compute: Table::new(),
            slots: (0..caps.heap_slots).map(|_| Mutex::new(None)).collect(),
            _surface: request.surface,
            _messages: request.messages,
            #[cfg(test)]
            between: Mutex::new(None),
        };
        Ok(Self { shared: Arc::new(shared) })
    }

    /// What reaches the device's loss and its stall from outside: the hooks a test uses,
    /// which the contract leaves to the backend. It outlives the device without keeping
    /// it, and does nothing once the device is gone.
    #[must_use]
    pub fn control(&self) -> Control {
        Control { shared: Arc::downgrade(&self.shared) }
    }

    #[cfg(test)]
    pub(crate) fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    /// A second handle to the same device, for a unit test's hook to call it with.
    #[cfg(test)]
    pub(crate) fn twin(&self) -> Self {
        Self { shared: self.shared.hold() }
    }

    fn records(&self) -> MutexGuard<'_, Records> {
        self.shared.records()
    }

    /// Writes a heap slot, or empties it, in the two holds the contract names.
    fn heap_write(&self, slot: HeapSlot, image: Option<ImageHandle>) -> Result<()> {
        let shared = &*self.shared;
        let _heap = lock(&shared.heap);
        let index = slot.0 as usize;
        let resolved = {
            let mut records = self.records();
            alive(&records)?;
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
            // The heap's lock, held through both holds, keeps any other write from
            // finding the slot's mark.
            let rec = &records.slots[index];
            if records.pending(&rec.stamps) {
                return Err(Rule::HeapWrite.refuse(format!(
                    "slot {} written while a pass of a pending submit declares it",
                    slot.0
                )));
            }
            records.slots[index].mark = true;
            if let Some(rec) = image.and_then(|image| records.images.get_mut(image)) {
                rec.slots += 1;
            }
            resolved
        };
        #[cfg(test)]
        shared.between(Between::HeapWrite);
        *lock(&shared.slots[index]) = resolved;
        let mut records = self.records();
        let rec = &mut records.slots[index];
        rec.mark = false;
        let old = core::mem::replace(&mut rec.image, image);
        if let Some(rec) = old.and_then(|old| records.images.get_mut(old)) {
            rec.slots -= 1;
        }
        Ok(())
    }
}

impl Drop for NullDevice {
    fn drop(&mut self) {
        // The stall is the device's: a device dropped while stalled is unstalled first, so
        // the swapchain and the encoders it leaves never wait on a hold no one will lift.
        let _unstalled = Control { shared: Arc::downgrade(&self.shared) }.stall(false);
        self.shared.let_go();
    }
}

/// The hooks a test reaches a [`NullDevice`] by: its loss, and its stall of presents.
#[derive(Clone)]
pub struct Control {
    shared: Weak<Shared>,
}

impl Control {
    /// Makes the device lost, as a driver would: every call that reaches it from now on
    /// returns `ErrorKind::DeviceLost`, and every wait returns at once. Returns `false`
    /// when the device is gone.
    #[must_use = "`false` means the device was gone and nothing was lost"]
    pub fn lose(&self) -> bool {
        let Some(shared) = self.shared.upgrade() else { return false };
        let mut records = shared.records();
        records.lost = true;
        // The flag is read by `is_lost` alone, which promises nothing about what else it
        // sees: Relaxed would do, and every call that acts on a loss reads the records'.
        shared.lost.store(true, Ordering::Release);
        drop(records);
        shared.progressed();
        true
    }

    /// With `on`, keeps the device's presents from completing, as a display that has not
    /// taken them would: their submits run, and the driver's read of each image does not
    /// end until this is called again with `on` false. Returns `false` when the device is
    /// gone.
    #[must_use = "`false` means the device was gone and nothing was stalled"]
    pub fn stall(&self, on: bool) -> bool {
        let Some(shared) = self.shared.upgrade() else { return false };
        let mut records = shared.records();
        records.stalled = on;
        records.settle_presents();
        drop(records);
        shared.progressed();
        true
    }
}

/// A surface a null device presents to: no window, and its handles unavailable. A device
/// made with it makes a swapchain; nothing shows the images.
#[derive(Clone, Copy, Default, Debug)]
pub struct NullSurface;

impl HasWindowHandle for NullSurface {
    fn window_handle(&self) -> core::result::Result<WindowHandle<'_>, HandleError> {
        Err(HandleError::Unavailable)
    }
}

impl HasDisplayHandle for NullSurface {
    fn display_handle(&self) -> core::result::Result<DisplayHandle<'_>, HandleError> {
        Err(HandleError::Unavailable)
    }
}

/// Refuses an image description no device honours, and one null cannot make.
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
    if extent.width > EXTENT_MAX || extent.height > EXTENT_MAX {
        return Err(Error::new(
            ErrorKind::Unsupported,
            format!("an image larger than {EXTENT_MAX} texels a side"),
        ));
    }
    Ok(())
}

/// What a block offers an image of `desc`: every mip of every layer, packed, rounded up.
fn placement(desc: &ImageDesc<'_>) -> Placement {
    let texel = u64::from(desc.format.texel_bytes());
    let mips: u64 = (0..desc.mips)
        .map(|mip| {
            let width = u64::from((desc.extent.width >> mip).max(1));
            let height = u64::from((desc.extent.height >> mip).max(1));
            width * height * texel
        })
        .sum();
    let size = (mips * u64::from(desc.layers)).next_multiple_of(PLACEMENT_ALIGNMENT);
    Placement { size, alignment: PLACEMENT_ALIGNMENT }
}

impl Device for NullDevice {
    fn info(&self) -> &DeviceInfo {
        &self.shared.info
    }

    fn capabilities(&self) -> &Capabilities {
        &self.shared.caps
    }

    fn is_lost(&self) -> bool {
        // Pairs with `lose`'s store, though nothing is read through it (see there).
        self.shared.lost.load(Ordering::Acquire)
    }

    fn create_buffer(&self, desc: &BufferDesc<'_>) -> Result<BufferHandle> {
        let mut records = self.records();
        alive(&records)?;
        if desc.size == 0 {
            return Err(Rule::Usage.refuse("a buffer of no bytes"));
        }
        if desc.size > BUFFER_MAX {
            return Err(Error::new(
                ErrorKind::ResourceExhausted,
                format!("a buffer of more than {BUFFER_MAX} bytes"),
            ));
        }
        let address = records.next_address;
        records.next_address +=
            desc.size.next_multiple_of(PLACEMENT_ALIGNMENT) + PLACEMENT_ALIGNMENT;
        let handle = records.buffers.insert(BufferRec {
            size: desc.size,
            memory: desc.memory,
            sharing: desc.sharing,
            sub: Sub::new(BufferState::Undefined),
            stamps: [0; QUEUES],
            writers: [0; QUEUES],
            changed: [0; QUEUES],
            ordered: [0; QUEUES],
            host: Default::default(),
            unflushed: Default::default(),
            uninvalidated: Default::default(),
        })?;
        let object = BufferObj {
            size: desc.size,
            usage: desc.usage,
            address,
            store: Mutex::new(Vec::new()),
        };
        self.shared.buffers.insert(handle, Arc::new(object));
        Ok(handle)
    }

    fn destroy_buffer(&self, buffer: BufferHandle) -> Result<()> {
        let mut records = self.records();
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
        let (object, end) = {
            let mut records = self.records();
            alive(&records)?;
            let rec = records.buffers.live(buffer, "a buffer")?;
            if rec.memory != MemoryClass::Upload {
                return Err(Rule::Usage.refuse("a host write of a buffer that is not Upload"));
            }
            let end = range(offset, len, rec.size, "a host write")?;
            if rec.host.overlaps(offset, end) {
                return Err(Rule::HostOverlap.refuse("a host write over a host access in progress"));
            }
            if records.pending(&rec.writers) {
                return Err(
                    Rule::Pending.refuse("a host write of a buffer a pending submit writes")
                );
            }
            let object = self.shared.buffers.resolve(buffer).ok_or_else(|| stale("a buffer"))?;
            records.buffers.live_mut(buffer, "a buffer")?.host.add(offset, end);
            (object, end)
        };
        let mut lend = Lend { shared: &self.shared, buffer, start: offset, end, wrote: false };
        #[cfg(test)]
        self.shared.between(Between::HostLend);
        let mut bytes = vec![0; usize::try_from(len).unwrap_or(usize::MAX)];
        fill(&mut bytes);
        object.write(usize::try_from(offset).unwrap_or(usize::MAX), &bytes);
        lend.wrote = true;
        Ok(())
    }

    fn read_buffer(
        &self,
        buffer: BufferHandle,
        offset: u64,
        len: u64,
        read: &mut dyn FnMut(&[u8]),
    ) -> Result<()> {
        let (object, end) = {
            let mut records = self.records();
            alive(&records)?;
            let rec = records.buffers.live(buffer, "a buffer")?;
            if rec.memory != MemoryClass::Readback {
                return Err(Rule::Usage.refuse("a host read of a buffer that is not Readback"));
            }
            let end = range(offset, len, rec.size, "a host read")?;
            if rec.host.overlaps(offset, end) {
                return Err(Rule::HostOverlap.refuse("a host read over a host access in progress"));
            }
            if records.pending(&rec.changed) {
                return Err(Rule::Pending.refuse(
                    "a host read of a buffer a pending submit writes or changes the state of",
                ));
            }
            if rec.sub.state != BufferState::HostRead {
                return Err(
                    Rule::State.refuse(format!("a host read of a buffer in {:?}", rec.sub.state))
                );
            }
            if !rec.sub.defined {
                return Err(Rule::UndefinedRead.refuse("a host read of a buffer nothing wrote"));
            }
            if rec.uninvalidated.overlaps(offset, end) {
                return Err(Rule::Unflushed
                    .refuse("a host read of a range the device wrote, with no invalidate since"));
            }
            let object = self.shared.buffers.resolve(buffer).ok_or_else(|| stale("a buffer"))?;
            records.buffers.live_mut(buffer, "a buffer")?.host.add(offset, end);
            (object, end)
        };
        let lend = Lend { shared: &self.shared, buffer, start: offset, end, wrote: false };
        #[cfg(test)]
        self.shared.between(Between::HostLend);
        let at = usize::try_from(offset).unwrap_or(usize::MAX);
        let bytes = object.read(at, usize::try_from(len).unwrap_or(usize::MAX));
        read(&bytes);
        drop(lend);
        Ok(())
    }

    fn flush_buffer(&self, buffer: BufferHandle, offset: u64, len: u64) -> Result<()> {
        let mut records = self.records();
        alive(&records)?;
        let rec = records.buffers.live_mut(buffer, "a buffer")?;
        let end = range(offset, len, rec.size, "a flush")?;
        rec.unflushed.subtract(offset, end);
        Ok(())
    }

    fn invalidate_buffer(&self, buffer: BufferHandle, offset: u64, len: u64) -> Result<()> {
        let mut records = self.records();
        alive(&records)?;
        let rec = records.buffers.live(buffer, "a buffer")?;
        let end = range(offset, len, rec.size, "an invalidate")?;
        // An invalidate made before the writer, or the barrier into `HostRead`, runs makes
        // nothing it writes visible.
        if records.pending(&rec.changed) {
            return Err(Rule::Pending.refuse(
                "an invalidate of a buffer a pending submit writes or changes the state of",
            ));
        }
        let rec = records.buffers.live_mut(buffer, "a buffer")?;
        rec.uninvalidated.subtract(offset, end);
        Ok(())
    }

    fn create_image(&self, desc: &ImageDesc<'_>) -> Result<ImageHandle> {
        let mut records = self.records();
        alive(&records)?;
        check_image(desc)?;
        let caps = &self.shared.caps;
        let usage = desc.usage;
        let memoryless_request = desc.memory == ImageMemory::Memoryless;
        let honoured = (usage.contains(ImageUsage::COLOUR)
            && usage.contains(ImageUsage::INPUT)
            && caps.memoryless_colour)
            || (usage.contains(ImageUsage::DEPTH) && caps.memoryless_depth);
        let placed = match desc.memory {
            ImageMemory::Placed { block, offset } => {
                let Placement { size, alignment } = placement(desc);
                let rec = records.blocks.live(block, "a memory block")?;
                if offset % alignment != 0 {
                    return Err(Rule::Bounds.refuse(format!(
                        "a placement at {offset}, not a multiple of {alignment}"
                    )));
                }
                let end = range(offset, size, rec.size, "a placement")?;
                Some(Placed { block, start: offset, end })
            }
            _ => None,
        };
        let rec = ImageRec::new(desc.mips, desc.layers, desc.sharing, placed, false);
        let handle = records.images.insert(rec)?;
        if let Some(placed) = placed {
            records.blocks.live_mut(placed.block, "a memory block")?.images.push(handle);
        }
        let object = ImageObj {
            format: desc.format,
            extent: desc.extent,
            mips: desc.mips,
            layers: desc.layers,
            usage,
            memoryless_request,
            memoryless: memoryless_request && honoured,
            placed: placed.is_some(),
            swapchain: false,
        };
        self.shared.images.insert(handle, Arc::new(object));
        Ok(handle)
    }

    fn destroy_image(&self, image: ImageHandle) -> Result<()> {
        let mut records = self.records();
        let rec = records.images.live(image, "an image")?;
        if rec.acquired.is_some() {
            return Err(Rule::Swapchain.refuse("a swapchain's image, destroyed but by its resize"));
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
        if let Some(block) = placed.and_then(|placed| records.blocks.get_mut(placed.block)) {
            block.images.retain(|other| *other != image);
        }
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
        check_image(desc)?;
        Ok(placement(desc))
    }

    fn create_memory_block(&self, desc: &MemoryBlockDesc<'_>) -> Result<MemoryBlockHandle> {
        let mut records = self.records();
        alive(&records)?;
        if desc.size == 0 {
            return Err(Rule::Usage.refuse("a memory block of no bytes"));
        }
        records.blocks.insert(BlockRec { size: desc.size, images: Vec::new() })
    }

    fn destroy_memory_block(&self, block: MemoryBlockHandle) -> Result<()> {
        let mut records = self.records();
        let rec = records.blocks.live(block, "a memory block")?;
        if !records.lost && !rec.images.is_empty() {
            return Err(
                Rule::Pending.refuse("a memory block destroyed while an image is placed in it")
            );
        }
        records.blocks.remove(block);
        Ok(())
    }

    fn write_heap_slot(&self, slot: HeapSlot, image: ImageHandle) -> Result<()> {
        self.heap_write(slot, Some(image))
    }

    fn clear_heap_slot(&self, slot: HeapSlot) -> Result<()> {
        self.heap_write(slot, None)
    }

    fn create_sampler(&self, desc: &SamplerDesc) -> Result<SamplerIndex> {
        let mut records = self.records();
        alive(&records)?;
        let Some(index) = records.samplers.iter().position(Option::is_none) else {
            return Err(Error::new(ErrorKind::ResourceExhausted, "the sampler table is full"));
        };
        records.samplers[index] = Some(*desc);
        Ok(SamplerIndex(u32::try_from(index).unwrap_or(u32::MAX)))
    }

    fn destroy_sampler(&self, sampler: SamplerIndex) -> Result<()> {
        let mut records = self.records();
        let index = sampler.0 as usize;
        if records.samplers.get(index).is_none_or(Option::is_none) {
            return Err(Rule::Destroyed.refuse(format!("sampler {} is not live", sampler.0)));
        }
        if !records.lost && records.any_pending() {
            return Err(Rule::Pending.refuse("a sampler destroyed while a submit is pending"));
        }
        records.samplers[index] = None;
        Ok(())
    }

    fn create_graphics_pipeline(
        &self,
        desc: &GraphicsPipelineDesc<'_>,
    ) -> Result<GraphicsPipelineHandle> {
        alive(&self.records())?;
        let start = Instant::now();
        let (object, stages) = pipeline::graphics(&self.shared.caps, desc)?;
        let mut records = self.records();
        alive(&records)?;
        pipeline::count(&mut records.pipelines, &stages, start);
        let handle = records.graphics.insert(PipelineRec::default())?;
        self.shared.graphics.insert(handle, Arc::new(object));
        Ok(handle)
    }

    fn create_compute_pipeline(
        &self,
        desc: &ComputePipelineDesc<'_>,
    ) -> Result<ComputePipelineHandle> {
        alive(&self.records())?;
        let start = Instant::now();
        let (object, stages) = pipeline::compute(&self.shared.caps, desc)?;
        let mut records = self.records();
        alive(&records)?;
        pipeline::count(&mut records.pipelines, &stages, start);
        let handle = records.compute.insert(PipelineRec::default())?;
        self.shared.compute.insert(handle, Arc::new(object));
        Ok(handle)
    }

    fn destroy_graphics_pipeline(&self, pipeline: GraphicsPipelineHandle) -> Result<()> {
        let mut records = self.records();
        let rec = records.graphics.live(pipeline, "a graphics pipeline")?;
        if !records.lost && records.pending(&rec.stamps) {
            return Err(Rule::Pending.refuse("a pipeline destroyed under a pending submit"));
        }
        records.graphics.remove(pipeline);
        self.shared.graphics.remove(pipeline);
        Ok(())
    }

    fn destroy_compute_pipeline(&self, pipeline: ComputePipelineHandle) -> Result<()> {
        let mut records = self.records();
        let rec = records.compute.live(pipeline, "a compute pipeline")?;
        if !records.lost && records.pending(&rec.stamps) {
            return Err(Rule::Pending.refuse("a pipeline destroyed under a pending submit"));
        }
        records.compute.remove(pipeline);
        self.shared.compute.remove(pipeline);
        Ok(())
    }

    fn pipeline_cache(&self) -> Result<Vec<u8>> {
        let records = self.records();
        alive(&records)?;
        pipeline::seal(&records.pipelines.stages)
    }

    fn pipeline_stats(&self) -> PipelineStats {
        self.records().pipelines.stats
    }

    fn create_timeline(&self, initial: u64) -> Result<TimelineHandle> {
        let mut records = self.records();
        alive(&records)?;
        records.timelines.insert(TimelineRec { value: initial, mark: None })
    }

    fn destroy_timeline(&self, timeline: TimelineHandle) -> Result<()> {
        let mut records = self.records();
        records.timeline(timeline)?;
        if !records.lost {
            let named = records.queues.iter().flat_map(|queue| &queue.submits).any(|submit| {
                submit.waits.iter().chain(&submit.signals).any(|p| p.timeline == timeline)
            });
            if named {
                return Err(Rule::Pending.refuse(
                    "a timeline destroyed while a pending submit waits for or signals it",
                ));
            }
        }
        records.timelines.remove(timeline);
        drop(records);
        // A present waiting on it walks again, and is refused.
        self.shared.progressed();
        Ok(())
    }

    fn timeline_value(&self, timeline: TimelineHandle) -> Result<u64> {
        let mut records = self.records();
        alive(&records)?;
        let value = records.timeline(timeline)?.value;
        records.observe(TimelinePoint { timeline, value });
        Ok(value)
    }

    fn signal_timeline(&self, point: TimelinePoint) -> Result<()> {
        {
            let mut records = self.records();
            alive(&records)?;
            records.begin_host_signal(point)?;
        }
        #[cfg(test)]
        self.shared.between(Between::HostSignal);
        let mut records = self.records();
        records.end_host_signal(point);
        records.settle_presents();
        drop(records);
        self.shared.progressed();
        Ok(())
    }

    fn wait_timeline(&self, point: TimelinePoint, timeout: Duration) -> Result<bool> {
        let until = Instant::now().checked_add(timeout);
        let mut records = self.records();
        loop {
            let reached = records.timeline(point.timeline)?.value >= point.value;
            if reached {
                records.observe(point);
                return Ok(true);
            }
            alive(&records)?;
            if until.is_some_and(|until| Instant::now() >= until) {
                return Ok(false);
            }
            records = self.shared.wait(records, until);
        }
    }

    fn create_encoder(&self, queue: QueueRole, _label: &str) -> Result<Box<dyn Encoder>> {
        alive(&self.records())?;
        Ok(Box::new(NullEncoder::new(self.shared.hold(), queue)))
    }

    fn submit(&self, queue: QueueRole, submit: Submit<'_>) -> Result<()> {
        if queue == QueueRole::Present {
            alive(&self.records())?;
            return Err(Rule::PresentQueue.refuse(
                "a submit to the present role, which is reached through the swapchain alone",
            ));
        }
        let shared = &*self.shared;
        let index = queue_index(shared.caps.queues.serving(queue));
        let _queue = lock(&shared.queues[index]);
        let mut records = self.records();
        alive(&records)?;
        let mut submit = submit;
        let plan = submit::check(shared, &records, queue, &submit, None)?;
        submit::commit(&mut records, plan, &mut submit);
        drop(records);
        shared.progressed();
        Ok(())
    }

    fn create_swapchain(&self, desc: &SwapchainDesc) -> Result<Box<dyn Swapchain>> {
        let mut records = self.records();
        alive(&records)?;
        if self.shared._surface.is_none() {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "a swapchain of a device made with no surface",
            ));
        }
        let swapchain = NullSwapchain::new(&self.shared, &mut records, desc.extent)?;
        Ok(Box::new(swapchain))
    }
}

/// A host lend in progress: its range, removed by the second hold when the lend ends,
/// however the caller's closure ends.
struct Lend<'a> {
    shared: &'a Shared,
    buffer: BufferHandle,
    start: u64,
    end: u64,
    /// The closure returned from a write: its bytes are the buffer's.
    wrote: bool,
}

impl Drop for Lend<'_> {
    fn drop(&mut self) {
        let coherent = self.shared.caps.coherent_host_memory;
        let mut records = self.shared.records();
        // After a destroy the handle resolves to nothing, and the hold changes nothing.
        let Some(rec) = records.buffers.get_mut(self.buffer) else { return };
        rec.host.remove_one(self.start, self.end);
        if self.wrote {
            rec.sub.defined = true;
            if !coherent {
                rec.unflushed.add(self.start, self.end);
            }
        }
    }
}

/// Whether `sharing` gives a resource an owner.
pub(crate) fn exclusive(sharing: Sharing) -> bool {
    sharing == Sharing::Exclusive
}
