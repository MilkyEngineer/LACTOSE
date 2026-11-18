//! The heap and the sampler table as one descriptor set (`lactose-rhi-api`'s README, "The
//! heap and the sampler table"): set 1, binding 0 the heap's sampled images and binding 1
//! the samplers, both partially bound, updated after bind and while unused by pending
//! submits; a placeholder image in every empty slot and a placeholder sampler in every free
//! entry. Beside it, the two pipeline layouts every pipeline of the device takes, so that
//! the sets bound at a pass's start serve each pipeline the pass binds.

use std::sync::Arc;

use ash::vk;
use lactose_core::Result;
use lactose_rhi_api::{Format, HEAP_SET, ImageUsage, PUSH_BYTES_MAX};

use crate::convert::image_usage;
use crate::raw::{Raw, Want, driver, image_info, unsupported};

/// The objects of the heap and the layouts. Destroyed with the device.
pub(crate) struct Heap {
    raw: Arc<Raw>,
    pool: vk::DescriptorPool,
    pub(crate) layout: vk::DescriptorSetLayout,
    pub(crate) set: vk::DescriptorSet,
    /// Set 0 of a graphics pipeline: an input attachment at binding `i` for colour
    /// attachment `i`, partially bound.
    pub(crate) inputs: vk::DescriptorSetLayout,
    /// Set 0 of a compute pipeline: nothing.
    empty: vk::DescriptorSetLayout,
    pub(crate) graphics: vk::PipelineLayout,
    pub(crate) compute: vk::PipelineLayout,
    pub(crate) input_count: u32,
    placeholder_image: vk::Image,
    placeholder_memory: vk::DeviceMemory,
    pub(crate) placeholder_view: vk::ImageView,
    pub(crate) placeholder_sampler: vk::Sampler,
}

/// The stages a graphics pipeline's push block reaches.
pub(crate) const GRAPHICS_PUSH: vk::ShaderStageFlags = vk::ShaderStageFlags::from_raw(
    vk::ShaderStageFlags::VERTEX.as_raw() | vk::ShaderStageFlags::FRAGMENT.as_raw(),
);

/// Refuses a set layout the device does not support, `what` naming it: a driver may support
/// fewer descriptors in one set than its limits name, as MoltenVK 1.4.2's
/// `maxPerSetDescriptors` of 1212 bounds the heap and the sampler table together.
fn supported(
    device: &ash::Device,
    info: &vk::DescriptorSetLayoutCreateInfo<'_>,
    what: impl FnOnce() -> String,
) -> Result<()> {
    let mut support = vk::DescriptorSetLayoutSupport::default();
    // SAFETY: the device is live; the info and what it chains live through the call.
    unsafe { device.get_descriptor_set_layout_support(info, &mut support) };
    if support.supported == vk::FALSE {
        return Err(unsupported(format!("{}: the device does not support its set", what())));
    }
    Ok(())
}

