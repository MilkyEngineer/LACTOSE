//! Cooked shaders, pipelines and the cache.

use lactose_core::{ErrorKind, xxh64};

use super::kit::{Body, Ctx, INT64_ATOMICS, attachment, begin, cooked, heap_of, image_to, spirv};
use crate::shader::MAGIC;
use crate::{
    COOKED_SHADER_VERSION, CacheIdentity, CacheStart, ColourTargetDesc, ComponentType,
    ComputePipelineDesc, Constant, ConstantSlot, ConstantType, ConstantValue, CookedShader, Cull,
    Format, GraphicsPipelineDesc, HeapArrayKind, ImageState, Load, PUSH_BYTES_MAX, PassReach,
    QueueRole, ShaderStage, ShaderTarget, StageDesc, Store, Submit, VARYING_LOCATIONS, Varying,
    WORKGROUP_INVOCATIONS_MAX, WORKGROUP_SIZE_MAX, WriteMask, open_cache, seal_cache,
};

type Applies = Result<(), &'static str>;

pub(super) const BODIES: &[(&str, Body)] = &[
    ("cooked_shader_round_trips", cooked_shader_round_trips),
    ("cooked_shader_of_another_version_is_refused", cooked_shader_of_another_version_is_refused),
    (
        "cooked_shader_corrupt_or_truncated_is_refused",
        cooked_shader_corrupt_or_truncated_is_refused,
    ),
    (
        "cooked_shader_forged_under_a_valid_checksum_is_refused",
        cooked_shader_forged_under_a_valid_checksum_is_refused,
    ),
    ("shader_for_another_heap_is_refused", shader_for_another_heap_is_refused),
    (
        "shader_target_the_backend_does_not_run_is_unsupported",
        shader_target_the_backend_does_not_run_is_unsupported,
    ),
    (
        "constant_the_shader_does_not_declare_is_refused",
        constant_the_shader_does_not_declare_is_refused,
    ),
    ("constant_of_another_type_is_refused", constant_of_another_type_is_refused),
    (
        "workgroup_outside_its_stage_or_bounds_is_refused",
        workgroup_outside_its_stage_or_bounds_is_refused,
    ),
    (
        "varying_the_vertex_stage_does_not_write_is_refused",
        varying_the_vertex_stage_does_not_write_is_refused,
    ),
    (
        "shader_needing_an_unenabled_capability_is_refused",
        shader_needing_an_unenabled_capability_is_refused,
    ),
    (
        "pipeline_of_other_formats_than_its_pass_is_refused",
        pipeline_of_other_formats_than_its_pass_is_refused,
    ),
    ("sealed_cache_round_trips", sealed_cache_round_trips),
    (
        "cache_corrupt_truncated_or_foreign_starts_empty",
        cache_corrupt_truncated_or_foreign_starts_empty,
    ),
];

/// A fragment shader that uses every field of the format.
fn rich(ctx: &Ctx<'_>) -> CookedShader {
    CookedShader {
        heap: Some(heap_of(&ctx.set)),
        constants: vec![
            ConstantSlot { id: 0, ty: ConstantType::Uint },
            ConstantSlot { id: 3, ty: ConstantType::Bool },
            ConstantSlot { id: 4, ty: ConstantType::Int },
            ConstantSlot { id: 9, ty: ConstantType::Float },
        ],
        push_bytes: 64,
        varyings: vec![
            Varying { location: 0, component: ComponentType::Float, count: 2 },
            Varying { location: 3, component: ComponentType::Uint, count: 1 },
            Varying { location: VARYING_LOCATIONS - 1, component: ComponentType::Int, count: 4 },
        ],
        ..cooked(ShaderStage::Fragment, 0b10, 0b101)
    }
}

/// A compute stage whose workgroup is `size`, its module's `LocalSize`.
fn group(size: [u32; 3]) -> CookedShader {
    CookedShader {
        code: spirv(ShaderStage::Compute, Some(size), &[]),
        workgroup: Some(size),
        ..cooked(ShaderStage::Compute, 0, 0)
    }
}

/// Varyings, each a location, a component type and a count.
type Reads = &'static [(u32, ComponentType, u32)];

/// A vertex or fragment stage of `varyings`.
fn varying(stage: ShaderStage, varyings: Reads) -> CookedShader {
    let varyings = varyings
        .iter()
        .map(|&(location, component, count)| Varying { location, component, count })
        .collect();
    let outputs = u32::from(stage == ShaderStage::Fragment);
    CookedShader { varyings, ..cooked(stage, 0, outputs) }
}

