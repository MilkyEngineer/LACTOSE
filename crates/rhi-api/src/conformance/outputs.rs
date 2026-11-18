//! The output cases: each draws, dispatches or copies on a device, reads the result back
//! through a copy into a `Readback` buffer and compares it with a reference computed here,
//! never with a stored image (decision 5.10). Their shaders are `shaders/conformance/`'s,
//! cooked before the run and given by [`Harness::shader`](super::Harness::shader).

use super::kit::{
    Body, Ctx, EXTENT, attachment, begin, buffer_to, compute_pass, image_across, image_desc,
    image_to, whole_image,
};
use crate::{AddressMode, QueuePath};
use crate::{
    BufferCopy, BufferHandle, BufferImageCopy, BufferReach, BufferState, BufferUsage,
    ColourAttachment, ColourTargetDesc, Compare, ComputePipelineDesc, ComputePipelineHandle,
    Constant, ConstantValue, Cull, DepthAttachment, DepthDesc, Device, Encoder, Extent, Filter,
    Format, GraphicsPipelineDesc, GraphicsPipelineHandle, HeapSlot, ImageHandle, ImageMemory,
    ImageRange, ImageState, ImageUsage, IndexedIndirectArgs, Load, MemoryBlockDesc, MemoryClass,
    PassReach, QueueRole, Rect, RenderPass, SamplerDesc, ShaderStage, StageDesc, Store, Target,
    Viewport, WriteMask,
};

type Applies = Result<(), &'static str>;

pub(super) const BODIES: &[(&str, Body)] = &[
    (
        "device_memory_is_filled_through_a_staging_copy",
        device_memory_is_filled_through_a_staging_copy,
    ),
    ("memoryless_g_buffer_is_lit_in_its_pass", memoryless_g_buffer_is_lit_in_its_pass),
    ("aliased_transients_render_in_turn", aliased_transients_render_in_turn),
    ("image_round_trips_through_copies", image_round_trips_through_copies),
    ("ownership_transfer_keeps_the_contents", ownership_transfer_keeps_the_contents),
    ("retake_after_a_store_on_one_queue_is_clean", retake_after_a_store_on_one_queue_is_clean),
    ("heap_is_sampled_past_slot_zero", heap_is_sampled_past_slot_zero),
    ("address_is_read_in_a_shader", address_is_read_in_a_shader),
    ("declared_reader_follows_a_lagging_writer", declared_reader_follows_a_lagging_writer),
    ("sampler_filters_as_described", sampler_filters_as_described),
    ("specialised_constant_changes_the_output", specialised_constant_changes_the_output),
    (
        "specialised_constant_of_each_type_is_read_back",
        specialised_constant_of_each_type_is_read_back,
    ),
    ("write_mask_keeps_a_channel", write_mask_keeps_a_channel),
    ("depth_state_orders_two_triangles", depth_state_orders_two_triangles),
    ("warm_cache_hits_every_stage", warm_cache_hits_every_stage),
    ("two_threads_record_at_once", two_threads_record_at_once),
    ("triangle_matches_the_reference", triangle_matches_the_reference),
    ("local_read_sees_what_the_pass_wrote", local_read_sees_what_the_pass_wrote),
    ("clear_and_keep_load_as_described", clear_and_keep_load_as_described),
    ("viewport_and_scissor_place_the_triangles", viewport_and_scissor_place_the_triangles),
    ("draw_parameters_reach_the_shader", draw_parameters_reach_the_shader),
    ("indexed_indirect_draw_matches_the_reference", indexed_indirect_draw_matches_the_reference),
    ("indirect_count_draws_where_reported", indirect_count_draws_where_reported),
    ("dispatch_writes_through_an_address", dispatch_writes_through_an_address),
    (
        "dispatches_of_one_pass_see_each_others_writes",
        dispatches_of_one_pass_see_each_others_writes,
    ),
    ("dispatch_indirect_reads_its_counts", dispatch_indirect_reads_its_counts),
    ("dispatch_runs_workgroups_of_the_shaders_size", dispatch_runs_workgroups_of_the_shaders_size),
    ("each_validator_reports_its_canary", each_validator_reports_its_canary),
];

const G: QueueRole = QueueRole::Graphics;
const C: QueueRole = QueueRole::Compute;
const W: u32 = EXTENT.width;
const H: u32 = EXTENT.height;
const RGBA8: Format = Format::Rgba8Unorm;

/// Rounds a lagging dispatch spins before it writes: enough that the work after it would
/// overtake it on a GPU that ran the two at once.
const LAG: u32 = 1 << 18;

/// A channel of `k` 255ths, which an 8-bit fraction holds exactly.
fn unorm(k: u8) -> f32 {
    f32::from(k) / 255.0
}

/// A colour of four 255ths.
fn colour(rgba: [u8; 4]) -> [f32; 4] {
    rgba.map(unorm)
}

/// Four little-endian floats.
fn float_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|value| value.to_le_bytes()).collect()
}

/// Little-endian 32-bit words.
fn words(bytes: &[u8]) -> Vec<u32> {
    bytes.as_chunks::<4>().0.iter().map(|word| u32::from_le_bytes(*word)).collect()
}

/// Submits `encoders` to `role` and waits for them.
fn run(ctx: &Ctx<'_>, device: &dyn Device, role: QueueRole, encoders: &mut [Box<dyn Encoder>]) {
    let done = ctx.point(device, 1);
    ctx.submit(device, role, encoders, &[], &[done]);
    ctx.reached(device, done);
}

/// An `Upload` buffer a shader reads through its address, holding `bytes`, in `ShaderRead`
/// on `role` once `encoder` has run.
fn address_source(
    ctx: &Ctx<'_>,
    device: &dyn Device,
    encoder: &mut dyn Encoder,
    bytes: &[u8],
) -> BufferHandle {
    let buffer =
        ctx.buffer(device, bytes.len() as u64, BufferUsage::DEVICE_ADDRESS, MemoryClass::Upload);
    ctx.write(device, buffer, 0, bytes);
    let queue = encoder.queue();
    encoder.transition_buffers(&[buffer_to(
        buffer,
        BufferState::Undefined,
        BufferState::ShaderRead,
        queue,
    )]);
    buffer
}

