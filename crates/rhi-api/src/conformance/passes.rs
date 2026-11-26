//! Render passes, the local read, the viewport and the scissor, and the bounds of indexed,
//! indirect and dispatched work.

use super::kit::{
    Body, Ctx, EXTENT, attachment, begin, buffer_to, compute_pass, image_desc, image_to,
};
use crate::{
    BufferCopy, BufferHandle, BufferState, BufferUsage, ColourAttachment, Device, Encoder, Extent,
    Format, GraphicsPipelineHandle, ImageHandle, ImageState, ImageUsage, IndexedIndirectArgs, Load,
    MemoryClass, PassReach, QueueRole, Rect, Rule, Store, Submit, Viewport, WriteMask,
};

type Applies = Result<(), &'static str>;

pub(super) const BODIES: &[(&str, Body)] = &[
    ("input_read_without_a_local_read_is_refused", input_read_without_a_local_read_is_refused),
    ("input_read_of_another_attachment_is_refused", input_read_of_another_attachment_is_refused),
    ("attachments_of_unequal_extent_are_refused", attachments_of_unequal_extent_are_refused),
    ("scissor_outside_the_attachments_is_refused", scissor_outside_the_attachments_is_refused),
    ("viewport_outside_its_rules_is_refused", viewport_outside_its_rules_is_refused),
    (
        "index_buffer_offset_outside_or_unaligned_is_refused",
        index_buffer_offset_outside_or_unaligned_is_refused,
    ),
    (
        "indexed_draw_past_its_index_buffer_is_refused",
        indexed_draw_past_its_index_buffer_is_refused,
    ),
    (
        "indirect_arguments_outside_or_unaligned_are_refused",
        indirect_arguments_outside_or_unaligned_are_refused,
    ),
    ("dispatch_beyond_the_group_limit_is_refused", dispatch_beyond_the_group_limit_is_refused),
];

const G: QueueRole = QueueRole::Graphics;

/// Bytes of the index and argument buffers: sixteen indices, three records and a bit.
const BYTES: u64 = 64;

fn submit(device: &dyn Device, encoders: &mut [Box<dyn Encoder>]) -> lactose_core::Result<()> {
    device.submit(G, Submit { encoders, waits: &[], signals: &[] })
}

/// Submits `encoders` and waits for their point.
fn run(ctx: &Ctx<'_>, device: &dyn Device, encoders: &mut [Box<dyn Encoder>]) {
    let done = ctx.point(device, 1);
    ctx.submit(device, G, encoders, &[], &[done]);
    ctx.reached(device, done);
}

/// A colour target whose usage holds `INPUT`, for an attachment the pass reads locally.
fn input(ctx: &Ctx<'_>, device: &dyn Device) -> ImageHandle {
    let usage = ImageUsage::COLOUR.union(ImageUsage::INPUT);
    ctx.image(device, &image_desc(Format::Rgba8Unorm, EXTENT, usage))
}

/// Records the transitions of `images`, each to its attachment's state, and begins a pass
/// over them, every attachment marked for the local read where `marks` says so.
fn local_pass(encoder: &mut dyn Encoder, images: &[ImageHandle], marks: &[bool]) {
    let mut colour = Vec::new();
    for (&image, &marked) in images.iter().zip(marks) {
        let state = if marked { ImageState::LocalRead } else { ImageState::ColourAttachment };
        encoder.transition_images(&[image_to(image, ImageState::Undefined, state, G)]);
        colour.push(attachment(image, Load::Clear([0.0; 4]), Store::Store, marked));
    }
    begin(encoder, &colour, PassReach::NOTHING);
}

/// A pipeline of two `Rgba8Unorm` targets, writing those of `writes` and reading the
/// inputs of `inputs`.
fn two_targets(
    ctx: &Ctx<'_>,
    device: &dyn Device,
    writes: [bool; 2],
    inputs: u32,
) -> GraphicsPipelineHandle {
    let mask = |write: bool| if write { WriteMask::ALL } else { WriteMask::NONE };
    let colour = [(Format::Rgba8Unorm, mask(writes[0])), (Format::Rgba8Unorm, mask(writes[1]))];
    ctx.graphics(device, &colour, inputs, None)
}

fn draw(encoder: &mut dyn Encoder, pipeline: GraphicsPipelineHandle) {
    encoder.bind_graphics_pipeline(pipeline);
    encoder.draw(0..3, 0..1);
}