/// A graphics pipeline of `vertex` and `fragment` writing one `Rgba8Unorm` target.
fn pair<'a>(vertex: &'a CookedShader, fragment: &'a CookedShader) -> GraphicsPipelineDesc<'a> {
    GraphicsPipelineDesc {
        label: "conformance",
        vertex: StageDesc { shader: vertex, constants: &[] },
        fragment: Some(StageDesc { shader: fragment, constants: &[] }),
        colour: &[ColourTargetDesc { format: Format::Rgba8Unorm, write_mask: WriteMask::ALL }],
        depth: None,
        cull: Cull::None,
    }
}

/// Makes a compute pipeline of `shader`, with `constants`.
fn compute(
    device: &dyn crate::Device,
    shader: &CookedShader,
    constants: &[Constant],
) -> lactose_core::Result<crate::ComputePipelineHandle> {
    device.create_compute_pipeline(&ComputePipelineDesc {
        label: "conformance",
        stage: StageDesc { shader, constants },
    })
}

fn cooked_shader_round_trips(ctx: &Ctx<'_>) -> Applies {
    let most = WORKGROUP_SIZE_MAX;
    let shaders = [
        rich(ctx),
        cooked(ShaderStage::Vertex, 0, 0),
        varying(ShaderStage::Vertex, &[(1, ComponentType::Float, 4), (2, ComponentType::Int, 3)]),
        cooked(ShaderStage::Compute, 0, 0),
        group([64, 2, 1]),
        group([most[0], 1, 1]),
        group([1, most[1], 1]),
        group([1, 2, most[2]]),
    ];
    for shader in shaders {
        let bytes = ctx.ok(shader.encode(), "an encode");
        ctx.check(
            bytes == ctx.ok(shader.encode(), "an encode"),
            "equal shaders encode to equal bytes",
        );
        ctx.check(
            ctx.ok(CookedShader::decode(&bytes), "a decode") == shader,
            "a shader round-trips",
        );
    }
    // What `decode` refuses, `encode` refuses too: the cooker writes no file the engine
    // cannot read.
    let mut heap = heap_of(&ctx.set);
    heap.set += 1;
    let mut twice = heap_of(&ctx.set);
    twice.arrays[1].kind = twice.arrays[0].kind;
    let breaches = [
        ("a heap set other than HEAP_SET", CookedShader { heap: Some(heap), ..rich(ctx) }),
        ("a heap array kind twice", CookedShader { heap: Some(twice), ..rich(ctx) }),
        (
            "a push block above the most",
            CookedShader { push_bytes: PUSH_BYTES_MAX + 1, ..rich(ctx) },
        ),
        (
            "constants out of order",
            CookedShader {
                constants: vec![
                    ConstantSlot { id: 3, ty: ConstantType::Uint },
                    ConstantSlot { id: 0, ty: ConstantType::Uint },
                ],
                ..rich(ctx)
            },
        ),
        ("inputs outside the fragment stage", cooked(ShaderStage::Vertex, 1, 0)),
        ("outputs outside the fragment stage", cooked(ShaderStage::Compute, 0, 1)),
        (
            "a compute stage without a workgroup",
            CookedShader { workgroup: None, ..cooked(ShaderStage::Compute, 0, 0) },
        ),
        (
            "a workgroup in the vertex stage",
            CookedShader { workgroup: Some([1, 1, 1]), ..cooked(ShaderStage::Vertex, 0, 0) },
        ),
        (
            "a workgroup in the fragment stage",
            CookedShader { workgroup: Some([1, 1, 1]), ..rich(ctx) },
        ),
        ("a workgroup of no invocations in x", group([0, 1, 1])),
        ("a workgroup of no invocations in z", group([64, 1, 0])),
        ("a workgroup past the most in x", group([most[0] + 1, 1, 1])),
        ("a workgroup past the most in y", group([1, most[1] + 1, 1])),
        ("a workgroup past the most in z", group([1, 1, most[2] + 1])),
        ("a workgroup of more invocations than the most", group([WORKGROUP_INVOCATIONS_MAX, 2, 1])),
        ("a workgroup whose product wraps", group([u32::MAX, u32::MAX, 1])),
        (
            "varyings in the compute stage",
            CookedShader { varyings: rich(ctx).varyings, ..cooked(ShaderStage::Compute, 0, 0) },
        ),
        (
            "varyings out of order",
            varying(
                ShaderStage::Vertex,
                &[(2, ComponentType::Float, 1), (1, ComponentType::Float, 1)],
            ),
        ),
        (
            "a location twice",
            varying(
                ShaderStage::Vertex,
                &[(1, ComponentType::Float, 1), (1, ComponentType::Uint, 1)],
            ),
        ),
        (
            "a location from VARYING_LOCATIONS",
            varying(ShaderStage::Fragment, &[(VARYING_LOCATIONS, ComponentType::Float, 1)]),
        ),
        (
            "a varying of no components",
            varying(ShaderStage::Vertex, &[(0, ComponentType::Float, 0)]),
        ),
        (
            "a varying of five components",
            varying(ShaderStage::Vertex, &[(0, ComponentType::Int, 5)]),
        ),
    ];
    for (what, shader) in breaches {
        ctx.fails(shader.encode(), ErrorKind::InvalidData, what);
    }
    Ok(())
}

