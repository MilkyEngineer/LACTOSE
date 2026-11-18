//! States and queues.

use super::kit::{
    Body, Ctx, EXTENT, attachment, begin, buffer_across, buffer_to, clear, compute_pass,
    image_across, image_desc, image_to, whole_image,
};
use crate::{
    BufferCopy, BufferHandle, BufferImageCopy, BufferReach, BufferState, BufferUsage, Device,
    Encoder, Extent, Format, ImageHandle, ImageRange, ImageState, ImageUsage, Load, MemoryClass,
    PassReach, QueuePath, QueueRole, Rect, Rule, Sharing, Store, Submit,
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

/// Compute reads an image it owns in a submit held pending; graphics then takes it, from
/// `Undefined`, with no wait for that submit.
fn image_taken_while_another_queue_reads_it_is_refused(ctx: &Ctx<'_>) -> Applies {
    several_queues(ctx)?;
    let device = ctx.device();
    let image = copyable(ctx, &*device);
    let readback =
        ctx.buffer(&*device, SIZE * 64, BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
    let mut reader = ctx.encoder(&*device, C);
    fill(&mut *reader, texels(ctx, &*device), image);
    reader.transition_images(&[image_to(
        image,
        ImageState::CopyDestination,
        ImageState::CopySource,
        C,
    )]);
    reader.transition_buffers(&[buffer_to(
        readback,
        BufferState::Undefined,
        BufferState::CopyDestination,
        C,
    )]);
    reader.copy_image_to_buffer(image, readback, &[whole_image()]);
    let mut readers = [reader];
    let gate = ctx.held(&*device, C, &mut readers);
    // A buffer's take is checked as an image's is: the buffer the reader writes.
    let mut buffer_taker = ctx.encoder(&*device, G);
    buffer_taker.transition_buffers(&[buffer_to(
        readback,
        BufferState::Undefined,
        BufferState::CopyDestination,
        G,
    )]);
    let mut buffer_takers = [buffer_taker];
    ctx.refused(
        submit(&*device, G, &mut buffer_takers),
        "a buffer's take with no wait for the other queue's writer",
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