fn input_read_without_a_local_read_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let images = [input(ctx, &*device), ctx.target(&*device)];
    let writer = two_targets(ctx, &*device, [true, false], 0);
    let reader = two_targets(ctx, &*device, [false, true], 0b1);
    let mut encoder = ctx.encoder(&*device, G);
    local_pass(&mut *encoder, &images, &[true, false]);
    draw(&mut *encoder, writer);
    draw(&mut *encoder, reader);
    encoder.end_pass();
    ctx.refused(
        submit(&*device, &mut [encoder]),
        "a read of input 0 written since the last local read",
    );
    let mut encoder = ctx.encoder(&*device, G);
    local_pass(&mut *encoder, &images, &[true, false]);
    draw(&mut *encoder, writer);
    encoder.local_read();
    draw(&mut *encoder, reader);
    draw(&mut *encoder, writer);
    draw(&mut *encoder, reader);
    encoder.end_pass();
    ctx.refused(
        submit(&*device, &mut [encoder]),
        "a read after a second write with no local read between",
    );
    let mut encoder = ctx.encoder(&*device, G);
    local_pass(&mut *encoder, &images, &[true, false]);
    draw(&mut *encoder, reader);
    draw(&mut *encoder, writer);
    encoder.local_read();
    draw(&mut *encoder, reader);
    encoder.end_pass();
    run(ctx, &*device, &mut [encoder]);
    Ok(())
}

fn input_read_of_another_attachment_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let images = [input(ctx, &*device), input(ctx, &*device)];
    let reader = two_targets(ctx, &*device, [true, false], 0b10);
    let mut encoder = ctx.encoder(&*device, G);
    local_pass(&mut *encoder, &images, &[true, false]);
    draw(&mut *encoder, reader);
    encoder.end_pass();
    ctx.refused(
        submit(&*device, &mut [encoder]),
        "a read of input 1 in a pass whose attachment 1 is not marked",
    );
    let mut encoder = ctx.encoder(&*device, G);
    local_pass(&mut *encoder, &images, &[true, true]);
    draw(&mut *encoder, reader);
    encoder.end_pass();
    run(ctx, &*device, &mut [encoder]);
    Ok(())
}

fn attachments_of_unequal_extent_are_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let usage = ImageUsage::COLOUR;
    let large = ctx.target(&*device);
    let half = Extent { width: EXTENT.width / 2, height: EXTENT.height / 2 };
    let small = ctx.image(&*device, &image_desc(Format::Rgba8Unorm, half, usage));
    let pipeline = ctx.graphics(&*device, &[(Format::Rgba8Unorm, WriteMask::ALL); 2], 0, None);
    let mut encoder = ctx.encoder(&*device, G);
    local_pass(&mut *encoder, &[large, small], &[false, false]);
    draw(&mut *encoder, pipeline);
    encoder.end_pass();
    ctx.refused(submit(&*device, &mut [encoder]), "attachments of two extents");
    let other = ctx.target(&*device);
    let mut encoder = ctx.encoder(&*device, G);
    local_pass(&mut *encoder, &[large, other], &[false, false]);
    draw(&mut *encoder, pipeline);
    encoder.end_pass();
    run(ctx, &*device, &mut [encoder]);
    Ok(())
}

/// Records a pass over a new target that binds the plain pipeline, records `set` and
/// draws, and submits it.
fn after(
    ctx: &Ctx<'_>,
    device: &dyn Device,
    set: &dyn Fn(&mut dyn Encoder),
) -> lactose_core::Result<()> {
    let target = ctx.target(device);
    let pipeline = ctx.plain(device);
    let mut encoder = ctx.encoder(device, G);
    encoder.transition_images(&[image_to(
        target,
        ImageState::Undefined,
        ImageState::ColourAttachment,
        G,
    )]);
    let colour: [ColourAttachment; 1] =
        [attachment(target, Load::Clear([0.0; 4]), Store::Store, false)];
    begin(&mut *encoder, &colour, PassReach::NOTHING);
    encoder.bind_graphics_pipeline(pipeline);
    set(&mut *encoder);
    encoder.draw(0..3, 0..1);
    encoder.end_pass();
    let done = ctx.point(device, 1);
    device.submit(G, Submit { encoders: &mut [encoder], waits: &[], signals: &[done] })?;
    ctx.reached(device, done);
    Ok(())
}

