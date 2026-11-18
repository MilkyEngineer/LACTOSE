//! Cooked shaders, pipelines and the cache.

use lactose_core::ErrorKind;

use super::kit::{Body, Ctx, INT64_ATOMICS, attachment, begin, cooked, heap_of, image_to, spirv};
use crate::shader::MAGIC;
use crate::{
    COOKED_SHADER_VERSION, CacheIdentity, CacheStart, ColourTargetDesc, ComponentType,
    ComputePipelineDesc, Constant, ConstantSlot, ConstantType, CookedShader, Cull, Format,
    GraphicsPipelineDesc, HeapArrayKind, ImageState, Load, PUSH_BYTES_MAX, PassReach, QueueRole,
    ShaderStage, ShaderTarget, StageDesc, Store, Submit, VARYING_LOCATIONS, Varying,
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
    ("shader_for_another_heap_is_refused", shader_for_another_heap_is_refused),
    (
        "shader_target_the_backend_does_not_run_is_unsupported",
        shader_target_the_backend_does_not_run_is_unsupported,
    ),
    ("constant_of_another_length_is_refused", constant_of_another_length_is_refused),
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

/// A compute stage whose workgroup is `size`.
fn group(size: [u32; 3]) -> CookedShader {
    CookedShader { workgroup: Some(size), ..cooked(ShaderStage::Compute, 0, 0) }
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
    constants: &[Constant<'_>],
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

fn constant_of_another_length_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let shader = CookedShader {
        constants: vec![ConstantSlot { id: 2, ty: ConstantType::Uint }],
        ..cooked(ShaderStage::Compute, 0, 0)
    };
    let short = [Constant { id: 2, bytes: &[1, 0] }];
    ctx.fails(
        compute(&*device, &shader, &short),
        ErrorKind::InvalidData,
        "a constant of another length",
    );
    let undeclared = [Constant { id: 7, bytes: &[1, 0, 0, 0] }];
    ctx.fails(
        compute(&*device, &shader, &undeclared),
        ErrorKind::InvalidData,
        "a constant the shader does not declare",
    );
    let right = [Constant { id: 2, bytes: &[1, 0, 0, 0] }];
    ctx.ok(compute(&*device, &shader, &right), "the declared constant");
    ctx.ok(compute(&*device, &shader, &[]), "no constants");
    Ok(())
}

fn constant_of_another_type_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let shader = CookedShader {
        constants: vec![
            ConstantSlot { id: 1, ty: ConstantType::Bool },
            ConstantSlot { id: 2, ty: ConstantType::Int },
            ConstantSlot { id: 4, ty: ConstantType::Float },
        ],
        ..cooked(ShaderStage::Compute, 0, 0)
    };
    // A bool's value is one byte: the four of a `VkBool32` are the backend's to widen to.
    let refused: [(&str, Constant<'_>); 4] = [
        ("a bool of four bytes", Constant { id: 1, bytes: &[1, 0, 0, 0] }),
        ("a bool that is neither 0 nor 1", Constant { id: 1, bytes: &[2] }),
        ("an int of one byte", Constant { id: 2, bytes: &[1] }),
        ("a float of one byte", Constant { id: 4, bytes: &[0] }),
    ];
    for (what, constant) in refused {
        ctx.fails(compute(&*device, &shader, &[constant]), ErrorKind::InvalidData, what);
    }
    let int = (-3i32).to_le_bytes();
    let float = 1.5f32.to_le_bytes();
    let right = [
        Constant { id: 1, bytes: &[1] },
        Constant { id: 2, bytes: &int },
        Constant { id: 4, bytes: &float },
    ];
    ctx.ok(compute(&*device, &shader, &right), "a value of each declared type");
    ctx.ok(compute(&*device, &shader, &[Constant { id: 1, bytes: &[0] }]), "a false bool");
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
        code: spirv(ShaderStage::Compute, &[INT64_ATOMICS]),
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
    ctx.check(stats.stage_hits + stats.stage_misses >= 1, "the device counts its pipeline's stage");
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