fn cooked_shader_of_another_version_is_refused(ctx: &Ctx<'_>) -> Applies {
    let mut bytes = ctx.ok(rich(ctx).encode(), "an encode");
    let at = MAGIC.len();
    bytes[at..at + 4].copy_from_slice(&(COOKED_SHADER_VERSION + 1).to_le_bytes());
    ctx.fails(CookedShader::decode(&bytes), ErrorKind::VersionMismatch, "a later version");
    bytes[at..at + 4].copy_from_slice(&0u32.to_le_bytes());
    ctx.fails(CookedShader::decode(&bytes), ErrorKind::VersionMismatch, "version zero");
    Ok(())
}

fn cooked_shader_corrupt_or_truncated_is_refused(ctx: &Ctx<'_>) -> Applies {
    let bytes = ctx.ok(rich(ctx).encode(), "an encode");
    for at in [0, MAGIC.len() + 5, bytes.len() / 2, bytes.len() - 1] {
        let mut corrupt = bytes.clone();
        corrupt[at] ^= 0x40;
        ctx.fails(CookedShader::decode(&corrupt), ErrorKind::InvalidData, "a byte flipped");
    }
    for len in [0, 3, MAGIC.len() + 4, bytes.len() / 2, bytes.len() - 1] {
        ctx.fails(CookedShader::decode(&bytes[..len]), ErrorKind::InvalidData, "a truncation");
    }
    let mut longer = bytes;
    longer.push(0);
    ctx.fails(CookedShader::decode(&longer), ErrorKind::InvalidData, "a byte appended");
    Ok(())
}

/// Writes `words` into `body` from `from_end` bytes before its end.
fn put(body: &mut [u8], from_end: usize, words: &[u32]) {
    let at = body.len() - from_end;
    for (word, bytes) in words.iter().zip(body[at..].as_chunks_mut::<4>().0) {
        *bytes = word.to_le_bytes();
    }
}

/// Replaces the varyings that end `body`, `from_end` bytes from its end with their count,
/// by `varyings`, each a location, a component type's code and a count.
fn put_varyings(body: &mut Vec<u8>, from_end: usize, varyings: &[(u32, u8, u32)]) {
    body.truncate(body.len() - from_end);
    body.extend_from_slice(&(varyings.len() as u32).to_le_bytes());
    for &(location, code, count) in varyings {
        body.extend_from_slice(&location.to_le_bytes());
        body.push(code);
        body.extend_from_slice(&count.to_le_bytes());
    }
}

