//! Pipelines: the checks a cooked SPIR-V stage passes, the stage keys the cache holds, and
//! the cache's sealed bytes. Null runs none of a module's code; it reads the module's
//! header and its capability and extension instructions.

use std::collections::HashSet;
use std::time::Instant;

use lactose_core::{Error, ErrorKind, Result, xxh64};
use lactose_rhi_api::{
    CacheIdentity, CacheStart, Capabilities, ComputePipelineDesc, ConstantValue,
    GraphicsPipelineDesc, HeapArrayKind, Rule, ShaderStage, ShaderTarget, StageDesc, WriteMask,
    check_varyings, open_cache, seal_cache,
};

use crate::device::{ComputeObj, GraphicsObj};
use crate::records::PipelineState;

/// What null's sealed cache carries as its driver's identity.
const IDENTITY: CacheIdentity<'static> = CacheIdentity(b"null");

/// The first word of a SPIR-V module.
const SPIRV_MAGIC: u32 = 0x0723_0203;

/// The SPIR-V versions a module may declare: 1.0 to 1.6.
const SPIRV_VERSIONS: core::ops::RangeInclusive<u32> = 0x0001_0000..=0x0001_0600;

/// `OpExtension` and `OpCapability`.
const OP_EXTENSION: u32 = 10;
const OP_CAPABILITY: u32 = 17;

/// The capabilities the required set enables (`lactose-rhi-api`'s README, "Pipelines and
/// the cooked shader format"): `Shader`, `Int64`, `InputAttachment`,
/// `StorageImageReadWithoutFormat`, `ShaderNonUniform`,
/// `SampledImageArrayNonUniformIndexing` and `PhysicalStorageBufferAddresses`.
const ENABLED: [u32; 7] = [1, 11, 40, 55, 5301, 5307, 5347];

/// `Int64Atomics`, enabled where `buffer_atomics_64` is reported.
const INT64_ATOMICS: u32 = 12;

/// The one extension a module may name.
const EXTENSION: &str = "SPV_KHR_physical_storage_buffer";

/// How the cache starts from a request's bytes, and the stages it holds.
pub(crate) fn open(bytes: &[u8]) -> (CacheStart, HashSet<u64>) {
    if bytes.is_empty() {
        return (CacheStart::Empty, HashSet::new());
    }
    let Ok(payload) = open_cache(bytes, IDENTITY) else {
        return (CacheStart::Rejected, HashSet::new());
    };
    if !payload.len().is_multiple_of(8) {
        return (CacheStart::Rejected, HashSet::new());
    }
    let stages = payload.as_chunks::<8>().0.iter().map(|key| u64::from_le_bytes(*key)).collect();
    (CacheStart::Loaded, stages)
}

/// The cache's bytes: its stage keys, in order, sealed.
pub(crate) fn seal(stages: &HashSet<u64>) -> Result<Vec<u8>> {
    let mut keys: Vec<u64> = stages.iter().copied().collect();
    keys.sort_unstable();
    let payload: Vec<u8> = keys.iter().flat_map(|key| key.to_le_bytes()).collect();
    seal_cache(IDENTITY, &payload)
}

/// Counts a pipeline made from `stages`, each a hit where the cache held it.
pub(crate) fn count(state: &mut PipelineState, stages: &[u64], start: Instant) {
    state.stats.pipelines += 1;
    for &key in stages {
        if state.stages.insert(key) {
            state.stats.stage_misses += 1;
        } else {
            state.stats.stage_hits += 1;
        }
    }
    state.stats.creation += start.elapsed();
}

fn invalid(what: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidData, what.into())
}