/// A `Readback` buffer of `size` bytes in `CopyDestination` once `encoder` has run.
fn readback(
    ctx: &Ctx<'_>,
    device: &dyn Device,
    encoder: &mut dyn Encoder,
    size: u64,
) -> BufferHandle {
    let buffer = ctx.buffer(device, size, BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
    let queue = encoder.queue();
    encoder.transition_buffers(&[buffer_to(
        buffer,
        BufferState::Undefined,
        BufferState::CopyDestination,
        queue,
    )]);
    buffer
}

/// Records the copy of `buffer`, in `state` on the encoder's queue, into a new `Readback`
/// buffer, ready for the host to read once the encoder has run.
fn copy_out(
    ctx: &Ctx<'_>,
    device: &dyn Device,
    encoder: &mut dyn Encoder,
    buffer: BufferHandle,
    state: BufferState,
    size: u64,
) -> BufferHandle {
    let queue = encoder.queue();
    let out = readback(ctx, device, encoder, size);
    encoder.transition_buffers(&[buffer_to(buffer, state, BufferState::CopySource, queue)]);
    encoder.copy_buffer(
        buffer,
        out,
        &[BufferCopy { source_offset: 0, destination_offset: 0, size }],
    );
    encoder.transition_buffers(&[buffer_to(
        out,
        BufferState::CopyDestination,
        BufferState::HostRead,
        queue,
    )]);
    out
}

/// Records the copy of an [`EXTENT`] `Rgba8Unorm` image's first subresource, in `state` on
/// the encoder's queue, into a new `Readback` buffer of packed rows.
fn image_out(
    ctx: &Ctx<'_>,
    device: &dyn Device,
    encoder: &mut dyn Encoder,
    image: ImageHandle,
    state: ImageState,
) -> BufferHandle {
    let out = ctx.buffer(
        device,
        u64::from(W * H * 4),
        BufferUsage::COPY_DESTINATION,
        MemoryClass::Readback,
    );
    copy_image_out(encoder, image, state, out);
    out
}

/// Records [`image_out`]'s copy into `out`, a `Readback` buffer of the image's size in
/// `Undefined`.
fn copy_image_out(
    encoder: &mut dyn Encoder,
    image: ImageHandle,
    state: ImageState,
    out: BufferHandle,
) {
    let queue = encoder.queue();
    encoder.transition_buffers(&[buffer_to(
        out,
        BufferState::Undefined,
        BufferState::CopyDestination,
        queue,
    )]);
    encoder.transition_images(&[image_to(image, state, ImageState::CopySource, queue)]);
    encoder.copy_image_to_buffer(image, out, &[whole_image()]);
    encoder.transition_buffers(&[buffer_to(
        out,
        BufferState::CopyDestination,
        BufferState::HostRead,
        queue,
    )]);
}

/// The pixels of an [`EXTENT`] image `image_out` copied, each four bytes.
fn pixels(ctx: &Ctx<'_>, device: &dyn Device, out: BufferHandle) -> Vec<[u8; 4]> {
    ctx.read(device, out, 0, u64::from(W * H * 4)).as_chunks::<4>().0.to_vec()
}

/// A colour target of [`EXTENT`] that a copy reads.
fn target(ctx: &Ctx<'_>, device: &dyn Device) -> ImageHandle {
    ctx.image(device, &image_desc(RGBA8, EXTENT, ImageUsage::COLOUR.union(ImageUsage::COPY_SOURCE)))
}

/// A graphics pipeline of the suite's `<source>.vs_main` and `<source>.<fragment>`.
#[expect(clippy::too_many_arguments, reason = "a pipeline description, flattened for the cases")]
fn pipeline(
    ctx: &Ctx<'_>,
    device: &dyn Device,
    source: &str,
    fragment: &str,
    colour: &[(Format, WriteMask)],
    depth: Option<DepthDesc>,
    cull: Cull,
    constants: &[Constant],
) -> GraphicsPipelineHandle {
    let vertex = ctx.shader(&format!("{source}.vs_main"), ShaderStage::Vertex);
    let fragment = ctx.shader(&format!("{source}.{fragment}"), ShaderStage::Fragment);
    let targets: Vec<ColourTargetDesc> = colour
        .iter()
        .map(|&(format, write_mask)| ColourTargetDesc { format, write_mask })
        .collect();
    let desc = GraphicsPipelineDesc {
        label: "conformance",
        vertex: StageDesc { shader: &vertex, constants: &[] },
        fragment: Some(StageDesc { shader: &fragment, constants }),
        colour: &targets,
        depth,
        cull,
    };
    ctx.ok(device.create_graphics_pipeline(&desc), "a graphics pipeline")
}

/// The pipeline of `<source>.vs_main` and `<source>.fs_main` into one `Rgba8Unorm` target.
fn simple(ctx: &Ctx<'_>, device: &dyn Device, source: &str) -> GraphicsPipelineHandle {
    pipeline(ctx, device, source, "fs_main", &[(RGBA8, WriteMask::ALL)], None, Cull::None, &[])
}

/// The compute pipeline of `<source>.cs_main`.
fn kernel(ctx: &Ctx<'_>, device: &dyn Device, source: &str) -> ComputePipelineHandle {
    specialised_kernel(ctx, device, source, &[])
}

/// The compute pipeline of `<source>.cs_main`, specialised with `constants`.
fn specialised_kernel(
    ctx: &Ctx<'_>,
    device: &dyn Device,
    source: &str,
    constants: &[Constant],
) -> ComputePipelineHandle {
    let shader = ctx.shader(&format!("{source}.cs_main"), ShaderStage::Compute);
    let desc = ComputePipelineDesc {
        label: "conformance",
        stage: StageDesc { shader: &shader, constants },
    };
    ctx.ok(device.create_compute_pipeline(&desc), "a compute pipeline")
}

/// Runs `<source>.cs_main`, specialised with `constants`, once over `groups` into a new
/// device buffer of `words` words that it reaches through the address at push byte 0, the
/// buffer first filled with `fill`; returns every word of the buffer after the run.
fn dispatch_into(
    ctx: &Ctx<'_>,
    source: &str,
    constants: &[Constant],
    groups: [u32; 3],
    words_of: u32,
    fill: u32,
) -> Vec<u32> {
    let device = ctx.device();
    let pipeline = specialised_kernel(ctx, &*device, source, constants);
    let size = u64::from(words_of * 4);
    let filled: Vec<u8> = (0..words_of).flat_map(|_| fill.to_le_bytes()).collect();
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &filled);
    let written = ctx.buffer(
        &*device,
        size,
        BufferUsage::DEVICE_ADDRESS
            .union(BufferUsage::COPY_SOURCE)
            .union(BufferUsage::COPY_DESTINATION),
        MemoryClass::Device,
    );
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_buffers(&[
        buffer_to(source, BufferState::Undefined, BufferState::CopySource, G),
        buffer_to(written, BufferState::Undefined, BufferState::CopyDestination, G),
    ]);
    encoder.copy_buffer(
        source,
        written,
        &[BufferCopy { source_offset: 0, destination_offset: 0, size }],
    );
    encoder.transition_buffers(&[buffer_to(
        written,
        BufferState::CopyDestination,
        BufferState::ShaderWrite,
        G,
    )]);
    let declared = [BufferReach { buffer: written, state: BufferState::ShaderWrite }];
    compute_pass(&mut *encoder, pipeline, PassReach { slots: &[], buffers: &declared });
    encoder.push_address(0, written, 0);
    encoder.dispatch(groups);
    encoder.end_pass();
    let out = copy_out(ctx, &*device, &mut *encoder, written, BufferState::ShaderWrite, size);
    run(ctx, &*device, G, &mut [encoder]);
    words(&ctx.read(&*device, out, 0, size))
}

/// Begins a pass over `image`, in `Undefined` before it, cleared to `clear` and stored.
fn cleared(encoder: &mut dyn Encoder, image: ImageHandle, clear: [f32; 4], reach: PassReach<'_>) {
    let queue = encoder.queue();
    encoder.transition_images(&[image_to(
        image,
        ImageState::Undefined,
        ImageState::ColourAttachment,
        queue,
    )]);
    begin(encoder, &[attachment(image, Load::Clear(clear), Store::Store, false)], reach);
}

/// Draws `fill.slang`'s triangle over the whole of the open pass, of `rgba`.
fn fill(encoder: &mut dyn Encoder, pipeline: GraphicsPipelineHandle, rgba: [u8; 4]) {
    encoder.bind_graphics_pipeline(pipeline);
    encoder.push_bytes(0, &float_bytes(&colour(rgba)));
    encoder.draw(0..3, 0..1);
}

/// Whether pixel `(x, y)`'s centre lies inside the triangle `corners`, given in clip space
/// over the whole [`EXTENT`]: `Some(inside)` where it is more than half a pixel from every
/// edge's line, `None` where it is closer and the rasteriser's rule decides.
fn covers(corners: [[f32; 2]; 3], x: u32, y: u32) -> Option<bool> {
    let point = [(x as f32 + 0.5) / W as f32 * 2.0 - 1.0, 1.0 - (y as f32 + 0.5) / H as f32 * 2.0];
    // Half a pixel in clip space, whose pixel is 2 / W wide: a vertex half a pixel off
    // moves an edge across some pixel's centre that is compared.
    let half = 1.0 / W as f32;
    let mut inside = true;
    for at in 0..3 {
        let (a, b) = (corners[at], corners[(at + 1) % 3]);
        let edge = [b[0] - a[0], b[1] - a[1]];
        let cross = edge[0] * (point[1] - a[1]) - edge[1] * (point[0] - a[0]);
        let distance = cross / (edge[0] * edge[0] + edge[1] * edge[1]).sqrt();
        if distance.abs() < half {
            return None;
        }
        // Anticlockwise corners: the inside is to the left of every edge.
        inside &= distance > 0.0;
    }
    Some(inside)
}

/// Compares every pixel whose expectation is known with it, and fails the case naming the
/// first that differs.
fn compare(
    ctx: &Ctx<'_>,
    got: &[[u8; 4]],
    what: &str,
    expected: impl Fn(u32, u32) -> Option<[u8; 4]>,
) {
    for y in 0..H {
        for x in 0..W {
            let Some(want) = expected(x, y) else { continue };
            let have = got[(y * W + x) as usize];
            if have != want {
                ctx.fail(format_args!("{what}: pixel ({x}, {y}) is {have:?}, {want:?} expected"));
            }
        }
    }
}

/// Four positions, each a clip-space corner at depth `z`, for a shader that reads them
/// through an address.
fn positions(corners: &[[f32; 2]], z: f32) -> Vec<u8> {
    let values: Vec<f32> =
        corners.iter().flat_map(|corner| [corner[0], corner[1], z, 1.0]).collect();
    float_bytes(&values)
}

