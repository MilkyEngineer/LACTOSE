//! Pipelines: the checks a cooked SPIR-V stage passes before the driver sees it, the
//! pipeline the driver makes from it, and the pipeline cache with its sealed bytes.
//!
//! A stage's module is scanned for the capabilities and the extension it declares, so a
//! module the device did not enable is refused by the contract's rule
//! ([`Rule::Capability`]) and never handed to the driver.

use std::sync::Arc;
use std::time::Instant;

use ash::vk;
use lactose_core::{Error, ErrorKind, Result};
use lactose_rhi_api::{
    CacheIdentity, CacheStart, Capabilities, ComputePipelineDesc, ConstantValue,
    GraphicsPipelineDesc, HeapArrayKind, PipelineStats, Rule, ShaderStage, ShaderTarget, StageDesc,
    WriteMask, check_varyings, open_cache, seal_cache,
};

use crate::convert;
use crate::device::{ComputeObj, GraphicsObj, Shared};
use crate::raw::{Raw, driver};

/// The first word of a SPIR-V module.
const SPIRV_MAGIC: u32 = 0x0723_0203;
/// The SPIR-V versions a module may declare: 1.0 to 1.6.
const SPIRV_VERSIONS: core::ops::RangeInclusive<u32> = 0x0001_0000..=0x0001_0600;
/// `OpExtension`, `OpEntryPoint`, `OpExecutionMode`, `OpCapability` and `OpDecorate`.
const OP_EXTENSION: u32 = 10;
const OP_ENTRY_POINT: u32 = 15;
const OP_EXECUTION_MODE: u32 = 16;
const OP_CAPABILITY: u32 = 17;
const OP_DECORATE: u32 = 71;
/// The execution mode `LocalSize`, the decoration `BuiltIn` and the built-in
/// `WorkgroupSize`.
const LOCAL_SIZE: u32 = 17;
const BUILT_IN: u32 = 11;
const WORKGROUP_SIZE: u32 = 25;
/// The capabilities the required set enables (`lactose-rhi-api`'s README, "Pipelines and
/// the cooked shader format"): `Shader`, `Int64`, `InputAttachment`,
/// `StorageImageReadWithoutFormat`, `ShaderNonUniform`,
/// `SampledImageArrayNonUniformIndexing` and `PhysicalStorageBufferAddresses`.
const ENABLED: [u32; 7] = [1, 11, 40, 55, 5301, 5307, 5347];
/// `Int64Atomics`, enabled where `buffer_atomics_64` is reported.
const INT64_ATOMICS: u32 = 12;
/// The one extension a module may name.
const EXTENSION: &str = "SPV_KHR_physical_storage_buffer";
/// The bytes of a Vulkan pipeline cache's own header, version one: its length, its
/// version, the vendor, the device and the cache's identifier.
const DRIVER_HEADER: usize = 32;

/// The driver's pipeline cache, and the identity its sealed bytes carry.
pub(crate) struct Cache {
    raw: Arc<Raw>,
    pub(crate) cache: vk::PipelineCache,
    identity: Vec<u8>,
}

impl Cache {
    /// Makes the cache from a request's bytes: started from their payload when the seal,
    /// the payload's own header and the driver all take it, and empty otherwise.
    pub(crate) fn open(
        raw: &Arc<Raw>,
        identity: Vec<u8>,
        bytes: &[u8],
    ) -> Result<(Self, CacheStart)> {
        let payload = if bytes.is_empty() {
            None
        } else {
            open_cache(bytes, CacheIdentity(&identity))
                .ok()
                .filter(|payload| header_matches(raw, payload))
        };
        let make = |data: &[u8]| {
            let info = vk::PipelineCacheCreateInfo::default().initial_data(data);
            // SAFETY: the device is live; the data is the payload a seal of this device's
            // identity framed, whose header names this device, and the driver checks the rest.
            unsafe { raw.device.create_pipeline_cache(&info, None) }
        };
        let (cache, start) = match payload {
            None if bytes.is_empty() => (make(&[]), CacheStart::Empty),
            None => (make(&[]), CacheStart::Rejected),
            Some(payload) => match make(payload) {
                Ok(cache) => (Ok(cache), CacheStart::Loaded),
                Err(_) => (make(&[]), CacheStart::Rejected),
            },
        };
        let cache = cache.map_err(|e| driver(e, "vkCreatePipelineCache"))?;
        Ok((Self { raw: Arc::clone(raw), cache, identity }, start))
    }

