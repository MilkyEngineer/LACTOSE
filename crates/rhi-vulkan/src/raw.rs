//! The driver objects every other object of a device holds: the loader, the instance, the
//! messenger, the device and its queues; the choice of adapter and queues; the required set
//! and the reported one (`lactose-rhi-api`'s README, "Capabilities").

use core::ffi::{CStr, c_void};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ash::vk;
use lactose_core::{Error, ErrorKind, Result};
use lactose_rhi_api::{
    Capabilities, DeviceInfo, DeviceRequest, Format, ImageUsage, Message, MessageSink,
    MessageSource, QueueLayout, QueuePath, Severity,
};

use crate::convert::{self, image_usage};

/// The vendor id of Apple's GPUs.
const APPLE: u32 = 0x106B;

/// The viewport bounds an Apple GPU honours, whatever MoltenVK reports (verification row
/// 183): the M1 reports -32768 to 32767 and draws wrongly past -16384 to 16383.
const APPLE_VIEWPORT_BOUNDS: [i32; 2] = [-16384, 16383];

/// The driver error of a call, as the contract's kinds name it.
pub(crate) fn driver(result: vk::Result, call: &str) -> Error {
    let kind = match result {
        vk::Result::ERROR_DEVICE_LOST => ErrorKind::DeviceLost,
        vk::Result::ERROR_OUT_OF_HOST_MEMORY
        | vk::Result::ERROR_OUT_OF_DEVICE_MEMORY
        | vk::Result::ERROR_OUT_OF_POOL_MEMORY
        | vk::Result::ERROR_FRAGMENTED_POOL
        | vk::Result::ERROR_TOO_MANY_OBJECTS => ErrorKind::ResourceExhausted,
        vk::Result::ERROR_FEATURE_NOT_PRESENT
        | vk::Result::ERROR_EXTENSION_NOT_PRESENT
        | vk::Result::ERROR_LAYER_NOT_PRESENT
        | vk::Result::ERROR_INCOMPATIBLE_DRIVER
        | vk::Result::ERROR_FORMAT_NOT_SUPPORTED => ErrorKind::Unsupported,
        _ => ErrorKind::Platform,
    };
    Error::new(kind, format!("{call}: {result:?}"))
}

/// One queue the device made: its driver handle, its family, whether the family draws, and
/// the timeline semaphore every submit to it signals.
pub(crate) struct Queue {
    pub(crate) queue: vk::Queue,
    pub(crate) family: u32,
    pub(crate) graphics: bool,
    pub(crate) timeline: vk::Semaphore,
}

/// The memory types the device allocates from.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MemoryTypes {
    pub(crate) properties: [vk::MemoryPropertyFlags; vk::MAX_MEMORY_TYPES],
    pub(crate) count: u32,
    /// The types a memory block takes, which every colour and depth image accepts.
    pub(crate) block: Option<u32>,
    /// The lazily allocated type of a memoryless colour attachment, and of a depth one.
    pub(crate) lazy_colour: Option<u32>,
    pub(crate) lazy_depth: Option<u32>,
    pub(crate) non_coherent_atom: u64,
}

/// What a memory class prefers of a type, best first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Want {
    /// Device-local, not host-visible where the device offers one.
    Device,
    /// Host-visible, coherent where offered.
    Upload,
    /// Host-visible, cached and coherent where offered.
    Readback,
}

impl MemoryTypes {
    /// The type of `bits` a resource of `want` takes: never a lazily allocated one.
    pub(crate) fn choose(&self, bits: u32, want: Want) -> Option<u32> {
        use vk::MemoryPropertyFlags as P;
        let tiers: &[(P, P)] = match want {
            Want::Device => &[
                (P::DEVICE_LOCAL, P::HOST_VISIBLE),
                (P::DEVICE_LOCAL, P::empty()),
                (P::empty(), P::empty()),
            ],
            Want::Upload => {
                &[(P::HOST_VISIBLE | P::HOST_COHERENT, P::empty()), (P::HOST_VISIBLE, P::empty())]
            }
            Want::Readback => &[
                (P::HOST_VISIBLE | P::HOST_CACHED | P::HOST_COHERENT, P::empty()),
                (P::HOST_VISIBLE | P::HOST_COHERENT, P::empty()),
                (P::HOST_VISIBLE | P::HOST_CACHED, P::empty()),
                (P::HOST_VISIBLE, P::empty()),
            ],
        };
        tiers.iter().find_map(|&(has, lacks)| {
            (0..self.count).find(|&at| {
                let props = self.properties[at as usize];
                bits & (1 << at) != 0
                    && props.contains(has)
                    && !props.intersects(lacks)
                    && !props.contains(P::LAZILY_ALLOCATED)
            })
        })
    }

