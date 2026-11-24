//! The renderer's two pipelines and the check of the cooked shaders they are made from
//! (`README.md`, "Pipelines and shaders"): each shader's reflection is held to "The shader
//! interface" before it reaches the device.

use lactose_core::{Error, ErrorKind, Result};
use lactose_math::render::RenderPosition;
use lactose_rhi_api::{
    ColourTargetDesc, Compare, ComputePipelineDesc, ComputePipelineHandle, Constant, ConstantSlot,
    ConstantType, ConstantValue, CookedShader, Cull, DepthDesc, Device, Format,
    GraphicsPipelineDesc, GraphicsPipelineHandle, ShaderStage, StageDesc, WriteMask,
};

use crate::shader::{
    CELL_SIZE_CONSTANT, SCATTER_GROUP, SCATTER_PUSH_BYTES, Shaders, TRIANGLE_PUSH_BYTES,
};

/// The depth image's format.
pub(crate) const DEPTH: Format = Format::Depth32Float;

/// The pipelines a renderer made with shaders holds.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Pipelines {
    pub(crate) scatter: ComputePipelineHandle,
    pub(crate) triangles: GraphicsPipelineHandle,
}

/// A refusal naming the shader and the field.
fn refuse(shader: &str, field: &str, what: impl core::fmt::Display) -> Error {
    Error::new(ErrorKind::InvalidData, format!("the {shader} shader's {field}: {what}"))
}

/// Holds one shader's reflection to the interface: its stage, its push block's bytes, one
/// of `push`, no heap, the constants `constants`, no input attachment, the outputs
/// `outputs`, and the workgroup `workgroup`.
fn hold(
    name: &str,
    shader: &CookedShader,
    stage: ShaderStage,
    push: &[u32],
    constants: &[ConstantSlot],
    outputs: u32,
    workgroup: Option<[u32; 3]>,
) -> Result<()> {
    if shader.stage != stage {
        return Err(refuse(name, "stage", format_args!("{:?}, not {stage:?}", shader.stage)));
    }
    if !push.contains(&shader.push_bytes) {
        let bytes = shader.push_bytes;
        return Err(refuse(name, "push block", format_args!("{bytes} bytes, not {push:?}")));
    }
    if shader.heap.is_some() {
        return Err(refuse(name, "heap", "the interface uses none"));
    }
    for slot in &shader.constants {
        match constants.iter().find(|named| named.id == slot.id) {
            None => {
                return Err(refuse(name, "constants", format_args!("id {} is not named", slot.id)));
            }
            Some(named) if named.ty != slot.ty => {
                let (id, ty, want) = (slot.id, slot.ty, named.ty);
                return Err(refuse(
                    name,
                    "constants",
                    format_args!("id {id} is {ty:?}, not {want:?}"),
                ));
            }
            Some(_) => {}
        }
    }
    if let Some(missing) =
        constants.iter().find(|named| !shader.constants.iter().any(|s| s.id == named.id))
    {
        return Err(refuse(name, "constants", format_args!("id {} is missing", missing.id)));
    }
    if shader.inputs != 0 {
        return Err(refuse(name, "inputs", format_args!("{:#b}, not none", shader.inputs)));
    }
    if shader.outputs != outputs {
        let (have, want) = (shader.outputs, outputs);
        return Err(refuse(name, "outputs", format_args!("{have:#b}, not {want:#b}")));
    }
    if shader.workgroup != workgroup {
        let (have, want) = (shader.workgroup, workgroup);
        return Err(refuse(name, "workgroup", format_args!("{have:?}, not {want:?}")));
    }
    Ok(())
}

/// Holds the three shaders to "The shader interface".
pub(crate) fn check(shaders: &Shaders<'_>) -> Result<()> {
    let cell = ConstantSlot { id: CELL_SIZE_CONSTANT, ty: ConstantType::Float };
    let scatter = Some([SCATTER_GROUP, 1, 1]);
    hold("scatter", shaders.scatter, ShaderStage::Compute, &[SCATTER_PUSH_BYTES], &[], 0, scatter)?;
    hold("vertex", shaders.vertex, ShaderStage::Vertex, &[TRIANGLE_PUSH_BYTES], &[cell], 0, None)?;
    let push = [0, TRIANGLE_PUSH_BYTES];
    hold("fragment", shaders.fragment, ShaderStage::Fragment, &push, &[], 1, None)
}

/// Makes the scatter's pipeline and the triangle pass's. On an error nothing is left made.
pub(crate) fn make(device: &dyn Device, shaders: &Shaders<'_>) -> Result<Pipelines> {
    let stage = |shader| StageDesc { shader, constants: &[] };
    let desc = ComputePipelineDesc { label: "cheddar::scatter", stage: stage(shaders.scatter) };
    let scatter = device.create_compute_pipeline(&desc)?;
    let cell = [Constant {
        id: CELL_SIZE_CONSTANT,
        value: ConstantValue::Float(RenderPosition::CELL_SIZE),
    }];
    let desc = GraphicsPipelineDesc {
        label: "cheddar::triangles",
        vertex: StageDesc { shader: shaders.vertex, constants: &cell },
        fragment: Some(stage(shaders.fragment)),
        colour: &[ColourTargetDesc { format: shaders.target, write_mask: WriteMask::ALL }],
        depth: Some(DepthDesc { format: DEPTH, compare: Compare::Less, write: true }),
        cull: Cull::None,
    };
    match device.create_graphics_pipeline(&desc) {
        Ok(triangles) => Ok(Pipelines { scatter, triangles }),
        Err(error) => {
            // A pipeline no submit has used is destroyed without refusal.
            let _ = device.destroy_compute_pipeline(scatter);
            Err(error)
        }
    }
}
