//! States and queues.

use super::kit::{
    Body, Ctx, EXTENT, Gate, attachment, begin, buffer_across, buffer_to, clear, compute_pass,
    image_across, image_desc, image_to, whole_image,
};
use crate::{
    BufferCopy, BufferHandle, BufferImageCopy, BufferReach, BufferState, BufferTransition,
    BufferUsage, Device, Encoder, Extent, Format, HeapSlot, ImageDesc, ImageHandle, ImageMemory,
    ImageRange, ImageState, ImageUsage, Load, MemoryBlockDesc, MemoryClass, PassReach, QueuePath,
    QueueRole, Rect, Rule, Sharing, Store, Submit,
};

type Applies = Result<(), &'static str>;

pub(super) const BODIES: &[(&str, Body)] = &[
    ("transition_from_another_state_is_refused", transition_from_another_state_is_refused),
    ("use_in_another_state_is_refused", use_in_another_state_is_refused),
    (
        "use_after_a_write_without_a_transition_is_refused",
        use_after_a_write_without_a_transition_is_refused,
    ),
    ("state_is_kept_for_each_mip_and_layer", state_is_kept_for_each_mip_and_layer),
    (
        "state_follows_submission_order_not_recording_order",
        state_follows_submission_order_not_recording_order,
    ),
    (
        "transition_naming_a_subresource_twice_is_refused",
        transition_naming_a_subresource_twice_is_refused,
    ),
    ("transition_of_overlapping_ranges_is_refused", transition_of_overlapping_ranges_is_refused),
    ("transition_naming_a_buffer_twice_is_refused", transition_naming_a_buffer_twice_is_refused),
    (
        "transitions_of_disjoint_mips_in_one_call_pass",
        transitions_of_disjoint_mips_in_one_call_pass,
    ),
    (
        "cross_queue_use_without_a_transfer_is_refused",
        cross_queue_use_without_a_transfer_is_refused,
    ),
    ("acquire_without_a_release_is_refused", acquire_without_a_release_is_refused),
    ("release_and_acquire_cross_a_resource", release_and_acquire_cross_a_resource),
    (
        "shared_and_discarded_resources_cross_without_a_transfer",
        shared_and_discarded_resources_cross_without_a_transfer,
    ),
    ("roles_on_one_queue_need_no_transfer", roles_on_one_queue_need_no_transfer),
    (
        "image_taken_while_another_queue_reads_it_is_refused",
        image_taken_while_another_queue_reads_it_is_refused,
    ),
    ("read_of_contents_left_undefined_is_refused", read_of_contents_left_undefined_is_refused),
    (
        "use_under_another_queues_pending_write_is_refused",
        use_under_another_queues_pending_write_is_refused,
    ),
    (
        "use_under_another_queues_pending_transition_is_refused",
        use_under_another_queues_pending_transition_is_refused,
    ),
    (
        "sampled_read_under_another_queues_pending_write_is_refused",
        sampled_read_under_another_queues_pending_write_is_refused,
    ),
    (
        "use_in_a_later_submit_than_the_waiting_change_is_refused",
        use_in_a_later_submit_than_the_waiting_change_is_refused,
    ),
    (
        "acquire_in_a_later_submit_than_the_wait_for_its_release_is_refused",
        acquire_in_a_later_submit_than_the_wait_for_its_release_is_refused,
    ),
    (
        "take_of_a_mip_after_an_acquire_of_another_is_refused",
        take_of_a_mip_after_an_acquire_of_another_is_refused,
    ),
    ("use_waiting_through_another_submit_passes", use_waiting_through_another_submit_passes),
    (
        "acquire_of_a_release_a_retake_ended_is_refused",
        acquire_of_a_release_a_retake_ended_is_refused,
    ),
    (
        "read_after_a_retake_of_an_unmet_release_is_refused",
        read_after_a_retake_of_an_unmet_release_is_refused,
    ),
];

const G: QueueRole = QueueRole::Graphics;
const C: QueueRole = QueueRole::Compute;
const SIZE: u64 = 256;

fn submit(
    device: &dyn Device,
    role: QueueRole,
    encoders: &mut [Box<dyn Encoder>],
) -> lactose_core::Result<()> {
    device.submit(role, Submit { encoders, waits: &[], signals: &[] })
}

/// Submits `encoders` and waits for their point.
fn run(ctx: &Ctx<'_>, device: &dyn Device, role: QueueRole, encoders: &mut [Box<dyn Encoder>]) {
    let done = ctx.point(device, 1);
    ctx.submit(device, role, encoders, &[], &[done]);
    ctx.reached(device, done);
}

fn copyable(ctx: &Ctx<'_>, device: &dyn Device) -> ImageHandle {
    let usage = ImageUsage::COPY_DESTINATION
        .union(ImageUsage::COPY_SOURCE)
        .union(ImageUsage::SAMPLED)
        .union(ImageUsage::COLOUR);
    ctx.image(device, &image_desc(Format::Rgba8Unorm, EXTENT, usage))
}

/// An upload buffer of an image's bytes, a copy's source.
fn texels(ctx: &Ctx<'_>, device: &dyn Device) -> BufferHandle {
    ctx.upload(device, BufferUsage::EMPTY, &vec![3; (EXTENT.width * EXTENT.height * 4) as usize])
}

/// Records a copy of `source`, from `Undefined`, into `image`, from `Undefined`, which it
/// leaves in `CopyDestination` on the encoder's queue.
fn fill(encoder: &mut dyn Encoder, source: BufferHandle, image: ImageHandle) {
    let queue = encoder.queue();
    encoder.transition_buffers(&[buffer_to(
        source,
        BufferState::Undefined,
        BufferState::CopySource,
        queue,
    )]);
    encoder.transition_images(&[image_to(
        image,
        ImageState::Undefined,
        ImageState::CopyDestination,
        queue,
    )]);
    encoder.copy_buffer_to_image(source, image, &[whole_image()]);
}

fn transition_from_another_state_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let image = copyable(ctx, &*device);
    let mut encoder = ctx.encoder(&*device, G);
    fill(&mut *encoder, texels(ctx, &*device), image);
    encoder.transition_images(&[image_to(
        image,
        ImageState::ColourAttachment,
        ImageState::SampledRead,
        G,
    )]);
    ctx.refused(
        submit(&*device, G, &mut [encoder]),
        "a transition from a state the image is not in",
    );
    Ok(())
}

fn use_in_another_state_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let image = copyable(ctx, &*device);
    let readback =
        ctx.buffer(&*device, SIZE * 64, BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
    let mut encoder = ctx.encoder(&*device, G);
    fill(&mut *encoder, texels(ctx, &*device), image);
    encoder.transition_images(&[image_to(
        image,
        ImageState::CopyDestination,
        ImageState::SampledRead,
        G,
    )]);
    encoder.transition_buffers(&[buffer_to(
        readback,
        BufferState::Undefined,
        BufferState::CopyDestination,
        G,
    )]);
    encoder.copy_image_to_buffer(image, readback, &[whole_image()]);
    ctx.refused(submit(&*device, G, &mut [encoder]), "a copy from an image in `SampledRead`");
    super::passes::index_and_indirect_in_another_state(ctx, &*device);
    Ok(())
}

