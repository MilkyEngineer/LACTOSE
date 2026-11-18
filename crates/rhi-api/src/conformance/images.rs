//! Images, memoryless requests and aliased transients.

use super::kit::{Body, Ctx, EXTENT, attachment, begin, clear, image_desc, image_to};
use crate::{
    Device, Format, ImageDesc, ImageHandle, ImageMemory, ImageState, ImageUsage, Load,
    MemoryBlockDesc, MemoryBlockHandle, PassReach, QueueRole, Store, Submit, WriteMask,
};

type Applies = Result<(), &'static str>;

pub(super) const BODIES: &[(&str, Body)] = &[
    ("image_use_outside_its_usage_is_refused", image_use_outside_its_usage_is_refused),
    (
        "memoryless_outcome_is_queryable_and_as_reported",
        memoryless_outcome_is_queryable_and_as_reported,
    ),
    (
        "memoryless_request_without_transient_is_refused",
        memoryless_request_without_transient_is_refused,
    ),
    ("memoryless_attachment_stored_is_refused", memoryless_attachment_stored_is_refused),
    ("memoryless_attachment_kept_is_refused", memoryless_attachment_kept_is_refused),
    (
        "memoryless_attachment_after_a_local_read_is_refused_where_lost",
        memoryless_attachment_after_a_local_read_is_refused_where_lost,
    ),
    ("placement_outside_the_block_is_refused", placement_outside_the_block_is_refused),
    (
        "destroy_of_a_block_an_image_is_placed_in_is_refused",
        destroy_of_a_block_an_image_is_placed_in_is_refused,
    ),
    (
        "aliased_image_used_without_the_alias_call_is_refused",
        aliased_image_used_without_the_alias_call_is_refused,
    ),
    ("alias_read_before_its_first_write_is_refused", alias_read_before_its_first_write_is_refused),
    (
        "aliases_used_in_turn_with_the_alias_call_pass",
        aliases_used_in_turn_with_the_alias_call_pass,
    ),
];

/// A G-buffer image: a transient colour attachment read as an input.
const G_BUFFER: ImageUsage =
    ImageUsage::TRANSIENT.union(ImageUsage::COLOUR).union(ImageUsage::INPUT);

fn memoryless(format: Format, usage: ImageUsage) -> ImageDesc<'static> {
    ImageDesc { memory: ImageMemory::Memoryless, ..image_desc(format, EXTENT, usage) }
}

fn submit_one(device: &dyn Device, encoder: Box<dyn crate::Encoder>) -> lactose_core::Result<()> {
    device
        .submit(QueueRole::Graphics, Submit { encoders: &mut [encoder], waits: &[], signals: &[] })
}

fn image_use_outside_its_usage_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let sampled = ctx.image(&*device, &image_desc(Format::Rgba8Unorm, EXTENT, ImageUsage::SAMPLED));
    let mut encoder = ctx.encoder(&*device, QueueRole::Graphics);
    clear(&mut *encoder, sampled, None);
    ctx.refused(submit_one(&*device, encoder), "a colour attachment made without `COLOUR`");
    let target = ctx.target(&*device);
    ctx.refused(
        device.write_heap_slot(crate::HeapSlot(0), target),
        "a heap slot's image made without `SAMPLED`",
    );
    Ok(())
}

fn memoryless_outcome_is_queryable_and_as_reported(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let cases = [
        (memoryless(Format::Rgba8Unorm, G_BUFFER), ctx.set.memoryless_colour),
        (
            memoryless(Format::Depth32Float, ImageUsage::TRANSIENT.union(ImageUsage::DEPTH)),
            ctx.set.memoryless_depth,
        ),
        (image_desc(Format::Rgba8Unorm, EXTENT, G_BUFFER), false),
    ];
    for (desc, honoured) in cases {
        let image = ctx.image(&*device, &desc);
        let outcome = ctx.ok(device.image_memoryless(image), "the memoryless outcome");
        ctx.check(
            outcome == honoured,
            "a memoryless request is honoured exactly as the set reports",
        );
        ctx.check(
            ctx.ok(device.image_memoryless(image), "the outcome again") == outcome,
            "the outcome is fixed",
        );
    }
    Ok(())
}