    pub(crate) fn coherent(&self, at: u32) -> bool {
        self.properties[at as usize].contains(vk::MemoryPropertyFlags::HOST_COHERENT)
    }

    pub(crate) fn host_visible(&self, at: u32) -> bool {
        self.properties[at as usize].contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
    }
}

/// The driver's own objects. Every object of the device holds it, so the device is
/// destroyed after the last of them.
pub(crate) struct Raw {
    /// The loader, kept open while any of its functions may be called.
    _entry: ash::Entry,
    pub(crate) instance: ash::Instance,
    pub(crate) physical: vk::PhysicalDevice,
    pub(crate) device: ash::Device,
    debug: Option<(ash::ext::debug_utils::Instance, vk::DebugUtilsMessengerEXT)>,
    /// The sink the messenger's callback reaches through its user data. Boxed so that its
    /// address holds from the instance's creation, whose `pNext` names it, to its
    /// destruction; dropped after both, as a field after `drop`.
    _sink: Box<Arc<dyn MessageSink>>,
    pub(crate) queues: Vec<Queue>,
    pub(crate) memory: MemoryTypes,
    pub(crate) properties: vk::PhysicalDeviceProperties,
    /// Set by a drop that unwinds a panic with work maybe in flight: every object of the
    /// device is then leaked, never destroyed under the GPU's use.
    leaking: AtomicBool,
}

impl Raw {
    /// Whether drops leak rather than destroy.
    pub(crate) fn leaking(&self) -> bool {
        // Relaxed: set once, before the last holder's drop; every object's drop that reads
        // it runs after that drop in the same thread, or after the `Arc`'s release.
        self.leaking.load(Ordering::Relaxed)
    }

    pub(crate) fn leak(&self) {
        self.leaking.store(true, Ordering::Relaxed);
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        if self.leaking() {
            return;
        }
        // SAFETY: every object made from the device holds this `Raw` and is gone, and the
        // last holder of the device waited for the device to idle; the messenger and the
        // instance outlive what was made from them, and are destroyed last.
        unsafe {
            for queue in &self.queues {
                self.device.destroy_semaphore(queue.timeline, None);
            }
            self.device.destroy_device(None);
            if let Some((utils, messenger)) = &self.debug {
                utils.destroy_debug_utils_messenger(*messenger, None);
            }
            self.instance.destroy_instance(None);
        }
    }
}

/// What the constructor learned of the adapter: the driver objects, the set it reports,
/// the queue layout's queues by role and what identifies it.
pub(crate) struct Opened {
    pub(crate) raw: Arc<Raw>,
    pub(crate) caps: Capabilities,
    pub(crate) info: DeviceInfo,
    /// The index in `Raw::queues` of each role's queue: graphics, compute, present.
    pub(crate) roles: [usize; 3],
    /// The vendor, the device and the cache identifier, for the sealed cache.
    pub(crate) cache_identity: Vec<u8>,
}

/// The messenger's callback: every warning and error of the validation layer, the loader or
/// a driver that reports through it, sent to the request's sink.
unsafe extern "system" fn on_message(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    kind: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    user: *mut c_void,
) -> vk::Bool32 {
    if data.is_null() || user.is_null() {
        return vk::FALSE;
    }
    // SAFETY: the loader passes callback data valid for the call; its strings are null or
    // NUL-terminated (VUID-VkDebugUtilsMessengerCallbackDataEXT-pMessage-parameter).
    let (id, text) = unsafe {
        let data = &*data;
        let string = |at: *const core::ffi::c_char| {
            if at.is_null() { String::new() } else { CStr::from_ptr(at).to_string_lossy().into() }
        };
        (string(data.p_message_id_name), string(data.p_message))
    };
    // SAFETY: `user` is the address of the `Box`ed sink in `Raw`, which outlives the
    // instance and the messenger (`Raw::_sink`).
    let sink = unsafe { &*user.cast::<Arc<dyn MessageSink>>() };
    let severity = if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        Severity::Error
    } else {
        Severity::Warning
    };
    let source = if kind.contains(vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION) {
        MessageSource::Validation
    } else {
        MessageSource::Driver
    };
    let message = Message { source, severity, id: &id, text: &text };
    // A panic must not unwind into the driver; the sink's panic is a test's failure, and
    // the abort reports it.
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink.message(&message))).is_err() {
        std::process::abort();
    }
    vk::FALSE
}