    /// The cache's bytes, sealed under the device's identity.
    pub(crate) fn seal(&self) -> Result<Vec<u8>> {
        // SAFETY: the cache is live; the driver synchronises its reads of it.
        let payload = unsafe { self.raw.device.get_pipeline_cache_data(self.cache) }
            .map_err(|e| driver(e, "vkGetPipelineCacheData"))?;
        seal_cache(CacheIdentity(&self.identity), &payload)
    }
}

impl Drop for Cache {
    fn drop(&mut self) {
        if !self.raw.leaking() {
            // SAFETY: the cache drops with the device, after every pipeline creation.
            unsafe { self.raw.device.destroy_pipeline_cache(self.cache, None) };
        }
    }
}

/// Whether a payload's own header is version one's and names this device: a stale one is
/// never handed to the driver.
fn header_matches(raw: &Raw, payload: &[u8]) -> bool {
    if payload.len() < DRIVER_HEADER {
        return false;
    }
    let word = |at: usize| {
        u32::from_le_bytes([payload[at], payload[at + 1], payload[at + 2], payload[at + 3]])
    };
    let properties = &raw.properties;
    word(0) as usize >= DRIVER_HEADER
        && word(4) == vk::PipelineCacheHeaderVersion::ONE.as_raw() as u32
        && word(8) == properties.vendor_id
        && word(12) == properties.device_id
        && payload[16..32] == properties.pipeline_cache_uuid
}

/// Adds a creation to the counts.
pub(crate) fn count(stats: &mut PipelineStats, hits: u64, misses: u64, start: Instant) {
    stats.pipelines += 1;
    stats.stage_hits += hits;
    stats.stage_misses += misses;
    stats.creation += start.elapsed();
}

fn invalid(what: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidData, what.into())
}

/// A stage as the driver takes it: its module's words and its specialisation data, each
/// constant four bytes (a boolean as a `VkBool32`).
struct Stage {
    words: Vec<u32>,
    entry: std::ffi::CString,
    entries: Vec<vk::SpecializationMapEntry>,
    data: Vec<u8>,
}