fn cooked_shader_forged_under_a_valid_checksum_is_refused(ctx: &Ctx<'_>) -> Applies {
    // A writer that skipped `check`: each breach is written into a valid encoding, and the
    // checksum made again. The compute stage's encoding ends with its last constant's type
    // code, the push block, inputs, outputs, the workgroup's tag and size, and no varyings;
    // the vertex stage's with no workgroup and one varying.
    let compute = CookedShader {
        constants: vec![ConstantSlot { id: 1, ty: ConstantType::Float }],
        ..group([8, 4, 2])
    };
    let vertex = varying(ShaderStage::Vertex, &[(3, ComponentType::Int, 2)]);
    type Forge = fn(&mut Vec<u8>);
    let forged: [(&str, &CookedShader, Forge); 15] = [
        ("a compute stage without a workgroup", &compute, |body| {
            let at = body.len() - 17;
            body[at] = 0;
            body.drain(at + 1..at + 13);
        }),
        ("a workgroup tag of 2", &compute, |body| {
            let at = body.len() - 17;
            body[at] = 2;
        }),
        ("a workgroup of no invocations in x", &compute, |body| put(body, 16, &[0, 1, 1])),
        ("a workgroup past the most in x", &compute, |body| put(body, 16, &[129, 1, 1])),
        ("a workgroup past the most in z", &compute, |body| put(body, 16, &[1, 1, 65])),
        ("a workgroup of more invocations than the most", &compute, |body| {
            put(body, 16, &[16, 16, 1]);
        }),
        ("a workgroup whose product wraps", &compute, |body| put(body, 16, &[65536, 65536, 1])),
        ("a push block above the most", &compute, |body| put(body, 29, &[PUSH_BYTES_MAX + 1])),
        ("varyings in the compute stage", &compute, |body| put_varyings(body, 4, &[(0, 0, 1)])),
        ("a constant type code of 4", &compute, |body| {
            let at = body.len() - 30;
            body[at] = 4;
        }),
        ("a workgroup in the vertex stage", &vertex, |body| {
            let at = body.len() - 14;
            body[at] = 1;
            let size = [1u32, 1, 1].map(u32::to_le_bytes);
            body.splice(at + 1..at + 1, size.into_iter().flatten());
        }),
        ("a varying component code of 3", &vertex, |body| {
            let at = body.len() - 5;
            body[at] = 3;
        }),
        ("a varying at VARYING_LOCATIONS", &vertex, |body| put(body, 9, &[VARYING_LOCATIONS])),
        ("a varying of no components", &vertex, |body| put(body, 4, &[0])),
        ("varyings out of order", &vertex, |body| {
            put_varyings(body, 13, &[(3, 0, 1), (2, 0, 1)]);
        }),
    ];
    let seal = |shader: &CookedShader, forge: Forge| {
        let bytes = ctx.ok(shader.encode(), "an encode");
        let mut body = bytes[..bytes.len() - 8].to_vec();
        forge(&mut body);
        let sum = xxh64(&body, 0);
        body.extend_from_slice(&sum.to_le_bytes());
        body
    };
    for shader in [&compute, &vertex] {
        let resealed = seal(shader, |_| {});
        ctx.check(
            ctx.ok(CookedShader::decode(&resealed), "a decode") == *shader,
            "a shader resealed unchanged decodes",
        );
    }
    for (what, shader, forge) in forged {
        ctx.fails(CookedShader::decode(&seal(shader, forge)), ErrorKind::InvalidData, what);
    }
    Ok(())
}

fn shader_for_another_heap_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    for kind in [HeapArrayKind::SampledImage2d, HeapArrayKind::Sampler] {
        let mut heap = heap_of(&ctx.set);
        for array in heap.arrays.iter_mut().filter(|array| array.kind == kind) {
            array.capacity += 1;
        }
        let shader = CookedShader { heap: Some(heap), ..cooked(ShaderStage::Compute, 0, 0) };
        ctx.fails(
            compute(&*device, &shader, &[]),
            ErrorKind::InvalidData,
            "a heap array of another capacity",
        );
    }
    let shader =
        CookedShader { heap: Some(heap_of(&ctx.set)), ..cooked(ShaderStage::Compute, 0, 0) };
    ctx.ok(compute(&*device, &shader, &[]), "the device's own heap");
    Ok(())
}

fn shader_target_the_backend_does_not_run_is_unsupported(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let spirv = cooked(ShaderStage::Compute, 0, 0);
    match compute(&*device, &spirv, &[]) {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::Unsupported => {
            return Err("the backend runs no SPIR-V: a Metal backend's case");
        }
        Err(error) => ctx.fail(format_args!("a SPIR-V pipeline failed: {error}")),
    }
    let msl = CookedShader { target: ShaderTarget::Msl, ..spirv };
    ctx.fails(compute(&*device, &msl, &[]), ErrorKind::Unsupported, "an MSL target");
    let library = CookedShader { target: ShaderTarget::MetalLibrary, ..msl };
    ctx.fails(compute(&*device, &library, &[]), ErrorKind::Unsupported, "a Metal library target");
    Ok(())
}