fn device_memory_is_filled_through_a_staging_copy(ctx: &Ctx<'_>) -> Applies {
    const SIZE: u64 = 64 * 1024;
    let device = ctx.device();
    let bytes: Vec<u8> = (0..SIZE).map(|at| (at * 31 % 251) as u8).collect();
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &bytes);
    let usage = BufferUsage::COPY_SOURCE.union(BufferUsage::COPY_DESTINATION);
    let middle = ctx.buffer(&*device, SIZE, usage, MemoryClass::Device);
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_buffers(&[
        buffer_to(source, BufferState::Undefined, BufferState::CopySource, G),
        buffer_to(middle, BufferState::Undefined, BufferState::CopyDestination, G),
    ]);
    encoder.copy_buffer(
        source,
        middle,
        &[BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }],
    );
    let out = copy_out(ctx, &*device, &mut *encoder, middle, BufferState::CopyDestination, SIZE);
    run(ctx, &*device, G, &mut [encoder]);
    ctx.check(ctx.read(&*device, out, 0, SIZE) == bytes, "the bytes came back through the device");
    Ok(())
}

fn image_round_trips_through_copies(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let usage = ImageUsage::COPY_SOURCE.union(ImageUsage::COPY_DESTINATION);
    let image = ctx.image(
        &*device,
        &crate::ImageDesc { mips: 2, layers: 2, ..image_desc(RGBA8, EXTENT, usage) },
    );
    // Each subresource's rows, one after another: mip 0's two layers, then mip 1's.
    let mut regions = Vec::new();
    let mut offset = 0;
    for mip in 0..2 {
        for layer in 0..2 {
            let extent = Extent { width: W >> mip, height: H >> mip };
            regions.push(BufferImageCopy {
                buffer_offset: offset,
                mip,
                layer,
                rect: Rect { x: 0, y: 0, extent },
            });
            offset += u64::from(extent.width * extent.height * 4);
        }
    }
    let bytes: Vec<u8> = (0..offset).map(|at| (at * 7 % 253) as u8).collect();
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &bytes);
    let all = ImageRange { mip: 0, mips: 2, layer: 0, layers: 2 };
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_buffers(&[buffer_to(
        source,
        BufferState::Undefined,
        BufferState::CopySource,
        G,
    )]);
    encoder.transition_images(&[image_across(
        image,
        all,
        (ImageState::Undefined, G),
        (ImageState::CopyDestination, G),
    )]);
    encoder.copy_buffer_to_image(source, image, &regions);
    encoder.transition_images(&[image_across(
        image,
        all,
        (ImageState::CopyDestination, G),
        (ImageState::CopySource, G),
    )]);
    let out = readback(ctx, &*device, &mut *encoder, offset);
    encoder.copy_image_to_buffer(image, out, &regions);
    encoder.transition_buffers(&[buffer_to(
        out,
        BufferState::CopyDestination,
        BufferState::HostRead,
        G,
    )]);
    run(ctx, &*device, G, &mut [encoder]);
    ctx.check(
        ctx.read(&*device, out, 0, offset) == bytes,
        "every mip and layer came back as it went in",
    );
    Ok(())
}

fn triangle_matches_the_reference(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let image = target(ctx, &*device);
    let pipeline = simple(ctx, &*device, "triangles");
    let corners = [[-0.8, -0.6], [0.7, -0.9], [0.1, 0.85]];
    let mut encoder = ctx.encoder(&*device, G);
    let source = address_source(ctx, &*device, &mut *encoder, &positions(&corners, 0.5));
    let reach = [BufferReach { buffer: source, state: BufferState::ShaderRead }];
    cleared(&mut *encoder, image, [0.0; 4], PassReach { slots: &[], buffers: &reach });
    encoder.bind_graphics_pipeline(pipeline);
    encoder.push_address(0, source, 0);
    encoder.push_bytes(16, &float_bytes(&colour([255, 128, 64, 255])));
    encoder.draw(0..3, 0..1);
    encoder.end_pass();
    let out = image_out(ctx, &*device, &mut *encoder, image, ImageState::ColourAttachment);
    run(ctx, &*device, G, &mut [encoder]);
    let got = pixels(ctx, &*device, out);
    compare(ctx, &got, "the triangle", |x, y| {
        covers(corners, x, y).map(|inside| if inside { [255, 128, 64, 255] } else { [0; 4] })
    });
    Ok(())
}

fn depth_state_orders_two_triangles(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let image = target(ctx, &*device);
    let depth_image =
        ctx.image(&*device, &image_desc(Format::Depth32Float, EXTENT, ImageUsage::DEPTH));
    let depth = DepthDesc { format: Format::Depth32Float, compare: Compare::Less, write: true };
    let pipeline = pipeline(
        ctx,
        &*device,
        "triangles",
        "fs_main",
        &[(RGBA8, WriteMask::ALL)],
        Some(depth),
        Cull::None,
        &[],
    );
    let near = [[-0.5, -0.5], [0.5, -0.5], [0.0, 0.5]];
    let far = [[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]];
    let mut bytes = positions(&near, 0.25);
    bytes.extend(positions(&far, 0.75));
    let mut encoder = ctx.encoder(&*device, G);
    let source = address_source(ctx, &*device, &mut *encoder, &bytes);
    let reach = [BufferReach { buffer: source, state: BufferState::ShaderRead }];
    encoder.transition_images(&[
        image_to(image, ImageState::Undefined, ImageState::ColourAttachment, G),
        image_to(depth_image, ImageState::Undefined, ImageState::DepthAttachment, G),
    ]);
    encoder.begin_render_pass(&RenderPass {
        label: "conformance",
        colour: &[attachment(image, Load::Clear([0.0; 4]), Store::Store, false)],
        depth: Some(DepthAttachment {
            target: Target { image: depth_image, mip: 0, layer: 0 },
            load: Load::Clear(1.0),
            store: Store::DontCare,
        }),
        reach: PassReach { slots: &[], buffers: &reach },
    });
    encoder.bind_graphics_pipeline(pipeline);
    encoder.push_address(0, source, 0);
    // The near triangle first, then the far one over the whole target: the far one is
    // kept out wherever the near one is.
    encoder.push_bytes(16, &float_bytes(&colour([255, 0, 0, 255])));
    encoder.draw(0..3, 0..1);
    encoder.push_bytes(16, &float_bytes(&colour([0, 255, 0, 255])));
    encoder.draw(3..6, 0..1);
    encoder.end_pass();
    let out = image_out(ctx, &*device, &mut *encoder, image, ImageState::ColourAttachment);
    run(ctx, &*device, G, &mut [encoder]);
    let got = pixels(ctx, &*device, out);
    compare(ctx, &got, "the nearer triangle in front", |x, y| {
        covers(near, x, y).map(|inside| if inside { [255, 0, 0, 255] } else { [0, 255, 0, 255] })
    });
    Ok(())
}

fn write_mask_keeps_a_channel(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let image = target(ctx, &*device);
    let mask = WriteMask(WriteMask::RED.0 | WriteMask::BLUE.0 | WriteMask::ALPHA.0);
    let pipeline =
        pipeline(ctx, &*device, "fill", "fs_main", &[(RGBA8, mask)], None, Cull::None, &[]);
    let mut encoder = ctx.encoder(&*device, G);
    cleared(&mut *encoder, image, colour([10, 20, 30, 40]), PassReach::NOTHING);
    fill(&mut *encoder, pipeline, [200, 210, 220, 230]);
    encoder.end_pass();
    let out = image_out(ctx, &*device, &mut *encoder, image, ImageState::ColourAttachment);
    run(ctx, &*device, G, &mut [encoder]);
    let got = pixels(ctx, &*device, out);
    compare(ctx, &got, "green kept", |_, _| Some([200, 20, 220, 230]));
    Ok(())
}

fn specialised_constant_changes_the_output(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let targets = [target(ctx, &*device), target(ctx, &*device)];
    let set = [(200u32, true), (50u32, false)];
    let mut encoder = ctx.encoder(&*device, G);
    let mut outs = Vec::new();
    for (&image, &(red, green)) in targets.iter().zip(&set) {
        let constants = [
            Constant { id: 0, value: ConstantValue::Uint(red) },
            Constant { id: 1, value: ConstantValue::Bool(green) },
        ];
        let pipeline = pipeline(
            ctx,
            &*device,
            "constant",
            "fs_main",
            &[(RGBA8, WriteMask::ALL)],
            None,
            Cull::None,
            &constants,
        );
        cleared(&mut *encoder, image, [0.0; 4], PassReach::NOTHING);
        encoder.bind_graphics_pipeline(pipeline);
        encoder.draw(0..3, 0..1);
        encoder.end_pass();
        outs.push(image_out(ctx, &*device, &mut *encoder, image, ImageState::ColourAttachment));
    }
    run(ctx, &*device, G, &mut [encoder]);
    compare(ctx, &pixels(ctx, &*device, outs[0]), "red 200, green true", |_, _| {
        Some([200, 255, 0, 255])
    });
    compare(ctx, &pixels(ctx, &*device, outs[1]), "red 50, green false", |_, _| {
        Some([50, 0, 0, 255])
    });
    Ok(())
}

