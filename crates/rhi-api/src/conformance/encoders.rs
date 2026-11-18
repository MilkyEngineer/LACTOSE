//! Encoders and submits.

use std::sync::mpsc;
use std::time::Duration;

use super::kit::{
    Body, Ctx, EXTENT, WOKEN, attachment, begin, buffer_to, clear, compute_pass, image_across,
    image_desc, image_to, whole_image,
};
use crate::{
    BufferCopy, BufferHandle, BufferImageCopy, BufferReach, BufferState, BufferUsage, Device,
    Encoder, Format, ImageDesc, ImageRange, ImageState, ImageUsage, Load, MemoryClass,
    PUSH_BYTES_MAX, PassReach, QueueRole, Rect, Rule, Store, Submit,
};

type Applies = Result<(), &'static str>;

pub(super) const BODIES: &[(&str, Body)] = &[
    ("encoder_recorded_on_another_thread_submits", encoder_recorded_on_another_thread_submits),
    ("encoder_reset_while_pending_is_refused", encoder_reset_while_pending_is_refused),
    ("pending_encoder_drop_waits_for_its_submit", pending_encoder_drop_waits_for_its_submit),
    ("encoder_in_two_submits_is_refused", encoder_in_two_submits_is_refused),
    ("encoder_resubmitted_without_reset_is_refused", encoder_resubmitted_without_reset_is_refused),
    ("recording_while_pending_records_nothing", recording_while_pending_records_nothing),
    ("encoder_of_another_role_or_device_is_refused", encoder_of_another_role_or_device_is_refused),
    ("command_outside_its_scope_is_refused", command_outside_its_scope_is_refused),
    ("draw_with_nothing_bound_is_refused", draw_with_nothing_bound_is_refused),
    ("pass_left_open_is_refused", pass_left_open_is_refused),
    ("push_outside_the_block_is_refused", push_outside_the_block_is_refused),
    ("copy_region_outside_its_resource_is_refused", copy_region_outside_its_resource_is_refused),
    (
        "copy_regions_overlapping_in_the_destination_are_refused",
        copy_regions_overlapping_in_the_destination_are_refused,
    ),
    ("refused_submit_changes_nothing", refused_submit_changes_nothing),
    ("submit_of_no_encoders_waits_and_signals", submit_of_no_encoders_waits_and_signals),
    ("submit_is_pending_until_its_wait_is_reached", submit_is_pending_until_its_wait_is_reached),
    ("empty_draws_and_dispatches_record_nothing", empty_draws_and_dispatches_record_nothing),
];

const G: QueueRole = QueueRole::Graphics;
const SIZE: u64 = 256;

/// What a refusal is of, and the commands that make it.
type Recording<'a> = (&'static str, &'a dyn Fn(&mut dyn Encoder));

fn submit(
    device: &dyn Device,
    role: QueueRole,
    encoders: &mut [Box<dyn Encoder>],
) -> lactose_core::Result<()> {
    device.submit(role, Submit { encoders, waits: &[], signals: &[] })
}

/// Submits `encoders` and waits for their point.
fn run(ctx: &Ctx<'_>, device: &dyn Device, encoders: &mut [Box<dyn Encoder>]) {
    let done = ctx.point(device, 1);
    ctx.submit(device, G, encoders, &[], &[done]);
    ctx.reached(device, done);
}

/// Records a copy of the whole of `source` into `destination`, both from `Undefined`.
fn copy(encoder: &mut dyn Encoder, source: BufferHandle, destination: BufferHandle) {
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

fn pair(ctx: &Ctx<'_>, device: &dyn Device) -> (BufferHandle, BufferHandle) {
    let source = ctx.upload(device, BufferUsage::EMPTY, &[2; SIZE as usize]);
    (source, ctx.buffer(device, SIZE, BufferUsage::COPY_DESTINATION, MemoryClass::Device))
}

/// The drop of an encoder whose submit is pending blocks until the submit completes: on a
/// device backend what it recorded is still the GPU's.
fn pending_encoder_drop_waits_for_its_submit(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let (source, destination) = pair(ctx, &*device);
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_buffers(&[
        buffer_to(source, BufferState::Undefined, BufferState::CopySource, G),
        buffer_to(destination, BufferState::Undefined, BufferState::CopyDestination, G),
    ]);
    encoder.copy_buffer(
        source,
        destination,
        &[BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }],
    );
    let mut encoders = [encoder];
    let gate = ctx.held(&*device, G, &mut encoders);
    let [encoder] = encoders;
    let (sender, receiver) = mpsc::channel();
    // Not scoped: a drop that is never let go fails the case rather than hanging its join.
    std::thread::spawn(move || {
        drop(encoder);
        sender.send(()).ok();
    });
    ctx.check(
        receiver.recv_timeout(Duration::from_millis(300)).is_err(),
        "the drop of a pending encoder returns only once its submit completes",
    );
    gate.open(ctx, &*device);
    ctx.check(
        receiver.recv_timeout(WOKEN).is_ok(),
        "the drop of an encoder returns once its submit completes",
    );
    Ok(())
}