fn constant_the_shader_does_not_declare_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let shader = CookedShader {
        constants: vec![ConstantSlot { id: 2, ty: ConstantType::Uint }],
        ..cooked(ShaderStage::Compute, 0, 0)
    };
    let one = Constant { id: 2, value: ConstantValue::Uint(1) };
    let undeclared = [Constant { id: 7, value: ConstantValue::Uint(1) }];
    ctx.fails(
        compute(&*device, &shader, &undeclared),
        ErrorKind::InvalidData,
        "a constant the shader does not declare",
    );
    ctx.fails(compute(&*device, &shader, &[one, one]), ErrorKind::InvalidData, "a constant twice");
    ctx.ok(compute(&*device, &shader, &[one]), "the declared constant");
    ctx.ok(compute(&*device, &shader, &[]), "no constants");
    Ok(())
}

fn constant_of_another_type_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let shader = CookedShader {
        constants: vec![
            ConstantSlot { id: 1, ty: ConstantType::Bool },
            ConstantSlot { id: 2, ty: ConstantType::Int },
            ConstantSlot { id: 3, ty: ConstantType::Uint },
            ConstantSlot { id: 4, ty: ConstantType::Float },
        ],
        ..cooked(ShaderStage::Compute, 0, 0)
    };
    // Each value is one whose bits another type holds as well: the type alone refuses it.
    let refused = [
        ("a uint for a bool", Constant { id: 1, value: ConstantValue::Uint(1) }),
        ("a uint for an int", Constant { id: 2, value: ConstantValue::Uint(3) }),
        ("an int for a uint", Constant { id: 3, value: ConstantValue::Int(3) }),
        ("a uint for a float", Constant { id: 4, value: ConstantValue::Uint(1.5f32.to_bits()) }),
        ("a bool for a float", Constant { id: 4, value: ConstantValue::Bool(true) }),
    ];
    for (what, constant) in refused {
        ctx.fails(compute(&*device, &shader, &[constant]), ErrorKind::InvalidData, what);
    }
    let right = [
        Constant { id: 1, value: ConstantValue::Bool(true) },
        Constant { id: 2, value: ConstantValue::Int(-3) },
        Constant { id: 3, value: ConstantValue::Uint(7) },
        Constant { id: 4, value: ConstantValue::Float(1.5) },
    ];
    ctx.ok(compute(&*device, &shader, &right), "a value of each declared type");
    let false_bool = [Constant { id: 1, value: ConstantValue::Bool(false) }];
    ctx.ok(compute(&*device, &shader, &false_bool), "a false bool");
    Ok(())
}

fn workgroup_outside_its_stage_or_bounds_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let most = WORKGROUP_SIZE_MAX;
    let refused = [
        (
            "a compute stage without a workgroup",
            CookedShader { workgroup: None, ..group([1, 1, 1]) },
        ),
        ("a workgroup of no invocations in y", group([1, 0, 1])),
        ("a workgroup past the most in x", group([most[0] + 1, 1, 1])),
        ("a workgroup of more invocations than the most", group([most[0], most[1], 1])),
        (
            "varyings in the compute stage",
            CookedShader { varyings: rich(ctx).varyings, ..group([1, 1, 1]) },
        ),
    ];
    for (what, shader) in refused {
        ctx.fails(compute(&*device, &shader, &[]), ErrorKind::InvalidData, what);
    }
    ctx.ok(compute(&*device, &group([8, 4, 2]), &[]), "a workgroup within the bounds");
    let vertex = CookedShader { workgroup: Some([1, 1, 1]), ..cooked(ShaderStage::Vertex, 0, 0) };
    let fragment = cooked(ShaderStage::Fragment, 0, 1);
    ctx.fails(
        device.create_graphics_pipeline(&pair(&vertex, &fragment)),
        ErrorKind::InvalidData,
        "a vertex stage with a workgroup",
    );
    Ok(())
}