fn use_after_a_write_without_a_transition_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    // A `ShaderWrite` reach is a write: a second pass that writes the buffer needs a
    // transition before it.
    let data = ctx.buffer(&*device, SIZE, BufferUsage::DEVICE_ADDRESS, MemoryClass::Device);
    let compute = ctx.compute(&*device);
    let reach = [BufferReach { buffer: data, state: BufferState::ShaderWrite }];
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_buffers(&[buffer_to(
        data,
        BufferState::Undefined,
        BufferState::ShaderWrite,
        G,
    )]);
    for _ in 0..2 {
        compute_pass(&mut *encoder, compute, PassReach { slots: &[], buffers: &reach });
        encoder.dispatch([1, 1, 1]);
        encoder.end_pass();
    }
    ctx.refused(
        submit(&*device, G, &mut [encoder]),
        "two passes that write a buffer through a reach, with no transition between",
    );
    let image = ctx.target(&*device);
    let pipeline = ctx.plain(&*device);
    let mut encoder = ctx.encoder(&*device, G);
    clear(&mut *encoder, image, Some(pipeline));
    super::kit::begin(
        &mut *encoder,
        &[super::kit::attachment(image, crate::Load::Keep, crate::Store::Store, false)],
        crate::PassReach::NOTHING,
    );
    encoder.end_pass();
    ctx.refused(
        submit(&*device, G, &mut [encoder]),
        "a second pass into an attachment with no transition",
    );
    let mut encoder = ctx.encoder(&*device, G);
    clear(&mut *encoder, image, Some(pipeline));
    encoder.transition_images(&[image_to(
        image,
        ImageState::ColourAttachment,
        ImageState::ColourAttachment,
        G,
    )]);
    super::kit::begin(
        &mut *encoder,
        &[super::kit::attachment(image, crate::Load::Keep, crate::Store::Store, false)],
        crate::PassReach::NOTHING,
    );
    encoder.end_pass();
    run(ctx, &*device, G, &mut [encoder]);
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &[1; SIZE as usize]);
    let destination =
        ctx.buffer(&*device, SIZE, BufferUsage::COPY_DESTINATION, MemoryClass::Device);
    let region = [BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }];
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_buffers(&[
        buffer_to(source, BufferState::Undefined, BufferState::CopySource, G),
        buffer_to(destination, BufferState::Undefined, BufferState::CopyDestination, G),
    ]);
    encoder.copy_buffer(source, destination, &region);
    encoder.copy_buffer(source, destination, &region);
    ctx.refused(
        submit(&*device, G, &mut [encoder]),
        "a second copy into a buffer with no transition",
    );
    Ok(())
}

fn state_is_kept_for_each_mip_and_layer(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let usage = ImageUsage::COPY_DESTINATION.union(ImageUsage::SAMPLED);
    let image = ctx.image(
        &*device,
        &crate::ImageDesc { mips: 2, layers: 2, ..image_desc(Format::Rgba8Unorm, EXTENT, usage) },
    );
    let all = ImageRange { mip: 0, mips: 2, layer: 0, layers: 2 };
    let last = ImageRange { mip: 1, mips: 1, layer: 1, layers: 1 };
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_images(&[image_to(
        image,
        ImageState::Undefined,
        ImageState::CopyDestination,
        G,
    )]);
    encoder.transition_images(&[image_across(
        image,
        last,
        (ImageState::CopyDestination, G),
        (ImageState::SampledRead, G),
    )]);
    ctx.refused_by(
        submit(&*device, G, &mut [encoder]),
        Rule::State,
        "a transition of a subresource another's transition moved",
    );
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_images(&[image_to(
        image,
        ImageState::Undefined,
        ImageState::CopyDestination,
        G,
    )]);
    encoder.transition_images(&[image_across(
        image,
        all,
        (ImageState::CopyDestination, G),
        (ImageState::SampledRead, G),
    )]);
    ctx.refused_by(
        submit(&*device, G, &mut [encoder]),
        Rule::State,
        "a whole-image transition from one subresource's state",
    );
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_images(&[image_to(
        image,
        ImageState::Undefined,
        ImageState::CopyDestination,
        G,
    )]);
    encoder.transition_images(&[image_across(
        image,
        last,
        (ImageState::Undefined, G),
        (ImageState::CopyDestination, G),
    )]);
    encoder.transition_images(&[
        image_across(
            image,
            ImageRange::FIRST,
            (ImageState::CopyDestination, G),
            (ImageState::SampledRead, G),
        ),
        image_across(image, last, (ImageState::CopyDestination, G), (ImageState::SampledRead, G)),
    ]);
    encoder.transition_images(&[image_across(
        image,
        all,
        (ImageState::Undefined, G),
        (ImageState::SampledRead, G),
    )]);
    run(ctx, &*device, G, &mut [encoder]);
    // A copy is checked against the state of the subresource it reaches alone: into the
    // last, in `CopyDestination`, with the first in `SampledRead`; and refused into the first.
    let source = texels(ctx, &*device);
    let half = Extent { width: EXTENT.width / 2, height: EXTENT.height / 2 };
    let into = |mip, layer, extent| BufferImageCopy {
        buffer_offset: 0,
        mip,
        layer,
        rect: Rect { x: 0, y: 0, extent },
    };
    let split = |encoder: &mut dyn Encoder| {
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
        encoder.transition_images(&[image_to(
            image,
            ImageState::CopyDestination,
            ImageState::SampledRead,
            G,
        )]);
    };
    let mut encoder = ctx.encoder(&*device, G);
    split(&mut *encoder);
    encoder.copy_buffer_to_image(source, image, &[into(1, 1, half)]);
    run(ctx, &*device, G, &mut [encoder]);
    let mut encoder = ctx.encoder(&*device, G);
    split(&mut *encoder);
    encoder.copy_buffer_to_image(source, image, &[into(0, 0, EXTENT)]);
    ctx.refused_by(
        submit(&*device, G, &mut [encoder]),
        Rule::State,
        "a copy into a subresource in another state than its neighbours'",
    );
    Ok(())
}

fn state_follows_submission_order_not_recording_order(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    for (image, order_kept) in [(copyable(ctx, &*device), true), (copyable(ctx, &*device), false)] {
        let source = texels(ctx, &*device);
        let mut later = ctx.encoder(&*device, G);
        later.transition_images(&[image_to(
            image,
            ImageState::CopyDestination,
            ImageState::SampledRead,
            G,
        )]);
        let mut earlier = ctx.encoder(&*device, G);
        fill(&mut *earlier, source, image);
        if order_kept {
            run(ctx, &*device, G, &mut [earlier, later]);
        } else {
            ctx.refused_by(
                submit(&*device, G, &mut [later, earlier]),
                Rule::State,
                "the same encoders in the other order",
            );
        }
    }
    Ok(())
}

/// One call of each chain a frame's end can make of one subresource, each element valid
/// alone: a change and then another, a change and then a release, a take from `Undefined`
/// and then a release, a change to the state the image is in and then a change, a change to
/// `Undefined` and then a take, and a change and then one to `Undefined`, which a device
/// records nothing for. Under a set whose compute role shares the
/// graphics queue the release is an ordinary change, refused all the same, and one more
/// chain starts on the compute role, as an acquire does where compute has a queue of its
/// own. Then a change and another with another image's element between them.
fn transition_naming_a_subresource_twice_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let undefined = (ImageState::Undefined, G);
    let copied = (ImageState::CopyDestination, G);
    let sampled = (ImageState::SampledRead, G);
    let source = (ImageState::CopySource, G);
    let mut chains = vec![
        [(copied, sampled), (sampled, source)],
        [(copied, sampled), (sampled, (ImageState::SampledRead, C))],
        [(undefined, copied), (copied, (ImageState::CopyDestination, C))],
        [(copied, copied), (copied, sampled)],
        [(copied, undefined), (undefined, copied)],
        [(copied, sampled), (sampled, undefined)],
    ];
    if ctx.set.queues.compute != QueuePath::Own {
        chains.push([((ImageState::CopyDestination, C), sampled), (sampled, source)]);
    }
    for chain in chains {
        let image = copyable(ctx, &*device);
        let mut encoder = ctx.encoder(&*device, G);
        fill(&mut *encoder, texels(ctx, &*device), image);
        let call = chain.map(|(from, to)| image_across(image, ImageRange::FIRST, from, to));
        encoder.transition_images(&call);
        ctx.refused(
            submit(&*device, G, &mut [encoder]),
            "one transition call that names a subresource twice",
        );
    }
    let (image, other) = (copyable(ctx, &*device), copyable(ctx, &*device));
    let mut encoder = ctx.encoder(&*device, G);
    fill(&mut *encoder, texels(ctx, &*device), image);
    encoder.transition_images(&[
        image_across(image, ImageRange::FIRST, copied, sampled),
        image_across(other, ImageRange::FIRST, undefined, copied),
        image_across(image, ImageRange::FIRST, sampled, source),
    ]);
    ctx.refused(
        submit(&*device, G, &mut [encoder]),
        "one transition call that names a subresource twice, another image's element between",
    );
    Ok(())
}