fn encoder_recorded_on_another_thread_submits(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let (source, destination) = pair(ctx, &*device);
    let target = ctx.target(&*device);
    let pipeline = ctx.plain(&*device);
    let encoders: Vec<Box<dyn Encoder>> = vec![ctx.encoder(&*device, G), ctx.encoder(&*device, G)];
    let mut encoders: Vec<Box<dyn Encoder>> = std::thread::scope(|scope| {
        let mut encoders = encoders.into_iter();
        let mut first = encoders.next().unwrap_or_else(|| ctx.fail("an encoder"));
        let mut second = encoders.next().unwrap_or_else(|| ctx.fail("an encoder"));
        let first = scope.spawn(move || {
            copy(&mut *first, source, destination);
            first
        });
        let second = scope.spawn(move || {
            clear(&mut *second, target, Some(pipeline));
            second
        });
        vec![
            first.join().unwrap_or_else(|_| ctx.fail("a recording thread panicked")),
            second.join().unwrap_or_else(|_| ctx.fail("a recording thread panicked")),
        ]
    });
    run(ctx, &*device, &mut encoders);
    Ok(())
}

fn encoder_reset_while_pending_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let (source, destination) = pair(ctx, &*device);
    let mut encoders = [ctx.encoder(&*device, G)];
    copy(&mut *encoders[0], source, destination);
    let gate = ctx.held(&*device, G, &mut encoders);
    ctx.refused(encoders[0].reset(), "a reset while pending");
    gate.open(ctx, &*device);
    ctx.ok(encoders[0].reset(), "the reset once the submit is seen complete");
    Ok(())
}

fn encoder_in_two_submits_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let (source, destination) = pair(ctx, &*device);
    let mut encoders = [ctx.encoder(&*device, G)];
    copy(&mut *encoders[0], source, destination);
    let gate = ctx.held(&*device, G, &mut encoders);
    ctx.refused(submit(&*device, G, &mut encoders), "a pending encoder in a second submit");
    let other = ctx.encoder(&*device, G);
    let mut both = [other, ctx.encoder(&*device, G)];
    core::mem::swap(&mut both[1], &mut encoders[0]);
    ctx.refused(submit(&*device, G, &mut both), "a pending encoder beside a fresh one");
    core::mem::swap(&mut both[1], &mut encoders[0]);
    gate.open(ctx, &*device);
    Ok(())
}

fn encoder_resubmitted_without_reset_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let (source, destination) = pair(ctx, &*device);
    let mut encoders = [ctx.encoder(&*device, G)];
    copy(&mut *encoders[0], source, destination);
    run(ctx, &*device, &mut encoders);
    ctx.refused(submit(&*device, G, &mut encoders), "a submit again with no reset");
    ctx.ok(encoders[0].reset(), "a reset");
    let queue = G;
    encoders[0].transition_buffers(&[buffer_to(
        destination,
        BufferState::Undefined,
        BufferState::CopyDestination,
        queue,
    )]);
    encoders[0].copy_buffer(
        source,
        destination,
        &[BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }],
    );
    run(ctx, &*device, &mut encoders);
    Ok(())
}