fn specialised_constant_of_each_type_is_read_back(ctx: &Ctx<'_>) -> Applies {
    // `specialised.slang`'s constants: two bools of defaults false and true, an int of 5, a
    // uint of 2 and a float of 0.25, each given another value; a bool of default true and a
    // uint of 9 given none.
    let given = [
        Constant { id: 0, value: ConstantValue::Bool(true) },
        Constant { id: 1, value: ConstantValue::Bool(false) },
        Constant { id: 2, value: ConstantValue::Int(-3) },
        Constant { id: 3, value: ConstantValue::Uint(7) },
        Constant { id: 4, value: ConstantValue::Float(1.5) },
    ];
    let got = dispatch_into(ctx, "specialised", &given, [1, 1, 1], 8, 0xdead_beef);
    let expected = [1, 0, (-3i32).cast_unsigned(), 7, 1.5f32.to_bits(), 1, 9, 0xdead_beef];
    for (at, (have, want)) in got.iter().zip(expected).enumerate() {
        if *have != want {
            ctx.fail(format_args!("word {at} is {have:#x}, {want:#x} expected"));
        }
    }
    Ok(())
}

fn dispatch_runs_workgroups_of_the_shaders_size(ctx: &Ctx<'_>) -> Applies {
    // `workgroup.slang`: workgroups of (8, 4, 2) over (3, 2, 2) groups cover (24, 8, 4), and
    // each invocation inside it writes a record of its local id, its global id and its local
    // index at the record its global id flattens to. Two groups in each of y and z tell a
    // size larger than the module's in any dimension from it.
    const SIZE: [u32; 3] = [8, 4, 2];
    const BOUNDS: [u32; 3] = [24, 8, 4];
    const RECORD: u32 = 7;
    const RECORDS: u32 = BOUNDS[0] * BOUNDS[1] * BOUNDS[2];
    const TAIL: u32 = 256;
    // No id or index the shader writes equals it.
    const SENTINEL: u32 = 0xdead_beef;
    let got = dispatch_into(ctx, "workgroup", &[], [3, 2, 2], RECORDS * RECORD + TAIL, SENTINEL);
    for z in 0..BOUNDS[2] {
        for y in 0..BOUNDS[1] {
            for x in 0..BOUNDS[0] {
                let local = [x % SIZE[0], y % SIZE[1], z % SIZE[2]];
                let index = local[0] + SIZE[0] * (local[1] + SIZE[1] * local[2]);
                let want = [local[0], local[1], local[2], x, y, z, index];
                let at = ((x + BOUNDS[0] * (y + BOUNDS[1] * z)) * RECORD) as usize;
                let have = &got[at..at + RECORD as usize];
                if have != want {
                    ctx.fail(format_args!(
                        "the record of global id ({x}, {y}, {z}) is {have:?}, {want:?} expected"
                    ));
                }
            }
        }
    }
    let tail = &got[(RECORDS * RECORD) as usize..];
    ctx.check(tail.iter().all(|&word| word == SENTINEL), "the words after the records unwritten");
    Ok(())
}

fn clear_and_keep_load_as_described(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let image = target(ctx, &*device);
    let pipeline = simple(ctx, &*device, "triangles");
    let corners = [[-0.9, -0.9], [0.9, -0.9], [-0.9, 0.9]];
    let mut encoder = ctx.encoder(&*device, G);
    let source = address_source(ctx, &*device, &mut *encoder, &positions(&corners, 0.5));
    cleared(&mut *encoder, image, colour([40, 80, 120, 160]), PassReach::NOTHING);
    encoder.end_pass();
    // A same-state transition orders the second pass's load after the first's store.
    encoder.transition_images(&[image_to(
        image,
        ImageState::ColourAttachment,
        ImageState::ColourAttachment,
        G,
    )]);
    let reach = [BufferReach { buffer: source, state: BufferState::ShaderRead }];
    begin(
        &mut *encoder,
        &[attachment(image, Load::Keep, Store::Store, false)],
        PassReach { slots: &[], buffers: &reach },
    );
    encoder.bind_graphics_pipeline(pipeline);
    encoder.push_address(0, source, 0);
    encoder.push_bytes(16, &float_bytes(&colour([250, 5, 15, 255])));
    encoder.draw(0..3, 0..1);
    encoder.end_pass();
    let out = image_out(ctx, &*device, &mut *encoder, image, ImageState::ColourAttachment);
    run(ctx, &*device, G, &mut [encoder]);
    let got = pixels(ctx, &*device, out);
    compare(ctx, &got, "the triangle over the kept clear", |x, y| {
        covers(corners, x, y)
            .map(|inside| if inside { [250, 5, 15, 255] } else { [40, 80, 120, 160] })
    });
    Ok(())
}

fn viewport_and_scissor_place_the_triangles(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let image = target(ctx, &*device);
    let pipeline = pipeline(
        ctx,
        &*device,
        "triangles",
        "fs_main",
        &[(RGBA8, WriteMask::ALL)],
        None,
        Cull::Back,
        &[],
    );
    // One triangle over the whole viewport each way round: the anticlockwise one is the
    // front and drawn, the clockwise one culled.
    let front = [[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]];
    let back = [[-1.0, -1.0], [-1.0, 3.0], [3.0, -1.0]];
    let mut bytes = positions(&front, 0.5);
    bytes.extend(positions(&back, 0.5));
    let mut encoder = ctx.encoder(&*device, G);
    let source = address_source(ctx, &*device, &mut *encoder, &bytes);
    let reach = [BufferReach { buffer: source, state: BufferState::ShaderRead }];
    cleared(&mut *encoder, image, [0.0; 4], PassReach { slots: &[], buffers: &reach });
    let (vx, vy, vw, vh) = (8, 4, 40, 48);
    encoder.set_viewport(Viewport {
        x: vx as f32,
        y: vy as f32,
        width: vw as f32,
        height: vh as f32,
        near: 0.0,
        far: 1.0,
    });
    let scissor = Rect { x: 12, y: 2, extent: Extent { width: 44, height: 40 } };
    encoder.set_scissor(scissor);
    encoder.bind_graphics_pipeline(pipeline);
    encoder.push_address(0, source, 0);
    encoder.push_bytes(16, &float_bytes(&colour([0, 0, 255, 255])));
    encoder.draw(3..6, 0..1);
    encoder.push_bytes(16, &float_bytes(&colour([255, 255, 0, 255])));
    encoder.draw(0..3, 0..1);
    encoder.end_pass();
    let out = image_out(ctx, &*device, &mut *encoder, image, ImageState::ColourAttachment);
    run(ctx, &*device, G, &mut [encoder]);
    let got = pixels(ctx, &*device, out);
    let within = |at: u32, from: u32, len: u32| at >= from && at < from + len;
    compare(ctx, &got, "the front triangle in the viewport and the scissor", |x, y| {
        let viewport = within(x, vx, vw) && within(y, vy, vh);
        let scissored = within(x, scissor.x, scissor.extent.width)
            && within(y, scissor.y, scissor.extent.height);
        Some(if viewport && scissored { [255, 255, 0, 255] } else { [0; 4] })
    });
    Ok(())
}

fn draw_parameters_reach_the_shader(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let image = target(ctx, &*device);
    let pipeline = simple(ctx, &*device, "ids");
    let whole = [[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]];
    let unused = [[0.0, 0.0]; 3];
    let mut bytes = positions(&unused, 0.5);
    bytes.extend(positions(&whole, 0.5));
    let mut encoder = ctx.encoder(&*device, G);
    let source = address_source(ctx, &*device, &mut *encoder, &bytes);
    let index_bytes: Vec<u8> = [0u32, 1, 2].iter().flat_map(|index| index.to_le_bytes()).collect();
    let indices = ctx.upload(&*device, BufferUsage::INDEX, &index_bytes);
    encoder.transition_buffers(&[buffer_to(
        indices,
        BufferState::Undefined,
        BufferState::Index,
        G,
    )]);
    let reach = [BufferReach { buffer: source, state: BufferState::ShaderRead }];
    cleared(&mut *encoder, image, [0.0; 4], PassReach { slots: &[], buffers: &reach });
    encoder.bind_graphics_pipeline(pipeline);
    encoder.push_address(0, source, 0);
    encoder.bind_index_buffer(indices, 0);
    // Base vertex 3 and first instance 2: the shader sees vertex 3 and instance 2.
    encoder.draw_indexed(0..3, 3, 2..3);
    encoder.end_pass();
    let out = image_out(ctx, &*device, &mut *encoder, image, ImageState::ColourAttachment);
    run(ctx, &*device, G, &mut [encoder]);
    let got = pixels(ctx, &*device, out);
    compare(ctx, &got, "vertex 3 and instance 2", |_, _| Some([3, 2, 0, 255]));
    Ok(())
}