fn scissor_outside_the_attachments_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let rect = |x, y, width, height| Rect { x, y, extent: Extent { width, height } };
    let outside = [
        ("a scissor one texel past the right edge", rect(1, 0, EXTENT.width, EXTENT.height)),
        ("a scissor one texel past the bottom edge", rect(0, 0, EXTENT.width, EXTENT.height + 1)),
        ("a scissor whose right edge wraps", rect(u32::MAX, 0, 2, 1)),
    ];
    for (what, scissor) in outside {
        ctx.refused(after(ctx, &*device, &|encoder| encoder.set_scissor(scissor)), what);
    }
    let inside = rect(8, 8, EXTENT.width - 8, EXTENT.height - 16);
    ctx.ok(after(ctx, &*device, &|encoder| encoder.set_scissor(inside)), "a scissor inside");
    Ok(())
}

fn viewport_outside_its_rules_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let [width, height] = ctx.set.viewport_extent.map(|side| side as f32);
    let [low, high] = ctx.set.viewport_bounds.map(|edge| edge as f32);
    let good = Viewport { x: 0.0, y: 0.0, width: 64.0, height: 64.0, near: 0.0, far: 1.0 };
    let outside = [
        ("a viewport wider than the limit", Viewport { width: width + 1.0, ..good }),
        ("a viewport taller than the limit", Viewport { height: height + 1.0, ..good }),
        ("a viewport of no width", Viewport { width: 0.0, ..good }),
        ("a viewport of a negative height", Viewport { height: -64.0, ..good }),
        ("a viewport whose x is not a number", Viewport { x: f32::NAN, ..good }),
        ("a viewport whose width is infinite", Viewport { width: f32::INFINITY, ..good }),
        ("a viewport whose near depth is above one", Viewport { near: 1.5, ..good }),
        ("a viewport whose far depth is below zero", Viewport { far: -0.5, ..good }),
        ("a viewport whose left edge is below the bounds", Viewport { x: low - 1.0, ..good }),
        ("a viewport whose bottom edge is above the bounds", Viewport { y: high - 32.0, ..good }),
    ];
    for (what, viewport) in outside {
        ctx.refused(after(ctx, &*device, &|encoder| encoder.set_viewport(viewport)), what);
    }
    let past = Viewport { x: -100.0, y: 40.0, width: 128.0, height: 32.0, near: 0.25, far: 0.75 };
    ctx.ok(
        after(ctx, &*device, &|encoder| encoder.set_viewport(past)),
        "a viewport past the attachments",
    );
    // The widest viewport the bounds also allow: an Apple GPU's are narrower than twice
    // its extent, which Vulkan's least assumes.
    let widest = Viewport {
        x: 0.0,
        y: 0.0,
        width: width.min(high),
        height: height.min(high),
        near: 0.0,
        far: 1.0,
    };
    ctx.ok(
        after(ctx, &*device, &|encoder| encoder.set_viewport(widest)),
        "a viewport at the limit",
    );
    Ok(())
}

/// A buffer of `usage` that a copy filled with zeros, left in `state` on the graphics
/// queue, the submit seen complete.
fn device_buffer(
    ctx: &Ctx<'_>,
    device: &dyn Device,
    usage: BufferUsage,
    state: BufferState,
) -> BufferHandle {
    let usage = usage.union(BufferUsage::COPY_DESTINATION);
    let buffer = ctx.buffer(device, BYTES, usage, MemoryClass::Device);
    let zeros = ctx.upload(device, BufferUsage::EMPTY, &[0; BYTES as usize]);
    let mut encoder = ctx.encoder(device, G);
    encoder.transition_buffers(&[
        buffer_to(zeros, BufferState::Undefined, BufferState::CopySource, G),
        buffer_to(buffer, BufferState::Undefined, BufferState::CopyDestination, G),
    ]);
    let region = BufferCopy { source_offset: 0, destination_offset: 0, size: BYTES };
    encoder.copy_buffer(zeros, buffer, &[region]);
    encoder.transition_buffers(&[buffer_to(buffer, BufferState::CopyDestination, state, G)]);
    run(ctx, device, &mut [encoder]);
    buffer
}

/// As [`after`], for the indexed and indirect draws `record` makes.
fn indexed(
    ctx: &Ctx<'_>,
    device: &dyn Device,
    record: &dyn Fn(&mut dyn Encoder),
) -> lactose_core::Result<()> {
    after(ctx, device, record)
}