/// Two elements of one call over ranges of one image that share a subresource: a range and
/// one inside it; two that cross, a mip of every layer and a layer of every mip; and two
/// that share a part of one axis, neither holding the other, the first two mips and the
/// last two.
fn transition_of_overlapping_ranges_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let usage = ImageUsage::COPY_DESTINATION.union(ImageUsage::SAMPLED);
    let image = ctx.image(
        &*device,
        &ImageDesc { mips: 3, layers: 2, ..image_desc(Format::Rgba8Unorm, EXTENT, usage) },
    );
    let all = ImageRange { mip: 0, mips: 3, layer: 0, layers: 2 };
    let last = ImageRange { mip: 2, mips: 1, layer: 1, layers: 1 };
    let mip = ImageRange { mip: 1, mips: 1, layer: 0, layers: 2 };
    let layer = ImageRange { mip: 0, mips: 3, layer: 0, layers: 1 };
    let low = ImageRange { mip: 0, mips: 2, layer: 0, layers: 2 };
    let high = ImageRange { mip: 1, mips: 2, layer: 0, layers: 2 };
    let take = |range| {
        image_across(image, range, (ImageState::Undefined, G), (ImageState::CopyDestination, G))
    };
    for pair in [[all, last], [mip, layer], [low, high]] {
        let mut encoder = ctx.encoder(&*device, G);
        encoder.transition_images(&pair.map(take));
        ctx.refused(
            submit(&*device, G, &mut [encoder]),
            "one transition call over two ranges of one image that overlap",
        );
    }
    Ok(())
}

/// One buffer named by two elements of one call: a take from `Undefined` and then a change,
/// and a take and then a release; under a set whose compute role shares the graphics queue,
/// a take on the compute role and then a change. Then a take and a change with another
/// buffer's element between them.
fn transition_naming_a_buffer_twice_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let undefined = (BufferState::Undefined, G);
    let read = (BufferState::ShaderRead, G);
    let source = (BufferState::CopySource, G);
    let destination = (BufferState::CopyDestination, G);
    let mut chains = vec![
        [(undefined, source), (source, destination)],
        [(undefined, read), (read, (BufferState::ShaderRead, C))],
    ];
    if ctx.set.queues.compute != QueuePath::Own {
        chains.push([((BufferState::Undefined, C), source), (source, destination)]);
    }
    let usage = BufferUsage::COPY_SOURCE
        .union(BufferUsage::COPY_DESTINATION)
        .union(BufferUsage::DEVICE_ADDRESS);
    for chain in chains {
        let buffer = ctx.buffer(&*device, SIZE, usage, MemoryClass::Device);
        let mut encoder = ctx.encoder(&*device, G);
        encoder.transition_buffers(&chain.map(|(from, to)| buffer_across(buffer, from, to)));
        ctx.refused(
            submit(&*device, G, &mut [encoder]),
            "one transition call that names a buffer twice",
        );
    }
    let buffer = ctx.buffer(&*device, SIZE, usage, MemoryClass::Device);
    let other = ctx.buffer(&*device, SIZE, usage, MemoryClass::Device);
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_buffers(&[
        buffer_across(buffer, undefined, source),
        buffer_across(other, undefined, source),
        buffer_across(buffer, source, destination),
    ]);
    ctx.refused(
        submit(&*device, G, &mut [encoder]),
        "one transition call that names a buffer twice, another buffer's element between",
    );
    Ok(())
}

/// One call that takes each mip of an image from `Undefined`, and the first mip of a second
/// image over the same range; one that moves the first two mips and each layer of the last
/// to two other states; one that moves them back in another order, each element from the
/// state the call before left it in; then a transition of the whole image, which is
/// accepted only where every element took effect. One call of two buffers beside.
fn transitions_of_disjoint_mips_in_one_call_pass(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let usage =
        ImageUsage::COPY_DESTINATION.union(ImageUsage::COPY_SOURCE).union(ImageUsage::SAMPLED);
    let desc = ImageDesc { mips: 3, layers: 2, ..image_desc(Format::Rgba8Unorm, EXTENT, usage) };
    let (image, other) = (ctx.image(&*device, &desc), ctx.image(&*device, &desc));
    let level = |mip| ImageRange { mip, mips: 1, layer: 0, layers: 2 };
    let cell = |layer| ImageRange { mip: 2, mips: 1, layer, layers: 1 };
    let copied = (ImageState::CopyDestination, G);
    let sampled = (ImageState::SampledRead, G);
    let source = (ImageState::CopySource, G);
    let buffers = [SIZE; 2]
        .map(|size| ctx.buffer(&*device, size, BufferUsage::COPY_DESTINATION, MemoryClass::Device));
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_images(&[
        image_across(image, level(0), (ImageState::Undefined, G), copied),
        image_across(image, level(1), (ImageState::Undefined, G), copied),
        image_across(image, level(2), (ImageState::Undefined, G), copied),
        image_across(other, level(0), (ImageState::Undefined, G), copied),
    ]);
    encoder.transition_images(&[
        image_across(image, level(0), copied, sampled),
        image_across(image, level(1), copied, source),
        image_across(image, cell(0), copied, sampled),
        image_across(image, cell(1), copied, source),
    ]);
    encoder.transition_images(&[
        image_across(image, level(1), source, copied),
        image_across(image, level(0), sampled, copied),
        image_across(image, cell(1), source, copied),
        image_across(image, cell(0), sampled, copied),
    ]);
    encoder.transition_buffers(
        &buffers.map(|buffer| {
            buffer_to(buffer, BufferState::Undefined, BufferState::CopyDestination, G)
        }),
    );
    run(ctx, &*device, G, &mut [encoder]);
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_images(&[image_across(
        image,
        ImageRange { mip: 0, mips: 3, layer: 0, layers: 2 },
        copied,
        sampled,
    )]);
    run(ctx, &*device, G, &mut [encoder]);
    Ok(())
}

fn several_queues(ctx: &Ctx<'_>) -> Result<(), &'static str> {
    if ctx.set.queues.compute == QueuePath::Own {
        Ok(())
    } else {
        Err("compute shares the graphics queue")
    }
}

/// An image the graphics queue filled and owns, in `CopyDestination`, seen complete.
fn owned_by_graphics(ctx: &Ctx<'_>, device: &dyn Device) -> ImageHandle {
    let image = copyable(ctx, device);
    let mut encoder = ctx.encoder(device, G);
    fill(&mut *encoder, texels(ctx, device), image);
    run(ctx, device, G, &mut [encoder]);
    image
}

fn cross_queue_use_without_a_transfer_is_refused(ctx: &Ctx<'_>) -> Applies {
    several_queues(ctx)?;
    let device = ctx.device();
    let image = owned_by_graphics(ctx, &*device);
    let source = texels(ctx, &*device);
    let mut encoder = ctx.encoder(&*device, C);
    encoder.transition_buffers(&[buffer_to(
        source,
        BufferState::Undefined,
        BufferState::CopySource,
        C,
    )]);
    encoder.copy_buffer_to_image(source, image, &[whole_image()]);
    ctx.refused(
        submit(&*device, C, &mut [encoder]),
        "a copy on compute into an image graphics owns",
    );
    Ok(())
}

fn acquire_without_a_release_is_refused(ctx: &Ctx<'_>) -> Applies {
    several_queues(ctx)?;
    let device = ctx.device();
    let image = owned_by_graphics(ctx, &*device);
    let mut encoder = ctx.encoder(&*device, C);
    encoder.transition_images(&[image_across(
        image,
        ImageRange::FIRST,
        (ImageState::CopyDestination, G),
        (ImageState::SampledRead, C),
    )]);
    ctx.refused(submit(&*device, C, &mut [encoder]), "an acquire with no release submitted");
    // Between a release and its acquire the resource is used by nothing, its releasing
    // queue included.
    let mut release = ctx.encoder(&*device, G);
    release.transition_images(&[image_across(
        image,
        ImageRange::FIRST,
        (ImageState::CopyDestination, G),
        (ImageState::SampledRead, C),
    )]);
    ctx.submit(&*device, G, &mut [release], &[], &[]);
    let mut encoder = ctx.encoder(&*device, G);
    fill_again(&mut *encoder, texels(ctx, &*device), image);
    ctx.refused(submit(&*device, G, &mut [encoder]), "a use between a release and its acquire");
    Ok(())
}

/// Records a copy of `source`, from `Undefined`, into `image`, in `CopyDestination` on
/// the encoder's queue.
fn fill_again(encoder: &mut dyn Encoder, source: BufferHandle, image: ImageHandle) {
    let queue = encoder.queue();
    encoder.transition_buffers(&[buffer_to(
        source,
        BufferState::Undefined,
        BufferState::CopySource,
        queue,
    )]);
    encoder.copy_buffer_to_image(source, image, &[whole_image()]);
}

fn release_and_acquire_cross_a_resource(ctx: &Ctx<'_>) -> Applies {
    several_queues(ctx)?;
    let device = ctx.device();
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &[5; SIZE as usize]);
    let local = ctx.buffer(
        &*device,
        SIZE,
        BufferUsage::COPY_SOURCE.union(BufferUsage::COPY_DESTINATION),
        MemoryClass::Device,
    );
    let readback = ctx.buffer(&*device, SIZE, BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
    let region = [BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }];
    let crossing =
        buffer_across(local, (BufferState::CopyDestination, G), (BufferState::CopySource, C));
    let mut release = ctx.encoder(&*device, G);
    release.transition_buffers(&[
        buffer_to(source, BufferState::Undefined, BufferState::CopySource, G),
        buffer_to(local, BufferState::Undefined, BufferState::CopyDestination, G),
    ]);
    release.copy_buffer(source, local, &region);
    release.transition_buffers(&[crossing]);
    let released = ctx.point(&*device, 1);
    ctx.submit(&*device, G, &mut [release], &[], &[released]);
    let mut acquire = ctx.encoder(&*device, C);
    acquire.transition_buffers(&[
        crossing,
        buffer_to(readback, BufferState::Undefined, BufferState::CopyDestination, C),
    ]);
    acquire.copy_buffer(local, readback, &region);
    acquire.transition_buffers(&[buffer_to(
        readback,
        BufferState::CopyDestination,
        BufferState::HostRead,
        C,
    )]);
    let done = ctx.point(&*device, 1);
    ctx.submit(&*device, C, &mut [acquire], &[released], &[done]);
    ctx.reached(&*device, done);
    ctx.check(ctx.read(&*device, readback, 0, SIZE) == [5; SIZE as usize], "the bytes crossed");
    Ok(())
}