/// The two triangles the indirect cases draw, as indexed records, and their colour.
const LEFT: [[f32; 2]; 3] = [[-0.9, -0.9], [-0.1, -0.9], [-0.5, 0.9]];
const RIGHT: [[f32; 2]; 3] = [[0.1, -0.9], [0.9, -0.9], [0.5, 0.9]];
const DRAWN: [u8; 4] = [30, 160, 90, 255];

/// Records a pass that draws the indirect cases' triangles by `draw`, and returns the
/// readback of the target.
fn indirect(
    ctx: &Ctx<'_>,
    device: &dyn Device,
    draw: &dyn Fn(&mut dyn Encoder, BufferHandle),
    count: Option<u32>,
) -> Vec<[u8; 4]> {
    let image = target(ctx, device);
    let pipeline = simple(ctx, device, "triangles");
    let mut bytes = positions(&LEFT, 0.5);
    bytes.extend(positions(&RIGHT, 0.5));
    let mut encoder = ctx.encoder(device, G);
    let source = address_source(ctx, device, &mut *encoder, &bytes);
    let index_bytes: Vec<u8> = (0u32..6).flat_map(|index| index.to_le_bytes()).collect();
    let indices = ctx.upload(device, BufferUsage::INDEX, &index_bytes);
    // Four bytes of padding, then the two records: the offset is not zero.
    let mut records = vec![0; 4];
    for first_index in [0, 3] {
        let args = IndexedIndirectArgs {
            index_count: 3,
            instance_count: 1,
            first_index,
            base_vertex: 0,
            first_instance: 0,
        };
        records.extend_from_slice(&args.to_bytes());
    }
    if let Some(count) = count {
        records.extend_from_slice(&count.to_le_bytes());
    }
    let arguments = ctx.upload(device, BufferUsage::INDIRECT, &records);
    encoder.transition_buffers(&[
        buffer_to(indices, BufferState::Undefined, BufferState::Index, G),
        buffer_to(arguments, BufferState::Undefined, BufferState::Indirect, G),
    ]);
    let reach = [BufferReach { buffer: source, state: BufferState::ShaderRead }];
    cleared(&mut *encoder, image, [0.0; 4], PassReach { slots: &[], buffers: &reach });
    encoder.bind_graphics_pipeline(pipeline);
    encoder.push_address(0, source, 0);
    encoder.push_bytes(16, &float_bytes(&colour(DRAWN)));
    encoder.bind_index_buffer(indices, 0);
    draw(&mut *encoder, arguments);
    encoder.end_pass();
    let out = image_out(ctx, device, &mut *encoder, image, ImageState::ColourAttachment);
    run(ctx, device, G, &mut [encoder]);
    pixels(ctx, device, out)
}

/// What a draw of the triangles `drawn` names leaves at pixel `(x, y)`.
fn expected_triangles(drawn: &[[[f32; 2]; 3]], x: u32, y: u32) -> Option<[u8; 4]> {
    let mut inside = false;
    for &corners in drawn {
        inside |= covers(corners, x, y)?;
    }
    Some(if inside { DRAWN } else { [0; 4] })
}

fn indexed_indirect_draw_matches_the_reference(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let stride = IndexedIndirectArgs::BYTES;
    let draw = |encoder: &mut dyn Encoder, arguments| {
        if ctx.set.multi_draw_indirect {
            encoder.draw_indexed_indirect(arguments, 4, 2, stride);
        } else {
            encoder.draw_indexed_indirect(arguments, 4, 1, stride);
            encoder.draw_indexed_indirect(arguments, 4 + u64::from(stride), 1, stride);
        }
    };
    let got = indirect(ctx, &*device, &draw, None);
    compare(ctx, &got, "both records drawn", |x, y| expected_triangles(&[LEFT, RIGHT], x, y));
    Ok(())
}

fn indirect_count_draws_where_reported(ctx: &Ctx<'_>) -> Applies {
    if !ctx.set.indirect_count {
        return Err("the device does not report indirect_count");
    }
    let device = ctx.device();
    let stride = IndexedIndirectArgs::BYTES;
    // The count, one, follows the two records; the draw is capped at two.
    let count_offset = 4 + 2 * u64::from(stride);
    let draw = |encoder: &mut dyn Encoder, arguments| {
        encoder.draw_indexed_indirect_count(arguments, 4, arguments, count_offset, 2, stride);
    };
    let got = indirect(ctx, &*device, &draw, Some(1));
    compare(ctx, &got, "the first record alone", |x, y| expected_triangles(&[LEFT], x, y));
    Ok(())
}

/// Puts `image` in `SampledRead` with `rgba` in every texel, or `texels` where given.
fn filled(
    ctx: &Ctx<'_>,
    device: &dyn Device,
    encoder: &mut dyn Encoder,
    image: ImageHandle,
    extent: Extent,
    texels: &[u8],
) {
    let source = ctx.upload(device, BufferUsage::EMPTY, texels);
    encoder.transition_buffers(&[buffer_to(
        source,
        BufferState::Undefined,
        BufferState::CopySource,
        G,
    )]);
    encoder.transition_images(&[image_to(
        image,
        ImageState::Undefined,
        ImageState::CopyDestination,
        G,
    )]);
    encoder.copy_buffer_to_image(
        source,
        image,
        &[BufferImageCopy {
            buffer_offset: 0,
            mip: 0,
            layer: 0,
            rect: Rect { x: 0, y: 0, extent },
        }],
    );
    encoder.transition_images(&[image_to(
        image,
        ImageState::CopyDestination,
        ImageState::SampledRead,
        G,
    )]);
}

/// A sampled image of `extent` with `texels`, in `SampledRead` once its submit completed.
fn sampled_image(ctx: &Ctx<'_>, device: &dyn Device, extent: Extent, texels: &[u8]) -> ImageHandle {
    let usage = ImageUsage::SAMPLED.union(ImageUsage::COPY_DESTINATION);
    let image = ctx.image(device, &image_desc(RGBA8, extent, usage));
    let mut encoder = ctx.encoder(device, G);
    filled(ctx, device, &mut *encoder, image, extent, texels);
    run(ctx, device, G, &mut [encoder]);
    image
}

/// Draws `sample.slang` from `slot`, the left half by sampler `left` and the right by
/// `right`, and returns the pixels.
fn sample(
    ctx: &Ctx<'_>,
    device: &dyn Device,
    slot: HeapSlot,
    left: u32,
    right: u32,
) -> Vec<[u8; 4]> {
    let image = target(ctx, device);
    let pipeline = simple(ctx, device, "sample");
    let mut encoder = ctx.encoder(device, G);
    let slots = [slot];
    cleared(&mut *encoder, image, [0.0; 4], PassReach { slots: &slots, buffers: &[] });
    encoder.bind_graphics_pipeline(pipeline);
    encoder.push_slot(0, slot);
    encoder.push_bytes(4, &left.to_le_bytes());
    encoder.push_bytes(8, &right.to_le_bytes());
    encoder.draw(0..3, 0..1);
    encoder.end_pass();
    let out = image_out(ctx, device, &mut *encoder, image, ImageState::ColourAttachment);
    run(ctx, device, G, &mut [encoder]);
    pixels(ctx, device, out)
}

const NEAREST: SamplerDesc = SamplerDesc {
    magnify: Filter::Nearest,
    minify: Filter::Nearest,
    mip: Filter::Nearest,
    address_u: AddressMode::ClampToEdge,
    address_v: AddressMode::ClampToEdge,
};

fn heap_is_sampled_past_slot_zero(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let solid = |rgba: [u8; 4]| rgba.repeat((W * H) as usize);
    let zero = sampled_image(ctx, &*device, EXTENT, &solid([255, 0, 0, 255]));
    let past = sampled_image(ctx, &*device, EXTENT, &solid([12, 34, 56, 78]));
    ctx.ok(device.write_heap_slot(HeapSlot(0), zero), "a heap write");
    ctx.ok(device.write_heap_slot(HeapSlot(5), past), "a heap write");
    let sampler = ctx.ok(device.create_sampler(&NEAREST), "a sampler");
    let got = sample(ctx, &*device, HeapSlot(5), sampler.0, sampler.0);
    compare(ctx, &got, "slot 5's image", |_, _| Some([12, 34, 56, 78]));
    Ok(())
}

/// The least `subTexelPrecisionBits` Vulkan allows a device.
const SUB_TEXEL_BITS: u32 = 4;