fn memoryless_request_without_transient_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let desc = memoryless(Format::Rgba8Unorm, ImageUsage::COLOUR.union(ImageUsage::INPUT));
    ctx.refused(device.create_image(&desc), "a memoryless request without `TRANSIENT`");
    Ok(())
}

/// A pass over a memoryless image, stored or kept as `load` and `store` say.
fn memoryless_pass(ctx: &Ctx<'_>, load: Load<[f32; 4]>, store: Store, what: &str) {
    let device = ctx.device();
    let queue = QueueRole::Graphics;
    let image = ctx.image(&*device, &memoryless(Format::Rgba8Unorm, G_BUFFER));
    let mut encoder = ctx.encoder(&*device, queue);
    encoder.transition_images(&[image_to(
        image,
        ImageState::Undefined,
        ImageState::ColourAttachment,
        queue,
    )]);
    begin(&mut *encoder, &[attachment(image, load, store, false)], PassReach::NOTHING);
    encoder.end_pass();
    ctx.refused(submit_one(&*device, encoder), what);
    let mut encoder = ctx.encoder(&*device, queue);
    encoder.transition_images(&[image_to(
        image,
        ImageState::Undefined,
        ImageState::ColourAttachment,
        queue,
    )]);
    begin(
        &mut *encoder,
        &[attachment(image, Load::Clear([0.0; 4]), Store::DontCare, false)],
        PassReach::NOTHING,
    );
    encoder.end_pass();
    ctx.ok(submit_one(&*device, encoder), "the pass cleared and not stored");
}

fn memoryless_attachment_stored_is_refused(ctx: &Ctx<'_>) -> Applies {
    memoryless_pass(ctx, Load::Clear([0.0; 4]), Store::Store, "a memoryless attachment stored");
    Ok(())
}

fn memoryless_attachment_kept_is_refused(ctx: &Ctx<'_>) -> Applies {
    memoryless_pass(ctx, Load::Keep, Store::DontCare, "a memoryless attachment loaded with `Keep`");
    Ok(())
}

/// A pass that draws into a memoryless attachment, calls the local read, and draws into
/// it again.
fn memoryless_attachment_after_a_local_read_is_refused_where_lost(ctx: &Ctx<'_>) -> Applies {
    if !ctx.set.memoryless_colour || ctx.set.memoryless_survives_local_read {
        return Err("memoryless colour is not offered, or survives a local read");
    }
    let device = ctx.device();
    let queue = QueueRole::Graphics;
    let g_buffer = ctx.image(&*device, &memoryless(Format::Rgba8Unorm, G_BUFFER));
    ctx.check(ctx.ok(device.image_memoryless(g_buffer), "the outcome"), "the request is honoured");
    let target = ctx.target(&*device);
    let pipeline = ctx.graphics(
        &*device,
        &[(Format::Rgba8Unorm, WriteMask::ALL), (Format::Rgba8Unorm, WriteMask::ALL)],
        0,
        None,
    );
    let mut encoder = ctx.encoder(&*device, queue);
    encoder.transition_images(&[
        image_to(g_buffer, ImageState::Undefined, ImageState::LocalRead, queue),
        image_to(target, ImageState::Undefined, ImageState::ColourAttachment, queue),
    ]);
    begin(
        &mut *encoder,
        &[
            attachment(g_buffer, Load::Clear([0.0; 4]), Store::DontCare, true),
            attachment(target, Load::Clear([0.0; 4]), Store::Store, false),
        ],
        PassReach::NOTHING,
    );
    encoder.bind_graphics_pipeline(pipeline);
    encoder.draw(0..3, 0..1);
    encoder.local_read();
    encoder.draw(0..3, 0..1);
    encoder.end_pass();
    ctx.refused(submit_one(&*device, encoder), "a memoryless attachment drawn after a local read");
    Ok(())
}

/// A block that holds one placement of `desc`, and that placement.
fn block(
    ctx: &Ctx<'_>,
    device: &dyn Device,
    desc: &ImageDesc<'_>,
) -> (MemoryBlockHandle, crate::Placement) {
    let placement = ctx.ok(device.image_placement(desc), "a placement");
    ctx.check(placement.alignment > 0 && placement.size > 0, "a placement takes bytes");
    let block = ctx.ok(
        device.create_memory_block(&MemoryBlockDesc { label: "conformance", size: placement.size }),
        "a block",
    );
    (block, placement)
}