/// Checks a stage against the contract and the device, and returns it as the driver takes
/// it. The cooked format's own rules come first ([`CookedShader::check`](lactose_rhi_api::CookedShader::check)): nothing of a stage
/// that breaks them reaches the driver.
fn stage(caps: &Capabilities, desc: &StageDesc<'_>, place: ShaderStage) -> Result<Stage> {
    let shader = desc.shader;
    if shader.target != ShaderTarget::SpirV {
        return Err(Error::new(
            ErrorKind::Unsupported,
            format!("a shader for {:?}: Vulkan takes SPIR-V alone", shader.target),
        ));
    }
    if shader.stage != place {
        return Err(invalid(format!("a {:?} shader as the {place:?} stage", shader.stage)));
    }
    shader.check()?;
    let module = module(&shader.code, caps.buffer_atomics_64)?;
    if let Some(size) = shader.workgroup {
        let stated = module.workgroup(&shader.entry);
        if stated != Some(size) {
            return Err(invalid(format!(
                "a compute stage of workgroup {size:?} whose module states {stated:?} for `{}`",
                shader.entry
            )));
        }
    }
    let mut entries = Vec::with_capacity(desc.constants.len());
    let mut data = Vec::with_capacity(desc.constants.len() * 4);
    for (at, constant) in desc.constants.iter().enumerate() {
        let Some(slot) = shader.constants.iter().find(|slot| slot.id == constant.id) else {
            return Err(invalid(format!("a constant {} the shader does not declare", constant.id)));
        };
        if constant.value.ty() != slot.ty {
            return Err(invalid(format!(
                "a constant {} given a value of {:?}, declared {:?}",
                constant.id, constant.value, slot.ty
            )));
        }
        if desc.constants[..at].iter().any(|other| other.id == constant.id) {
            return Err(invalid(format!("a constant {} given twice", constant.id)));
        }
        entries.push(vk::SpecializationMapEntry {
            constant_id: constant.id,
            offset: u32::try_from(data.len()).unwrap_or(u32::MAX),
            size: 4,
        });
        data.extend_from_slice(&specialisation(constant.value));
    }
    if let Some(heap) = &shader.heap {
        for array in &heap.arrays {
            let capacity = match array.kind {
                HeapArrayKind::SampledImage2d => caps.heap_slots,
                HeapArrayKind::Sampler => caps.samplers,
            };
            if array.capacity != capacity {
                return Err(invalid(format!(
                    "a heap array of {:?} for {} entries; the device's holds {capacity}",
                    array.kind, array.capacity
                )));
            }
        }
    }
    let entry = std::ffi::CString::new(shader.entry.as_str())
        .map_err(|_| invalid("an entry point's name with a zero byte"))?;
    Ok(Stage { words: module.words, entry, entries, data })
}

/// A constant's specialisation data: four little-endian bytes, a `Bool` widened to a
/// `VkBool32` of 0 or 1, since a map entry is of its constant's size and a boolean's is
/// `sizeof(VkBool32)` (VUID-VkSpecializationMapEntry-constantID-00776).
fn specialisation(value: ConstantValue) -> [u8; 4] {
    match value {
        ConstantValue::Bool(value) => vk::Bool32::from(value).to_le_bytes(),
        ConstantValue::Int(value) => value.to_le_bytes(),
        ConstantValue::Uint(value) => value.to_le_bytes(),
        ConstantValue::Float(value) => value.to_le_bytes(),
    }
}

/// A checked SPIR-V module: its words, and what its entry points state of their workgroups.
struct Module {
    words: Vec<u32>,
    /// Each entry point's name and id.
    entries: Vec<(Vec<u8>, u32)>,
    /// Each `OpExecutionMode LocalSize`: the entry point's id and its three sizes.
    local_sizes: Vec<(u32, [u32; 3])>,
    /// Whether an id is decorated `BuiltIn WorkgroupSize`, which sets the workgroup in place
    /// of `LocalSize`.
    workgroup_builtin: bool,
}

impl Module {
    /// The workgroup the module states for `entry`: its one `LocalSize`, and `None` where it
    /// has none, more than one, or a `BuiltIn WorkgroupSize` overrides it.
    fn workgroup(&self, entry: &str) -> Option<[u32; 3]> {
        if self.workgroup_builtin {
            return None;
        }
        let mut ids = self.entries.iter().filter(|(name, _)| name == entry.as_bytes());
        let (_, id) = ids.next()?;
        let mut sizes = self.local_sizes.iter().filter(|(of, _)| of == id);
        match (sizes.next(), sizes.next()) {
            (Some(&(_, size)), None) => Some(size),
            _ => None,
        }
    }
}

/// The bytes of a SPIR-V literal string, up to its first zero byte.
fn literal(operands: &[u32]) -> Vec<u8> {
    let bytes: Vec<u8> = operands.iter().flat_map(|word| word.to_le_bytes()).collect();
    bytes.split(|&byte| byte == 0).next().unwrap_or_default().to_vec()
}