fn sampler_filters_as_described(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    // Four texels in a row, red rising by 80 from zero; the rest of each texel fixed.
    let reds = [0u8, 80, 160, 240];
    let texels: Vec<u8> = reds.iter().flat_map(|&red| [red, 7, 9, 255]).collect();
    let extent = Extent { width: 4, height: 1 };
    let image = sampled_image(ctx, &*device, extent, &texels);
    let other = sampled_image(ctx, &*device, extent, &[0; 16]);
    ctx.ok(device.write_heap_slot(HeapSlot(0), other), "a heap write");
    ctx.ok(device.write_heap_slot(HeapSlot(3), image), "a heap write");
    let linear = SamplerDesc { magnify: Filter::Linear, minify: Filter::Linear, ..NEAREST };
    // A sampler at index zero that neither half uses, then one for each half.
    let _unused = ctx.ok(device.create_sampler(&linear), "a sampler");
    let left = ctx.ok(device.create_sampler(&NEAREST), "a sampler");
    let right = ctx.ok(device.create_sampler(&linear), "a sampler");
    ctx.check(left.0 > 0 && right.0 > 0, "both samplers past index zero");
    let got = sample(ctx, &*device, HeapSlot(3), left.0, right.0);
    for (x, texel) in got.iter().take(W as usize).enumerate() {
        let u = (x as f32 + 0.5) / W as f32;
        let red = if u < 0.5 {
            reds[(u * 4.0) as usize]
        } else {
            let t = (u * 4.0 - 0.5).clamp(0.0, 3.0);
            let (low, frac) = (t.floor() as usize, t.fract());
            let high = (low + 1).min(3);
            let value = f32::from(reds[low]) * (1.0 - frac) + f32::from(reds[high]) * frac;
            // A filter weight is exact to 2^-bits of `subTexelPrecisionBits`, so a step of
            // 80 lands up to 80 / 2^(bits + 1) off, and one more for the format's rounding.
            // No capability reports the bits: the tolerance is of the least a device may
            // have, 4, which every device meets.
            let tolerance = 80.0 / f32::from(1u16 << (SUB_TEXEL_BITS + 1)) + 1.0;
            let have = f32::from(texel[0]);
            if (have - value).abs() > tolerance {
                ctx.fail(format_args!("pixel {x} filtered to {have}, {value} expected"));
            }
            texel[0]
        };
        if *texel != [red, 7, 9, 255] {
            ctx.fail(format_args!("pixel {x} is {texel:?}, red {red} expected"));
        }
    }
    for y in 1..H {
        let row = &got[(y * W) as usize..((y + 1) * W) as usize];
        ctx.check(row == &got[..W as usize], "every row alike: the image is one texel high");
    }
    Ok(())
}

fn address_is_read_in_a_shader(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let image = target(ctx, &*device);
    let pipeline = simple(ctx, &*device, "address");
    let mut encoder = ctx.encoder(&*device, G);
    let source =
        address_source(ctx, &*device, &mut *encoder, &float_bytes(&colour([32, 64, 96, 128])));
    let reach = [BufferReach { buffer: source, state: BufferState::ShaderRead }];
    cleared(&mut *encoder, image, [0.0; 4], PassReach { slots: &[], buffers: &reach });
    encoder.bind_graphics_pipeline(pipeline);
    encoder.push_address(0, source, 0);
    encoder.draw(0..3, 0..1);
    encoder.end_pass();
    let out = image_out(ctx, &*device, &mut *encoder, image, ImageState::ColourAttachment);
    run(ctx, &*device, G, &mut [encoder]);
    compare(ctx, &pixels(ctx, &*device, out), "the colour at the address", |_, _| {
        Some([32, 64, 96, 128])
    });
    Ok(())
}

/// Pushes `store.slang`'s block: `count` words from `value` at `buffer`'s byte `offset`,
/// after `lag` rounds.
fn push_store(
    encoder: &mut dyn Encoder,
    buffer: BufferHandle,
    offset: u64,
    count: u32,
    lag: u32,
    value: u32,
) {
    encoder.push_address(0, buffer, offset);
    encoder.push_bytes(8, &count.to_le_bytes());
    encoder.push_bytes(12, &lag.to_le_bytes());
    encoder.push_bytes(16, &value.to_le_bytes());
}

/// A graphics encoder whose one pass is a dispatch of `store.slang` that spins `lag` rounds
/// before it writes a word of a buffer of its own: at the head of a submit, it holds what
/// the submit records after it in flight.
fn lagging(ctx: &Ctx<'_>, device: &dyn Device, lag: u32) -> Box<dyn Encoder> {
    let writer = kernel(ctx, device, "store");
    let scratch = ctx.buffer(device, 4, BufferUsage::DEVICE_ADDRESS, MemoryClass::Device);
    let mut encoder = ctx.encoder(device, G);
    encoder.transition_buffers(&[buffer_to(
        scratch,
        BufferState::Undefined,
        BufferState::ShaderWrite,
        G,
    )]);
    let declared = [BufferReach { buffer: scratch, state: BufferState::ShaderWrite }];
    compute_pass(&mut *encoder, writer, PassReach { slots: &[], buffers: &declared });
    push_store(&mut *encoder, scratch, 0, 1, lag, 7);
    encoder.dispatch([1, 1, 1]);
    encoder.end_pass();
    encoder
}

fn declared_reader_follows_a_lagging_writer(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let image = target(ctx, &*device);
    let writer = kernel(ctx, &*device, "store");
    let reader = simple(ctx, &*device, "address");
    let usage = BufferUsage::DEVICE_ADDRESS;
    let written = ctx.buffer(&*device, 16, usage, MemoryClass::Device);
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_buffers(&[buffer_to(
        written,
        BufferState::Undefined,
        BufferState::ShaderWrite,
        G,
    )]);
    // The four channels' bits: a quarter, and the next three floats above it, each of which
    // an 8-bit fraction reads as 64.
    let quarter = 0.25f32.to_bits();
    let declared = [BufferReach { buffer: written, state: BufferState::ShaderWrite }];
    compute_pass(&mut *encoder, writer, PassReach { slots: &[], buffers: &declared });
    push_store(&mut *encoder, written, 0, 4, LAG, quarter);
    encoder.dispatch([1, 1, 1]);
    encoder.end_pass();
    encoder.transition_buffers(&[buffer_to(
        written,
        BufferState::ShaderWrite,
        BufferState::ShaderRead,
        G,
    )]);
    let read = [BufferReach { buffer: written, state: BufferState::ShaderRead }];
    cleared(&mut *encoder, image, [0.0; 4], PassReach { slots: &[], buffers: &read });
    encoder.bind_graphics_pipeline(reader);
    encoder.push_address(0, written, 0);
    encoder.draw(0..3, 0..1);
    encoder.end_pass();
    let out = image_out(ctx, &*device, &mut *encoder, image, ImageState::ColourAttachment);
    run(ctx, &*device, G, &mut [encoder]);
    compare(ctx, &pixels(ctx, &*device, out), "the writer's colour", |_, _| Some([64; 4]));
    Ok(())
}

fn dispatch_writes_through_an_address(ctx: &Ctx<'_>) -> Applies {
    const COUNT: u32 = 1000;
    let device = ctx.device();
    let writer = kernel(ctx, &*device, "store");
    let usage = BufferUsage::DEVICE_ADDRESS.union(BufferUsage::COPY_SOURCE);
    let size = u64::from(COUNT * 4);
    let written = ctx.buffer(&*device, size, usage, MemoryClass::Device);
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_buffers(&[buffer_to(
        written,
        BufferState::Undefined,
        BufferState::ShaderWrite,
        G,
    )]);
    let declared = [BufferReach { buffer: written, state: BufferState::ShaderWrite }];
    compute_pass(&mut *encoder, writer, PassReach { slots: &[], buffers: &declared });
    push_store(&mut *encoder, written, 0, COUNT, 0, 5000);
    encoder.dispatch([COUNT.div_ceil(64), 1, 1]);
    encoder.end_pass();
    let out = copy_out(ctx, &*device, &mut *encoder, written, BufferState::ShaderWrite, size);
    run(ctx, &*device, G, &mut [encoder]);
    let expected: Vec<u32> = (0..COUNT).map(|at| 5000 + at).collect();
    ctx.check(words(&ctx.read(&*device, out, 0, size)) == expected, "every word written");
    Ok(())
}