fn placed(desc: ImageDesc<'static>, block: MemoryBlockHandle, offset: u64) -> ImageDesc<'static> {
    ImageDesc { memory: ImageMemory::Placed { block, offset }, ..desc }
}

fn placement_outside_the_block_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let desc = image_desc(Format::Rgba8Unorm, EXTENT, ImageUsage::COLOUR);
    let (block, placement) = block(ctx, &*device, &desc);
    ctx.refused(
        device.create_image(&placed(desc, block, placement.alignment)),
        "a placement past the block's end",
    );
    ctx.refused(
        device.create_image(&placed(desc, block, u64::MAX - 1)),
        "a placement whose end wraps",
    );
    ctx.ok(device.create_image(&placed(desc, block, 0)), "the placement that fits");
    Ok(())
}

fn destroy_of_a_block_an_image_is_placed_in_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let desc = image_desc(Format::Rgba8Unorm, EXTENT, ImageUsage::COLOUR);
    let (block, _) = block(ctx, &*device, &desc);
    let image = ctx.image(&*device, &placed(desc, block, 0));
    ctx.refused(device.destroy_memory_block(block), "a block's destroy with an image placed in it");
    ctx.ok(device.destroy_image(image), "the image's destroy");
    ctx.ok(device.destroy_memory_block(block), "the block's destroy once empty");
    Ok(())
}

/// Two images placed at the same bytes of one block.
fn aliases(ctx: &Ctx<'_>, device: &dyn Device) -> (ImageHandle, ImageHandle) {
    let desc = image_desc(Format::Rgba8Unorm, EXTENT, ImageUsage::COLOUR);
    let (block, _) = block(ctx, device, &desc);
    (ctx.image(device, &placed(desc, block, 0)), ctx.image(device, &placed(desc, block, 0)))
}

fn aliased_image_used_without_the_alias_call_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let (first, second) = aliases(ctx, &*device);
    let mut encoder = ctx.encoder(&*device, QueueRole::Graphics);
    clear(&mut *encoder, first, None);
    clear(&mut *encoder, second, None);
    ctx.refused(submit_one(&*device, encoder), "an alias used with no alias call");
    let mut encoder = ctx.encoder(&*device, QueueRole::Graphics);
    clear(&mut *encoder, first, None);
    ctx.ok(submit_one(&*device, encoder), "the first image alone");
    let mut encoder = ctx.encoder(&*device, QueueRole::Graphics);
    clear(&mut *encoder, second, None);
    ctx.refused(
        submit_one(&*device, encoder),
        "an alias used in a later submit with no alias call",
    );
    Ok(())
}

fn alias_read_before_its_first_write_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let queue = QueueRole::Graphics;
    let (first, second) = aliases(ctx, &*device);
    let mut encoder = ctx.encoder(&*device, queue);
    clear(&mut *encoder, first, None);
    encoder.alias(second);
    encoder.transition_images(&[image_to(
        second,
        ImageState::Undefined,
        ImageState::ColourAttachment,
        queue,
    )]);
    begin(
        &mut *encoder,
        &[attachment(second, Load::Keep, Store::Store, false)],
        PassReach::NOTHING,
    );
    encoder.end_pass();
    ctx.refused(submit_one(&*device, encoder), "an alias kept before its first write");
    Ok(())
}

fn aliases_used_in_turn_with_the_alias_call_pass(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let (first, second) = aliases(ctx, &*device);
    let pipeline = ctx.plain(&*device);
    let mut encoder = ctx.encoder(&*device, QueueRole::Graphics);
    encoder.alias(first);
    clear(&mut *encoder, first, Some(pipeline));
    encoder.alias(second);
    clear(&mut *encoder, second, Some(pipeline));
    ctx.ok(submit_one(&*device, encoder), "aliases in turn in one submit");
    let mut encoder = ctx.encoder(&*device, QueueRole::Graphics);
    encoder.alias(first);
    clear(&mut *encoder, first, Some(pipeline));
    let done = ctx.point(&*device, 1);
    ctx.submit(&*device, QueueRole::Graphics, &mut [encoder], &[], &[done]);
    ctx.reached(&*device, done);
    Ok(())
}