fn shared_and_discarded_resources_cross_without_a_transfer(ctx: &Ctx<'_>) -> Applies {
    several_queues(ctx)?;
    let device = ctx.device();
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &[6; SIZE as usize]);
    let usage = BufferUsage::COPY_SOURCE.union(BufferUsage::COPY_DESTINATION);
    let shared = ctx.buffer_shared(&*device, SIZE, usage, MemoryClass::Device, Sharing::Concurrent);
    let discarded = ctx.buffer(&*device, SIZE, usage, MemoryClass::Device);
    let region = [BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }];
    let mut writer = ctx.encoder(&*device, G);
    writer.transition_buffers(&[
        buffer_to(source, BufferState::Undefined, BufferState::CopySource, G),
        buffer_to(shared, BufferState::Undefined, BufferState::CopyDestination, G),
        buffer_to(discarded, BufferState::Undefined, BufferState::CopyDestination, G),
    ]);
    writer.copy_buffer(source, shared, &region);
    writer.copy_buffer(source, discarded, &region);
    let written = ctx.point(&*device, 1);
    ctx.submit(&*device, G, &mut [writer], &[], &[written]);
    let mut reader = ctx.encoder(&*device, C);
    reader.transition_buffers(&[
        buffer_across(shared, (BufferState::CopyDestination, G), (BufferState::CopySource, C)),
        buffer_across(discarded, (BufferState::Undefined, G), (BufferState::CopyDestination, C)),
    ]);
    reader.copy_buffer(shared, discarded, &region);
    let done = ctx.point(&*device, 1);
    ctx.submit(&*device, C, &mut [reader], &[written], &[done]);
    ctx.reached(&*device, done);
    Ok(())
}

fn roles_on_one_queue_need_no_transfer(ctx: &Ctx<'_>) -> Applies {
    if ctx.set.queues.compute == QueuePath::Own {
        return Err("compute has a queue of its own");
    }
    let device = ctx.device();
    let image = owned_by_graphics(ctx, &*device);
    let mut encoder = ctx.encoder(&*device, C);
    encoder.transition_images(&[image_across(
        image,
        ImageRange::FIRST,
        (ImageState::CopyDestination, G),
        (ImageState::SampledRead, C),
    )]);
    run(ctx, &*device, C, &mut [encoder]);
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_images(&[image_across(
        image,
        ImageRange::FIRST,
        (ImageState::SampledRead, C),
        (ImageState::CopySource, G),
    )]);
    run(ctx, &*device, G, &mut [encoder]);
    Ok(())
}

/// Compute reads an image and a buffer it owns in a submit held pending; graphics then
/// takes either, from `Undefined`, with no wait for that submit. The reader neither writes
/// nor changes them, and compute's earlier submit that did is seen complete: the take is
/// refused for the pending read alone.
fn image_taken_while_another_queue_reads_it_is_refused(ctx: &Ctx<'_>) -> Applies {
    several_queues(ctx)?;
    let device = ctx.device();
    let image = copyable(ctx, &*device);
    let source = texels(ctx, &*device);
    let mut setup = ctx.encoder(&*device, C);
    fill(&mut *setup, source, image);
    setup.transition_images(&[image_to(
        image,
        ImageState::CopyDestination,
        ImageState::CopySource,
        C,
    )]);
    run(ctx, &*device, C, &mut [setup]);
    let readback =
        ctx.buffer(&*device, SIZE * 64, BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
    let scratch = copied(ctx, &*device, Sharing::Exclusive);
    let mut reader = ctx.encoder(&*device, C);
    reader.transition_buffers(&[
        buffer_to(readback, BufferState::Undefined, BufferState::CopyDestination, C),
        buffer_to(scratch, BufferState::Undefined, BufferState::CopyDestination, C),
    ]);
    reader.copy_image_to_buffer(image, readback, &[whole_image()]);
    reader.copy_buffer(
        source,
        scratch,
        &[BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }],
    );
    let mut readers = [reader];
    let gate = ctx.held(&*device, C, &mut readers);
    let mut buffer_taker = ctx.encoder(&*device, G);
    buffer_taker.transition_buffers(&[buffer_to(
        source,
        BufferState::Undefined,
        BufferState::CopySource,
        G,
    )]);
    let mut buffer_takers = [buffer_taker];
    ctx.refused(
        submit(&*device, G, &mut buffer_takers),
        "a buffer's take with no wait for the other queue's reader",
    );
    let mut taker = ctx.encoder(&*device, G);
    taker.transition_images(&[image_to(
        image,
        ImageState::Undefined,
        ImageState::CopyDestination,
        G,
    )]);
    let mut takers = [taker];
    ctx.refused(
        submit(&*device, G, &mut takers),
        "a take with no wait for the other queue's reader",
    );
    ctx.submit(&*device, G, &mut buffer_takers, &[gate.done], &[]);
    let done = ctx.point(&*device, 1);
    ctx.submit(&*device, G, &mut takers, &[gate.done], &[done]);
    gate.open(ctx, &*device);
    ctx.reached(&*device, done);
    Ok(())
}

/// Contents are undefined after a transition from `Undefined`, a store of `DontCare`, and,
/// for a buffer, a take from `Undefined` onto another queue: a read of them is refused.
fn read_of_contents_left_undefined_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    // An image transitioned from `Undefined` has lost its contents: a history resource.
    let image = ctx.target(&*device);
    let mut encoder = ctx.encoder(&*device, G);
    clear(&mut *encoder, image, None);
    encoder.transition_images(&[image_to(
        image,
        ImageState::Undefined,
        ImageState::ColourAttachment,
        G,
    )]);
    begin(&mut *encoder, &[attachment(image, Load::Keep, Store::Store, false)], PassReach::NOTHING);
    encoder.end_pass();
    ctx.refused(
        submit(&*device, G, &mut [encoder]),
        "a load of an image after a transition from Undefined",
    );
    // A store of `DontCare` leaves the contents undefined.
    let image = ctx.target(&*device);
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_images(&[image_to(
        image,
        ImageState::Undefined,
        ImageState::ColourAttachment,
        G,
    )]);
    begin(
        &mut *encoder,
        &[attachment(image, Load::Clear([0.0; 4]), Store::DontCare, false)],
        PassReach::NOTHING,
    );
    encoder.end_pass();
    encoder.transition_images(&[image_to(
        image,
        ImageState::ColourAttachment,
        ImageState::ColourAttachment,
        G,
    )]);
    begin(&mut *encoder, &[attachment(image, Load::Keep, Store::Store, false)], PassReach::NOTHING);
    encoder.end_pass();
    ctx.refused(submit(&*device, G, &mut [encoder]), "a load after a store of DontCare");
    // A buffer taken from `Undefined` onto another queue has lost its contents; on its own
    // queue it keeps them ("States").
    if ctx.set.queues.compute != QueuePath::Own {
        return Ok(());
    }
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &[1; SIZE as usize]);
    let usage = BufferUsage::COPY_SOURCE.union(BufferUsage::COPY_DESTINATION);
    let data = ctx.buffer(&*device, SIZE, usage, MemoryClass::Device);
    let region = [BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }];
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_buffers(&[
        buffer_to(source, BufferState::Undefined, BufferState::CopySource, G),
        buffer_to(data, BufferState::Undefined, BufferState::CopyDestination, G),
    ]);
    encoder.copy_buffer(source, data, &region);
    run(ctx, &*device, G, &mut [encoder]);
    let readback = ctx.buffer(&*device, SIZE, BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
    let mut encoder = ctx.encoder(&*device, C);
    encoder.transition_buffers(&[
        buffer_across(data, (BufferState::Undefined, C), (BufferState::CopySource, C)),
        buffer_to(readback, BufferState::Undefined, BufferState::CopyDestination, C),
    ]);
    encoder.copy_buffer(data, readback, &region);
    ctx.refused(
        submit(&*device, C, &mut [encoder]),
        "a copy from a buffer taken from Undefined onto another queue",
    );
    Ok(())
}