fn unsupported(what: impl Into<String>) -> Error {
    Error::new(ErrorKind::Unsupported, what.into())
}

/// The features of the required set the adapter lacks, by name.
fn missing(
    core: &vk::PhysicalDeviceFeatures,
    v12: &vk::PhysicalDeviceVulkan12Features<'_>,
    v13: &vk::PhysicalDeviceVulkan13Features<'_>,
    local: &vk::PhysicalDeviceDynamicRenderingLocalReadFeaturesKHR<'_>,
) -> Vec<&'static str> {
    let required = [
        ("bufferDeviceAddress", v12.buffer_device_address),
        ("descriptorBindingPartiallyBound", v12.descriptor_binding_partially_bound),
        (
            "descriptorBindingSampledImageUpdateAfterBind",
            v12.descriptor_binding_sampled_image_update_after_bind,
        ),
        (
            "descriptorBindingUpdateUnusedWhilePending",
            v12.descriptor_binding_update_unused_while_pending,
        ),
        (
            "shaderSampledImageArrayNonUniformIndexing",
            v12.shader_sampled_image_array_non_uniform_indexing,
        ),
        ("timelineSemaphore", v12.timeline_semaphore),
        ("synchronization2", v13.synchronization2),
        ("dynamicRendering", v13.dynamic_rendering),
        ("dynamicRenderingLocalRead", local.dynamic_rendering_local_read),
        ("independentBlend", core.independent_blend),
        ("shaderInt64", core.shader_int64),
        ("shaderStorageImageReadWithoutFormat", core.shader_storage_image_read_without_format),
        ("drawIndirectFirstInstance", core.draw_indirect_first_instance),
    ];
    required.iter().filter(|(_, on)| *on == vk::FALSE).map(|(name, _)| *name).collect()
}

/// The adapter's name, as the driver reports it.
fn name(properties: &vk::PhysicalDeviceProperties) -> String {
    properties
        .device_name_as_c_str()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The queue families a device takes: the graphics family, and the compute queue's family
/// and index where it has a queue of its own. A family that computes and does not draw
/// first; then another family that computes; then a second queue of the graphics family.
fn families(families: &[vk::QueueFamilyProperties]) -> Option<(u32, Option<(u32, u32)>)> {
    let has = |at: usize, flags: vk::QueueFlags| families[at].queue_flags.contains(flags);
    let both = vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE;
    let graphics = (0..families.len()).find(|&at| has(at, both) && families[at].queue_count > 0)?;
    let other = |at: usize| at != graphics && families[at].queue_count > 0;
    let compute = (0..families.len())
        .find(|&at| {
            other(at) && has(at, vk::QueueFlags::COMPUTE) && !has(at, vk::QueueFlags::GRAPHICS)
        })
        .or_else(|| (0..families.len()).find(|&at| other(at) && has(at, vk::QueueFlags::COMPUTE)))
        .map(|at| (at as u32, 0))
        .or_else(|| (families[graphics].queue_count > 1).then_some((graphics as u32, 1)));
    Some((graphics as u32, compute))
}

/// A viewport bound rounded inward to a whole texel.
fn bounds(range: [f32; 2], vendor: u32) -> [i32; 2] {
    let low = range[0].ceil() as i32;
    let high = range[1].floor() as i32;
    if vendor == APPLE {
        [low.max(APPLE_VIEWPORT_BOUNDS[0]), high.min(APPLE_VIEWPORT_BOUNDS[1])]
    } else {
        [low, high]
    }
}

/// The memory requirements of an image the device would make, without making it.
fn image_requirements(
    device: &ash::Device,
    format: Format,
    usage: ImageUsage,
) -> vk::MemoryRequirements {
    let info = image_info(format, vk::Extent2D { width: 64, height: 64 }, 1, 1, image_usage(usage));
    let request = vk::DeviceImageMemoryRequirements::default().create_info(&info);
    let mut out = vk::MemoryRequirements2::default();
    // SAFETY: `info` is a valid 2D image's description for a format the device offers as
    // an attachment (checked by the caller), and `out` has no chain.
    unsafe { device.get_device_image_memory_requirements(&request, &mut out) };
    out.memory_requirements
}

/// The description of a 2D image the device makes.
pub(crate) fn image_info(
    format: Format,
    extent: vk::Extent2D,
    mips: u32,
    layers: u32,
    usage: vk::ImageUsageFlags,
) -> vk::ImageCreateInfo<'static> {
    vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(convert::format(format))
        .extent(vk::Extent3D { width: extent.width, height: extent.height, depth: 1 })
        .mip_levels(mips)
        .array_layers(layers)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
}