impl Heap {
    /// Makes the heap of `slots` and `samplers` entries, each holding its placeholder,
    /// and the layouts for `colour_attachments` inputs. Submits one command buffer to
    /// `queue` and waits for it, to put the placeholder image in its layout: the device's
    /// first submit, made before any caller's.
    pub(crate) fn new(
        raw: &Arc<Raw>,
        slots: u32,
        samplers: u32,
        colour_attachments: u32,
    ) -> Result<Self> {
        let device = &raw.device;
        let mut heap = Self {
            raw: Arc::clone(raw),
            pool: vk::DescriptorPool::null(),
            layout: vk::DescriptorSetLayout::null(),
            set: vk::DescriptorSet::null(),
            inputs: vk::DescriptorSetLayout::null(),
            empty: vk::DescriptorSetLayout::null(),
            graphics: vk::PipelineLayout::null(),
            compute: vk::PipelineLayout::null(),
            input_count: colour_attachments.min(32),
            placeholder_image: vk::Image::null(),
            placeholder_memory: vk::DeviceMemory::null(),
            placeholder_view: vk::ImageView::null(),
            placeholder_sampler: vk::Sampler::null(),
        };
        // Each step fills a field, so a failure's drop destroys what was made.
        let flags = vk::DescriptorBindingFlags::PARTIALLY_BOUND
            | vk::DescriptorBindingFlags::UPDATE_AFTER_BIND
            | vk::DescriptorBindingFlags::UPDATE_UNUSED_WHILE_PENDING;
        let stages = vk::ShaderStageFlags::VERTEX
            | vk::ShaderStageFlags::FRAGMENT
            | vk::ShaderStageFlags::COMPUTE;
        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(slots)
                .stage_flags(stages),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(samplers)
                .stage_flags(stages),
        ];
        let binding_flags = [flags, flags];
        let mut flags_info =
            vk::DescriptorSetLayoutBindingFlagsCreateInfo::default().binding_flags(&binding_flags);
        let info = vk::DescriptorSetLayoutCreateInfo::default()
            .flags(vk::DescriptorSetLayoutCreateFlags::UPDATE_AFTER_BIND_POOL)
            .bindings(&bindings)
            .push_next(&mut flags_info);
        supported(device, &info, || format!("a heap of {slots} slots and {samplers} samplers"))?;
        // SAFETY: the device is live and supports the layout; the info and what it chains
        // live through the call.
        heap.layout = unsafe { device.create_descriptor_set_layout(&info, None) }
            .map_err(|e| driver(e, "vkCreateDescriptorSetLayout"))?;
        let input_bindings: Vec<_> = (0..heap.input_count)
            .map(|at| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(at)
                    .descriptor_type(vk::DescriptorType::INPUT_ATTACHMENT)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT)
            })
            .collect();
        let input_flags = vec![vk::DescriptorBindingFlags::PARTIALLY_BOUND; input_bindings.len()];
        let mut input_flags_info =
            vk::DescriptorSetLayoutBindingFlagsCreateInfo::default().binding_flags(&input_flags);
        let info = vk::DescriptorSetLayoutCreateInfo::default()
            .bindings(&input_bindings)
            .push_next(&mut input_flags_info);
        let count = heap.input_count;
        supported(device, &info, || format!("a render pass's {count} input attachments"))?;
        // SAFETY: as above.
        heap.inputs = unsafe { device.create_descriptor_set_layout(&info, None) }
            .map_err(|e| driver(e, "vkCreateDescriptorSetLayout"))?;
        // SAFETY: as above.
        heap.empty = unsafe {
            device.create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default(), None)
        }
        .map_err(|e| driver(e, "vkCreateDescriptorSetLayout"))?;
        debug_assert_eq!(HEAP_SET, 1, "the layouts put the heap at set 1");
        let layout = |set0: vk::DescriptorSetLayout, push: vk::ShaderStageFlags| {
            let sets = [set0, heap.layout];
            let ranges =
                [vk::PushConstantRange::default().stage_flags(push).offset(0).size(PUSH_BYTES_MAX)];
            let info = vk::PipelineLayoutCreateInfo::default()
                .set_layouts(&sets)
                .push_constant_ranges(&ranges);
            // SAFETY: as above; both set layouts are live.
            unsafe { device.create_pipeline_layout(&info, None) }
                .map_err(|e| driver(e, "vkCreatePipelineLayout"))
        };
        heap.graphics = layout(heap.inputs, GRAPHICS_PUSH)?;
        heap.compute = layout(heap.empty, vk::ShaderStageFlags::COMPUTE)?;
        let sizes = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::SAMPLED_IMAGE,
                descriptor_count: slots,
            },
            vk::DescriptorPoolSize { ty: vk::DescriptorType::SAMPLER, descriptor_count: samplers },
        ];
        let info = vk::DescriptorPoolCreateInfo::default()
            .flags(vk::DescriptorPoolCreateFlags::UPDATE_AFTER_BIND)
            .max_sets(1)
            .pool_sizes(&sizes);
        // SAFETY: as above.
        heap.pool = unsafe { device.create_descriptor_pool(&info, None) }
            .map_err(|e| driver(e, "vkCreateDescriptorPool"))?;
        let layouts = [heap.layout];
        let info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(heap.pool)
            .set_layouts(&layouts);
        // SAFETY: the pool and the layout are live, and the pool holds one set of it.
        heap.set = unsafe { device.allocate_descriptor_sets(&info) }
            .map_err(|e| driver(e, "vkAllocateDescriptorSets"))?[0];
        heap.placeholders()?;
        let infos: Vec<_> = (0..slots)
            .map(|_| {
                vk::DescriptorImageInfo::default()
                    .image_view(heap.placeholder_view)
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            })
            .collect();
        let samplers_info: Vec<_> = (0..samplers)
            .map(|_| vk::DescriptorImageInfo::default().sampler(heap.placeholder_sampler))
            .collect();
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(heap.set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&infos),
            vk::WriteDescriptorSet::default()
                .dst_set(heap.set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&samplers_info),
        ];
        // SAFETY: the set and every view and sampler written are live; nothing else
        // reaches the set before the constructor returns.
        unsafe { device.update_descriptor_sets(&writes, &[]) };
        Ok(heap)
    }

    /// The placeholder image, in `SHADER_READ_ONLY_OPTIMAL`, its view, and the placeholder
    /// sampler.
    fn placeholders(&mut self) -> Result<()> {
        let device = &self.raw.device;
        let usage = image_usage(ImageUsage::SAMPLED);
        let info =
            image_info(Format::Rgba8Unorm, vk::Extent2D { width: 1, height: 1 }, 1, 1, usage);
        // SAFETY: the device is live and the info valid.
        self.placeholder_image =
            unsafe { device.create_image(&info, None) }.map_err(|e| driver(e, "vkCreateImage"))?;
        // SAFETY: the image is live.
        let requirements = unsafe { device.get_image_memory_requirements(self.placeholder_image) };
        let memory_type =
            self.raw.memory.choose(requirements.memory_type_bits, Want::Device).ok_or_else(
                || driver(vk::Result::ERROR_OUT_OF_DEVICE_MEMORY, "the placeholder's memory type"),
            )?;
        let allocate = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type);
        // SAFETY: a type the image accepts, of its size.
        self.placeholder_memory = unsafe { device.allocate_memory(&allocate, None) }
            .map_err(|e| driver(e, "vkAllocateMemory"))?;
        // SAFETY: both live, the memory unbound and large enough.
        unsafe { device.bind_image_memory(self.placeholder_image, self.placeholder_memory, 0) }
            .map_err(|e| driver(e, "vkBindImageMemory"))?;
        let range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .level_count(1)
            .layer_count(1);
        let view = vk::ImageViewCreateInfo::default()
            .image(self.placeholder_image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .subresource_range(range);
        // SAFETY: the image is live and bound.
        self.placeholder_view = unsafe { device.create_image_view(&view, None) }
            .map_err(|e| driver(e, "vkCreateImageView"))?;
        // SAFETY: the device is live; a default sampler is valid on every device.
        self.placeholder_sampler =
            unsafe { device.create_sampler(&vk::SamplerCreateInfo::default(), None) }
                .map_err(|e| driver(e, "vkCreateSampler"))?;
        self.settle_placeholder(range)
    }

    /// Moves the placeholder image to `SHADER_READ_ONLY_OPTIMAL` with one submit to the
    /// graphics queue, and waits for it on a fence of its own.
    fn settle_placeholder(&self, range: vk::ImageSubresourceRange) -> Result<()> {
        let device = &self.raw.device;
        let queue = &self.raw.queues[0];
        let info = vk::CommandPoolCreateInfo::default()
            .flags(vk::CommandPoolCreateFlags::TRANSIENT)
            .queue_family_index(queue.family);
        // SAFETY: the device is live.
        let pool = unsafe { device.create_command_pool(&info, None) }
            .map_err(|e| driver(e, "vkCreateCommandPool"))?;
        let result = (|| {
            let allocate = vk::CommandBufferAllocateInfo::default()
                .command_pool(pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1);
            // SAFETY: the pool is live and this thread's alone.
            let cb = unsafe { device.allocate_command_buffers(&allocate) }
                .map_err(|e| driver(e, "vkAllocateCommandBuffers"))?[0];
            let begin = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            let barrier = [vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::NONE)
                .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(self.placeholder_image)
                .subresource_range(range)];
            // SAFETY: the command buffer is new and recorded on this thread alone; the
            // image is live.
            unsafe {
                device
                    .begin_command_buffer(cb, &begin)
                    .map_err(|e| driver(e, "vkBeginCommandBuffer"))?;
                device.cmd_pipeline_barrier2(
                    cb,
                    &vk::DependencyInfo::default().image_memory_barriers(&barrier),
                );
                device.end_command_buffer(cb).map_err(|e| driver(e, "vkEndCommandBuffer"))?;
            }
            // SAFETY: the device is live.
            let fence = unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }
                .map_err(|e| driver(e, "vkCreateFence"))?;
            let buffers = [vk::CommandBufferSubmitInfo::default().command_buffer(cb)];
            let submit = [vk::SubmitInfo2::default().command_buffer_infos(&buffers)];
            // SAFETY: the queue is the device's and no other thread has the device yet; the
            // command buffer is executable; the fence unsignalled.
            let submitted = unsafe { device.queue_submit2(queue.queue, &submit, fence) }
                .map_err(|e| driver(e, "vkQueueSubmit2"))
                .and_then(|()| {
                    // SAFETY: the fence is live and was submitted.
                    unsafe { device.wait_for_fences(&[fence], true, u64::MAX) }
                        .map_err(|e| driver(e, "vkWaitForFences"))
                });
            // SAFETY: the fence's submit has completed, or never reached the queue.
            unsafe { device.destroy_fence(fence, None) };
            submitted
        })();
        // SAFETY: the pool's one submit has completed or failed; nothing else uses it.
        unsafe { device.destroy_command_pool(pool, None) };
        result
    }

    /// Writes slot `slot` to `view`, or to the placeholder for `None`.
    ///
    /// # Safety
    ///
    /// The caller holds the device's heap lock, and `view` lives until the slot is written
    /// again and no submit that may read it is pending.
    pub(crate) unsafe fn write_slot(&self, slot: u32, view: Option<vk::ImageView>) {
        let info = [vk::DescriptorImageInfo::default()
            .image_view(view.unwrap_or(self.placeholder_view))
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let write = vk::WriteDescriptorSet::default()
            .dst_set(self.set)
            .dst_binding(0)
            .dst_array_element(slot)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .image_info(&info);
        // SAFETY: the heap lock serialises every write of the set; the binding is
        // update-after-bind and unused-while-pending, and the caller's checks refused a write
        // of a slot a pending submit declares (`uuwp-used`).
        unsafe { self.raw.device.update_descriptor_sets(&[write], &[]) };
    }

    /// Writes sampler entry `index` to `sampler`, or to the placeholder for `None`.
    ///
    /// # Safety
    ///
    /// As [`write_slot`](Self::write_slot), for the sampler.
    pub(crate) unsafe fn write_sampler(&self, index: u32, sampler: Option<vk::Sampler>) {
        let info =
            [vk::DescriptorImageInfo::default()
                .sampler(sampler.unwrap_or(self.placeholder_sampler))];
        let write = vk::WriteDescriptorSet::default()
            .dst_set(self.set)
            .dst_binding(1)
            .dst_array_element(index)
            .descriptor_type(vk::DescriptorType::SAMPLER)
            .image_info(&info);
        // SAFETY: as `write_slot`'s.
        unsafe { self.raw.device.update_descriptor_sets(&[write], &[]) };
    }
}

impl Drop for Heap {
    fn drop(&mut self) {
        if self.raw.leaking() {
            return;
        }
        let device = &self.raw.device;
        // SAFETY: the heap drops with the device's shared state, after its last holder
        // waited for the device to idle; each object is destroyed once, null ones skipped
        // by the driver.
        unsafe {
            device.destroy_descriptor_pool(self.pool, None);
            device.destroy_pipeline_layout(self.graphics, None);
            device.destroy_pipeline_layout(self.compute, None);
            device.destroy_descriptor_set_layout(self.layout, None);
            device.destroy_descriptor_set_layout(self.inputs, None);
            device.destroy_descriptor_set_layout(self.empty, None);
            device.destroy_sampler(self.placeholder_sampler, None);
            device.destroy_image_view(self.placeholder_view, None);
            device.destroy_image(self.placeholder_image, None);
            device.free_memory(self.placeholder_memory, None);
        }
    }
}