/// A buffer of `SIZE` bytes that copies read and write, in device memory.
fn copied(ctx: &Ctx<'_>, device: &dyn Device, sharing: Sharing) -> BufferHandle {
    let usage = BufferUsage::COPY_SOURCE.union(BufferUsage::COPY_DESTINATION);
    ctx.buffer_shared(device, SIZE, usage, MemoryClass::Device, sharing)
}

/// Records a copy of the whole of `source` into `destination`, each transitioned from
/// `Undefined` on the encoder's queue first.
fn copy_fresh(encoder: &mut dyn Encoder, source: BufferHandle, destination: BufferHandle) {
    let queue = encoder.queue();
    encoder.transition_buffers(&[
        buffer_to(source, BufferState::Undefined, BufferState::CopySource, queue),
        buffer_to(destination, BufferState::Undefined, BufferState::CopyDestination, queue),
    ]);
    encoder.copy_buffer(
        source,
        destination,
        &[BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }],
    );
}

/// A `Concurrent` image that copies read and write.
fn shared_image(ctx: &Ctx<'_>, device: &dyn Device) -> ImageHandle {
    let usage = ImageUsage::COPY_DESTINATION.union(ImageUsage::COPY_SOURCE);
    ctx.image(
        device,
        &ImageDesc {
            sharing: Sharing::Concurrent,
            ..image_desc(Format::Rgba8Unorm, EXTENT, usage)
        },
    )
}

/// Records copies from `buffer` and from `image` in two graphics submits. Each is refused
/// with no wait for the submit `gate` holds, and passes with it.
fn copies_from_refused(
    ctx: &Ctx<'_>,
    device: &dyn Device,
    gate: Gate,
    (buffer, image): (BufferHandle, ImageHandle),
    what: &str,
) {
    let local = copied(ctx, device, Sharing::Exclusive);
    let mut buffer_reader = ctx.encoder(device, G);
    buffer_reader.transition_buffers(&[buffer_to(
        local,
        BufferState::Undefined,
        BufferState::CopyDestination,
        G,
    )]);
    buffer_reader.copy_buffer(
        buffer,
        local,
        &[BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }],
    );
    let mut buffer_readers = [buffer_reader];
    ctx.refused(
        submit(device, G, &mut buffer_readers),
        &format!("a copy from a buffer {what}, with no wait for it"),
    );
    let readback =
        ctx.buffer(device, SIZE * 64, BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
    let mut image_reader = ctx.encoder(device, G);
    image_reader.transition_buffers(&[buffer_to(
        readback,
        BufferState::Undefined,
        BufferState::CopyDestination,
        G,
    )]);
    image_reader.copy_image_to_buffer(image, readback, &[whole_image()]);
    let mut image_readers = [image_reader];
    ctx.refused(
        submit(device, G, &mut image_readers),
        &format!("a copy from an image {what}, with no wait for it"),
    );
    ctx.submit(device, G, &mut buffer_readers, &[gate.done], &[]);
    let done = ctx.point(device, 1);
    ctx.submit(device, G, &mut image_readers, &[gate.done], &[done]);
    gate.open(ctx, device);
    ctx.reached(device, done);
}

/// Compute writes a `Concurrent` buffer and a `Concurrent` image in a submit held pending.
/// A graphics submit that copies from either with no wait for that submit is refused; with
/// the wait it passes.
fn use_under_another_queues_pending_write_is_refused(ctx: &Ctx<'_>) -> Applies {
    several_queues(ctx)?;
    let device = ctx.device();
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &[7; SIZE as usize]);
    let shared = copied(ctx, &*device, Sharing::Concurrent);
    let image = shared_image(ctx, &*device);
    let mut writer = ctx.encoder(&*device, C);
    copy_fresh(&mut *writer, source, shared);
    fill(&mut *writer, texels(ctx, &*device), image);
    writer.transition_buffers(&[buffer_to(
        shared,
        BufferState::CopyDestination,
        BufferState::CopySource,
        C,
    )]);
    writer.transition_images(&[image_to(
        image,
        ImageState::CopyDestination,
        ImageState::CopySource,
        C,
    )]);
    let mut writers = [writer];
    let gate = ctx.held(&*device, C, &mut writers);
    copies_from_refused(
        ctx,
        &*device,
        gate,
        (shared, image),
        "another queue's pending submit writes",
    );
    Ok(())
}

/// Compute fills a `Concurrent` image and moves it to `SampledRead` in a submit held pending.
/// A graphics dispatch whose pass declares a heap slot that holds the image, with no wait for
/// that submit, is refused; with the wait it passes.
fn sampled_read_under_another_queues_pending_write_is_refused(ctx: &Ctx<'_>) -> Applies {
    several_queues(ctx)?;
    let device = ctx.device();
    let usage = ImageUsage::SAMPLED.union(ImageUsage::COPY_DESTINATION);
    let image = ctx.image(
        &*device,
        &ImageDesc {
            sharing: Sharing::Concurrent,
            ..image_desc(Format::Rgba8Unorm, EXTENT, usage)
        },
    );
    let mut writer = ctx.encoder(&*device, C);
    fill(&mut *writer, texels(ctx, &*device), image);
    writer.transition_images(&[image_to(
        image,
        ImageState::CopyDestination,
        ImageState::SampledRead,
        C,
    )]);
    let mut writers = [writer];
    let gate = ctx.held(&*device, C, &mut writers);
    let slot = HeapSlot(3);
    ctx.ok(device.write_heap_slot(slot, image), "a write of the slot");
    let pipeline = ctx.compute(&*device);
    let mut reader = ctx.encoder(&*device, G);
    compute_pass(&mut *reader, pipeline, PassReach { slots: &[slot], buffers: &[] });
    reader.push_slot(0, slot);
    reader.dispatch([1, 1, 1]);
    reader.end_pass();
    let mut readers = [reader];
    ctx.refused(
        submit(&*device, G, &mut readers),
        "a sampled read of an image another queue's pending submit writes, with no wait for it",
    );
    let done = ctx.point(&*device, 1);
    ctx.submit(&*device, G, &mut readers, &[gate.done], &[done]);
    gate.open(ctx, &*device);
    ctx.reached(&*device, done);
    Ok(())
}