/// A held submit copies a pattern into a readback buffer; while it is pending its encoder
/// records a copy of other bytes over it, and a transition that would change its state.
/// Neither runs.
fn recording_while_pending_records_nothing(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let first = ctx.upload(&*device, BufferUsage::EMPTY, &[1; SIZE as usize]);
    let second = ctx.upload(&*device, BufferUsage::EMPTY, &[2; SIZE as usize]);
    let readback = ctx.buffer(&*device, SIZE, BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
    let mut encoders = [ctx.encoder(&*device, G)];
    copy(&mut *encoders[0], first, readback);
    encoders[0].transition_buffers(&[buffer_to(
        readback,
        BufferState::CopyDestination,
        BufferState::HostRead,
        G,
    )]);
    let gate = ctx.held(&*device, G, &mut encoders);
    encoders[0].transition_buffers(&[
        buffer_to(second, BufferState::Undefined, BufferState::CopySource, G),
        buffer_to(readback, BufferState::HostRead, BufferState::CopyDestination, G),
    ]);
    encoders[0].copy_buffer(
        second,
        readback,
        &[BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }],
    );
    encoders[0].begin_render_pass(&crate::RenderPass {
        label: "never ended",
        colour: &[],
        depth: None,
        reach: PassReach::NOTHING,
    });
    gate.open(ctx, &*device);
    ctx.check(
        ctx.read(&*device, readback, 0, SIZE) == [1; SIZE as usize],
        "the submit ran what it took, alone",
    );
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_buffers(&[buffer_to(
        readback,
        BufferState::HostRead,
        BufferState::CopyDestination,
        G,
    )]);
    run(ctx, &*device, &mut [encoder]);
    ctx.ok(encoders[0].reset(), "a reset");
    Ok(())
}

fn encoder_of_another_role_or_device_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let other = ctx.device();
    let compute = ctx.encoder(&*device, QueueRole::Compute);
    ctx.refused(submit(&*device, G, &mut [compute]), "a compute encoder submitted to graphics");
    let graphics = ctx.encoder(&*device, G);
    ctx.refused(
        submit(&*device, QueueRole::Compute, &mut [graphics]),
        "a graphics encoder submitted to compute",
    );
    let present = ctx.encoder(&*device, QueueRole::Present);
    ctx.refused(submit(&*device, G, &mut [present]), "a present encoder submitted to graphics");
    let foreign = ctx.encoder(&*other, G);
    ctx.refused(submit(&*device, G, &mut [foreign]), "another device's encoder");
    run(ctx, &*device, &mut [ctx.encoder(&*device, G)]);
    Ok(())
}

fn command_outside_its_scope_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let target = ctx.target(&*device);
    let (source, destination) = pair(ctx, &*device);
    let graphics = ctx.plain(&*device);
    let compute = ctx.compute(&*device);
    let render = |encoder: &mut dyn Encoder| {
        encoder.transition_images(&[image_to(
            target,
            ImageState::Undefined,
            ImageState::ColourAttachment,
            G,
        )]);
        encoder.transition_buffers(&[
            buffer_to(source, BufferState::Undefined, BufferState::CopySource, G),
            buffer_to(destination, BufferState::Undefined, BufferState::CopyDestination, G),
        ]);
        begin(
            encoder,
            &[attachment(target, Load::Clear([0.0; 4]), Store::Store, false)],
            PassReach::NOTHING,
        );
    };
    let breaches: [Recording<'_>; 7] = [
        ("a draw outside a pass", &|encoder| {
            encoder.bind_graphics_pipeline(graphics);
            encoder.draw(0..3, 0..1);
        }),
        ("a dispatch outside a pass", &|encoder| encoder.dispatch([1, 1, 1])),
        ("a copy inside a render pass", &|encoder| {
            render(encoder);
            encoder.copy_buffer(
                source,
                destination,
                &[BufferCopy { source_offset: 0, destination_offset: 0, size: 4 }],
            );
            encoder.end_pass();
        }),
        ("a dispatch inside a render pass", &|encoder| {
            render(encoder);
            encoder.dispatch([1, 1, 1]);
            encoder.end_pass();
        }),
        ("a draw inside a compute pass", &|encoder| {
            compute_pass(encoder, compute, PassReach::NOTHING);
            encoder.draw(0..3, 0..1);
            encoder.end_pass();
        }),
        ("a second begin inside a pass", &|encoder| {
            compute_pass(encoder, compute, PassReach::NOTHING);
            compute_pass(encoder, compute, PassReach::NOTHING);
            encoder.end_pass();
        }),
        ("an end with no pass", &|encoder| encoder.end_pass()),
    ];
    for (what, record) in breaches {
        let mut encoder = ctx.encoder(&*device, G);
        record(&mut *encoder);
        ctx.refused(submit(&*device, G, &mut [encoder]), what);
    }
    Ok(())
}