/// Opens the loader, makes the instance with its messenger, picks the adapter the request
/// names, checks the required set and makes the device with every reported capability on.
pub(crate) fn open(request: &DeviceRequest) -> Result<Opened> {
    // SAFETY: loading the system's loader library runs its initialisers, which the loader
    // documents as safe to run in any process.
    let entry = unsafe { ash::Entry::load() }.map_err(|error| {
        Error::new(ErrorKind::Unsupported, format!("no Vulkan loader: {error}"))
    })?;
    // SAFETY: a query with no layer named.
    let offered = unsafe { entry.enumerate_instance_extension_properties(None) }
        .map_err(|e| driver(e, "vkEnumerateInstanceExtensionProperties"))?;
    let has = |name: &CStr| offered.iter().any(|e| e.extension_name_as_c_str() == Ok(name));
    let mut extensions = Vec::new();
    let portability = has(ash::khr::portability_enumeration::NAME);
    if portability {
        extensions.push(ash::khr::portability_enumeration::NAME.as_ptr());
    }
    let debug = has(ash::ext::debug_utils::NAME);
    if debug {
        extensions.push(ash::ext::debug_utils::NAME.as_ptr());
    }
    let sink: Box<Arc<dyn MessageSink>> = Box::new(Arc::clone(&request.messages));
    let user = core::ptr::from_ref::<Arc<dyn MessageSink>>(&sink).cast_mut().cast::<c_void>();
    let mut messenger_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
        .message_severity(
            vk::DebugUtilsMessageSeverityFlagsEXT::ERROR
                | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING,
        )
        .message_type(
            vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
        )
        .pfn_user_callback(Some(on_message))
        .user_data(user);
    let app = vk::ApplicationInfo::default()
        .application_name(c"lactose")
        .engine_name(c"lactose")
        .api_version(vk::API_VERSION_1_3);
    let flags = if portability {
        vk::InstanceCreateFlags::ENUMERATE_PORTABILITY_KHR
    } else {
        vk::InstanceCreateFlags::empty()
    };
    let mut instance_info = vk::InstanceCreateInfo::default()
        .application_info(&app)
        .enabled_extension_names(&extensions)
        .flags(flags);
    if debug {
        instance_info = instance_info.push_next(&mut messenger_info);
    }
    // SAFETY: the create info and what it names live through the call; the messenger's
    // user data is the boxed sink, which outlives the instance (`Raw::_sink`).
    let instance = unsafe { entry.create_instance(&instance_info, None) }
        .map_err(|e| driver(e, "vkCreateInstance"))?;
    // From here every failure destroys what it made: `Partial` does on its drop.
    let mut partial =
        Partial { instance: Some(instance.clone()), debug: None, device: None, sink: Some(sink) };
    if debug {
        let utils = ash::ext::debug_utils::Instance::new(&entry, &instance);
        // SAFETY: as the instance's, with the instance live.
        let messenger = unsafe { utils.create_debug_utils_messenger(&messenger_info, None) }
            .map_err(|e| driver(e, "vkCreateDebugUtilsMessengerEXT"))?;
        partial.debug = Some((utils, messenger));
    }
    // SAFETY: the instance is live.
    let adapters = unsafe { instance.enumerate_physical_devices() }
        .map_err(|e| driver(e, "vkEnumeratePhysicalDevices"))?;
    let named = |physical: vk::PhysicalDevice| {
        // SAFETY: `physical` is one the instance enumerated.
        let properties = unsafe { instance.get_physical_device_properties(physical) };
        name(&properties)
    };
    let physical = match &request.adapter {
        Some(wanted) => {
            adapters.iter().copied().find(|&p| named(p).contains(wanted.as_str())).ok_or_else(
                || {
                    let names: Vec<String> = adapters.iter().map(|&p| named(p)).collect();
                    Error::new(
                        ErrorKind::NotFound,
                        format!("no adapter named like `{wanted}`; the loader offers {names:?}"),
                    )
                },
            )?
        }
        None => *adapters.first().ok_or_else(|| unsupported("the loader offers no adapter"))?,
    };
    device(&entry, &instance, physical, request, &mut partial)
}