/// `Concurrent` buffers and images, each filled and seen where it is read. Compute only
/// transitions them, with no write, in a submit held pending. A graphics submit with no wait
/// for that submit is refused, as under a write, when it copies from a buffer or an image,
/// draws to an image as an attachment, declares a buffer in a pass, draws with a buffer of
/// indices, or dispatches through a buffer of counts; with the wait it passes.
fn use_under_another_queues_pending_transition_is_refused(ctx: &Ctx<'_>) -> Applies {
    several_queues(ctx)?;
    let device = ctx.device();
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &[2; SIZE as usize]);
    let zeros = ctx.upload(&*device, BufferUsage::EMPTY, &[0; SIZE as usize]);
    let shared = copied(ctx, &*device, Sharing::Concurrent);
    let image = shared_image(ctx, &*device);
    let declared = shared_buffer(ctx, &*device, BufferUsage::DEVICE_ADDRESS);
    let indices = shared_buffer(ctx, &*device, BufferUsage::INDEX);
    let ones: Vec<u8> = [1u32; 3].iter().flat_map(|count| count.to_le_bytes()).collect();
    let groups = ctx.upload(&*device, BufferUsage::EMPTY, &ones);
    let counts = shared_buffer(ctx, &*device, BufferUsage::INDIRECT);
    let whole = [BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }];
    let mut setup = ctx.encoder(&*device, C);
    copy_fresh(&mut *setup, source, shared);
    setup.transition_buffers(&[
        buffer_to(zeros, BufferState::Undefined, BufferState::CopySource, C),
        buffer_to(declared, BufferState::Undefined, BufferState::CopyDestination, C),
        buffer_to(indices, BufferState::Undefined, BufferState::CopyDestination, C),
        buffer_to(groups, BufferState::Undefined, BufferState::CopySource, C),
        buffer_to(counts, BufferState::Undefined, BufferState::CopyDestination, C),
    ]);
    setup.copy_buffer(source, declared, &whole);
    setup.copy_buffer(zeros, indices, &whole);
    let three = [BufferCopy { source_offset: 0, destination_offset: 0, size: 12 }];
    setup.copy_buffer(groups, counts, &three);
    fill(&mut *setup, texels(ctx, &*device), image);
    run(ctx, &*device, C, &mut [setup]);
    let mut changer = ctx.encoder(&*device, C);
    changer.transition_buffers(&[buffer_to(
        shared,
        BufferState::CopyDestination,
        BufferState::CopySource,
        C,
    )]);
    changer.transition_images(&[image_to(
        image,
        ImageState::CopyDestination,
        ImageState::CopySource,
        C,
    )]);
    let mut changers = [changer];
    let gate = ctx.held(&*device, C, &mut changers);
    copies_from_refused(
        ctx,
        &*device,
        gate,
        (shared, image),
        "another queue's pending submit only transitions",
    );
    let usage = ImageUsage::COLOUR.union(ImageUsage::COPY_SOURCE);
    let desc =
        ImageDesc { sharing: Sharing::Concurrent, ..image_desc(Format::Rgba8Unorm, EXTENT, usage) };
    let target = ctx.image(&*device, &desc);
    let mut changer = ctx.encoder(&*device, C);
    changer.transition_images(&[image_to(
        target,
        ImageState::Undefined,
        ImageState::ColourAttachment,
        C,
    )]);
    let mut user = ctx.encoder(&*device, G);
    begin(
        &mut *user,
        &[attachment(target, Load::Clear([0.0; 4]), Store::Store, false)],
        PassReach::NOTHING,
    );
    user.end_pass();
    pending_refused(
        ctx,
        &*device,
        changer,
        user,
        "an attachment another queue's pending submit only transitions",
    );
    let mut changer = ctx.encoder(&*device, C);
    changer.transition_buffers(&[buffer_to(
        declared,
        BufferState::CopyDestination,
        BufferState::ShaderRead,
        C,
    )]);
    let mut user = ctx.encoder(&*device, G);
    let read = [BufferReach { buffer: declared, state: BufferState::ShaderRead }];
    compute_pass(&mut *user, ctx.compute(&*device), PassReach { slots: &[], buffers: &read });
    user.end_pass();
    pending_refused(
        ctx,
        &*device,
        changer,
        user,
        "a pass that declares a buffer another queue's pending submit only transitions",
    );
    let mut changer = ctx.encoder(&*device, C);
    changer.transition_buffers(&[buffer_to(
        indices,
        BufferState::CopyDestination,
        BufferState::Index,
        C,
    )]);
    let target = ctx.target(&*device);
    let mut user = ctx.encoder(&*device, G);
    user.transition_images(&[image_to(
        target,
        ImageState::Undefined,
        ImageState::ColourAttachment,
        G,
    )]);
    begin(
        &mut *user,
        &[attachment(target, Load::Clear([0.0; 4]), Store::Store, false)],
        PassReach::NOTHING,
    );
    user.bind_graphics_pipeline(ctx.plain(&*device));
    user.bind_index_buffer(indices, 0);
    user.draw_indexed(0..3, 0, 0..1);
    user.end_pass();
    pending_refused(
        ctx,
        &*device,
        changer,
        user,
        "a draw with indices another queue's pending submit only transitions",
    );
    let mut changer = ctx.encoder(&*device, C);
    changer.transition_buffers(&[buffer_to(
        counts,
        BufferState::CopyDestination,
        BufferState::Indirect,
        C,
    )]);
    let mut user = ctx.encoder(&*device, G);
    compute_pass(&mut *user, ctx.compute(&*device), PassReach::NOTHING);
    user.dispatch_indirect(counts, 0);
    user.end_pass();
    pending_refused(
        ctx,
        &*device,
        changer,
        user,
        "a dispatch through counts another queue's pending submit only transitions",
    );
    Ok(())
}

/// A `Concurrent` buffer of [`SIZE`] bytes that a copy fills, with `usage` besides.
fn shared_buffer(ctx: &Ctx<'_>, device: &dyn Device, usage: BufferUsage) -> BufferHandle {
    let usage = usage.union(BufferUsage::COPY_DESTINATION);
    ctx.buffer_shared(device, SIZE, usage, MemoryClass::Device, Sharing::Concurrent)
}

/// Holds `changer` pending on compute. `user`, submitted to graphics with no wait for it, is
/// refused, and passes with the wait.
fn pending_refused(
    ctx: &Ctx<'_>,
    device: &dyn Device,
    changer: Box<dyn Encoder>,
    user: Box<dyn Encoder>,
    what: &str,
) {
    let mut changers = [changer];
    let gate = ctx.held(device, C, &mut changers);
    let mut users = [user];
    ctx.refused(submit(device, G, &mut users), &format!("{what}, with no wait for it"));
    let done = ctx.point(device, 1);
    ctx.submit(device, G, &mut users, &[gate.done], &[done]);
    gate.open(ctx, device);
    ctx.reached(device, done);
}