fn draw_with_nothing_bound_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let target = ctx.target(&*device);
    let pipeline = ctx.plain(&*device);
    let draws: [Recording<'_>; 3] = [
        ("a draw with no pipeline", &|encoder| encoder.draw(0..3, 0..1)),
        ("an indexed draw with no index buffer", &|encoder| {
            encoder.bind_graphics_pipeline(pipeline);
            encoder.draw_indexed(0..3, 0, 0..1);
        }),
        ("an indexed draw with no pipeline", &|encoder| encoder.draw_indexed(0..3, 0, 0..1)),
    ];
    for (what, draw) in draws {
        let mut encoder = ctx.encoder(&*device, G);
        encoder.transition_images(&[image_to(
            target,
            ImageState::Undefined,
            ImageState::ColourAttachment,
            G,
        )]);
        begin(
            &mut *encoder,
            &[attachment(target, Load::Clear([0.0; 4]), Store::Store, false)],
            PassReach::NOTHING,
        );
        draw(&mut *encoder);
        encoder.end_pass();
        ctx.refused(submit(&*device, G, &mut [encoder]), what);
    }
    let mut encoder = ctx.encoder(&*device, G);
    encoder.begin_compute_pass(&crate::ComputePass {
        label: "conformance",
        reach: PassReach::NOTHING,
    });
    encoder.dispatch([1, 1, 1]);
    encoder.end_pass();
    ctx.refused(submit(&*device, G, &mut [encoder]), "a dispatch with no pipeline");
    Ok(())
}

fn pass_left_open_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let target = ctx.target(&*device);
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_images(&[image_to(
        target,
        ImageState::Undefined,
        ImageState::ColourAttachment,
        G,
    )]);
    begin(
        &mut *encoder,
        &[attachment(target, Load::Clear([0.0; 4]), Store::Store, false)],
        PassReach::NOTHING,
    );
    ctx.refused(submit(&*device, G, &mut [encoder]), "a render pass left open");
    let mut encoder = ctx.encoder(&*device, G);
    encoder.begin_compute_pass(&crate::ComputePass {
        label: "conformance",
        reach: PassReach::NOTHING,
    });
    ctx.refused(submit(&*device, G, &mut [encoder]), "a compute pass left open");
    Ok(())
}

fn push_outside_the_block_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let pipeline = ctx.compute(&*device);
    let buffer = ctx.buffer(&*device, SIZE, BufferUsage::DEVICE_ADDRESS, MemoryClass::Device);
    let reach = [BufferReach { buffer, state: BufferState::ShaderWrite }];
    let pushes: [Recording<'_>; 4] = [
        ("push bytes past the block", &|encoder| encoder.push_bytes(PUSH_BYTES_MAX - 4, &[0; 8])),
        ("push bytes at the block's end", &|encoder| encoder.push_bytes(PUSH_BYTES_MAX, &[0])),
        ("an address pushed past the block", &|encoder| {
            encoder.push_address(PUSH_BYTES_MAX - 4, buffer, 0)
        }),
        ("bytes pushed at an offset that wraps", &|encoder| encoder.push_bytes(u32::MAX, &[0; 2])),
    ];
    for (what, push) in pushes {
        let mut encoder = ctx.encoder(&*device, G);
        encoder.transition_buffers(&[buffer_to(
            buffer,
            BufferState::Undefined,
            BufferState::ShaderWrite,
            G,
        )]);
        compute_pass(&mut *encoder, pipeline, PassReach { slots: &[], buffers: &reach });
        push(&mut *encoder);
        encoder.end_pass();
        ctx.refused(submit(&*device, G, &mut [encoder]), what);
    }
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_buffers(&[buffer_to(
        buffer,
        BufferState::Undefined,
        BufferState::ShaderWrite,
        G,
    )]);
    compute_pass(&mut *encoder, pipeline, PassReach { slots: &[], buffers: &reach });
    encoder.push_bytes(PUSH_BYTES_MAX - 8, &[0; 8]);
    encoder.push_address(0, buffer, SIZE - 1);
    encoder.dispatch([1, 1, 1]);
    encoder.end_pass();
    run(ctx, &*device, &mut [encoder]);
    Ok(())
}

