//! The encoder: a command pool and one primary command buffer on its queue's family,
//! recorded as the caller calls, after every check of the contract's "Every backend"
//! column a command can be refused by while recording. Recording takes no lock but one
//! table entry's, once for each handle between resets. The first breach stops the
//! recording; the submit refuses the encoder and the command buffer never reaches the
//! queue.

use core::ops::Range;
use std::collections::HashMap;
use std::sync::Arc;

use ash::vk;
use lactose_core::{Error, Result};
use lactose_rhi_api::{
    BufferCopy, BufferHandle, BufferImageCopy, BufferReach, BufferState, BufferTransition,
    BufferUsage, ComputePass, ComputePipelineHandle, Encoder, Extent, Format,
    GraphicsPipelineHandle, HeapSlot, ImageHandle, ImageState, ImageTransition, ImageUsage,
    IndexedIndirectArgs, Load, PUSH_BYTES_MAX, PassReach, QueueRole, Rect, RenderPass, Rule, Store,
    Target, Viewport,
};

use crate::convert::{self, Change, Side, aspect, buffer_scope, image_scope};
use crate::device::{BufferObj, ComputeObj, GraphicsObj, ImageObj, Shared, range};
use crate::heap::GRAPHICS_PUSH;
use crate::lock;
use crate::raw::driver;

/// What a submit of the encoder does to a buffer it names.
#[derive(Clone, Copy, Default)]
pub(crate) struct Touch {
    /// A copy into it, or a pass that declares it `ShaderWrite`.
    pub(crate) written: bool,
    /// A transition of it.
    pub(crate) changed: bool,
}

/// A heap slot a pass declares, and the image it resolved to as the pass began.
pub(crate) type SlotDecl = (HeapSlot, Option<ImageHandle>);

/// What a render pass being recorded holds.
struct RenderScope {
    extent: Extent,
    formats: Vec<Format>,
    depth: Option<Format>,
    /// Bit `i`: colour attachment `i` is marked for the local read.
    marked: u32,
    slots: Vec<SlotDecl>,
    buffers: Vec<BufferHandle>,
    pipeline: Option<Arc<GraphicsObj>>,
    /// The bound index buffer, its offset and size.
    index: Option<(BufferHandle, u64, u64)>,
}

/// What a compute pass being recorded holds.
struct ComputeScope {
    slots: Vec<SlotDecl>,
    buffers: Vec<BufferHandle>,
    pipeline: bool,
    /// The pass declares a buffer `ShaderWrite`, so each dispatch after the first waits
    /// for the writes of those before it.
    writes: bool,
    dispatched: bool,
}

enum Scope {
    Outside,
    Render(Box<RenderScope>),
    Compute(ComputeScope),
}

/// The sets one input pool holds; each holds an input attachment for each of them.
const INPUT_SETS: u32 = 16;

/// A Vulkan encoder. `Send`: its pool and command buffer are used by the thread that has
/// it alone, and what it holds is counted and owned.
pub(crate) struct VulkanEncoder {
    pub(crate) shared: Arc<Shared>,
    pub(crate) role: QueueRole,
    side: Side,
    pool: vk::CommandPool,
    pub(crate) command_buffer: vk::CommandBuffer,
    /// The pools of the input attachment sets of its render passes, and how many sets the
    /// last has given.
    inputs: Vec<vk::DescriptorPool>,
    inputs_given: u32,
    /// The first breach found while recording; nothing is recorded after it.
    pub(crate) breach: Option<(Rule, String)>,
    /// A driver failure while recording, which the submit returns.
    pub(crate) failure: Option<Error>,
    /// The queue and stamp of the submit that took it, until its reset.
    pub(crate) submitted: Option<(usize, u64)>,
    scope: Scope,
    pub(crate) buffers: HashMap<BufferHandle, (Arc<BufferObj>, Touch)>,
    pub(crate) images: HashMap<ImageHandle, Arc<ImageObj>>,
    pub(crate) graphics: HashMap<GraphicsPipelineHandle, Arc<GraphicsObj>>,
    pub(crate) compute: HashMap<ComputePipelineHandle, Arc<ComputeObj>>,
    /// Every slot a pass declared, with the image it resolved to.
    pub(crate) slots: Vec<SlotDecl>,
    /// The pass's push-constant block, and the range written since the last draw or
    /// dispatch.
    push: [u8; PUSH_BYTES_MAX as usize],
    dirty: Option<(u32, u32)>,
}