fn varying_the_vertex_stage_does_not_write_is_refused(ctx: &Ctx<'_>) -> Applies {
    use ComponentType::{Float, Int, Uint};
    let device = ctx.device();
    let vertex = varying(ShaderStage::Vertex, &[(0, Float, 2), (1, Uint, 1), (2, Int, 3)]);
    let refused: [(&str, Reads); 5] = [
        ("a read of more components than are written", &[(0, Float, 4)]),
        ("a read of fewer components than are written", &[(2, Int, 2)]),
        ("a read of a uint as an int", &[(1, Int, 1)]),
        ("a read of a float as a uint", &[(0, Uint, 2)]),
        ("a read of a location not written", &[(0, Float, 2), (5, Float, 1)]),
    ];
    for (what, read) in refused {
        let fragment = varying(ShaderStage::Fragment, read);
        ctx.fails(
            device.create_graphics_pipeline(&pair(&vertex, &fragment)),
            ErrorKind::InvalidData,
            what,
        );
    }
    // A varying written and not read is no breach on either target.
    let read: [Reads; 3] =
        [&[], &[(0, Float, 2), (2, Int, 3)], &[(0, Float, 2), (1, Uint, 1), (2, Int, 3)]];
    for read in read {
        let fragment = varying(ShaderStage::Fragment, read);
        ctx.ok(
            device.create_graphics_pipeline(&pair(&vertex, &fragment)),
            "a read of what is written",
        );
    }
    Ok(())
}

fn shader_needing_an_unenabled_capability_is_refused(ctx: &Ctx<'_>) -> Applies {
    if ctx.set.buffer_atomics_64 {
        return Err("64-bit buffer atomics are reported");
    }
    let device = ctx.device();
    let shader = CookedShader {
        code: spirv(ShaderStage::Compute, Some([1, 1, 1]), &[INT64_ATOMICS]),
        ..cooked(ShaderStage::Compute, 0, 0)
    };
    ctx.refused(compute(&*device, &shader, &[]), "a module that declares `Int64Atomics`");
    Ok(())
}

fn pipeline_of_other_formats_than_its_pass_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let queue = QueueRole::Graphics;
    let target = ctx.target(&*device);
    let pipeline = ctx.graphics(&*device, &[(Format::Rgba16Float, WriteMask::ALL)], 0, None);
    let mut encoder = ctx.encoder(&*device, queue);
    encoder.transition_images(&[image_to(
        target,
        ImageState::Undefined,
        ImageState::ColourAttachment,
        queue,
    )]);
    begin(
        &mut *encoder,
        &[attachment(target, Load::Clear([0.0; 4]), Store::Store, false)],
        PassReach::NOTHING,
    );
    encoder.bind_graphics_pipeline(pipeline);
    encoder.draw(0..3, 0..1);
    encoder.end_pass();
    let refused =
        device.submit(queue, Submit { encoders: &mut [encoder], waits: &[], signals: &[] });
    ctx.refused(refused, "a pipeline of another colour format than its pass's");
    let depth = ctx.graphics(
        &*device,
        &[(Format::Rgba8Unorm, WriteMask::ALL)],
        0,
        Some(Format::Depth32Float),
    );
    let mut encoder = ctx.encoder(&*device, queue);
    encoder.transition_images(&[image_to(
        target,
        ImageState::Undefined,
        ImageState::ColourAttachment,
        queue,
    )]);
    begin(
        &mut *encoder,
        &[attachment(target, Load::Clear([0.0; 4]), Store::Store, false)],
        PassReach::NOTHING,
    );
    encoder.bind_graphics_pipeline(depth);
    encoder.draw(0..3, 0..1);
    encoder.end_pass();
    let refused =
        device.submit(queue, Submit { encoders: &mut [encoder], waits: &[], signals: &[] });
    ctx.refused(refused, "a pipeline with depth in a pass with none");
    let vertex = cooked(ShaderStage::Vertex, 0, 0);
    let fragment = cooked(ShaderStage::Fragment, 0, 1);
    let desc = GraphicsPipelineDesc {
        label: "conformance",
        vertex: StageDesc { shader: &vertex, constants: &[] },
        fragment: Some(StageDesc { shader: &fragment, constants: &[] }),
        colour: &[ColourTargetDesc { format: Format::Depth32Float, write_mask: WriteMask::ALL }],
        depth: None,
        cull: Cull::Back,
    };
    ctx.fails(
        device.create_graphics_pipeline(&desc),
        ErrorKind::InvalidData,
        "a colour target of a depth format",
    );
    // A pipeline of more colour targets than a pass may have: Vulkan's VUID 09533.
    let fragment = cooked(ShaderStage::Fragment, 0, 0);
    let unwritten = ColourTargetDesc { format: Format::Rgba8Unorm, write_mask: WriteMask::NONE };
    let colour = vec![unwritten; ctx.set.colour_attachments as usize + 1];
    let desc = GraphicsPipelineDesc {
        fragment: Some(StageDesc { shader: &fragment, constants: &[] }),
        colour: &colour,
        ..desc
    };
    ctx.refused(device.create_graphics_pipeline(&desc), "a pipeline of a target past the limit");
    let desc = GraphicsPipelineDesc { colour: &colour[1..], ..desc };
    ctx.ok(device.create_graphics_pipeline(&desc), "a pipeline of the most targets");
    Ok(())
}