/// Checks a SPIR-V module's header, and its capabilities and extensions against those the
/// device enabled, `Int64Atomics` where `int64_atomics`; returns its words with what its
/// entry points state of their workgroups.
fn module(code: &[u8], int64_atomics: bool) -> Result<Module> {
    if !code.len().is_multiple_of(4) || code.len() < 20 {
        return Err(invalid("a SPIR-V module that is not whole words, or shorter than its header"));
    }
    let words: Vec<u32> =
        code.as_chunks::<4>().0.iter().map(|word| u32::from_le_bytes(*word)).collect();
    if words[0] != SPIRV_MAGIC {
        return Err(invalid("a module whose magic is not SPIR-V's"));
    }
    if !SPIRV_VERSIONS.contains(&words[1]) {
        return Err(invalid(format!("a SPIR-V module of version {:#x}", words[1])));
    }
    let mut entries = Vec::new();
    let mut local_sizes = Vec::new();
    let mut workgroup_builtin = false;
    let mut at = 5;
    while at < words.len() {
        let count = (words[at] >> 16) as usize;
        let opcode = words[at] & 0xffff;
        if count == 0 || at + count > words.len() {
            return Err(invalid("a SPIR-V instruction whose word count leaves the module"));
        }
        let operands = &words[at + 1..at + count];
        match (opcode, operands) {
            (OP_CAPABILITY, _) => {
                let capability = operands.first().copied().unwrap_or(u32::MAX);
                let enabled =
                    ENABLED.contains(&capability) || (capability == INT64_ATOMICS && int64_atomics);
                if !enabled {
                    return Err(Rule::Capability.refuse(format!(
                        "a module that declares capability {capability}, which the device did not enable"
                    )));
                }
            }
            (OP_EXTENSION, _) => {
                let name = literal(operands);
                if name != EXTENSION.as_bytes() {
                    return Err(Rule::Capability.refuse(format!(
                        "a module that names extension {}, which the device did not enable",
                        String::from_utf8_lossy(&name)
                    )));
                }
            }
            (OP_ENTRY_POINT, [_, id, name @ ..]) => entries.push((literal(name), *id)),
            (OP_EXECUTION_MODE, &[id, LOCAL_SIZE, x, y, z]) => local_sizes.push((id, [x, y, z])),
            (OP_DECORATE, &[_, BUILT_IN, WORKGROUP_SIZE]) => workgroup_builtin = true,
            _ => {}
        }
        at += count;
    }
    Ok(Module { words, entries, local_sizes, workgroup_builtin })
}

/// Shader modules made for one creation, destroyed when it ends however it ends.
struct Modules<'a> {
    raw: &'a Raw,
    made: Vec<vk::ShaderModule>,
}

impl Modules<'_> {
    fn make(&mut self, stage: &Stage) -> Result<vk::ShaderModule> {
        let info = vk::ShaderModuleCreateInfo::default().code(&stage.words);
        // SAFETY: the device is live; the words are a whole SPIR-V module whose header and
        // capabilities were checked.
        let module = unsafe { self.raw.device.create_shader_module(&info, None) }
            .map_err(|e| driver(e, "vkCreateShaderModule"))?;
        self.made.push(module);
        Ok(module)
    }
}

impl Drop for Modules<'_> {
    fn drop(&mut self) {
        for module in self.made.drain(..) {
            // SAFETY: the creation that used the module has returned.
            unsafe { self.raw.device.destroy_shader_module(module, None) };
        }
    }
}

/// The hits and misses the driver's creation feedback reports for the stages.
fn feedback(stages: &[vk::PipelineCreationFeedback]) -> (u64, u64) {
    let valid = stages
        .iter()
        .filter(|stage| stage.flags.contains(vk::PipelineCreationFeedbackFlags::VALID));
    let hits = valid
        .clone()
        .filter(|stage| {
            stage.flags.contains(vk::PipelineCreationFeedbackFlags::APPLICATION_PIPELINE_CACHE_HIT)
        })
        .count() as u64;
    (hits, valid.count() as u64 - hits)
}