/// Checks one stage, in the place `stage` names, and returns its cache key.
fn stage(caps: &Capabilities, desc: &StageDesc<'_>, place: ShaderStage) -> Result<u64> {
    let shader = desc.shader;
    if shader.target != ShaderTarget::SpirV {
        return Err(Error::new(
            ErrorKind::Unsupported,
            format!("a shader for {:?}: null takes SPIR-V alone", shader.target),
        ));
    }
    if shader.stage != place {
        return Err(invalid(format!("a {:?} shader as the {place:?} stage", shader.stage)));
    }
    module(caps, &shader.code)?;
    shader.check()?;
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
    let mut bytes = shader.encode()?;
    for constant in desc.constants {
        bytes.extend_from_slice(&constant.id.to_le_bytes());
        bytes.extend_from_slice(&value_bytes(constant.value));
    }
    Ok(xxh64(&bytes, 0))
}

/// A constant's value as the stage's key holds it, its type being its slot's.
fn value_bytes(value: ConstantValue) -> [u8; 4] {
    match value {
        ConstantValue::Bool(value) => u32::from(value).to_le_bytes(),
        ConstantValue::Int(value) => value.to_le_bytes(),
        ConstantValue::Uint(value) => value.to_le_bytes(),
        ConstantValue::Float(value) => value.to_le_bytes(),
    }
}

/// Checks a SPIR-V module's header, and its capabilities and extensions against those the
/// device enabled.
fn module(caps: &Capabilities, code: &[u8]) -> Result<()> {
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
    let mut at = 5;
    while at < words.len() {
        let count = (words[at] >> 16) as usize;
        let opcode = words[at] & 0xffff;
        if count == 0 || at + count > words.len() {
            return Err(invalid("a SPIR-V instruction whose word count leaves the module"));
        }
        let operands = &words[at + 1..at + count];
        match opcode {
            OP_CAPABILITY => {
                let capability = operands.first().copied().unwrap_or(u32::MAX);
                let enabled = ENABLED.contains(&capability)
                    || (capability == INT64_ATOMICS && caps.buffer_atomics_64);
                if !enabled {
                    return Err(Rule::Capability.refuse(format!(
                        "a module that declares capability {capability}, which the device did not enable"
                    )));
                }
            }
            OP_EXTENSION => {
                let bytes: Vec<u8> = operands.iter().flat_map(|word| word.to_le_bytes()).collect();
                let name = bytes.split(|&byte| byte == 0).next().unwrap_or_default();
                if name != EXTENSION.as_bytes() {
                    return Err(Rule::Capability.refuse(format!(
                        "a module that names extension {}, which the device did not enable",
                        String::from_utf8_lossy(name)
                    )));
                }
            }
            _ => {}
        }
        at += count;
    }
    Ok(())
}

/// Checks a graphics pipeline's description, and returns what its handle resolves to and
/// its stages' keys.
pub(crate) fn graphics(
    caps: &Capabilities,
    desc: &GraphicsPipelineDesc<'_>,
) -> Result<(GraphicsObj, Vec<u64>)> {
    let mut keys = vec![stage(caps, &desc.vertex, ShaderStage::Vertex)?];
    if let Some(fragment) = &desc.fragment {
        keys.push(stage(caps, fragment, ShaderStage::Fragment)?);
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
    let mut writes = 0;
    for (at, target) in desc.colour.iter().enumerate() {
        if target.format.is_depth() {
            return Err(invalid(format!("colour target {at} of a depth format")));
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
        writes |= bit;
    }
    if let Some(depth) = &desc.depth
        && !depth.format.is_depth()
    {
        return Err(invalid("a depth target of a format that is not a depth format"));
    }
    let object = GraphicsObj {
        colour: desc.colour.iter().map(|target| target.format).collect(),
        depth: desc.depth.map(|depth| depth.format),
        inputs,
        writes,
    };
    Ok((object, keys))
}

/// Checks a compute pipeline's description.
pub(crate) fn compute(
    caps: &Capabilities,
    desc: &ComputePipelineDesc<'_>,
) -> Result<(ComputeObj, Vec<u64>)> {
    Ok((ComputeObj, vec![stage(caps, &desc.stage, ShaderStage::Compute)?]))
}