/// Compute, in a submit held pending, writes an `Exclusive` buffer and a `Concurrent` one,
/// each taken from `Undefined` there, only transitions a second `Concurrent` buffer, and only
/// writes a third. A graphics submit that waits for it takes the first from `Undefined` and
/// moves the third to `CopySource`. A later graphics submit with no wait of its own is
/// refused when it copies into the first, transitions the `Concurrent` one or the second, or
/// copies from the second or the third; each passes with the wait. An image a pending submit
/// of the other queue wrote by a copy or an attachment, moved by an alias call or only took
/// from `Undefined`, with no transition in that submit, binds a later use the same way; so
/// does a buffer or an image it released to the using queue. Neither a take from `Undefined`
/// nor an acquire in an earlier submit orders a later one.
fn use_in_a_later_submit_than_the_waiting_change_is_refused(ctx: &Ctx<'_>) -> Applies {
    several_queues(ctx)?;
    let device = ctx.device();
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &[4; SIZE as usize]);
    let taken = copied(ctx, &*device, Sharing::Exclusive);
    let shared = copied(ctx, &*device, Sharing::Concurrent);
    let turned = copied(ctx, &*device, Sharing::Concurrent);
    let written = copied(ctx, &*device, Sharing::Concurrent);
    let whole = [BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }];
    let mut setup = ctx.encoder(&*device, C);
    copy_fresh(&mut *setup, ctx.upload(&*device, BufferUsage::EMPTY, &[3; SIZE as usize]), turned);
    setup.transition_buffers(&[buffer_to(
        written,
        BufferState::Undefined,
        BufferState::CopyDestination,
        C,
    )]);
    run(ctx, &*device, C, &mut [setup]);
    let mut writer = ctx.encoder(&*device, C);
    copy_fresh(&mut *writer, source, taken);
    writer.transition_buffers(&[
        buffer_to(shared, BufferState::Undefined, BufferState::CopyDestination, C),
        buffer_to(turned, BufferState::CopyDestination, BufferState::CopySource, C),
    ]);
    writer.copy_buffer(source, shared, &whole);
    writer.copy_buffer(source, written, &whole);
    let mut writers = [writer];
    let gate = ctx.held(&*device, C, &mut writers);
    let mut take = ctx.encoder(&*device, G);
    take.transition_buffers(&[
        buffer_to(taken, BufferState::Undefined, BufferState::CopyDestination, G),
        buffer_to(written, BufferState::CopyDestination, BufferState::CopySource, G),
    ]);
    let mut takes = [take];
    ctx.submit(&*device, G, &mut takes, &[gate.done], &[]);
    let other = ctx.upload(&*device, BufferUsage::EMPTY, &[5; SIZE as usize]);
    let mut user = ctx.encoder(&*device, G);
    user.transition_buffers(&[buffer_to(
        other,
        BufferState::Undefined,
        BufferState::CopySource,
        G,
    )]);
    user.copy_buffer(other, taken, &whole);
    let mut changer = ctx.encoder(&*device, G);
    changer.transition_buffers(&[buffer_to(
        shared,
        BufferState::CopyDestination,
        BufferState::CopySource,
        G,
    )]);
    let reader = |buffer: BufferHandle| {
        let local = copied(ctx, &*device, Sharing::Exclusive);
        let mut reader = ctx.encoder(&*device, G);
        reader.transition_buffers(&[buffer_to(
            local,
            BufferState::Undefined,
            BufferState::CopyDestination,
            G,
        )]);
        reader.copy_buffer(buffer, local, &whole);
        reader
    };
    let mut turner = ctx.encoder(&*device, G);
    turner.transition_buffers(&[buffer_to(
        turned,
        BufferState::CopySource,
        BufferState::CopyDestination,
        G,
    )]);
    // In the order they pass once they wait.
    let mut laters = [user, changer, reader(turned), turner, reader(written)];
    let refusals = [
        "a copy into a buffer taken in an earlier submit that waited",
        "a transition of a `Concurrent` buffer after an earlier submit that waited",
        "a copy from a buffer the other queue's pending submit only transitioned, after an \
         earlier submit that waited",
        "a transition of a buffer the other queue's pending submit only transitioned, after an \
         earlier submit that waited",
        "a copy from a buffer the other queue's pending submit only wrote, after an earlier \
         submit that waited and transitioned it",
    ];
    for (later, what) in laters.iter_mut().zip(refusals) {
        ctx.refused(
            submit(&*device, G, core::slice::from_mut(later)),
            &format!("{what}, with no wait of its own"),
        );
    }
    let done = ctx.point(&*device, 1);
    let last = laters.len() - 1;
    for (at, later) in laters.iter_mut().enumerate() {
        let signals: &[_] = if at == last { &[done] } else { &[] };
        ctx.submit(&*device, G, core::slice::from_mut(later), &[gate.done], signals);
    }
    gate.open(ctx, &*device);
    ctx.reached(&*device, done);
    // A buffer acquired in an earlier submit that waited for its release.
    let crossed = copied(ctx, &*device, Sharing::Exclusive);
    let mut release = ctx.encoder(&*device, C);
    copy_fresh(&mut *release, source, crossed);
    let crossing =
        buffer_across(crossed, (BufferState::CopyDestination, C), (BufferState::CopySource, G));
    release.transition_buffers(&[crossing]);
    let mut acquire = ctx.encoder(&*device, G);
    acquire.transition_buffers(&[crossing]);
    later_refused(
        ctx,
        &*device,
        (C, release),
        (G, &mut [acquire], reader(crossed)),
        "a copy from a buffer acquired in an earlier submit that waited",
    );
    // An image a pending submit writes, by a copy, an attachment or an alias call, or only
    // takes from `Undefined`, with no transition in that submit after it, binds a later use
    // as a transitioned one does.
    let copied_into = shared_image(ctx, &*device);
    let mut setup = ctx.encoder(&*device, C);
    setup.transition_images(&[image_to(
        copied_into,
        ImageState::Undefined,
        ImageState::CopyDestination,
        C,
    )]);
    run(ctx, &*device, C, &mut [setup]);
    let mut writer = ctx.encoder(&*device, C);
    fill_again(&mut *writer, texels(ctx, &*device), copied_into);
    let mut later = ctx.encoder(&*device, G);
    later.transition_images(&[image_to(
        copied_into,
        ImageState::CopyDestination,
        ImageState::CopySource,
        G,
    )]);
    later_refused(
        ctx,
        &*device,
        (C, writer),
        (G, &mut [], later),
        "a transition of an image a copy wrote, after an earlier submit that waited",
    );
    let usage = ImageUsage::COLOUR.union(ImageUsage::COPY_SOURCE);
    let desc =
        ImageDesc { sharing: Sharing::Concurrent, ..image_desc(Format::Rgba8Unorm, EXTENT, usage) };
    let target = ctx.image(&*device, &desc);
    let mut setup = ctx.encoder(&*device, G);
    setup.transition_images(&[image_to(
        target,
        ImageState::Undefined,
        ImageState::ColourAttachment,
        G,
    )]);
    run(ctx, &*device, G, &mut [setup]);
    let mut writer = ctx.encoder(&*device, G);
    begin(
        &mut *writer,
        &[attachment(target, Load::Clear([0.0; 4]), Store::Store, false)],
        PassReach::NOTHING,
    );
    writer.end_pass();
    let mut later = ctx.encoder(&*device, C);
    later.transition_images(&[image_to(
        target,
        ImageState::ColourAttachment,
        ImageState::CopySource,
        C,
    )]);
    later_refused(
        ctx,
        &*device,
        (G, writer),
        (C, &mut [], later),
        "a transition of an image a pass stored, after an earlier submit that waited",
    );
    let desc = image_desc(Format::Rgba8Unorm, EXTENT, ImageUsage::COLOUR);
    let placement = ctx.ok(device.image_placement(&desc), "a placement");
    let block = ctx.ok(
        device.create_memory_block(&MemoryBlockDesc { label: "conformance", size: placement.size }),
        "a block",
    );
    let placed = ctx
        .image(&*device, &ImageDesc { memory: ImageMemory::Placed { block, offset: 0 }, ..desc });
    let mut writer = ctx.encoder(&*device, C);
    writer.alias(placed);
    let mut later = ctx.encoder(&*device, G);
    later.transition_images(&[image_to(
        placed,
        ImageState::Undefined,
        ImageState::ColourAttachment,
        G,
    )]);
    later_refused(
        ctx,
        &*device,
        (C, writer),
        (G, &mut [], later),
        "a take of an image an alias call moved, after an earlier submit that waited",
    );
    // An `Exclusive` image the other queue's pending submit only took from `Undefined`, taken
    // back from `Undefined` by an earlier submit that waited.
    let retaken = copyable(ctx, &*device);
    let mut writer = ctx.encoder(&*device, C);
    writer.transition_images(&[image_to(
        retaken,
        ImageState::Undefined,
        ImageState::CopyDestination,
        C,
    )]);
    let mut retake = ctx.encoder(&*device, G);
    retake.transition_images(&[image_to(
        retaken,
        ImageState::Undefined,
        ImageState::CopyDestination,
        G,
    )]);
    let mut later = ctx.encoder(&*device, G);
    fill_again(&mut *later, texels(ctx, &*device), retaken);
    later_refused(
        ctx,
        &*device,
        (C, writer),
        (G, &mut [retake], later),
        "a copy into an image the other queue only took from Undefined, after an earlier submit \
         that waited and took it back",
    );
    // An image acquired in an earlier submit that waited for its release.
    let crossed = copyable(ctx, &*device);
    let mut release = ctx.encoder(&*device, C);
    fill(&mut *release, texels(ctx, &*device), crossed);
    let crossing = image_across(
        crossed,
        ImageRange::FIRST,
        (ImageState::CopyDestination, C),
        (ImageState::CopySource, G),
    );
    release.transition_images(&[crossing]);
    let mut acquire = ctx.encoder(&*device, G);
    acquire.transition_images(&[crossing]);
    let readback =
        ctx.buffer(&*device, SIZE * 64, BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
    let mut reader = ctx.encoder(&*device, G);
    reader.transition_buffers(&[buffer_to(
        readback,
        BufferState::Undefined,
        BufferState::CopyDestination,
        G,
    )]);
    reader.copy_image_to_buffer(crossed, readback, &[whole_image()]);
    later_refused(
        ctx,
        &*device,
        (C, release),
        (G, &mut [acquire], reader),
        "a copy from an image acquired in an earlier submit that waited",
    );
    Ok(())
}