fn copy_region_outside_its_resource_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let (source, destination) = pair(ctx, &*device);
    let usage =
        ImageUsage::COPY_DESTINATION.union(ImageUsage::SAMPLED).union(ImageUsage::COPY_SOURCE);
    let image = ctx.image(&*device, &image_desc(Format::Rgba8Unorm, EXTENT, usage));
    let texels = ctx.upload(
        &*device,
        BufferUsage::EMPTY,
        &vec![0; (EXTENT.width * EXTENT.height * 4) as usize],
    );
    let buffer_copies: [(&str, BufferCopy); 3] = [
        (
            "a copy past its source",
            BufferCopy { source_offset: 8, destination_offset: 0, size: SIZE },
        ),
        (
            "a copy past its destination",
            BufferCopy { source_offset: 0, destination_offset: SIZE - 4, size: 8 },
        ),
        (
            "a copy whose end wraps",
            BufferCopy { source_offset: u64::MAX, destination_offset: 0, size: 2 },
        ),
    ];
    for (what, region) in buffer_copies {
        let mut encoder = ctx.encoder(&*device, G);
        encoder.transition_buffers(&[
            buffer_to(source, BufferState::Undefined, BufferState::CopySource, G),
            buffer_to(destination, BufferState::Undefined, BufferState::CopyDestination, G),
        ]);
        encoder.copy_buffer(source, destination, &[region]);
        ctx.refused(submit(&*device, G, &mut [encoder]), what);
    }
    let rect = |x, width| Rect { x, y: 0, extent: crate::Extent { width, height: EXTENT.height } };
    // Vulkan's VUIDs 07975 and 07978: the buffer's offset is a multiple of the texel.
    let unaligned = BufferImageCopy { buffer_offset: 2, rect: rect(0, 8), ..whole_image() };
    let image_copies: [(&str, BufferImageCopy); 4] = [
        (
            "a rectangle past the image",
            BufferImageCopy { rect: rect(8, EXTENT.width), ..whole_image() },
        ),
        ("a mip the image does not have", BufferImageCopy { mip: 1, ..whole_image() }),
        ("rows past the buffer", BufferImageCopy { buffer_offset: 4, ..whole_image() }),
        ("a buffer offset that is not a multiple of the texel", unaligned),
    ];
    for (what, region) in image_copies {
        let mut encoder = ctx.encoder(&*device, G);
        encoder.transition_buffers(&[buffer_to(
            texels,
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
        encoder.copy_buffer_to_image(texels, image, &[region]);
        ctx.refused(submit(&*device, G, &mut [encoder]), what);
    }
    let readback = ctx.buffer(
        &*device,
        u64::from(EXTENT.width * EXTENT.height * 4),
        BufferUsage::COPY_DESTINATION,
        MemoryClass::Readback,
    );
    let mut encoder = ctx.encoder(&*device, G);
    encoder.copy_image_to_buffer(image, readback, &[unaligned]);
    ctx.refused(
        submit(&*device, G, &mut [encoder]),
        "a copy to a buffer offset that is not a multiple of the texel",
    );
    Ok(())
}

/// Two regions of one copy call that write one byte of the destination buffer, or one
/// texel of the destination image, are refused, in either order; regions that read one
/// source byte twice, or meet in the destination without sharing a byte, are accepted.
fn copy_regions_overlapping_in_the_destination_are_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let (source, destination) = pair(ctx, &*device);
    let region = |source_offset, destination_offset| BufferCopy {
        source_offset,
        destination_offset,
        size: 16,
    };
    let buffer_copies = [
        ("two regions that share a destination byte", [region(0, 0), region(32, 15)], false),
        ("two regions, the later first, that share one", [region(32, 40), region(0, 32)], false),
        ("two regions, the later first, that read one byte", [region(0, 16), region(0, 0)], true),
    ];
    for (what, regions, accepted) in buffer_copies {
        let mut encoder = ctx.encoder(&*device, G);
        encoder.transition_buffers(&[
            buffer_to(source, BufferState::Undefined, BufferState::CopySource, G),
            buffer_to(destination, BufferState::Undefined, BufferState::CopyDestination, G),
        ]);
        encoder.copy_buffer(source, destination, &regions);
        if accepted {
            run(ctx, &*device, &mut [encoder]);
        } else {
            ctx.refused(submit(&*device, G, &mut [encoder]), what);
        }
    }
    let usage = ImageUsage::COPY_DESTINATION.union(ImageUsage::COPY_SOURCE);
    let desc = ImageDesc { layers: 2, ..image_desc(Format::Rgba8Unorm, EXTENT, usage) };
    let image = ctx.image(&*device, &desc);
    let layers = ImageRange { layers: 2, ..ImageRange::FIRST };
    let half = u64::from(EXTENT.width / 2 * EXTENT.height * 4);
    let texels = ctx.upload(&*device, BufferUsage::EMPTY, &vec![3; half as usize]);
    let rect = |x| Rect {
        x,
        y: 0,
        extent: crate::Extent { width: EXTENT.width / 2, height: EXTENT.height },
    };
    let at = |buffer_offset, x| BufferImageCopy { buffer_offset, rect: rect(x), ..whole_image() };
    let write = |encoder: &mut dyn Encoder, regions: &[BufferImageCopy]| {
        encoder.transition_buffers(&[buffer_to(
            texels,
            BufferState::Undefined,
            BufferState::CopySource,
            G,
        )]);
        encoder.transition_images(&[image_across(
            image,
            layers,
            (ImageState::Undefined, G),
            (ImageState::CopyDestination, G),
        )]);
        encoder.copy_buffer_to_image(texels, image, regions);
    };
    let mut encoder = ctx.encoder(&*device, G);
    write(&mut *encoder, &[at(0, 0), at(0, EXTENT.width / 4)]);
    ctx.refused(submit(&*device, G, &mut [encoder]), "two regions that share a texel");
    // Rectangles apart by their rows alone, and one rectangle of two layers, share none.
    let rows = |y| Rect {
        x: 0,
        y,
        extent: crate::Extent { width: EXTENT.width / 2, height: EXTENT.height / 2 },
    };
    let top = BufferImageCopy { rect: rows(0), ..whole_image() };
    let bottom = BufferImageCopy { rect: rows(EXTENT.height / 2), ..whole_image() };
    let mut encoder = ctx.encoder(&*device, G);
    write(&mut *encoder, &[top, bottom]);
    run(ctx, &*device, &mut [encoder]);
    let mut encoder = ctx.encoder(&*device, G);
    write(&mut *encoder, &[at(0, 0), BufferImageCopy { layer: 1, ..at(0, 0) }]);
    run(ctx, &*device, &mut [encoder]);
    let mut encoder = ctx.encoder(&*device, G);
    write(&mut *encoder, &[at(0, 0), at(0, EXTENT.width / 2)]);
    run(ctx, &*device, &mut [encoder]);
    let readback =
        ctx.buffer(&*device, 2 * half, BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
    for (what, regions, accepted) in [
        ("two regions that share a buffer byte", [at(0, 0), at(4, EXTENT.width / 2)], false),
        ("two regions that meet in the buffer", [at(0, 0), at(half, EXTENT.width / 2)], true),
    ] {
        let mut encoder = ctx.encoder(&*device, G);
        encoder.transition_images(&[image_to(
            image,
            ImageState::CopyDestination,
            ImageState::CopySource,
            G,
        )]);
        encoder.transition_buffers(&[buffer_to(
            readback,
            BufferState::Undefined,
            BufferState::CopyDestination,
            G,
        )]);
        encoder.copy_image_to_buffer(image, readback, &regions);
        if accepted {
            run(ctx, &*device, &mut [encoder]);
        } else {
            ctx.refused(submit(&*device, G, &mut [encoder]), what);
        }
    }
    Ok(())
}

/// A submit of a good encoder and a bad one is refused: the good one is still recorded,
/// the state it would have changed is unchanged, and the point it would have signalled is
/// free for another submit.
fn refused_submit_changes_nothing(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let usage =
        ImageUsage::COPY_DESTINATION.union(ImageUsage::SAMPLED).union(ImageUsage::COPY_SOURCE);
    let image = ctx.image(&*device, &image_desc(Format::Rgba8Unorm, EXTENT, usage));
    let mut setup = ctx.encoder(&*device, G);
    setup.transition_images(&[image_to(
        image,
        ImageState::Undefined,
        ImageState::CopyDestination,
        G,
    )]);
    run(ctx, &*device, &mut [setup]);
    let mut good = ctx.encoder(&*device, G);
    good.transition_images(&[image_to(
        image,
        ImageState::CopyDestination,
        ImageState::SampledRead,
        G,
    )]);
    let mut bad = ctx.encoder(&*device, G);
    bad.transition_images(&[image_to(
        image,
        ImageState::ColourAttachment,
        ImageState::CopySource,
        G,
    )]);
    let mut encoders = [good, bad];
    let done = ctx.point(&*device, 1);
    let refused =
        device.submit(G, Submit { encoders: &mut encoders, waits: &[], signals: &[done] });
    ctx.refused_by(refused, Rule::State, "a submit with a bad encoder");
    ctx.check(
        ctx.ok(device.timeline_value(done.timeline), "a read") == 0,
        "a refused submit signals nothing",
    );
    let [good, _bad] = encoders;
    ctx.submit(&*device, G, &mut [good], &[], &[done]);
    ctx.reached(&*device, done);
    Ok(())
}

fn submit_of_no_encoders_waits_and_signals(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let wait = ctx.point(&*device, 1);
    let signal = ctx.point(&*device, 1);
    ctx.submit(&*device, G, &mut [], &[wait], &[signal]);
    ctx.check(
        !ctx.ok(device.wait_timeline(signal, Duration::from_millis(20)), "a wait"),
        "no signal before the wait is met",
    );
    ctx.ok(device.signal_timeline(wait), "the host signal");
    ctx.reached(&*device, signal);
    Ok(())
}

fn submit_is_pending_until_its_wait_is_reached(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let (source, destination) = pair(ctx, &*device);
    let mut encoders = [ctx.encoder(&*device, G)];
    copy(&mut *encoders[0], source, destination);
    let gate = ctx.held(&*device, G, &mut encoders);
    ctx.check(
        !ctx.ok(device.wait_timeline(gate.done, Duration::from_millis(20)), "a wait"),
        "a held submit does not complete",
    );
    ctx.refused_by(
        device.destroy_buffer(destination),
        Rule::Pending,
        "a destroy under the held submit",
    );
    ctx.refused_by(encoders[0].reset(), Rule::Pending, "a reset of the held encoder");
    gate.open(ctx, &*device);
    ctx.ok(device.destroy_buffer(destination), "the destroy once the wait is reached");
    ctx.ok(encoders[0].reset(), "the reset");
    Ok(())
}

/// With nothing bound, an empty draw or dispatch is accepted: it recorded nothing that
/// needs a binding.
fn empty_draws_and_dispatches_record_nothing(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let target = ctx.target(&*device);
    let arguments = ctx.buffer(&*device, 64, BufferUsage::INDIRECT, MemoryClass::Device);
    let mut encoder = ctx.encoder(&*device, G);
    encoder.transition_images(&[image_to(
        target,
        ImageState::Undefined,
        ImageState::ColourAttachment,
        G,
    )]);
    begin(
        &mut *encoder,
        &[attachment(target, Load::Clear([0.0; 4]), Store::Store, false)],
        PassReach::NOTHING,
    );
    encoder.draw(0..0, 0..1);
    encoder.draw(0..3, 1..1);
    #[allow(clippy::reversed_empty_ranges, reason = "a range whose start passes its end is empty")]
    {
        encoder.draw(3..1, 0..1);
        encoder.draw_indexed(5..2, 0, 0..1);
    }
    encoder.draw_indexed(0..3, 0, 0..0);
    encoder.draw_indexed_indirect(arguments, 0, 0, 20);
    encoder.end_pass();
    encoder.begin_compute_pass(&crate::ComputePass {
        label: "conformance",
        reach: PassReach::NOTHING,
    });
    encoder.dispatch([0, 1, 1]);
    encoder.dispatch([1, 0, 1]);
    encoder.dispatch([1, 1, 0]);
    encoder.end_pass();
    run(ctx, &*device, &mut [encoder]);
    Ok(())
}