fn index_buffer_offset_outside_or_unaligned_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let indices = device_buffer(ctx, &*device, BufferUsage::INDEX, BufferState::Index);
    let binds = [
        ("an offset that is not a multiple of four", 2),
        ("an offset past the buffer", BYTES + 4),
        ("an offset that wraps", u64::MAX - 3),
    ];
    for (what, offset) in binds {
        let bind = |encoder: &mut dyn Encoder| {
            encoder.bind_index_buffer(indices, offset);
            encoder.draw_indexed(0..1, 0, 0..1);
        };
        ctx.refused(indexed(ctx, &*device, &bind), what);
    }
    let bind = |encoder: &mut dyn Encoder| {
        encoder.bind_index_buffer(indices, BYTES - 4);
        encoder.draw_indexed(0..1, 0, 0..1);
    };
    ctx.ok(indexed(ctx, &*device, &bind), "the last index's offset");
    Ok(())
}

fn indexed_draw_past_its_index_buffer_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let indices = device_buffer(ctx, &*device, BufferUsage::INDEX, BufferState::Index);
    let count = (BYTES / 4) as u32;
    let draws = [
        ("one index past the buffer", 0, 0..count + 1),
        ("indices past the buffer from an offset", 8, 0..count - 1),
        ("a range that starts past the buffer", 0, count..count + 1),
        ("a range whose end is the last number", 0, u32::MAX - 1..u32::MAX),
    ];
    for (what, offset, range) in draws {
        let draw = |encoder: &mut dyn Encoder| {
            encoder.bind_index_buffer(indices, offset);
            encoder.draw_indexed(range.clone(), 0, 0..1);
        };
        ctx.refused(indexed(ctx, &*device, &draw), what);
    }
    let draw = |encoder: &mut dyn Encoder| {
        encoder.bind_index_buffer(indices, 8);
        encoder.draw_indexed(0..count - 2, 0, 0..1);
        encoder.bind_index_buffer(indices, 0);
        encoder.draw_indexed(0..count, 0, 0..1);
    };
    ctx.ok(indexed(ctx, &*device, &draw), "every index of the buffer");
    Ok(())
}

fn indirect_arguments_outside_or_unaligned_are_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let indices = device_buffer(ctx, &*device, BufferUsage::INDEX, BufferState::Index);
    let arguments = device_buffer(ctx, &*device, BufferUsage::INDIRECT, BufferState::Indirect);
    let record = u64::from(IndexedIndirectArgs::BYTES);
    let mut draws = vec![
        ("an offset that is not a multiple of four", 2, 1, 0),
        ("a record past the buffer", BYTES - record + 4, 1, 0),
        ("an offset past the buffer", BYTES + 4, 1, 0),
        ("an offset that wraps", u64::MAX - 3, 1, 0),
    ];
    if ctx.set.multi_draw_indirect {
        draws.extend([
            ("a stride below a record", 0, 2, IndexedIndirectArgs::BYTES - 4),
            ("a stride that is not a multiple of four", 0, 2, IndexedIndirectArgs::BYTES + 2),
            ("records past the buffer", 0, 3, IndexedIndirectArgs::BYTES + 4),
        ]);
    }
    for (what, offset, count, stride) in draws {
        let draw = |encoder: &mut dyn Encoder| {
            encoder.bind_index_buffer(indices, 0);
            encoder.draw_indexed_indirect(arguments, offset, count, stride);
        };
        ctx.refused(indexed(ctx, &*device, &draw), what);
    }
    if ctx.set.indirect_count {
        let counts = [
            ("a count offset that is not a multiple of four", 0, 2, 1, IndexedIndirectArgs::BYTES),
            ("a count past its buffer", 0, BYTES, 1, IndexedIndirectArgs::BYTES),
            ("a stride below a record", 0, 0, 1, IndexedIndirectArgs::BYTES - 4),
            ("records past the buffer", 8, 0, 3, IndexedIndirectArgs::BYTES),
        ];
        for (what, offset, count_offset, max_count, stride) in counts {
            let draw = |encoder: &mut dyn Encoder| {
                encoder.bind_index_buffer(indices, 0);
                encoder.draw_indexed_indirect_count(
                    arguments,
                    offset,
                    arguments,
                    count_offset,
                    max_count,
                    stride,
                );
            };
            ctx.refused(indexed(ctx, &*device, &draw), what);
        }
    }
    let draw = |encoder: &mut dyn Encoder| {
        encoder.bind_index_buffer(indices, 0);
        encoder.draw_indexed_indirect(arguments, BYTES - record, 1, 0);
    };
    ctx.ok(indexed(ctx, &*device, &draw), "the last record of the buffer");
    let pipeline = ctx.compute(&*device);
    for (what, offset) in
        [("an offset that is not a multiple of four", 2), ("counts past the buffer", BYTES - 8)]
    {
        let mut encoder = ctx.encoder(&*device, G);
        compute_pass(&mut *encoder, pipeline, PassReach::NOTHING);
        encoder.dispatch_indirect(arguments, offset);
        encoder.end_pass();
        ctx.refused(submit(&*device, &mut [encoder]), what);
    }
    let mut encoder = ctx.encoder(&*device, G);
    compute_pass(&mut *encoder, pipeline, PassReach::NOTHING);
    encoder.dispatch_indirect(arguments, BYTES - 12);
    encoder.end_pass();
    run(ctx, &*device, &mut [encoder]);
    Ok(())
}