fn dispatches_of_one_pass_see_each_others_writes(ctx: &Ctx<'_>) -> Applies {
    const COUNT: u32 = 256;
    let device = ctx.device();
    let writer = kernel(ctx, &*device, "store");
    let copier = kernel(ctx, &*device, "copy");
    let size = u64::from(COUNT * 4);
    let usage = BufferUsage::DEVICE_ADDRESS.union(BufferUsage::COPY_SOURCE);
    let first = ctx.buffer(&*device, size, usage, MemoryClass::Device);
    let second = ctx.buffer(&*device, size, usage, MemoryClass::Device);
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_buffers(&[
        buffer_to(first, BufferState::Undefined, BufferState::ShaderWrite, G),
        buffer_to(second, BufferState::Undefined, BufferState::ShaderWrite, G),
    ]);
    let declared = [
        BufferReach { buffer: first, state: BufferState::ShaderWrite },
        BufferReach { buffer: second, state: BufferState::ShaderWrite },
    ];
    compute_pass(&mut *encoder, writer, PassReach { slots: &[], buffers: &declared });
    push_store(&mut *encoder, first, 0, COUNT, LAG, 70);
    encoder.dispatch([COUNT / 64, 1, 1]);
    encoder.bind_compute_pipeline(copier);
    encoder.push_address(0, first, 0);
    encoder.push_address(8, second, 0);
    encoder.push_bytes(16, &COUNT.to_le_bytes());
    encoder.push_bytes(20, &1u32.to_le_bytes());
    encoder.dispatch([COUNT / 64, 1, 1]);
    encoder.end_pass();
    let out = copy_out(ctx, &*device, &mut *encoder, second, BufferState::ShaderWrite, size);
    run(ctx, &*device, G, &mut [encoder]);
    let expected: Vec<u32> = (0..COUNT).map(|at| 71 + at).collect();
    ctx.check(
        words(&ctx.read(&*device, out, 0, size)) == expected,
        "the second dispatch read every word the lagging first wrote",
    );
    Ok(())
}

fn dispatch_indirect_reads_its_counts(ctx: &Ctx<'_>) -> Applies {
    // The counts a lagging dispatch writes: 3, 4 and 5, at byte 16.
    const OFFSET: u64 = 16;
    const GROUPS: u32 = 3 * 4 * 5;
    const CELLS: u32 = 256;
    let device = ctx.device();
    let writer = kernel(ctx, &*device, "store");
    let groups = kernel(ctx, &*device, "groups");
    let counts = ctx.buffer(
        &*device,
        64,
        BufferUsage::DEVICE_ADDRESS.union(BufferUsage::INDIRECT),
        MemoryClass::Device,
    );
    let size = u64::from(CELLS * 4);
    let cells = ctx.buffer(
        &*device,
        size,
        BufferUsage::DEVICE_ADDRESS
            .union(BufferUsage::COPY_SOURCE)
            .union(BufferUsage::COPY_DESTINATION),
        MemoryClass::Device,
    );
    let zeros = ctx.upload(&*device, BufferUsage::EMPTY, &vec![0; size as usize]);
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_buffers(&[
        buffer_to(zeros, BufferState::Undefined, BufferState::CopySource, G),
        buffer_to(cells, BufferState::Undefined, BufferState::CopyDestination, G),
        buffer_to(counts, BufferState::Undefined, BufferState::ShaderWrite, G),
    ]);
    encoder.copy_buffer(
        zeros,
        cells,
        &[BufferCopy { source_offset: 0, destination_offset: 0, size }],
    );
    encoder.transition_buffers(&[buffer_to(
        cells,
        BufferState::CopyDestination,
        BufferState::ShaderWrite,
        G,
    )]);
    let declared = [BufferReach { buffer: counts, state: BufferState::ShaderWrite }];
    compute_pass(&mut *encoder, writer, PassReach { slots: &[], buffers: &declared });
    push_store(&mut *encoder, counts, OFFSET, 3, LAG, 3);
    encoder.dispatch([1, 1, 1]);
    encoder.end_pass();
    encoder.transition_buffers(&[buffer_to(
        counts,
        BufferState::ShaderWrite,
        BufferState::Indirect,
        G,
    )]);
    let declared = [BufferReach { buffer: cells, state: BufferState::ShaderWrite }];
    compute_pass(&mut *encoder, groups, PassReach { slots: &[], buffers: &declared });
    encoder.push_address(0, cells, 0);
    encoder.push_bytes(8, &3u32.to_le_bytes());
    encoder.push_bytes(12, &4u32.to_le_bytes());
    encoder.dispatch_indirect(counts, OFFSET);
    encoder.end_pass();
    let out = copy_out(ctx, &*device, &mut *encoder, cells, BufferState::ShaderWrite, size);
    run(ctx, &*device, G, &mut [encoder]);
    let expected: Vec<u32> = (0..CELLS).map(|at| u32::from(at < GROUPS)).collect();
    ctx.check(
        words(&ctx.read(&*device, out, 0, size)) == expected,
        "one cell for each of the 3 by 4 by 5 groups the counts named",
    );
    Ok(())
}

/// Records `gbuffer.slang`'s two draws over `output` and `written`, the second reading the
/// first's output through the local read.
fn local_read(ctx: &Ctx<'_>, device: &dyn Device, written: ImageHandle) -> Vec<[u8; 4]> {
    let output = target(ctx, device);
    let writes = pipeline(
        ctx,
        device,
        "gbuffer",
        "fs_write",
        &[(RGBA8, WriteMask::NONE), (RGBA8, WriteMask::ALL)],
        None,
        Cull::None,
        &[],
    );
    let lights = pipeline(
        ctx,
        device,
        "gbuffer",
        "fs_light",
        &[(RGBA8, WriteMask::ALL), (RGBA8, WriteMask::NONE)],
        None,
        Cull::None,
        &[],
    );
    let mut encoder = ctx.encoder(device, G);
    encoder.transition_images(&[
        image_to(output, ImageState::Undefined, ImageState::ColourAttachment, G),
        image_to(written, ImageState::Undefined, ImageState::LocalRead, G),
    ]);
    let colour: [ColourAttachment; 2] = [
        attachment(output, Load::Clear([0.0; 4]), Store::Store, false),
        attachment(written, Load::Clear([0.0; 4]), Store::DontCare, true),
    ];
    begin(&mut *encoder, &colour, PassReach::NOTHING);
    encoder.bind_graphics_pipeline(writes);
    encoder.draw(0..3, 0..1);
    encoder.local_read();
    encoder.bind_graphics_pipeline(lights);
    encoder.draw(0..3, 0..1);
    encoder.end_pass();
    let out = image_out(ctx, device, &mut *encoder, output, ImageState::ColourAttachment);
    run(ctx, device, G, &mut [encoder]);
    pixels(ctx, device, out)
}

/// What `gbuffer.slang` leaves at pixel `(x, y)`: its written half, then its coordinates,
/// swizzled.
fn lit(x: u32, y: u32) -> Option<[u8; 4]> {
    Some([128, y as u8, x as u8, 255])
}

fn local_read_sees_what_the_pass_wrote(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let usage = ImageUsage::COLOUR.union(ImageUsage::INPUT);
    let written = ctx.image(&*device, &image_desc(RGBA8, EXTENT, usage));
    let got = local_read(ctx, &*device, written);
    compare(ctx, &got, "the input read back as written", lit);
    Ok(())
}

fn memoryless_g_buffer_is_lit_in_its_pass(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let usage = ImageUsage::TRANSIENT.union(ImageUsage::COLOUR).union(ImageUsage::INPUT);
    let desc =
        crate::ImageDesc { memory: ImageMemory::Memoryless, ..image_desc(RGBA8, EXTENT, usage) };
    let written = ctx.image(&*device, &desc);
    let memoryless = ctx.ok(device.image_memoryless(written), "the memoryless query");
    ctx.check(
        memoryless == ctx.set.memoryless_colour,
        "a colour image is memoryless as the device reports",
    );
    if memoryless && !ctx.set.memoryless_survives_local_read {
        return Err("a memoryless colour image does not survive the local read");
    }
    let got = local_read(ctx, &*device, written);
    compare(ctx, &got, "the G-buffer lit in its pass", lit);
    Ok(())
}

fn aliased_transients_render_in_turn(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let pipeline = simple(ctx, &*device, "fill");
    let desc = image_desc(RGBA8, EXTENT, ImageUsage::COLOUR.union(ImageUsage::COPY_SOURCE));
    let placement = ctx.ok(device.image_placement(&desc), "a placement");
    let block = ctx.ok(
        device.create_memory_block(&MemoryBlockDesc { label: "conformance", size: placement.size }),
        "a memory block",
    );
    let placed = crate::ImageDesc { memory: ImageMemory::Placed { block, offset: 0 }, ..desc };
    let images = [ctx.image(&*device, &placed), ctx.image(&*device, &placed)];
    let colours = [[90, 0, 30, 255], [0, 140, 200, 255]];
    let mut encoder = ctx.encoder(&*device, G);
    let mut outs = Vec::new();
    for (&image, &rgba) in images.iter().zip(&colours) {
        encoder.alias(image);
        cleared(&mut *encoder, image, [0.0; 4], PassReach::NOTHING);
        fill(&mut *encoder, pipeline, rgba);
        encoder.end_pass();
        outs.push(image_out(ctx, &*device, &mut *encoder, image, ImageState::ColourAttachment));
    }
    run(ctx, &*device, G, &mut [encoder]);
    for (&out, &rgba) in outs.iter().zip(&colours) {
        compare(ctx, &pixels(ctx, &*device, out), "each alias in its turn", |_, _| Some(rgba));
    }
    Ok(())
}