const IDENTITY: CacheIdentity<'static> = CacheIdentity(b"conformance");

fn sealed(ctx: &Ctx<'_>, identity: CacheIdentity<'_>, payload: &[u8]) -> Vec<u8> {
    ctx.ok(seal_cache(identity, payload), "a seal")
}

/// The bytes of a device's cache, which holds the stage of `ctx.compute`.
fn warm_cache(ctx: &Ctx<'_>) -> Vec<u8> {
    let device = ctx.device();
    ctx.check(device.pipeline_stats().cache == CacheStart::Empty, "a cache started from no bytes");
    ctx.compute(&*device);
    let stats = device.pipeline_stats();
    ctx.check(stats.pipelines == 1, "the device counts its pipeline");
    // The stage counts come where the driver says so, and lavapipe says nothing of a stage:
    // a driver that reports counts the pipeline's one stage once. Where none is reported,
    // `warm_cache_hits_every_stage` skips and says so.
    let counted = stats.stage_hits + stats.stage_misses;
    ctx.check(counted <= 1, "the device counts its pipeline's one stage at most once");
    ctx.ok(device.pipeline_cache(), "the device's cache")
}

fn sealed_cache_round_trips(ctx: &Ctx<'_>) -> Applies {
    for payload in [&b""[..], b"a payload", &[0xff; 4096]] {
        let bytes = sealed(ctx, IDENTITY, payload);
        ctx.check(bytes == sealed(ctx, IDENTITY, payload), "equal payloads seal to equal bytes");
        ctx.check(
            ctx.ok(open_cache(&bytes, IDENTITY), "an open") == payload,
            "a payload round-trips",
        );
    }
    let bytes = warm_cache(ctx);
    let device = ctx.cached(bytes, "a device started from another's cache");
    ctx.check(device.pipeline_stats().cache == CacheStart::Loaded, "a device's own cache loads");
    ctx.compute(&*device);
    let stats = device.pipeline_stats();
    // Where the driver counts stages at all, the one the cache holds is a hit.
    let counted = stats.stage_hits + stats.stage_misses > 0;
    ctx.check(!counted || stats.stage_hits >= 1, "the cached stage is a hit");
    ctx.check(ctx.messages() == 0, "a cache that loads sends no message");
    Ok(())
}

fn cache_corrupt_truncated_or_foreign_starts_empty(ctx: &Ctx<'_>) -> Applies {
    let bytes = sealed(ctx, IDENTITY, b"a driver's cache");
    ctx.check(
        open_cache(&sealed(ctx, CacheIdentity(b"another"), b"a driver's cache"), IDENTITY).is_err(),
        "a foreign identity",
    );
    for len in [0, 4, bytes.len() / 2, bytes.len() - 1] {
        ctx.check(open_cache(&bytes[..len], IDENTITY).is_err(), "a truncation");
    }
    for at in [0, bytes.len() / 2, bytes.len() - 1] {
        let mut corrupt = bytes.clone();
        corrupt[at] ^= 1;
        ctx.check(open_cache(&corrupt, IDENTITY).is_err(), "a byte flipped");
    }
    // A backend's side: its own bytes spoiled, or bytes of another identity, start the
    // cache empty with no error and no message, and no bytes start it `Empty`.
    let own = warm_cache(ctx);
    let mut flipped = own.clone();
    let middle = flipped.len() / 2;
    flipped[middle] ^= 1;
    let mut spoiled = vec![
        ("its own bytes with a byte flipped", flipped),
        ("its own bytes truncated", own[..own.len() - 1].to_vec()),
        ("bytes of another identity", sealed(ctx, CacheIdentity(b"another"), &own)),
        ("bytes that are no sealed cache", b"not a cache".to_vec()),
    ];
    spoiled.push(("no bytes", Vec::new()));
    for (what, bytes) in spoiled {
        let expected = if bytes.is_empty() { CacheStart::Empty } else { CacheStart::Rejected };
        let device = ctx.cached(bytes, what);
        ctx.check(device.pipeline_stats().cache == expected, what);
        ctx.compute(&*device);
    }
    ctx.check(ctx.messages() == 0, "a cache that fails a check sends no message");
    Ok(())
}