fn dispatch_beyond_the_group_limit_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let pipeline = ctx.compute(&*device);
    let limit = ctx.set.compute_groups;
    // A dimension whose limit is `u32::MAX`, as AMD's Windows driver reports in x, cannot be
    // exceeded: the case checks the others, and does not apply where there are none.
    let exceeded: Vec<usize> = (0..3).filter(|&axis| limit[axis] < u32::MAX).collect();
    if exceeded.is_empty() {
        return Err("each dimension's group limit is u32::MAX, which no dispatch exceeds");
    }
    if exceeded.len() < 3 {
        ctx.skip_part(&format!(
            "a dispatch past the limit of {limit:?} in the dimensions whose limit is u32::MAX, \
             which no dispatch exceeds"
        ));
    }
    for axis in exceeded {
        let mut groups = [1; 3];
        groups[axis] = limit[axis] + 1;
        let mut encoder = ctx.encoder(&*device, G);
        compute_pass(&mut *encoder, pipeline, PassReach::NOTHING);
        encoder.dispatch(groups);
        encoder.end_pass();
        ctx.refused(submit(&*device, &mut [encoder]), "a dispatch one group past the limit");
    }
    // At the limit on one axis at a time: on all three at once a device would run the
    // product, 2^90 groups where MoltenVK reports 2^30.
    let mut encoder = ctx.encoder(&*device, G);
    compute_pass(&mut *encoder, pipeline, PassReach::NOTHING);
    for axis in 0..3 {
        let mut groups = [1; 3];
        groups[axis] = limit[axis];
        encoder.dispatch(groups);
    }
    encoder.end_pass();
    run(ctx, &*device, &mut [encoder]);
    Ok(())
}

/// The sub-checks of `use_in_another_state_is_refused` for what a draw or dispatch names
/// alone: an index buffer, and indirect arguments for a draw and for a dispatch, each in
/// `CopyDestination`. Synchronisation validation tracks neither, so `state` is their one
/// guard.
pub(super) fn index_and_indirect_in_another_state(ctx: &Ctx<'_>, device: &dyn Device) {
    let indices = device_buffer(ctx, device, BufferUsage::INDEX, BufferState::Index);
    let wrong = device_buffer(ctx, device, BufferUsage::INDEX, BufferState::CopyDestination);
    let draw = |encoder: &mut dyn Encoder| {
        encoder.bind_index_buffer(wrong, 0);
        encoder.draw_indexed(0..3, 0, 0..1);
    };
    ctx.refused_by(indexed(ctx, device, &draw), Rule::State, "an index buffer in CopyDestination");
    let wrong = device_buffer(ctx, device, BufferUsage::INDIRECT, BufferState::CopyDestination);
    let draw = |encoder: &mut dyn Encoder| {
        encoder.bind_index_buffer(indices, 0);
        encoder.draw_indexed_indirect(wrong, 0, 1, 0);
    };
    ctx.refused_by(
        indexed(ctx, device, &draw),
        Rule::State,
        "an indirect draw's arguments in CopyDestination",
    );
    let pipeline = ctx.compute(device);
    let mut encoder = ctx.encoder(device, G);
    compute_pass(&mut *encoder, pipeline, PassReach::NOTHING);
    encoder.dispatch_indirect(wrong, 0);
    encoder.end_pass();
    ctx.refused_by(
        submit(device, &mut [encoder]),
        Rule::State,
        "an indirect dispatch's arguments in CopyDestination",
    );
}