/// Checks a graphics pipeline's description and makes it; returns it with its stages'
/// cache hits and misses.
pub(crate) fn graphics(
    shared: &Shared,
    desc: &GraphicsPipelineDesc<'_>,
) -> Result<(GraphicsObj, u64, u64)> {
    let caps = &shared.caps;
    let vertex = stage(caps, &desc.vertex, ShaderStage::Vertex)?;
    let fragment = desc
        .fragment
        .as_ref()
        .map(|fragment| stage(caps, fragment, ShaderStage::Fragment))
        .transpose()?;
    if let Some(fragment) = &desc.fragment {
        check_varyings(desc.vertex.shader, fragment.shader)?;
    }
    let (inputs, outputs) =
        desc.fragment.map_or((0, 0), |fragment| (fragment.shader.inputs, fragment.shader.outputs));
    if desc.colour.len() > caps.colour_attachments as usize {
        return Err(Rule::Usage.refuse(format!(
            "a pipeline of {} colour targets, past the {} the device reports",
            desc.colour.len(),
            caps.colour_attachments
        )));
    }
    for (at, target) in desc.colour.iter().enumerate() {
        if target.format.is_depth() {
            return Err(invalid(format!("colour target {at} of a depth format")));
        }
        if convert::format(target.format) == vk::Format::UNDEFINED {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!("colour target {at} of {:?}", target.format),
            ));
        }
        if target.write_mask.0 & !WriteMask::ALL.0 != 0 {
            return Err(invalid(format!(
                "colour target {at} with a write mask of bits past alpha"
            )));
        }
        if target.write_mask == WriteMask::NONE {
            continue;
        }
        let bit = 1u32.checked_shl(at as u32).unwrap_or(0);
        if outputs & bit == 0 || inputs & bit != 0 {
            return Err(invalid(format!(
                "colour target {at} written, which the fragment stage does not write or reads as an input"
            )));
        }
    }
    if let Some(depth) = &desc.depth
        && !depth.format.is_depth()
    {
        return Err(invalid("a depth target of a format that is not a depth format"));
    }
    let raw = &*shared.raw;
    let mut modules = Modules { raw, made: Vec::new() };
    let vertex_module = modules.make(&vertex)?;
    let fragment_module = fragment.as_ref().map(|fragment| modules.make(fragment)).transpose()?;
    let vertex_spec =
        vk::SpecializationInfo::default().map_entries(&vertex.entries).data(&vertex.data);
    let fragment_spec = fragment.as_ref().map(|fragment| {
        vk::SpecializationInfo::default().map_entries(&fragment.entries).data(&fragment.data)
    });
    let mut stages = vec![
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vertex_module)
            .name(&vertex.entry)
            .specialization_info(&vertex_spec),
    ];
    if let (Some(fragment), Some(module), Some(spec)) = (&fragment, fragment_module, &fragment_spec)
    {
        stages.push(
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(module)
                .name(&fragment.entry)
                .specialization_info(spec),
        );
    }
    let colour_formats: Vec<_> =
        desc.colour.iter().map(|target| convert::format(target.format)).collect();
    let mut rendering =
        vk::PipelineRenderingCreateInfo::default().color_attachment_formats(&colour_formats);
    if let Some(depth) = &desc.depth {
        rendering = rendering.depth_attachment_format(convert::format(depth.format));
    }
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
    let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
    let viewport =
        vk::PipelineViewportStateCreateInfo::default().viewport_count(1).scissor_count(1);
    // Vulkan's anticlockwise is reckoned in framebuffer space, whose origin is the top
    // left, and so is the contract's front whatever the viewport's flip.
    let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(convert::cull(desc.cull))
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0);
    let multisample = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);
    let depth_state =
        desc.depth.map_or_else(vk::PipelineDepthStencilStateCreateInfo::default, |depth| {
            vk::PipelineDepthStencilStateCreateInfo::default()
                .depth_test_enable(true)
                .depth_write_enable(depth.write)
                .depth_compare_op(convert::compare(depth.compare))
        });
    let attachments: Vec<_> = desc
        .colour
        .iter()
        .map(|target| {
            vk::PipelineColorBlendAttachmentState::default()
                .color_write_mask(vk::ColorComponentFlags::from_raw(u32::from(target.write_mask.0)))
        })
        .collect();
    let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&attachments);
    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
    let mut stage_feedback = vec![vk::PipelineCreationFeedback::default(); stages.len()];
    let mut pipeline_feedback = vk::PipelineCreationFeedback::default();
    let mut feedback_info = vk::PipelineCreationFeedbackCreateInfo::default()
        .pipeline_creation_feedback(&mut pipeline_feedback)
        .pipeline_stage_creation_feedbacks(&mut stage_feedback);
    let mut info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&assembly)
        .viewport_state(&viewport)
        .rasterization_state(&rasterization)
        .multisample_state(&multisample)
        .color_blend_state(&blend)
        .dynamic_state(&dynamic)
        .layout(shared.heap.graphics)
        .push_next(&mut rendering)
        .push_next(&mut feedback_info);
    if desc.depth.is_some() {
        info = info.depth_stencil_state(&depth_state);
    }
    // SAFETY: the device, the cache and the layout are live; every stage's module passed the
    // contract's checks; the info and all it points at live through the call.
    let made = unsafe { raw.device.create_graphics_pipelines(shared.cache.cache, &[info], None) };
    let pipeline = made.map_err(|(_, e)| driver(e, "vkCreateGraphicsPipelines"))?[0];
    drop(modules);
    let (hits, misses) = feedback(&stage_feedback);
    let object = GraphicsObj {
        raw: Arc::clone(&shared.raw),
        pipeline,
        colour: desc.colour.iter().map(|target| target.format).collect(),
        depth: desc.depth.map(|depth| depth.format),
        inputs,
    };
    Ok((object, hits, misses))
}