/// Compute takes an `Exclusive` image and an `Exclusive` buffer and releases each to
/// graphics, in a submit held pending. A graphics submit waits for it and records nothing; a
/// later graphics submit that acquires either with no wait of its own is refused, and passes
/// with the wait. An acquire is a use: the wait of an earlier submit of its queue does not
/// cover it.
fn acquire_in_a_later_submit_than_the_wait_for_its_release_is_refused(ctx: &Ctx<'_>) -> Applies {
    several_queues(ctx)?;
    let device = ctx.device();
    let image = copyable(ctx, &*device);
    let crossing = image_across(
        image,
        ImageRange::FIRST,
        (ImageState::CopyDestination, C),
        (ImageState::CopySource, G),
    );
    let mut release = ctx.encoder(&*device, C);
    release.transition_images(&[image_to(
        image,
        ImageState::Undefined,
        ImageState::CopyDestination,
        C,
    )]);
    release.transition_images(&[crossing]);
    let mut acquire = ctx.encoder(&*device, G);
    acquire.transition_images(&[crossing]);
    later_refused(
        ctx,
        &*device,
        (C, release),
        (G, &mut [], acquire),
        "an acquire of an image after an earlier submit that waited for its release",
    );
    let buffer = copied(ctx, &*device, Sharing::Exclusive);
    let crossing =
        buffer_across(buffer, (BufferState::CopyDestination, C), (BufferState::CopySource, G));
    let mut release = ctx.encoder(&*device, C);
    release.transition_buffers(&[buffer_to(
        buffer,
        BufferState::Undefined,
        BufferState::CopyDestination,
        C,
    )]);
    release.transition_buffers(&[crossing]);
    let mut acquire = ctx.encoder(&*device, G);
    acquire.transition_buffers(&[crossing]);
    later_refused(
        ctx,
        &*device,
        (C, release),
        (G, &mut [], acquire),
        "an acquire of a buffer after an earlier submit that waited for its release",
    );
    Ok(())
}

/// Compute takes both mips of an `Exclusive` image and releases mip 0 to graphics, in a
/// submit held pending. A graphics submit that waits for it acquires mip 0, or only waits;
/// either way a later graphics submit with no wait of its own that takes mip 1 from
/// `Undefined` is refused, and passes with the wait. An acquire of one subresource orders no
/// later use of another.
fn take_of_a_mip_after_an_acquire_of_another_is_refused(ctx: &Ctx<'_>) -> Applies {
    several_queues(ctx)?;
    let usage = ImageUsage::COPY_DESTINATION.union(ImageUsage::COPY_SOURCE);
    let desc = ImageDesc { mips: 2, ..image_desc(Format::Rgba8Unorm, EXTENT, usage) };
    let [both, first, second] =
        [(0, 2), (0, 1), (1, 1)].map(|(mip, mips)| ImageRange { mip, mips, layer: 0, layers: 1 });
    for acquires in [false, true] {
        let device = ctx.device();
        let image = ctx.image(&*device, &desc);
        let mut release = ctx.encoder(&*device, C);
        release.transition_images(&[image_across(
            image,
            both,
            (ImageState::Undefined, C),
            (ImageState::CopyDestination, C),
        )]);
        let crossing = image_across(
            image,
            first,
            (ImageState::CopyDestination, C),
            (ImageState::CopySource, G),
        );
        release.transition_images(&[crossing]);
        let mut acquire = ctx.encoder(&*device, G);
        if acquires {
            acquire.transition_images(&[crossing]);
        }
        let mut take = ctx.encoder(&*device, G);
        take.transition_images(&[image_across(
            image,
            second,
            (ImageState::Undefined, G),
            (ImageState::CopyDestination, G),
        )]);
        let what = if acquires {
            "a take of mip 1 after an earlier submit that waited and acquired mip 0"
        } else {
            "a take of mip 1 after an earlier submit that only waited"
        };
        later_refused(ctx, &*device, (C, release), (G, &mut [acquire], take), what);
    }
    Ok(())
}

/// Holds `writer` pending on its queue, then submits `between` to the other queue in a
/// submit that waits for it, and `later`, which waits for nothing and is refused; `later`
/// passes once it waits too.
fn later_refused(
    ctx: &Ctx<'_>,
    device: &dyn Device,
    (on, writer): (QueueRole, Box<dyn Encoder>),
    (other, between, later): (QueueRole, &mut [Box<dyn Encoder>], Box<dyn Encoder>),
    what: &str,
) {
    let mut writers = [writer];
    let gate = ctx.held(device, on, &mut writers);
    ctx.submit(device, other, between, &[gate.done], &[]);
    let mut later = [later];
    ctx.refused(submit(device, other, &mut later), &format!("{what}, with no wait of its own"));
    let done = ctx.point(device, 1);
    ctx.submit(device, other, &mut later, &[gate.done], &[done]);
    gate.open(ctx, device);
    ctx.reached(device, done);
}

/// Compute writes a `Concurrent` buffer in a submit held pending. A graphics submit waits
/// for it and signals a point; a later graphics submit that waits only for that point
/// copies from the buffer, ordered after the write through the submit it waits for.
fn use_waiting_through_another_submit_passes(ctx: &Ctx<'_>) -> Applies {
    several_queues(ctx)?;
    let device = ctx.device();
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &[8; SIZE as usize]);
    let shared = copied(ctx, &*device, Sharing::Concurrent);
    let mut writer = ctx.encoder(&*device, C);
    copy_fresh(&mut *writer, source, shared);
    writer.transition_buffers(&[buffer_to(
        shared,
        BufferState::CopyDestination,
        BufferState::CopySource,
        C,
    )]);
    let mut writers = [writer];
    let gate = ctx.held(&*device, C, &mut writers);
    let relay = ctx.point(&*device, 1);
    ctx.submit(&*device, G, &mut [], &[gate.done], &[relay]);
    let local = copied(ctx, &*device, Sharing::Exclusive);
    let mut reader = ctx.encoder(&*device, G);
    reader.transition_buffers(&[buffer_to(
        local,
        BufferState::Undefined,
        BufferState::CopyDestination,
        G,
    )]);
    reader.copy_buffer(
        shared,
        local,
        &[BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }],
    );
    let done = ctx.point(&*device, 1);
    let mut readers = [reader];
    ctx.submit(&*device, G, &mut readers, &[relay], &[done]);
    gate.open(ctx, &*device);
    ctx.reached(&*device, done);
    Ok(())
}

/// A buffer graphics wrote, released to compute and seen complete.
fn released(ctx: &Ctx<'_>, device: &dyn Device) -> (BufferHandle, BufferTransition) {
    let source = ctx.upload(device, BufferUsage::EMPTY, &[6; SIZE as usize]);
    let local = copied(ctx, device, Sharing::Exclusive);
    let crossing =
        buffer_across(local, (BufferState::CopyDestination, G), (BufferState::CopySource, C));
    let mut release = ctx.encoder(device, G);
    copy_fresh(&mut *release, source, local);
    release.transition_buffers(&[crossing]);
    run(ctx, device, G, &mut [release]);
    (local, crossing)
}

/// Graphics takes back from `Undefined` a buffer it released, before compute acquires it:
/// the take is accepted and ends the release, so the acquire is refused.
fn acquire_of_a_release_a_retake_ended_is_refused(ctx: &Ctx<'_>) -> Applies {
    several_queues(ctx)?;
    let device = ctx.device();
    let (local, crossing) = released(ctx, &*device);
    let mut take = ctx.encoder(&*device, G);
    take.transition_buffers(&[buffer_to(
        local,
        BufferState::Undefined,
        BufferState::CopyDestination,
        G,
    )]);
    run(ctx, &*device, G, &mut [take]);
    let mut acquire = ctx.encoder(&*device, C);
    acquire.transition_buffers(&[crossing]);
    ctx.refused(submit(&*device, C, &mut [acquire]), "an acquire of a release a take ended");
    Ok(())
}

/// A buffer taken back from `Undefined` by the queue that released it, with no acquire,
/// holds undefined contents: a copy from it is refused. One taken so with no release
/// pending keeps its bytes.
fn read_after_a_retake_of_an_unmet_release_is_refused(ctx: &Ctx<'_>) -> Applies {
    several_queues(ctx)?;
    let device = ctx.device();
    let (local, _) = released(ctx, &*device);
    let readback = ctx.buffer(&*device, SIZE, BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
    let mut reader = ctx.encoder(&*device, G);
    copy_fresh(&mut *reader, local, readback);
    ctx.refused(
        submit(&*device, G, &mut [reader]),
        "a copy from a buffer taken back from `Undefined` with its release unmet",
    );
    let kept = copied(ctx, &*device, Sharing::Exclusive);
    let mut writer = ctx.encoder(&*device, G);
    copy_fresh(&mut *writer, ctx.upload(&*device, BufferUsage::EMPTY, &[3; SIZE as usize]), kept);
    run(ctx, &*device, G, &mut [writer]);
    let mut reader = ctx.encoder(&*device, G);
    copy_fresh(&mut *reader, kept, readback);
    run(ctx, &*device, G, &mut [reader]);
    Ok(())
}