impl VulkanEncoder {
    /// Makes an encoder for `role`, recording, holding the device.
    pub(crate) fn new(shared: &Arc<Shared>, role: QueueRole) -> Result<Self> {
        let queue = shared.queue_of(role);
        let side = shared.side(queue);
        let device = &shared.raw.device;
        let info = vk::CommandPoolCreateInfo::default()
            .flags(vk::CommandPoolCreateFlags::TRANSIENT)
            .queue_family_index(side.family);
        // SAFETY: the device is live.
        let pool = unsafe { device.create_command_pool(&info, None) }
            .map_err(|e| driver(e, "vkCreateCommandPool"))?;
        let allocate = vk::CommandBufferAllocateInfo::default()
            .command_pool(pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // SAFETY: the pool is live and this thread's alone.
        let made = unsafe { device.allocate_command_buffers(&allocate) };
        let command_buffer = match made {
            Ok(buffers) => buffers[0],
            Err(error) => {
                // SAFETY: nothing was allocated from the pool.
                unsafe { device.destroy_command_pool(pool, None) };
                return Err(driver(error, "vkAllocateCommandBuffers"));
            }
        };
        let mut encoder = Self {
            shared: shared.hold(),
            role,
            side,
            pool,
            command_buffer,
            inputs: Vec::new(),
            inputs_given: 0,
            breach: None,
            failure: None,
            submitted: None,
            scope: Scope::Outside,
            buffers: HashMap::new(),
            images: HashMap::new(),
            graphics: HashMap::new(),
            compute: HashMap::new(),
            slots: Vec::new(),
            push: [0; PUSH_BYTES_MAX as usize],
            dirty: None,
        };
        encoder.begin()?;
        Ok(encoder)
    }

    fn device(&self) -> &ash::Device {
        &self.shared.raw.device
    }

    /// Begins the command buffer, reset or new.
    fn begin(&mut self) -> Result<()> {
        let info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        // SAFETY: the command buffer is in the initial state, and this encoder's alone.
        unsafe { self.device().begin_command_buffer(self.command_buffer, &info) }
            .map_err(|e| driver(e, "vkBeginCommandBuffer"))
    }

    /// Whether a recording call records: the encoder is not taken by a submit and has
    /// found no breach.
    const fn recording(&self) -> bool {
        self.submitted.is_none() && self.breach.is_none() && self.failure.is_none()
    }

    /// Whether a pass is left open.
    pub(crate) const fn open(&self) -> bool {
        !matches!(self.scope, Scope::Outside)
    }

    fn fail(&mut self, rule: Rule, detail: impl Into<String>) {
        if self.breach.is_none() {
            self.breach = Some((rule, detail.into()));
        }
    }

    /// Whether the encoder may record `what`, which the graphics role alone takes: the
    /// compute role's queue may be of a family that does not draw, so a `Compute` encoder
    /// records none on any layout (`lactose-rhi-api`, "Queues by role").
    fn draws(&mut self, what: &str) -> bool {
        if self.role == QueueRole::Compute {
            self.fail(Rule::Encoding, format!("{what} on a compute encoder"));
            return false;
        }
        true
    }

    fn outside(&mut self, what: &str) -> bool {
        if self.open() {
            self.fail(Rule::Encoding, format!("{what} inside a pass"));
            return false;
        }
        true
    }

    fn buffer(&mut self, handle: BufferHandle) -> Option<Arc<BufferObj>> {
        if let Some((object, _)) = self.buffers.get(&handle) {
            return Some(Arc::clone(object));
        }
        let Some(object) = self.shared.buffers.resolve(handle) else {
            self.fail(Rule::Destroyed, "a buffer that is destroyed or of another device");
            return None;
        };
        self.buffers.insert(handle, (Arc::clone(&object), Touch::default()));
        Some(object)
    }

    fn touch(&mut self, handle: BufferHandle, written: bool, changed: bool) {
        if let Some((_, touch)) = self.buffers.get_mut(&handle) {
            touch.written |= written;
            touch.changed |= changed;
        }
    }

    fn image(&mut self, handle: ImageHandle) -> Option<Arc<ImageObj>> {
        if let Some(object) = self.images.get(&handle) {
            return Some(Arc::clone(object));
        }
        let Some(object) = self.shared.images.resolve(handle) else {
            self.fail(Rule::Destroyed, "an image that is destroyed or of another device");
            return None;
        };
        self.images.insert(handle, Arc::clone(&object));
        Some(object)
    }

    /// A buffer with every flag of `usage`, or the breach.
    fn buffer_for(&mut self, handle: BufferHandle, usage: BufferUsage) -> Option<Arc<BufferObj>> {
        let object = self.buffer(handle)?;
        if !object.usage.contains(usage) {
            self.fail(Rule::Usage, format!("a buffer used outside its usage, {usage:?}"));
            return None;
        }
        Some(object)
    }

    fn image_for(&mut self, handle: ImageHandle, usage: ImageUsage) -> Option<Arc<ImageObj>> {
        let object = self.image(handle)?;
        if !object.usage.contains(usage) {
            self.fail(Rule::Usage, format!("an image used outside its usage, {usage:?}"));
            return None;
        }
        Some(object)
    }

    fn bounded(&mut self, end: Result<u64>) -> bool {
        match end {
            Ok(_) => true,
            Err(error) => {
                self.fail(Rule::Bounds, error.to_string());
                false
            }
        }
    }

    /// Resolves the slots a pass declares, through each slot's entry lock.
    fn declare_slots(&mut self, slots: &[HeapSlot]) -> Option<Vec<SlotDecl>> {
        let mut slots = slots.to_vec();
        slots.sort_unstable();
        slots.dedup();
        let mut resolved = Vec::with_capacity(slots.len());
        for slot in slots {
            let Some(entry) = self.shared.slots.get(slot.0 as usize) else {
                self.fail(Rule::Bounds, format!("a declared slot {} outside the heap", slot.0));
                return None;
            };
            let entry = lock(entry).clone();
            let image = entry.map(|(handle, object)| {
                self.images.entry(handle).or_insert(object);
                handle
            });
            resolved.push((slot, image));
        }
        Some(resolved)
    }

    /// Resolves a pass's reach: its slots, and its buffers, each in one state.
    fn reach(&mut self, reach: &PassReach<'_>) -> Option<(Vec<SlotDecl>, Vec<BufferHandle>, bool)> {
        let slots = self.declare_slots(reach.slots)?;
        let mut writes = false;
        for declared in reach.buffers {
            self.buffer_for(declared.buffer, BufferUsage::DEVICE_ADDRESS)?;
            if !matches!(declared.state, BufferState::ShaderRead | BufferState::ShaderWrite) {
                self.fail(Rule::Usage, "a buffer declared in a state no shader reaches it in");
                return None;
            }
        }
        let mut buffers: Vec<BufferReach> = reach.buffers.to_vec();
        buffers.sort_unstable_by_key(|reach| (reach.buffer.index(), reach.buffer.generation()));
        if buffers
            .windows(2)
            .any(|two| two[0].buffer == two[1].buffer && two[0].state != two[1].state)
        {
            self.fail(Rule::State, "a pass that declares one buffer in two states");
            return None;
        }
        buffers.dedup_by_key(|reach| reach.buffer);
        for declared in &buffers {
            let write = declared.state == BufferState::ShaderWrite;
            writes |= write;
            self.touch(declared.buffer, write, false);
        }
        self.slots.extend_from_slice(&slots);
        Some((slots, buffers.iter().map(|reach| reach.buffer).collect(), writes))
    }

    /// The declarations of the open pass, if one is open.
    fn declared(&self) -> Option<(&[SlotDecl], &[BufferHandle])> {
        match &self.scope {
            Scope::Render(pass) => Some((&pass.slots, &pass.buffers)),
            Scope::Compute(pass) => Some((&pass.slots, &pass.buffers)),
            Scope::Outside => None,
        }
    }

    fn attachment(
        &mut self,
        target: Target,
        usage: ImageUsage,
        memoryless_store: bool,
        seen: &mut Vec<ImageHandle>,
        extent: &mut Option<Extent>,
    ) -> Option<Arc<ImageObj>> {
        let object = self.image_for(target.image, usage)?;
        if target.mip >= object.mips || target.layer >= object.layers {
            self.fail(Rule::Bounds, "an attachment of a mip or layer the image does not have");
            return None;
        }
        if seen.contains(&target.image) {
            self.fail(Rule::Usage, "an image named twice among a pass's attachments");
            return None;
        }
        seen.push(target.image);
        let size = object.mip_extent(target.mip);
        if extent.is_some_and(|extent| extent != size) {
            self.fail(Rule::Usage, "attachments of unequal extent");
            return None;
        }
        *extent = Some(size);
        if object.memoryless_request && memoryless_store {
            self.fail(
                Rule::MemorylessStore,
                "an attachment made with a memoryless request, stored or kept",
            );
            return None;
        }
        Some(object)
    }

    /// Writes into the push-constant block, or the breach of a write past its end.
    fn push(&mut self, offset: u32, bytes: &[u8]) {
        let end = u64::from(offset) + bytes.len() as u64;
        if end > u64::from(PUSH_BYTES_MAX) {
            self.fail(
                Rule::Bounds,
                format!("a push of {} bytes at {offset}, past {PUSH_BYTES_MAX}", bytes.len()),
            );
            return;
        }
        if bytes.is_empty() {
            return;
        }
        let end = end as u32;
        self.push[offset as usize..end as usize].copy_from_slice(bytes);
        let (start, stop) =
            self.dirty.map_or((offset, end), |(start, stop)| (start.min(offset), stop.max(end)));
        self.dirty = Some((start, stop));
    }

    /// Records the block's bytes written since the last draw or dispatch, rounded out to
    /// whole words as Vulkan's push takes them.
    fn flush_push(&mut self, layout: vk::PipelineLayout, stages: vk::ShaderStageFlags) {
        let Some((start, end)) = self.dirty.take() else { return };
        let start = start / 4 * 4;
        let end = end.div_ceil(4) * 4;
        // SAFETY: recording on this encoder's thread; the range is inside the layout's push
        // range, which is the whole block for the stages given.
        unsafe {
            self.device().cmd_push_constants(
                self.command_buffer,
                layout,
                stages,
                start,
                &self.push[start as usize..end as usize],
            );
        }
    }

    /// A pass's start: the heap bound and the block zeroed, in full at the first draw.
    fn start_pass(&mut self, bind: vk::PipelineBindPoint, layout: vk::PipelineLayout) {
        let sets = [self.shared.heap.set];
        // SAFETY: recording; the heap's set lives as long as the device, which the encoder
        // holds, and is updated after bind only in entries no pending submit reads.
        unsafe {
            self.device().cmd_bind_descriptor_sets(self.command_buffer, bind, layout, 1, &sets, &[])
        };
        self.push = [0; PUSH_BYTES_MAX as usize];
        self.dirty = Some((0, PUSH_BYTES_MAX));
    }

    /// A set of the input attachments of a render pass, from the encoder's pools.
    fn input_set(&mut self) -> Result<vk::DescriptorSet> {
        let device = self.shared.raw.device.clone();
        let count = self.shared.heap.input_count.max(1);
        if self.inputs.is_empty() || self.inputs_given == INPUT_SETS {
            let sizes = [vk::DescriptorPoolSize {
                ty: vk::DescriptorType::INPUT_ATTACHMENT,
                descriptor_count: INPUT_SETS * count,
            }];
            let info =
                vk::DescriptorPoolCreateInfo::default().max_sets(INPUT_SETS).pool_sizes(&sizes);
            // SAFETY: the device is live.
            let pool = unsafe { device.create_descriptor_pool(&info, None) }
                .map_err(|e| driver(e, "vkCreateDescriptorPool"))?;
            self.inputs.push(pool);
            self.inputs_given = 0;
        }
        let pool = self.inputs.last().copied().unwrap_or_default();
        let layouts = [self.shared.heap.inputs];
        let info =
            vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(&layouts);
        // SAFETY: the pool is this encoder's alone and has a set left.
        let set = unsafe { device.allocate_descriptor_sets(&info) }
            .map_err(|e| driver(e, "vkAllocateDescriptorSets"))?[0];
        self.inputs_given += 1;
        Ok(set)
    }

    /// Records a barrier for each change, in one call.
    fn barriers(
        &self,
        images: &[vk::ImageMemoryBarrier2<'_>],
        buffers: &[vk::BufferMemoryBarrier2<'_>],
    ) {
        if images.is_empty() && buffers.is_empty() {
            return;
        }
        let info = vk::DependencyInfo::default()
            .image_memory_barriers(images)
            .buffer_memory_barriers(buffers);
        // SAFETY: recording outside a pass; every resource named is held by the encoder.
        unsafe { self.device().cmd_pipeline_barrier2(self.command_buffer, &info) };
    }

    /// The change a transition between two states on two roles is.
    fn change(
        &self,
        from: convert::Scope,
        to: convert::Scope,
        roles: (QueueRole, QueueRole),
        undefined: bool,
        exclusive: bool,
    ) -> Change {
        Change {
            from,
            to,
            from_undefined: undefined,
            from_queue: self.shared.side(self.shared.queue_of(roles.0)),
            to_queue: self.shared.side(self.shared.queue_of(roles.1)),
            exclusive,
        }
    }

    /// The checks a draw makes of its render pass, then the push-constant bytes it sees.
    fn draw_checks(&mut self, indexed: bool) -> bool {
        if !self.draws("a draw") {
            return false;
        }
        let checked = match &self.scope {
            Scope::Render(pass) => draw_checks(pass, indexed),
            _ => Err((Rule::Encoding, "a draw outside a render pass".to_owned())),
        };
        if let Err((rule, detail)) = checked {
            self.fail(rule, detail);
            return false;
        }
        self.flush_push(self.shared.heap.graphics, GRAPHICS_PUSH);
        true
    }

    /// The checks a dispatch makes of its compute pass, then the barrier between its
    /// writes and those of the dispatches before it, and the push-constant bytes.
    fn dispatch_checks(&mut self) -> bool {
        let checked = match &self.scope {
            Scope::Compute(pass) if !pass.pipeline => {
                Err((Rule::Encoding, "a dispatch with no compute pipeline bound".to_owned()))
            }
            Scope::Compute(pass) => empty_slot(&pass.slots, "a dispatch"),
            _ => Err((Rule::Encoding, "a dispatch outside a compute pass".to_owned())),
        };
        if let Err((rule, detail)) = checked {
            self.fail(rule, detail);
            return false;
        }
        let Scope::Compute(pass) = &mut self.scope else { return false };
        let ordered = pass.writes && pass.dispatched;
        pass.dispatched = true;
        if ordered {
            let barrier = [vk::MemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
                .src_access_mask(vk::AccessFlags2::SHADER_STORAGE_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
                .dst_access_mask(
                    vk::AccessFlags2::SHADER_STORAGE_READ | vk::AccessFlags2::SHADER_STORAGE_WRITE,
                )];
            let info = vk::DependencyInfo::default().memory_barriers(&barrier);
            // SAFETY: recording outside a render pass.
            unsafe { self.device().cmd_pipeline_barrier2(self.command_buffer, &info) };
        }
        self.flush_push(self.shared.heap.compute, vk::ShaderStageFlags::COMPUTE);
        true
    }

    /// Checks an indirect draw's offset, stride and the records it spans.
    fn records_inside(
        &mut self,
        offset: u64,
        count: u32,
        stride: u32,
        strided: bool,
        size: u64,
    ) -> bool {
        let record = u64::from(IndexedIndirectArgs::BYTES);
        if !offset.is_multiple_of(4)
            || (strided && (!stride.is_multiple_of(4) || stride < IndexedIndirectArgs::BYTES))
        {
            self.fail(Rule::Bounds, "indirect arguments at an unaligned offset or stride");
            return false;
        }
        let span = u64::from(count - 1) * u64::from(stride) + record;
        self.bounded(range(offset, span, size, "indirect records"))
    }

    /// Checks the regions of a copy between a buffer and an image, and returns them as the
    /// driver takes them with the buffer ranges they name.
    fn image_regions(
        &mut self,
        buffer: &BufferObj,
        image: &ImageObj,
        regions: &[BufferImageCopy],
    ) -> Option<(Vec<vk::BufferImageCopy>, Vec<Span>)> {
        let mut copies = Vec::with_capacity(regions.len());
        let mut ranges = Vec::with_capacity(regions.len());
        for region in regions {
            let rect = region.rect;
            if region.mip >= image.mips || region.layer >= image.layers {
                self.fail(Rule::Bounds, "a copy of a mip or layer the image does not have");
                return None;
            }
            let extent = image.mip_extent(region.mip);
            let inside = rect.extent.width > 0
                && rect.extent.height > 0
                && rect.x.checked_add(rect.extent.width).is_some_and(|end| end <= extent.width)
                && rect.y.checked_add(rect.extent.height).is_some_and(|end| end <= extent.height);
            if !inside {
                self.fail(Rule::Bounds, "a copy's rectangle outside its mip");
                return None;
            }
            let texel = u64::from(image.format.texel_bytes());
            let len = u64::from(rect.extent.width) * u64::from(rect.extent.height) * texel;
            // VUIDs 07975 and 07978: a multiple of the texel, which is 4 bytes of a depth
            // format too.
            if !region.buffer_offset.is_multiple_of(texel) {
                self.fail(Rule::Bounds, "a copy's buffer offset not a multiple of its texel");
                return None;
            }
            let Ok(end) = range(region.buffer_offset, len, buffer.size, "a copy's buffer side")
            else {
                self.fail(Rule::Bounds, "a copy's rows outside its buffer");
                return None;
            };
            copies.push(
                vk::BufferImageCopy::default()
                    .buffer_offset(region.buffer_offset)
                    .image_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(aspect(image.format))
                            .mip_level(region.mip)
                            .base_array_layer(region.layer)
                            .layer_count(1),
                    )
                    .image_offset(vk::Offset3D { x: rect.x as i32, y: rect.y as i32, z: 0 })
                    .image_extent(vk::Extent3D {
                        width: rect.extent.width,
                        height: rect.extent.height,
                        depth: 1,
                    }),
            );
            ranges.push((region.buffer_offset, end));
        }
        Some((copies, ranges))
    }

    /// Returns the encoder's command memory and input sets to their pools.
    fn release(&mut self) -> Result<()> {
        let device = self.shared.raw.device.clone();
        // SAFETY: no submit of the command buffer is pending: the caller checked it, or the
        // device is lost and its work will not run.
        unsafe { device.reset_command_pool(self.pool, vk::CommandPoolResetFlags::empty()) }
            .map_err(|e| driver(e, "vkResetCommandPool"))?;
        for pool in &self.inputs {
            // SAFETY: as above, for the sets the command buffer bound.
            unsafe { device.reset_descriptor_pool(*pool, vk::DescriptorPoolResetFlags::empty()) }
                .map_err(|e| driver(e, "vkResetDescriptorPool"))?;
        }
        self.inputs_given = 0;
        if self.inputs.len() > 1 {
            for pool in self.inputs.drain(1..) {
                // SAFETY: as above.
                unsafe { device.destroy_descriptor_pool(pool, None) };
            }
        }
        Ok(())
    }
}

impl Drop for VulkanEncoder {
    fn drop(&mut self) {
        let raw = Arc::clone(&self.shared.raw);
        let mut pending = false;
        if let Some((queue, seq)) = self.submitted {
            let read = self.shared.read_queue(queue);
            let (lost, known) = {
                let records = self.shared.records();
                (records.lost, records.queues[queue].known)
            };
            pending = !lost && read.unwrap_or(0).max(known) < seq;
            if pending && !std::thread::panicking() {
                let semaphores = [self.shared.queue(queue).timeline];
                let values = [seq];
                let info = vk::SemaphoreWaitInfo::default().semaphores(&semaphores).values(&values);
                // SAFETY: the queue's semaphore lives as long as the device, which the
                // encoder holds.
                let waited = unsafe { raw.device.wait_semaphores(&info, u64::MAX) };
                if let Err(error) = waited {
                    self.shared.lost_on(error);
                }
                pending = waited.is_err() && !self.shared.records().lost;
            }
        }
        if pending {
            // Unwinding with its submit pending: the drop does not wait, and the last holder
            // destroys the pools once the device is idle, or leaks them with the device.
            self.shared.orphan(self.pool, core::mem::take(&mut self.inputs));
        } else if !raw.leaking() {
            // SAFETY: no submit of the command buffer is pending, or the device is lost;
            // the pools are this encoder's.
            unsafe {
                for pool in &self.inputs {
                    raw.device.destroy_descriptor_pool(*pool, None);
                }
                raw.device.destroy_command_pool(self.pool, None);
            }
        }
        self.shared.let_go();
    }
}

impl Encoder for VulkanEncoder {
    fn queue(&self) -> QueueRole {
        self.role
    }

    fn reset(&mut self) -> Result<()> {
        if let Some((queue, seq)) = self.submitted {
            let known = self.shared.read_queue(queue);
            let records = self.shared.records();
            if !records.lost
                && known.is_none_or(|known| known < seq)
                && records.queues[queue].known < seq
            {
                return Err(
                    Rule::Pending.refuse("a reset of an encoder whose submit has not completed")
                );
            }
        }
        self.release()?;
        self.breach = None;
        self.failure = None;
        self.submitted = None;
        self.scope = Scope::Outside;
        self.buffers.clear();
        self.images.clear();
        self.graphics.clear();
        self.compute.clear();
        self.slots.clear();
        self.dirty = None;
        self.begin()
    }

    fn transition_images(&mut self, transitions: &[ImageTransition]) {
        if !self.recording() || !self.outside("a transition") {
            return;
        }
        let mut barriers = Vec::with_capacity(transitions.len());
        for transition in transitions {
            let Some(object) = self.image(transition.image) else { return };
            let range = transition.range;
            let inside = range.mips > 0
                && range.layers > 0
                && range.mip.checked_add(range.mips).is_some_and(|end| end <= object.mips)
                && range.layer.checked_add(range.layers).is_some_and(|end| end <= object.layers);
            if !inside {
                self.fail(Rule::Bounds, "a transition of mips or layers the image does not have");
                return;
            }
            let needs = state_usage(transition.to.state);
            if !object.usage.contains(needs) {
                self.fail(
                    Rule::Usage,
                    format!(
                        "a transition to {:?} of an image without {needs:?}",
                        transition.to.state
                    ),
                );
                return;
            }
            if transition.to.state == ImageState::Undefined {
                continue;
            }
            let change = self.change(
                image_scope(transition.from.state),
                image_scope(transition.to.state),
                (transition.from.queue, transition.to.queue),
                transition.from.state == ImageState::Undefined,
                object.exclusive,
            );
            let Some(barrier) = convert::barrier(&change, self.side) else { continue };
            let (src_family, dst_family) =
                barrier.families.unwrap_or((vk::QUEUE_FAMILY_IGNORED, vk::QUEUE_FAMILY_IGNORED));
            barriers.push(
                vk::ImageMemoryBarrier2::default()
                    .src_stage_mask(barrier.src_stages)
                    .src_access_mask(barrier.src_access)
                    .dst_stage_mask(barrier.dst_stages)
                    .dst_access_mask(barrier.dst_access)
                    .old_layout(barrier.old_layout)
                    .new_layout(barrier.new_layout)
                    .src_queue_family_index(src_family)
                    .dst_queue_family_index(dst_family)
                    .image(object.image)
                    .subresource_range(
                        vk::ImageSubresourceRange::default()
                            .aspect_mask(aspect(object.format))
                            .base_mip_level(range.mip)
                            .level_count(range.mips)
                            .base_array_layer(range.layer)
                            .layer_count(range.layers),
                    ),
            );
        }
        self.barriers(&barriers, &[]);
    }

    fn transition_buffers(&mut self, transitions: &[BufferTransition]) {
        if !self.recording() || !self.outside("a transition") {
            return;
        }
        let mut barriers = Vec::with_capacity(transitions.len());
        for transition in transitions {
            let Some(object) = self.buffer(transition.buffer) else { return };
            self.touch(transition.buffer, false, true);
            let change = self.change(
                buffer_scope(transition.from.state),
                buffer_scope(transition.to.state),
                (transition.from.queue, transition.to.queue),
                transition.from.state == BufferState::Undefined,
                object.exclusive,
            );
            let Some(barrier) = convert::barrier(&change, self.side) else { continue };
            let (src_family, dst_family) =
                barrier.families.unwrap_or((vk::QUEUE_FAMILY_IGNORED, vk::QUEUE_FAMILY_IGNORED));
            barriers.push(
                vk::BufferMemoryBarrier2::default()
                    .src_stage_mask(barrier.src_stages)
                    .src_access_mask(barrier.src_access)
                    .dst_stage_mask(barrier.dst_stages)
                    .dst_access_mask(barrier.dst_access)
                    .src_queue_family_index(src_family)
                    .dst_queue_family_index(dst_family)
                    .buffer(object.buffer)
                    .offset(0)
                    .size(vk::WHOLE_SIZE),
            );
        }
        self.barriers(&[], &barriers);
    }

    fn alias(&mut self, image: ImageHandle) {
        if !self.recording() || !self.outside("an alias call") {
            return;
        }
        let Some(object) = self.image(image) else { return };
        if !object.placed {
            self.fail(Rule::Usage, "an alias call of an image not placed in a block");
            return;
        }
        // A global memory dependency: every earlier command's writes, a store that discards
        // included, are made available and visible to every later access. The image's
        // first transition, from `Undefined`, would not do: an image barrier's access scopes
        // are its own range, which leaves the ending alias's writes unordered against the
        // new one's, where the aliasing rule wants a memory dependency.
        let barrier = [vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .src_access_mask(vk::AccessFlags2::MEMORY_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .dst_access_mask(vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE)];
        let info = vk::DependencyInfo::default().memory_barriers(&barrier);
        // SAFETY: recording outside a pass.
        unsafe { self.device().cmd_pipeline_barrier2(self.command_buffer, &info) };
    }

    fn copy_buffer(
        &mut self,
        source: BufferHandle,
        destination: BufferHandle,
        regions: &[BufferCopy],
    ) {
        if !self.recording() || !self.outside("a copy") {
            return;
        }
        let Some(from) = self.buffer_for(source, BufferUsage::COPY_SOURCE) else { return };
        let Some(to) = self.buffer_for(destination, BufferUsage::COPY_DESTINATION) else { return };
        if source == destination {
            self.fail(Rule::Usage, "a copy from a buffer to itself");
            return;
        }
        for region in regions {
            if region.size == 0 {
                self.fail(Rule::Bounds, "a copy region of no bytes");
                return;
            }
            let ok = self.bounded(range(
                region.source_offset,
                region.size,
                from.size,
                "a copy's source",
            )) && self.bounded(range(
                region.destination_offset,
                region.size,
                to.size,
                "a copy's destination",
            ));
            if !ok {
                return;
            }
        }
        let written: Vec<_> = regions
            .iter()
            .map(|region| (region.destination_offset, region.destination_offset + region.size))
            .collect();
        if overlap(&written) {
            self.fail(Rule::Bounds, "two regions of one copy that write one destination byte");
            return;
        }
        self.touch(destination, true, false);
        if regions.is_empty() {
            return;
        }
        let copies: Vec<_> = regions
            .iter()
            .map(|region| vk::BufferCopy {
                src_offset: region.source_offset,
                dst_offset: region.destination_offset,
                size: region.size,
            })
            .collect();
        // SAFETY: recording outside a pass; both buffers are held and each region inside
        // both, the written ones disjoint.
        unsafe {
            self.device().cmd_copy_buffer(self.command_buffer, from.buffer, to.buffer, &copies)
        };
    }

    fn copy_buffer_to_image(
        &mut self,
        source: BufferHandle,
        destination: ImageHandle,
        regions: &[BufferImageCopy],
    ) {
        if !self.recording() || !self.outside("a copy") {
            return;
        }
        let Some(buffer) = self.buffer_for(source, BufferUsage::COPY_SOURCE) else { return };
        let Some(image) = self.image_for(destination, ImageUsage::COPY_DESTINATION) else { return };
        if image.format.is_depth() && !self.draws("a depth image's copy") {
            return;
        }
        let Some((copies, _)) = self.image_regions(&buffer, &image, regions) else { return };
        let shared = regions.iter().enumerate().any(|(at, one)| {
            regions[at + 1..].iter().any(|other| {
                (one.mip, one.layer) == (other.mip, other.layer) && meet(one.rect, other.rect)
            })
        });
        if shared {
            self.fail(Rule::Bounds, "two regions of one copy that write one destination texel");
            return;
        }
        if copies.is_empty() {
            return;
        }
        // SAFETY: recording outside a pass; both are held, every region inside both.
        unsafe {
            self.device().cmd_copy_buffer_to_image(
                self.command_buffer,
                buffer.buffer,
                image.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &copies,
            );
        }
    }

    fn copy_image_to_buffer(
        &mut self,
        source: ImageHandle,
        destination: BufferHandle,
        regions: &[BufferImageCopy],
    ) {
        if !self.recording() || !self.outside("a copy") {
            return;
        }
        let Some(image) = self.image_for(source, ImageUsage::COPY_SOURCE) else { return };
        if image.format.is_depth() && !self.draws("a depth image's copy") {
            return;
        }
        let Some(buffer) = self.buffer_for(destination, BufferUsage::COPY_DESTINATION) else {
            return;
        };
        let Some((copies, ranges)) = self.image_regions(&buffer, &image, regions) else { return };
        if overlap(&ranges) {
            self.fail(Rule::Bounds, "two regions of one copy that write one destination byte");
            return;
        }
        self.touch(destination, true, false);
        if copies.is_empty() {
            return;
        }
        // SAFETY: as `copy_buffer_to_image`'s.
        unsafe {
            self.device().cmd_copy_image_to_buffer(
                self.command_buffer,
                image.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                buffer.buffer,
                &copies,
            );
        }
    }

    fn begin_render_pass(&mut self, pass: &RenderPass<'_>) {
        if !self.recording() || !self.outside("a pass begun") || !self.draws("a render pass") {
            return;
        }
        let limit = self.shared.caps.colour_attachments;
        if pass.colour.len() > limit as usize {
            self.fail(Rule::Usage, format!("a pass of more than {limit} colour attachments"));
            return;
        }
        if pass.colour.is_empty() && pass.depth.is_none() {
            self.fail(Rule::Usage, "a render pass with no attachment");
            return;
        }
        let mut seen = Vec::new();
        let mut extent = None;
        let mut colour = Vec::with_capacity(pass.colour.len());
        let mut formats = Vec::with_capacity(pass.colour.len());
        let mut marked = 0;
        for (at, attachment) in pass.colour.iter().enumerate() {
            let usage = if attachment.local_read {
                ImageUsage::COLOUR.union(ImageUsage::INPUT)
            } else {
                ImageUsage::COLOUR
            };
            let keep = attachment.load == Load::Keep;
            let store = attachment.store == Store::Store;
            let Some(object) =
                self.attachment(attachment.target, usage, keep || store, &mut seen, &mut extent)
            else {
                return;
            };
            formats.push(object.format);
            if attachment.local_read {
                marked |= 1 << at;
            }
            let (load, clear) = match attachment.load {
                Load::Clear(value) => (
                    vk::AttachmentLoadOp::CLEAR,
                    vk::ClearValue { color: vk::ClearColorValue { float32: value } },
                ),
                Load::DontCare => (vk::AttachmentLoadOp::DONT_CARE, vk::ClearValue::default()),
                Load::Keep => (vk::AttachmentLoadOp::LOAD, vk::ClearValue::default()),
            };
            let layout = if attachment.local_read {
                vk::ImageLayout::RENDERING_LOCAL_READ_KHR
            } else {
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
            };
            colour.push(
                vk::RenderingAttachmentInfo::default()
                    .image_view(object.view(attachment.target.mip, attachment.target.layer))
                    .image_layout(layout)
                    .load_op(load)
                    .store_op(if store {
                        vk::AttachmentStoreOp::STORE
                    } else {
                        vk::AttachmentStoreOp::DONT_CARE
                    })
                    .clear_value(clear),
            );
        }
        let mut depth = None;
        let mut depth_format = None;
        if let Some(attachment) = pass.depth {
            let keep = attachment.load == Load::Keep;
            let store = attachment.store == Store::Store;
            let Some(object) = self.attachment(
                attachment.target,
                ImageUsage::DEPTH,
                keep || store,
                &mut seen,
                &mut extent,
            ) else {
                return;
            };
            depth_format = Some(object.format);
            let (load, clear) = match attachment.load {
                Load::Clear(value) => (
                    vk::AttachmentLoadOp::CLEAR,
                    vk::ClearValue {
                        depth_stencil: vk::ClearDepthStencilValue { depth: value, stencil: 0 },
                    },
                ),
                Load::DontCare => (vk::AttachmentLoadOp::DONT_CARE, vk::ClearValue::default()),
                Load::Keep => (vk::AttachmentLoadOp::LOAD, vk::ClearValue::default()),
            };
            depth = Some(
                vk::RenderingAttachmentInfo::default()
                    .image_view(object.view(attachment.target.mip, attachment.target.layer))
                    .image_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                    .load_op(load)
                    .store_op(if store {
                        vk::AttachmentStoreOp::STORE
                    } else {
                        vk::AttachmentStoreOp::DONT_CARE
                    })
                    .clear_value(clear),
            );
        }
        let Some((slots, buffers, _)) = self.reach(&pass.reach) else { return };
        let Some(extent) = extent else { return };
        let input_set = if marked == 0 {
            None
        } else {
            match self.input_set() {
                Ok(set) => Some(set),
                Err(error) => {
                    self.failure = Some(error);
                    return;
                }
            }
        };
        if let Some(set) = input_set {
            let infos: Vec<_> = pass
                .colour
                .iter()
                .enumerate()
                .filter(|(at, _)| marked & (1 << at) != 0)
                .map(|(at, attachment)| {
                    let view = self
                        .images
                        .get(&attachment.target.image)
                        .map_or_else(vk::ImageView::null, |object| {
                            object.view(attachment.target.mip, attachment.target.layer)
                        });
                    (
                        at,
                        [vk::DescriptorImageInfo::default()
                            .image_view(view)
                            .image_layout(vk::ImageLayout::RENDERING_LOCAL_READ_KHR)],
                    )
                })
                .collect();
            let writes: Vec<_> = infos
                .iter()
                .map(|(at, info)| {
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(*at as u32)
                        .descriptor_type(vk::DescriptorType::INPUT_ATTACHMENT)
                        .image_info(info)
                })
                .collect();
            // SAFETY: the set is this encoder's alone and not yet bound; every view is of an
            // image the encoder holds.
            unsafe { self.device().update_descriptor_sets(&writes, &[]) };
        }
        let area = vk::Rect2D {
            offset: vk::Offset2D::default(),
            extent: vk::Extent2D { width: extent.width, height: extent.height },
        };
        let mut info = vk::RenderingInfo::default()
            .render_area(area)
            .layer_count(1)
            .color_attachments(&colour);
        if let Some(depth) = &depth {
            info = info.depth_attachment(depth);
        }
        let layout = self.shared.heap.graphics;
        let full = Viewport {
            x: 0.0,
            y: 0.0,
            width: extent.width as f32,
            height: extent.height as f32,
            near: 0.0,
            far: 1.0,
        };
        // SAFETY: recording outside a pass; every attachment's view is of an image the
        // encoder holds, of the pass's extent.
        unsafe {
            self.device().cmd_begin_rendering(self.command_buffer, &info);
            if let Some(set) = input_set {
                self.device().cmd_bind_descriptor_sets(
                    self.command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    layout,
                    0,
                    &[set],
                    &[],
                );
            }
            self.device().cmd_set_viewport(self.command_buffer, 0, &[flipped(full)]);
            self.device().cmd_set_scissor(self.command_buffer, 0, &[area]);
        }
        self.start_pass(vk::PipelineBindPoint::GRAPHICS, layout);
        self.scope = Scope::Render(Box::new(RenderScope {
            extent,
            formats,
            depth: depth_format,
            marked,
            slots,
            buffers,
            pipeline: None,
            index: None,
        }));
    }

    fn begin_compute_pass(&mut self, pass: &ComputePass<'_>) {
        if !self.recording() || !self.outside("a pass begun") {
            return;
        }
        let Some((slots, buffers, writes)) = self.reach(&pass.reach) else { return };
        self.start_pass(vk::PipelineBindPoint::COMPUTE, self.shared.heap.compute);
        self.scope = Scope::Compute(ComputeScope {
            slots,
            buffers,
            pipeline: false,
            writes,
            dispatched: false,
        });
    }

    fn end_pass(&mut self) {
        if !self.recording() {
            return;
        }
        match self.scope {
            Scope::Outside => self.fail(Rule::Encoding, "a pass ended with none open"),
            Scope::Render(_) => {
                // SAFETY: recording inside the render pass this ends.
                unsafe { self.device().cmd_end_rendering(self.command_buffer) };
                self.scope = Scope::Outside;
            }
            Scope::Compute(_) => self.scope = Scope::Outside,
        }
    }

    fn set_viewport(&mut self, viewport: Viewport) {
        if !self.recording() {
            return;
        }
        if !matches!(self.scope, Scope::Render(_)) {
            self.fail(Rule::Encoding, "a viewport set outside a render pass");
            return;
        }
        let caps = &self.shared.caps;
        let Viewport { x, y, width, height, near, far } = viewport;
        let fields = [x, y, width, height, near, far];
        let (low, high) = (f64::from(caps.viewport_bounds[0]), f64::from(caps.viewport_bounds[1]));
        let ok = fields.iter().all(|field| field.is_finite())
            && width > 0.0
            && height > 0.0
            && f64::from(width) <= f64::from(caps.viewport_extent[0])
            && f64::from(height) <= f64::from(caps.viewport_extent[1])
            && f64::from(x) >= low
            && f64::from(y) >= low
            && f64::from(x) + f64::from(width) <= high
            && f64::from(y) + f64::from(height) <= high
            && (0.0..=1.0).contains(&near)
            && (0.0..=1.0).contains(&far);
        if !ok {
            self.fail(Rule::Bounds, format!("a viewport outside its rules: {viewport:?}"));
            return;
        }
        // SAFETY: recording inside a render pass; the viewport is within the device's
        // limits, as checked.
        unsafe { self.device().cmd_set_viewport(self.command_buffer, 0, &[flipped(viewport)]) };
    }

    fn set_scissor(&mut self, scissor: Rect) {
        if !self.recording() {
            return;
        }
        let Scope::Render(pass) = &self.scope else {
            self.fail(Rule::Encoding, "a scissor set outside a render pass");
            return;
        };
        let extent = pass.extent;
        let inside = scissor
            .x
            .checked_add(scissor.extent.width)
            .is_some_and(|end| end <= extent.width)
            && scissor.y.checked_add(scissor.extent.height).is_some_and(|end| end <= extent.height);
        if !inside {
            self.fail(Rule::Bounds, "a scissor outside the attachments");
            return;
        }
        let rect = vk::Rect2D {
            offset: vk::Offset2D { x: scissor.x as i32, y: scissor.y as i32 },
            extent: vk::Extent2D { width: scissor.extent.width, height: scissor.extent.height },
        };
        // SAFETY: recording inside a render pass; the rectangle is inside the attachments.
        unsafe { self.device().cmd_set_scissor(self.command_buffer, 0, &[rect]) };
    }

    fn bind_graphics_pipeline(&mut self, pipeline: GraphicsPipelineHandle) {
        if !self.recording() || !self.draws("a graphics pipeline bound") {
            return;
        }
        if !matches!(self.scope, Scope::Render(_)) {
            self.fail(Rule::Encoding, "a graphics pipeline bound outside a render pass");
            return;
        }
        let object = match self.graphics.get(&pipeline) {
            Some(object) => Arc::clone(object),
            None => {
                let Some(object) = self.shared.graphics.resolve(pipeline) else {
                    self.fail(
                        Rule::Destroyed,
                        "a graphics pipeline that is destroyed or of another device",
                    );
                    return;
                };
                self.graphics.insert(pipeline, Arc::clone(&object));
                object
            }
        };
        let Scope::Render(pass) = &mut self.scope else { return };
        if object.colour != pass.formats || object.depth != pass.depth {
            self.fail(Rule::Usage, "a pipeline of other formats than its pass's attachments");
            return;
        }
        let handle = object.pipeline;
        pass.pipeline = Some(object);
        // SAFETY: recording inside a render pass whose formats are the pipeline's.
        unsafe {
            self.device().cmd_bind_pipeline(
                self.command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                handle,
            )
        };
    }

    fn bind_compute_pipeline(&mut self, pipeline: ComputePipelineHandle) {
        if !self.recording() {
            return;
        }
        if !matches!(self.scope, Scope::Compute(_)) {
            self.fail(Rule::Encoding, "a compute pipeline bound outside a compute pass");
            return;
        }
        let object = match self.compute.get(&pipeline) {
            Some(object) => Arc::clone(object),
            None => {
                let Some(object) = self.shared.compute.resolve(pipeline) else {
                    self.fail(
                        Rule::Destroyed,
                        "a compute pipeline that is destroyed or of another device",
                    );
                    return;
                };
                self.compute.insert(pipeline, Arc::clone(&object));
                object
            }
        };
        if let Scope::Compute(pass) = &mut self.scope {
            pass.pipeline = true;
        }
        // SAFETY: recording; the pipeline is held.
        unsafe {
            self.device().cmd_bind_pipeline(
                self.command_buffer,
                vk::PipelineBindPoint::COMPUTE,
                object.pipeline,
            )
        };
    }

    fn bind_index_buffer(&mut self, buffer: BufferHandle, offset: u64) {
        if !self.recording() {
            return;
        }
        if !matches!(self.scope, Scope::Render(_)) {
            self.fail(Rule::Encoding, "an index buffer bound outside a render pass");
            return;
        }
        let Some(object) = self.buffer_for(buffer, BufferUsage::INDEX) else { return };
        if !offset.is_multiple_of(4) || offset >= object.size {
            self.fail(
                Rule::Bounds,
                format!("an index buffer bound at {offset}, unaligned or outside"),
            );
            return;
        }
        if let Scope::Render(pass) = &mut self.scope {
            pass.index = Some((buffer, offset, object.size));
        }
        // SAFETY: recording inside a render pass; the buffer is held, the offset aligned
        // and inside it.
        unsafe {
            self.device().cmd_bind_index_buffer(
                self.command_buffer,
                object.buffer,
                offset,
                vk::IndexType::UINT32,
            )
        };
    }

    fn push_bytes(&mut self, offset: u32, bytes: &[u8]) {
        if !self.recording() {
            return;
        }
        if self.declared().is_none() {
            self.fail(Rule::Encoding, "a push outside a pass");
            return;
        }
        self.push(offset, bytes);
    }

    fn push_address(&mut self, offset: u32, buffer: BufferHandle, byte_offset: u64) {
        if !self.recording() {
            return;
        }
        let Some(declared) = self.declared().map(|(_, buffers)| buffers.contains(&buffer)) else {
            self.fail(Rule::Encoding, "a push outside a pass");
            return;
        };
        let Some(object) = self.buffer(buffer) else { return };
        if !declared {
            self.fail(Rule::Undeclared, "an address of a buffer the pass did not declare");
            return;
        }
        if byte_offset >= object.size {
            self.fail(Rule::Bounds, "an address past the end of its buffer");
            return;
        }
        self.push(offset, &(object.address + byte_offset).to_le_bytes());
    }

    fn push_slot(&mut self, offset: u32, slot: HeapSlot) {
        if !self.recording() {
            return;
        }
        let Some(declared) =
            self.declared().map(|(slots, _)| slots.iter().any(|(declared, _)| *declared == slot))
        else {
            self.fail(Rule::Encoding, "a push outside a pass");
            return;
        };
        if !declared {
            self.fail(
                Rule::Undeclared,
                format!("slot {} pushed and not declared by the pass", slot.0),
            );
            return;
        }
        self.push(offset, &slot.0.to_le_bytes());
    }

    fn local_read(&mut self) {
        if !self.recording() {
            return;
        }
        if !matches!(self.scope, Scope::Render(_)) {
            self.fail(Rule::Encoding, "a local read outside a render pass");
            return;
        }
        let barrier = [vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
            .dst_access_mask(vk::AccessFlags2::INPUT_ATTACHMENT_READ)];
        let info = vk::DependencyInfo::default()
            .dependency_flags(vk::DependencyFlags::BY_REGION)
            .memory_barriers(&barrier);
        // SAFETY: recording inside a render pass begun with dynamic rendering, on a device
        // with `dynamicRenderingLocalRead`: a by-region barrier of framebuffer-space stages.
        unsafe { self.device().cmd_pipeline_barrier2(self.command_buffer, &info) };
    }

    fn draw(&mut self, vertices: Range<u32>, instances: Range<u32>) {
        if !self.recording()
            || vertices.is_empty()
            || instances.is_empty()
            || !self.draw_checks(false)
        {
            return;
        }
        // SAFETY: recording inside a render pass with a pipeline of its formats bound.
        unsafe {
            self.device().cmd_draw(
                self.command_buffer,
                vertices.len() as u32,
                instances.len() as u32,
                vertices.start,
                instances.start,
            );
        }
    }

    fn draw_indexed(&mut self, indices: Range<u32>, base_vertex: i32, instances: Range<u32>) {
        if !self.recording() || indices.is_empty() || instances.is_empty() {
            return;
        }
        if let Scope::Render(pass) = &self.scope
            && let Some((_, offset, size)) = pass.index
            && offset + u64::from(indices.end) * 4 > size
        {
            self.fail(Rule::Bounds, "an indexed draw that reads past its index buffer");
            return;
        }
        if !self.draw_checks(true) {
            return;
        }
        // SAFETY: as `draw`'s, with an index buffer bound whose indices read are inside it.
        unsafe {
            self.device().cmd_draw_indexed(
                self.command_buffer,
                indices.len() as u32,
                instances.len() as u32,
                indices.start,
                base_vertex,
                instances.start,
            );
        }
    }

    fn draw_indexed_indirect(
        &mut self,
        arguments: BufferHandle,
        offset: u64,
        count: u32,
        stride: u32,
    ) {
        if !self.recording() || count == 0 {
            return;
        }
        if count > 1 && !self.shared.caps.multi_draw_indirect {
            self.fail(Rule::Capability, "a count above one with no multi_draw_indirect");
            return;
        }
        if !matches!(self.scope, Scope::Render(_)) {
            self.fail(Rule::Encoding, "a draw outside a render pass");
            return;
        }
        let Some(object) = self.buffer_for(arguments, BufferUsage::INDIRECT) else { return };
        if !self.records_inside(offset, count, stride, count > 1, object.size)
            || !self.draw_checks(true)
        {
            return;
        }
        // A count of one takes the record's size as its stride, which Vulkan ignores.
        let stride = if count > 1 { stride } else { IndexedIndirectArgs::BYTES };
        // SAFETY: as `draw_indexed`'s; the records are inside the buffer at an aligned offset
        // and stride, and a count above one is a reported capability.
        unsafe {
            self.device().cmd_draw_indexed_indirect(
                self.command_buffer,
                object.buffer,
                offset,
                count,
                stride,
            )
        };
    }

    fn draw_indexed_indirect_count(
        &mut self,
        arguments: BufferHandle,
        offset: u64,
        count: BufferHandle,
        count_offset: u64,
        max_count: u32,
        stride: u32,
    ) {
        if !self.recording() {
            return;
        }
        if !self.shared.caps.indirect_count {
            self.fail(Rule::Capability, "an indirect count draw with no indirect_count");
            return;
        }
        if max_count == 0 {
            return;
        }
        if !matches!(self.scope, Scope::Render(_)) {
            self.fail(Rule::Encoding, "a draw outside a render pass");
            return;
        }
        let Some(object) = self.buffer_for(arguments, BufferUsage::INDIRECT) else { return };
        let Some(counter) = self.buffer_for(count, BufferUsage::INDIRECT) else { return };
        if !count_offset.is_multiple_of(4)
            || !self.bounded(range(count_offset, 4, counter.size, "a draw's count"))
        {
            self.fail(Rule::Bounds, "a count offset unaligned or outside its buffer");
            return;
        }
        if !self.records_inside(offset, max_count, stride, true, object.size)
            || !self.draw_checks(true)
        {
            return;
        }
        // SAFETY: as `draw_indexed_indirect`'s, with the count's word inside its buffer and
        // `drawIndirectCount` enabled.
        unsafe {
            self.device().cmd_draw_indexed_indirect_count(
                self.command_buffer,
                object.buffer,
                offset,
                counter.buffer,
                count_offset,
                max_count,
                stride,
            );
        }
    }

    fn dispatch(&mut self, groups: [u32; 3]) {
        if !self.recording() || groups.contains(&0) {
            return;
        }
        let limit = self.shared.caps.compute_groups;
        if (0..3).any(|at| groups[at] > limit[at]) {
            self.fail(Rule::Bounds, format!("a dispatch of {groups:?} groups, above {limit:?}"));
            return;
        }
        if !self.dispatch_checks() {
            return;
        }
        // SAFETY: recording inside a compute pass with a pipeline bound; each count within
        // the device's limit.
        unsafe { self.device().cmd_dispatch(self.command_buffer, groups[0], groups[1], groups[2]) };
    }

    fn dispatch_indirect(&mut self, arguments: BufferHandle, offset: u64) {
        if !self.recording() {
            return;
        }
        if !matches!(self.scope, Scope::Compute(_)) {
            self.fail(Rule::Encoding, "a dispatch outside a compute pass");
            return;
        }
        let Some(object) = self.buffer_for(arguments, BufferUsage::INDIRECT) else { return };
        if !offset.is_multiple_of(4)
            || !self.bounded(range(offset, 12, object.size, "a dispatch's counts"))
        {
            self.fail(Rule::Bounds, "a dispatch's counts unaligned or outside their buffer");
            return;
        }
        if !self.dispatch_checks() {
            return;
        }
        // SAFETY: as `dispatch`'s; the counts are inside the buffer at an aligned offset.
        unsafe { self.device().cmd_dispatch_indirect(self.command_buffer, object.buffer, offset) };
    }
}

/// The viewport as Vulkan takes it: flipped, so that clip space's y is up as the
/// contract's is, with its origin still the framebuffer's top left.
fn flipped(viewport: Viewport) -> vk::Viewport {
    vk::Viewport {
        x: viewport.x,
        y: viewport.y + viewport.height,
        width: viewport.width,
        height: -viewport.height,
        min_depth: viewport.near,
        max_depth: viewport.far,
    }
}

/// The usage an image needs for a transition to `state`: Vulkan's layout of each is valid
/// only for an image with it.
const fn state_usage(state: ImageState) -> ImageUsage {
    match state {
        ImageState::Undefined | ImageState::Present => ImageUsage::EMPTY,
        ImageState::ColourAttachment => ImageUsage::COLOUR,
        ImageState::LocalRead => ImageUsage::COLOUR.union(ImageUsage::INPUT),
        ImageState::DepthAttachment => ImageUsage::DEPTH,
        ImageState::SampledRead => ImageUsage::SAMPLED,
        ImageState::CopySource => ImageUsage::COPY_SOURCE,
        ImageState::CopyDestination => ImageUsage::COPY_DESTINATION,
    }
}

/// A byte range of a buffer: its start and the end past it.
type Span = (u64, u64);

/// Whether two of `ranges` share a byte.
fn overlap(ranges: &[Span]) -> bool {
    let mut sorted = ranges.to_vec();
    sorted.sort_unstable();
    sorted.windows(2).any(|pair| pair[1].0 < pair[0].1)
}

/// Whether two rectangles, each inside its mip, share a texel.
fn meet(one: Rect, other: Rect) -> bool {
    one.x < other.x + other.extent.width
        && other.x < one.x + one.extent.width
        && one.y < other.y + other.extent.height
        && other.y < one.y + one.extent.height
}

/// A breach found by a check of what a pass holds.
type Breach = (Rule, String);

/// Refuses a draw or dispatch in a pass that declares an empty slot.
fn empty_slot(slots: &[SlotDecl], what: &str) -> core::result::Result<(), Breach> {
    match slots.iter().find(|(_, image)| image.is_none()) {
        Some((slot, _)) => Err((
            Rule::HeapLayout,
            format!("{what} in a pass that declares slot {}, empty", slot.0),
        )),
        None => Ok(()),
    }
}

/// The checks a draw makes of its render pass. A draw that reads an input attachment its
/// pass did not mark is refused here as well as on null: the driver would read a
/// descriptor no write filled.
fn draw_checks(pass: &RenderScope, indexed: bool) -> core::result::Result<(), Breach> {
    let Some(pipeline) = &pass.pipeline else {
        return Err((Rule::Encoding, "a draw with no graphics pipeline bound".to_owned()));
    };
    if indexed && pass.index.is_none() {
        return Err((Rule::Encoding, "an indexed draw with no index buffer bound".to_owned()));
    }
    empty_slot(&pass.slots, "a draw")?;
    let unmarked = pipeline.inputs & !pass.marked;
    if unmarked != 0 {
        return Err((
            Rule::LocalMap,
            format!(
                "a draw that reads input {}, with no attachment of its index marked for the local read",
                unmarked.trailing_zeros()
            ),
        ));
    }
    Ok(())
}