fn ownership_transfer_keeps_the_contents(ctx: &Ctx<'_>) -> Applies {
    if ctx.set.queues.compute != QueuePath::Own {
        return Err("compute has no queue of its own");
    }
    let device = ctx.device();
    let usage = ImageUsage::COPY_SOURCE.union(ImageUsage::COPY_DESTINATION);
    let image = ctx.image(&*device, &image_desc(RGBA8, EXTENT, usage));
    let texels: Vec<u8> = (0..W * H * 4).map(|at| (at % 241) as u8).collect();
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &texels);
    let crossing = image_across(
        image,
        ImageRange::FIRST,
        (ImageState::CopyDestination, G),
        (ImageState::CopySource, C),
    );
    let mut release = ctx.encoder(&*device, G);
    release.transition_buffers(&[buffer_to(
        source,
        BufferState::Undefined,
        BufferState::CopySource,
        G,
    )]);
    release.transition_images(&[image_to(
        image,
        ImageState::Undefined,
        ImageState::CopyDestination,
        G,
    )]);
    release.copy_buffer_to_image(source, image, &[whole_image()]);
    release.transition_images(&[crossing]);
    let released = ctx.point(&*device, 1);
    ctx.submit(&*device, G, &mut [release], &[], &[released]);
    let mut acquire = ctx.encoder(&*device, C);
    acquire.transition_images(&[crossing]);
    let out = readback(ctx, &*device, &mut *acquire, u64::from(W * H * 4));
    acquire.copy_image_to_buffer(image, out, &[whole_image()]);
    acquire.transition_buffers(&[buffer_to(
        out,
        BufferState::CopyDestination,
        BufferState::HostRead,
        C,
    )]);
    let done = ctx.point(&*device, 1);
    ctx.submit(&*device, C, &mut [acquire], &[released], &[done]);
    ctx.reached(&*device, done);
    ctx.check(
        ctx.read(&*device, out, 0, u64::from(W * H * 4)) == texels,
        "the contents crossed to the compute queue",
    );
    Ok(())
}

fn retake_after_a_store_on_one_queue_is_clean(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let pipeline = simple(ctx, &*device, "fill");
    let stores = |image: ImageHandle| {
        let mut encoder = ctx.encoder(&*device, G);
        cleared(&mut *encoder, image, [0.0; 4], PassReach::NOTHING);
        fill(&mut *encoder, pipeline, [1, 2, 3, 4]);
        encoder.end_pass();
        encoder
    };
    let retakes = |image: ImageHandle| {
        let mut encoder = ctx.encoder(&*device, G);
        cleared(&mut *encoder, image, colour([200, 100, 50, 25]), PassReach::NOTHING);
        encoder.end_pass();
        let out = image_out(ctx, &*device, &mut *encoder, image, ImageState::ColourAttachment);
        (encoder, out)
    };
    // A later submit on the same queue, with no wait, made while a lagging dispatch at the
    // head of the first holds the store in flight: the take from `Undefined` orders its
    // clear after the store. The first submit's encoders live until the second is made, as
    // a backend's drop of a submitted encoder may wait for its submit.
    let image = target(ctx, &*device);
    let first = ctx.point(&*device, 1);
    let mut earlier = [lagging(ctx, &*device, LAG << 4), stores(image)];
    ctx.submit(&*device, G, &mut earlier, &[], &[first]);
    let (encoder, later) = retakes(image);
    run(ctx, &*device, G, &mut [encoder]);
    ctx.reached(&*device, first);
    drop(earlier);
    // The same in one submit of the two encoders, which synchronisation validation checks
    // whatever the host has seen complete: a narrower source scope is a write after a write.
    let image = target(ctx, &*device);
    let stored = stores(image);
    let (encoder, one) = retakes(image);
    run(ctx, &*device, G, &mut [stored, encoder]);
    for (out, what) in [(later, "the clear of the later submit"), (one, "the clear in one submit")]
    {
        compare(ctx, &pixels(ctx, &*device, out), what, |_, _| Some([200, 100, 50, 25]));
    }
    Ok(())
}

fn warm_cache_hits_every_stage(ctx: &Ctx<'_>) -> Applies {
    let make = |device: &dyn Device| {
        simple(ctx, device, "triangles");
        kernel(ctx, device, "store");
    };
    let cold = ctx.device();
    make(&*cold);
    let bytes = ctx.ok(cold.pipeline_cache(), "the cache's bytes");
    drop(cold);
    let warm = ctx.cached(bytes, "a device from the cache");
    make(&*warm);
    let stats = warm.pipeline_stats();
    if stats.stage_hits + stats.stage_misses == 0 {
        return Err("the driver reports no stage's creation feedback");
    }
    ctx.check(
        stats.stage_hits == 3 && stats.stage_misses == 0,
        "the three stages of the two pipelines hit the cache",
    );
    Ok(())
}

fn two_threads_record_at_once(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let pipeline = simple(ctx, &*device, "fill");
    let colours = [[10, 20, 30, 40], [50, 60, 70, 80]];
    let targets = [target(ctx, &*device), target(ctx, &*device)];
    let size = u64::from(W * H * 4);
    let outs = [
        ctx.buffer(&*device, size, BufferUsage::COPY_DESTINATION, MemoryClass::Readback),
        ctx.buffer(&*device, size, BufferUsage::COPY_DESTINATION, MemoryClass::Readback),
    ];
    // Each encoder is made here and recorded on a thread of its own, both at once.
    let mut encoders: Vec<Box<dyn Encoder>> = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..2)
            .map(|at| {
                let mut encoder = ctx.encoder(&*device, G);
                let (image, out, rgba) = (targets[at], outs[at], colours[at]);
                scope.spawn(move || {
                    cleared(&mut *encoder, image, [0.0; 4], PassReach::NOTHING);
                    fill(&mut *encoder, pipeline, rgba);
                    encoder.end_pass();
                    copy_image_out(&mut *encoder, image, ImageState::ColourAttachment, out);
                    encoder
                })
            })
            .collect();
        threads.into_iter().map(|thread| thread.join().expect("a recording thread")).collect()
    });
    run(ctx, &*device, G, &mut encoders);
    for (&out, &rgba) in outs.iter().zip(&colours) {
        compare(ctx, &pixels(ctx, &*device, out), "each thread's target", |_, _| Some(rgba));
    }
    Ok(())
}

/// The canaries: two misuses of the "Null alone" column that a device backend leaves to
/// the validators, each on a device of its own, and each must provoke a message. A run whose
/// validators did not load provokes none and fails here. Which message each is, is the
/// harness's to check, since its wording is the validator's (`cargo xtask gpu`).
fn each_validator_reports_its_canary(ctx: &Ctx<'_>) -> Applies {
    // A transition from a state the image is not in: core validation's layout check.
    let core = ctx.provoking(|| {
        let device = ctx.device();
        let usage = ImageUsage::SAMPLED.union(ImageUsage::COPY_DESTINATION);
        let image = ctx.image(&*device, &image_desc(RGBA8, EXTENT, usage));
        let source = ctx.upload(&*device, BufferUsage::EMPTY, &vec![0; (W * H * 4) as usize]);
        let mut encoder = ctx.encoder(&*device, G);
        encoder.transition_buffers(&[buffer_to(
            source,
            BufferState::Undefined,
            BufferState::CopySource,
            G,
        )]);
        encoder.transition_images(&[image_to(
            image,
            ImageState::SampledRead,
            ImageState::CopyDestination,
            G,
        )]);
        encoder.copy_buffer_to_image(source, image, &[whole_image()]);
        run(ctx, &*device, G, &mut [encoder]);
    });
    ctx.check(core > 0, "the misdeclared state was reported");
    // Two copies into one range with no transition between: synchronisation validation's
    // write after write.
    let race = ctx.provoking(|| {
        let device = ctx.device();
        let source = ctx.upload(&*device, BufferUsage::EMPTY, &[0; 256]);
        let target = ctx.buffer(&*device, 256, BufferUsage::COPY_DESTINATION, MemoryClass::Device);
        let mut encoder = ctx.encoder(&*device, G);
        encoder.transition_buffers(&[
            buffer_to(source, BufferState::Undefined, BufferState::CopySource, G),
            buffer_to(target, BufferState::Undefined, BufferState::CopyDestination, G),
        ]);
        let all = [BufferCopy { source_offset: 0, destination_offset: 0, size: 256 }];
        encoder.copy_buffer(source, target, &all);
        encoder.copy_buffer(source, target, &all);
        run(ctx, &*device, G, &mut [encoder]);
    });
    ctx.check(race > 0, "the race between two copies was reported");
    Ok(())
}