/// Checks a compute pipeline's description and makes it.
pub(crate) fn compute(
    shared: &Shared,
    desc: &ComputePipelineDesc<'_>,
) -> Result<(ComputeObj, u64, u64)> {
    let stage = stage(&shared.caps, &desc.stage, ShaderStage::Compute)?;
    let raw = &*shared.raw;
    let mut modules = Modules { raw, made: Vec::new() };
    let module = modules.make(&stage)?;
    let spec = vk::SpecializationInfo::default().map_entries(&stage.entries).data(&stage.data);
    let mut stage_feedback = [vk::PipelineCreationFeedback::default()];
    let mut pipeline_feedback = vk::PipelineCreationFeedback::default();
    let mut feedback_info = vk::PipelineCreationFeedbackCreateInfo::default()
        .pipeline_creation_feedback(&mut pipeline_feedback)
        .pipeline_stage_creation_feedbacks(&mut stage_feedback);
    let info = vk::ComputePipelineCreateInfo::default()
        .stage(
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::COMPUTE)
                .module(module)
                .name(&stage.entry)
                .specialization_info(&spec),
        )
        .layout(shared.heap.compute)
        .push_next(&mut feedback_info);
    // SAFETY: as `graphics`'.
    let made = unsafe { raw.device.create_compute_pipelines(shared.cache.cache, &[info], None) };
    let pipeline = made.map_err(|(_, e)| driver(e, "vkCreateComputePipelines"))?[0];
    drop(modules);
    let (hits, misses) = feedback(&stage_feedback);
    Ok((ComputeObj { raw: Arc::clone(&shared.raw), pipeline }, hits, misses))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A SPIR-V 1.0 module declaring `Shader`, whose entry points are `entries`, each a
    /// name and an id, with `rest` after them: execution modes and decorations, each an
    /// opcode and its operands.
    fn words(entries: &[(&str, u32)], rest: &[(u32, &[u32])]) -> Vec<u8> {
        let mut words = vec![SPIRV_MAGIC, 0x0001_0000, 0, 16, 0];
        let mut op = |code: u32, operands: &[u32]| {
            words.push(((operands.len() as u32 + 1) << 16) | code);
            words.extend_from_slice(operands);
        };
        op(OP_CAPABILITY, &[1]);
        for &(name, id) in entries {
            let mut bytes = name.as_bytes().to_vec();
            bytes.resize((bytes.len() / 4 + 1) * 4, 0);
            let mut operands = vec![5, id];
            operands.extend(bytes.as_chunks::<4>().0.iter().map(|word| u32::from_le_bytes(*word)));
            op(OP_ENTRY_POINT, &operands);
        }
        for &(code, operands) in rest {
            op(code, operands);
        }
        words.iter().flat_map(|word| word.to_le_bytes()).collect()
    }

    /// The workgroup a module of `entries` and `rest` states for `entry`.
    fn stated(entries: &[(&str, u32)], rest: &[(u32, &[u32])], entry: &str) -> Option<[u32; 3]> {
        module(&words(entries, rest), false).expect("a module").workgroup(entry)
    }

    #[test]
    fn the_workgroup_is_the_entrys_one_local_size() {
        let two = [("cs_main", 3), ("cs_other", 4)];
        let sizes: [(u32, &[u32]); 2] = [
            (OP_EXECUTION_MODE, &[4, LOCAL_SIZE, 2, 2, 2]),
            (OP_EXECUTION_MODE, &[3, LOCAL_SIZE, 8, 4, 2]),
        ];
        assert_eq!(stated(&two, &sizes, "cs_main"), Some([8, 4, 2]));
        assert_eq!(stated(&two, &sizes, "cs_other"), Some([2, 2, 2]));
        assert_eq!(stated(&two, &sizes, "main"), None, "an entry the module does not have");
    }

    #[test]
    fn no_size_two_sizes_or_the_built_in_state_no_workgroup() {
        let one = [("main", 3)];
        let local: (u32, &[u32]) = (OP_EXECUTION_MODE, &[3, LOCAL_SIZE, 1, 1, 1]);
        assert_eq!(stated(&one, &[], "main"), None, "no LocalSize");
        // `LocalSizeId` (38), whose sizes are ids, through `OpExecutionModeId` (331).
        assert_eq!(stated(&one, &[(331, &[3, 38, 9, 9, 9])], "main"), None, "LocalSizeId");
        assert_eq!(stated(&one, &[local, local], "main"), None, "two LocalSize modes");
        let built_in: (u32, &[u32]) = (OP_DECORATE, &[7, BUILT_IN, WORKGROUP_SIZE]);
        assert_eq!(stated(&one, &[local, built_in], "main"), None, "BuiltIn WorkgroupSize");
        assert_eq!(stated(&one, &[local], "main"), Some([1, 1, 1]));
    }

    #[test]
    fn int64_atomics_is_refused_unless_enabled() {
        let code = words(&[("main", 3)], &[(OP_CAPABILITY, &[INT64_ATOMICS])]);
        assert!(module(&code, false).is_err());
        assert!(module(&code, true).is_ok());
    }

    #[test]
    fn a_bool_is_widened_to_a_vk_bool32() {
        assert_eq!(specialisation(ConstantValue::Bool(true)), [1, 0, 0, 0]);
        assert_eq!(specialisation(ConstantValue::Bool(false)), [0; 4]);
        assert_eq!(specialisation(ConstantValue::Int(-3)), (-3i32).to_le_bytes());
        assert_eq!(specialisation(ConstantValue::Uint(7)), [7, 0, 0, 0]);
        assert_eq!(specialisation(ConstantValue::Float(1.5)), 1.5f32.to_le_bytes());
    }
}