/// What `open` has made so far, destroyed on a failure. The sink is a field, dropped after
/// the instance is destroyed: the messenger the instance's creation named may call it until
/// then.
struct Partial {
    instance: Option<ash::Instance>,
    debug: Option<(ash::ext::debug_utils::Instance, vk::DebugUtilsMessengerEXT)>,
    device: Option<ash::Device>,
    sink: Option<Box<Arc<dyn MessageSink>>>,
}

impl Drop for Partial {
    fn drop(&mut self) {
        // SAFETY: each object is destroyed once, children first, and only on a failure
        // path: a success takes every field out before the drop.
        unsafe {
            if let Some(device) = self.device.take() {
                device.destroy_device(None);
            }
            if let Some((utils, messenger)) = self.debug.take() {
                utils.destroy_debug_utils_messenger(messenger, None);
            }
            if let Some(instance) = self.instance.take() {
                instance.destroy_instance(None);
            }
        }
    }
}

fn device(
    entry: &ash::Entry,
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    request: &DeviceRequest,
    partial: &mut Partial,
) -> Result<Opened> {
    let mut driver_props = vk::PhysicalDeviceDriverProperties::default();
    let mut v12_props = vk::PhysicalDeviceVulkan12Properties::default();
    let mut properties2 = vk::PhysicalDeviceProperties2::default()
        .push_next(&mut driver_props)
        .push_next(&mut v12_props);
    // SAFETY: `physical` is the instance's; the chain's structures live through the call.
    unsafe { instance.get_physical_device_properties2(physical, &mut properties2) };
    let properties = properties2.properties;
    let adapter = name(&properties);
    if properties.api_version < vk::API_VERSION_1_3 {
        return Err(unsupported(format!("{adapter} offers Vulkan below 1.3")));
    }
    // SAFETY: as above.
    let offered = unsafe { instance.enumerate_device_extension_properties(physical) }
        .map_err(|e| driver(e, "vkEnumerateDeviceExtensionProperties"))?;
    let has = |name: &CStr| offered.iter().any(|e| e.extension_name_as_c_str() == Ok(name));
    if !has(ash::khr::dynamic_rendering_local_read::NAME) {
        return Err(unsupported(format!("{adapter} lacks VK_KHR_dynamic_rendering_local_read")));
    }
    let subset = has(ash::khr::portability_subset::NAME);
    let mut v12 = vk::PhysicalDeviceVulkan12Features::default();
    let mut v13 = vk::PhysicalDeviceVulkan13Features::default();
    let mut local = vk::PhysicalDeviceDynamicRenderingLocalReadFeaturesKHR::default();
    let mut port = vk::PhysicalDevicePortabilitySubsetFeaturesKHR::default();
    let mut features2 = vk::PhysicalDeviceFeatures2::default()
        .push_next(&mut v12)
        .push_next(&mut v13)
        .push_next(&mut local);
    if subset {
        features2 = features2.push_next(&mut port);
    }
    // SAFETY: as above.
    unsafe { instance.get_physical_device_features2(physical, &mut features2) };
    let core = features2.features;
    let lacking = missing(&core, &v12, &v13, &local);
    if !lacking.is_empty() {
        return Err(unsupported(format!("{adapter} lacks {}", lacking.join(", "))));
    }
    let limits = properties.limits;
    let descriptor_limits = [
        (
            "maxPerStageDescriptorUpdateAfterBindSampledImages",
            v12_props.max_per_stage_descriptor_update_after_bind_sampled_images,
            request.heap_slots,
        ),
        (
            "maxDescriptorSetUpdateAfterBindSampledImages",
            v12_props.max_descriptor_set_update_after_bind_sampled_images,
            request.heap_slots,
        ),
        (
            "maxPerStageDescriptorUpdateAfterBindSamplers",
            v12_props.max_per_stage_descriptor_update_after_bind_samplers,
            request.samplers,
        ),
        (
            "maxDescriptorSetUpdateAfterBindSamplers",
            v12_props.max_descriptor_set_update_after_bind_samplers,
            request.samplers,
        ),
    ];
    for (limit, offered, wanted) in descriptor_limits {
        if offered < wanted {
            return Err(unsupported(format!("{adapter}: {limit} is {offered}, below {wanted}")));
        }
    }
    if request.heap_slots == 0 || request.samplers == 0 {
        return Err(unsupported("a heap or sampler table of no entries"));
    }
    // SAFETY: as above.
    let family_props = unsafe { instance.get_physical_device_queue_family_properties(physical) };
    let Some((graphics_family, compute)) = families(&family_props) else {
        return Err(unsupported(format!("{adapter} has no family that draws and computes")));
    };
    let priorities = [1.0f32, 1.0];
    let mut queue_infos = vec![
        vk::DeviceQueueCreateInfo::default().queue_family_index(graphics_family).queue_priorities(
            &priorities[..1 + usize::from(compute.is_some_and(|(f, _)| f == graphics_family))],
        ),
    ];
    if let Some((family, _)) = compute
        && family != graphics_family
    {
        queue_infos.push(
            vk::DeviceQueueCreateInfo::default()
                .queue_family_index(family)
                .queue_priorities(&priorities[..1]),
        );
    }
    // Every reported capability offered is turned on; the rest of the required set too.
    let multi_draw = core.multi_draw_indirect == vk::TRUE;
    let indirect_count = v12.draw_indirect_count == vk::TRUE;
    let atomics64 = v12.shader_buffer_int64_atomics == vk::TRUE;
    let enable_core = vk::PhysicalDeviceFeatures::default()
        .independent_blend(true)
        .shader_int64(true)
        .shader_storage_image_read_without_format(true)
        .draw_indirect_first_instance(true)
        .multi_draw_indirect(multi_draw);
    let mut enable_v12 = vk::PhysicalDeviceVulkan12Features::default()
        .buffer_device_address(true)
        .descriptor_binding_partially_bound(true)
        .descriptor_binding_sampled_image_update_after_bind(true)
        .descriptor_binding_update_unused_while_pending(true)
        .shader_sampled_image_array_non_uniform_indexing(true)
        .timeline_semaphore(true)
        .draw_indirect_count(indirect_count)
        .shader_buffer_int64_atomics(atomics64);
    let mut enable_v13 = vk::PhysicalDeviceVulkan13Features::default()
        .synchronization2(true)
        .dynamic_rendering(true);
    let mut enable_local = vk::PhysicalDeviceDynamicRenderingLocalReadFeaturesKHR::default()
        .dynamic_rendering_local_read(true);
    // The portability subset's features are turned on as offered: the backend uses none it
    // lacks, and naming them keeps the layer from reporting a use as unrequested.
    let mut enable_port = port;
    enable_port.p_next = core::ptr::null_mut();
    let mut device_extensions = vec![ash::khr::dynamic_rendering_local_read::NAME.as_ptr()];
    if subset {
        device_extensions.push(ash::khr::portability_subset::NAME.as_ptr());
    }
    let mut device_info = vk::DeviceCreateInfo::default()
        .queue_create_infos(&queue_infos)
        .enabled_extension_names(&device_extensions)
        .enabled_features(&enable_core)
        .push_next(&mut enable_v12)
        .push_next(&mut enable_v13)
        .push_next(&mut enable_local);
    if subset {
        device_info = device_info.push_next(&mut enable_port);
    }
    // SAFETY: the create info and every structure it chains live through the call; each
    // feature turned on is one the adapter reported.
    let device = unsafe { instance.create_device(physical, &device_info, None) }
        .map_err(|e| driver(e, "vkCreateDevice"))?;
    partial.device = Some(device.clone());
    // SAFETY: as above.
    let memory_props = unsafe { instance.get_physical_device_memory_properties(physical) };
    let mut memory = MemoryTypes {
        properties: memory_props.memory_types.map(|t| t.property_flags),
        count: memory_props.memory_type_count,
        block: None,
        lazy_colour: None,
        lazy_depth: None,
        non_coherent_atom: limits.non_coherent_atom_size.max(1),
    };
    let colour = image_requirements(
        &device,
        Format::Rgba8Unorm,
        ImageUsage::COLOUR.union(ImageUsage::SAMPLED),
    );
    let depth = image_requirements(
        &device,
        Format::Depth32Float,
        ImageUsage::DEPTH.union(ImageUsage::SAMPLED),
    );
    memory.block = memory.choose(colour.memory_type_bits & depth.memory_type_bits, Want::Device);
    let lazy = |bits: u32| {
        (0..memory.count).find(|&at| {
            bits & (1 << at) != 0
                && memory.properties[at as usize]
                    .contains(vk::MemoryPropertyFlags::LAZILY_ALLOCATED)
        })
    };
    let transient = ImageUsage::TRANSIENT;
    memory.lazy_colour = lazy(
        image_requirements(
            &device,
            Format::Rgba8Unorm,
            ImageUsage::COLOUR.union(ImageUsage::INPUT).union(transient),
        )
        .memory_type_bits,
    );
    memory.lazy_depth = lazy(
        image_requirements(&device, Format::Depth32Float, ImageUsage::DEPTH.union(transient))
            .memory_type_bits,
    );
    let all_types = (1u32 << memory.count) - 1;
    let coherent = [Want::Upload, Want::Readback]
        .iter()
        .all(|&want| memory.choose(all_types, want).is_some_and(|at| memory.coherent(at)));
    if [Want::Upload, Want::Readback].iter().any(|&want| memory.choose(all_types, want).is_none()) {
        return Err(unsupported(format!("{adapter} offers no host-visible memory")));
    }
    let caps = Capabilities {
        multi_draw_indirect: multi_draw,
        indirect_count,
        buffer_atomics_64: atomics64,
        memoryless_colour: memory.lazy_colour.is_some(),
        memoryless_depth: memory.lazy_depth.is_some(),
        // Whether a lazily allocated attachment keeps its contents across the local read's
        // barrier is no query's answer; MoltenVK loses depth there. Reported false, which
        // costs a caller a split it may not need and never a wrong image.
        memoryless_survives_local_read: false,
        coherent_host_memory: coherent,
        queues: QueueLayout {
            compute: if compute.is_some() { QueuePath::Own } else { QueuePath::Shared },
            // No surface: the swapchain lane gives the present role a queue.
            present: QueuePath::Shared,
        },
        heap_slots: request.heap_slots,
        samplers: request.samplers,
        colour_attachments: limits.max_color_attachments,
        compute_groups: limits.max_compute_work_group_count,
        viewport_extent: limits.max_viewport_dimensions,
        viewport_bounds: bounds(limits.viewport_bounds_range, properties.vendor_id),
    };
    let least = Capabilities::LEAST;
    let below = caps.colour_attachments < least.colour_attachments
        || (0..3).any(|at| caps.compute_groups[at] < least.compute_groups[at])
        || (0..2).any(|at| caps.viewport_extent[at] < least.viewport_extent[at])
        || caps.viewport_bounds[0] > least.viewport_bounds[0]
        || caps.viewport_bounds[1] < least.viewport_bounds[1];
    if below {
        return Err(unsupported(format!("{adapter}'s limits are below the least a device offers")));
    }
    let timeline = |device: &ash::Device| -> Result<vk::Semaphore> {
        let mut kind = vk::SemaphoreTypeCreateInfo::default()
            .semaphore_type(vk::SemaphoreType::TIMELINE)
            .initial_value(0);
        let info = vk::SemaphoreCreateInfo::default().push_next(&mut kind);
        // SAFETY: the device is live and the info valid.
        unsafe { device.create_semaphore(&info, None) }.map_err(|e| driver(e, "vkCreateSemaphore"))
    };
    let mut queues = Vec::with_capacity(2);
    let make_queue = |family: u32, index: u32, queues: &mut Vec<Queue>| -> Result<()> {
        // SAFETY: the family and index are among those the device was made with.
        let queue = unsafe { device.get_device_queue(family, index) };
        let graphics = family_props[family as usize].queue_flags.contains(vk::QueueFlags::GRAPHICS);
        let semaphore = timeline(&device)?;
        queues.push(Queue { queue, family, graphics, timeline: semaphore });
        Ok(())
    };
    let made = make_queue(graphics_family, 0, &mut queues).and_then(|()| {
        compute.map_or(Ok(()), |(family, index)| make_queue(family, index, &mut queues))
    });
    if let Err(error) = made {
        for queue in &queues {
            // SAFETY: semaphores this call made, used by nothing yet.
            unsafe { device.destroy_semaphore(queue.timeline, None) };
        }
        return Err(error);
    }
    let compute_index = if compute.is_some() { 1 } else { 0 };
    let text = |raw: &[core::ffi::c_char]| {
        let bytes: Vec<u8> = raw.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let info = DeviceInfo {
        backend: "vulkan",
        adapter,
        driver: format!("{} {}", text(&driver_props.driver_name), text(&driver_props.driver_info))
            .trim()
            .to_owned(),
    };
    let mut cache_identity = Vec::with_capacity(24);
    cache_identity.extend_from_slice(&properties.vendor_id.to_le_bytes());
    cache_identity.extend_from_slice(&properties.device_id.to_le_bytes());
    cache_identity.extend_from_slice(&properties.pipeline_cache_uuid);
    // Every object is the `Raw`'s from here: the partial record lets them go.
    let instance = partial.instance.take().ok_or_else(|| unsupported("no instance"))?;
    let debug = partial.debug.take();
    let device = partial.device.take().ok_or_else(|| unsupported("no device"))?;
    let sink = partial.sink.take().ok_or_else(|| unsupported("no sink"))?;
    let raw = Raw {
        _entry: entry.clone(),
        instance,
        physical,
        device,
        debug,
        _sink: sink,
        queues,
        memory,
        properties,
        leaking: AtomicBool::new(false),
    };
    Ok(Opened { raw: Arc::new(raw), caps, info, roles: [0, compute_index, 0], cache_identity })
}

#[cfg(test)]
mod tests {
    use ash::vk;

    use super::{APPLE, bounds, families};

    fn family(flags: vk::QueueFlags, count: u32) -> vk::QueueFamilyProperties {
        vk::QueueFamilyProperties { queue_flags: flags, queue_count: count, ..Default::default() }
    }

    #[test]
    fn compute_takes_a_family_that_does_not_draw_first() {
        let all = vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE | vk::QueueFlags::TRANSFER;
        let compute = vk::QueueFlags::COMPUTE | vk::QueueFlags::TRANSFER;
        // MoltenVK's four families of one queue each.
        assert_eq!(families(&[family(all, 1); 4]), Some((0, Some((1, 0)))));
        // lavapipe's one family of one queue.
        assert_eq!(families(&[family(all, 1)]), Some((0, None)));
        assert_eq!(families(&[family(all, 2)]), Some((0, Some((0, 1)))));
        assert_eq!(
            families(&[family(all, 2), family(all, 1), family(compute, 1)]),
            Some((0, Some((2, 0))))
        );
        assert_eq!(families(&[family(compute, 1)]), None);
    }

    #[test]
    fn viewport_bounds_round_inward_and_hold_to_what_apple_honours() {
        assert_eq!(bounds([-32768.0, 32767.0], APPLE), [-16384, 16383]);
        assert_eq!(bounds([-32768.0, 32767.0], 0x10DE), [-32768, 32767]);
        assert_eq!(bounds([-8192.5, 8191.5], 0x10DE), [-8192, 8191]);
    }
}
