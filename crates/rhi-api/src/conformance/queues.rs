//! Capabilities and queues; device loss and messages.

use std::time::{Duration, Instant};

use lactose_core::ErrorKind;

use super::kit::{Body, Ctx, buffer_to, clear};
use crate::{
    BufferState, BufferUsage, Encoder, Format, ImageState, IndexedIndirectArgs, MemoryClass,
    PassReach, QueuePath, QueueRole, Submit, TimelinePoint, WriteMask,
};

type Applies = Result<(), &'static str>;

pub(super) const BODIES: &[(&str, Body)] = &[
    ("capabilities_are_fixed", capabilities_are_fixed),
    ("unreported_capability_is_refused", unreported_capability_is_refused),
    ("roles_resolve_as_the_layout_reports", roles_resolve_as_the_layout_reports),
    ("submit_to_the_present_role_is_refused", submit_to_the_present_role_is_refused),
    ("loss_is_returned_and_stays", loss_is_returned_and_stays),
    ("waits_return_at_once_after_a_loss", waits_return_at_once_after_a_loss),
    ("teardown_succeeds_after_a_loss", teardown_succeeds_after_a_loss),
    ("clean_run_sends_no_message", clean_run_sends_no_message),
];

fn capabilities_are_fixed(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let first = *device.capabilities();
    ctx.check(first == ctx.set, "the device reports the set it was made for");
    ctx.check(!device.info().backend.is_empty(), "the device names its backend");
    let mut encoder = ctx.encoder(&*device, QueueRole::Graphics);
    clear(&mut *encoder, ctx.target(&*device), Some(ctx.plain(&*device)));
    let done = ctx.point(&*device, 1);
    ctx.submit(&*device, QueueRole::Graphics, &mut [encoder], &[], &[done]);
    ctx.reached(&*device, done);
    ctx.check(*device.capabilities() == first, "the reported set is the same after work");
    Ok(())
}

/// Records, into a render pass ready to draw indexed, a draw that needs `multi_draw_indirect`
/// or `indirect_count` where the set reports it `false`.
fn unreported_capability_is_refused(ctx: &Ctx<'_>) -> Applies {
    if ctx.set.multi_draw_indirect && ctx.set.indirect_count {
        return Err("every optional capability a draw needs is reported");
    }
    let device = ctx.device();
    let queue = QueueRole::Graphics;
    let record = IndexedIndirectArgs {
        index_count: 3,
        instance_count: 1,
        first_index: 0,
        base_vertex: 0,
        first_instance: 0,
    };
    let records: Vec<u8> = [record.to_bytes(), record.to_bytes()].concat();
    let arguments = ctx.upload(&*device, BufferUsage::INDIRECT, &records);
    let count = ctx.upload(&*device, BufferUsage::INDIRECT, &2u32.to_le_bytes());
    let index = ctx.upload(&*device, BufferUsage::INDEX, &[0; 12]);
    let target = ctx.target(&*device);
    let pipeline = ctx.plain(&*device);
    let mut encoder = ctx.encoder(&*device, queue);
    encoder.transition_buffers(&[
        buffer_to(arguments, BufferState::Undefined, BufferState::Indirect, queue),
        buffer_to(count, BufferState::Undefined, BufferState::Indirect, queue),
        buffer_to(index, BufferState::Undefined, BufferState::Index, queue),
    ]);
    encoder.transition_images(&[super::kit::image_to(
        target,
        ImageState::Undefined,
        ImageState::ColourAttachment,
        queue,
    )]);
    super::kit::begin(
        &mut *encoder,
        &[super::kit::attachment(target, crate::Load::Clear([0.0; 4]), crate::Store::Store, false)],
        PassReach::NOTHING,
    );
    encoder.bind_graphics_pipeline(pipeline);
    encoder.bind_index_buffer(index, 0);
    if ctx.set.multi_draw_indirect {
        encoder.draw_indexed_indirect_count(arguments, 0, count, 0, 2, IndexedIndirectArgs::BYTES);
    } else {
        encoder.draw_indexed_indirect(arguments, 0, 2, IndexedIndirectArgs::BYTES);
    }
    encoder.end_pass();
    let refused =
        device.submit(queue, Submit { encoders: &mut [encoder], waits: &[], signals: &[] });
    ctx.refused(refused, "a draw that needs an unreported capability");
    Ok(())
}

fn roles_resolve_as_the_layout_reports(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let layout = device.capabilities().queues;
    ctx.check(layout.serving(QueueRole::Graphics) == QueueRole::Graphics, "graphics serves itself");
    for (role, path) in [(QueueRole::Compute, layout.compute), (QueueRole::Present, layout.present)]
    {
        let serving = layout.serving(role);
        let expected = if path == QueuePath::Own { role } else { QueueRole::Graphics };
        ctx.check(serving == expected, "a role resolves to its own queue or to graphics's");
        ctx.check(
            layout.same_queue(role, QueueRole::Graphics) == (path == QueuePath::Shared),
            "two roles share a queue exactly where the layout says so",
        );
    }
    for role in [QueueRole::Graphics, QueueRole::Compute, QueueRole::Present] {
        ctx.check(ctx.encoder(&*device, role).queue() == role, "an encoder keeps its role");
    }
    for role in [QueueRole::Graphics, QueueRole::Compute] {
        let done = ctx.point(&*device, 1);
        ctx.submit(&*device, role, &mut [ctx.encoder(&*device, role)], &[], &[done]);
        ctx.reached(&*device, done);
    }
    Ok(())
}

fn submit_to_the_present_role_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let encoder = ctx.encoder(&*device, QueueRole::Present);
    let refused = device
        .submit(QueueRole::Present, Submit { encoders: &mut [encoder], waits: &[], signals: &[] });
    ctx.refused(refused, "a submit of a present encoder to the present role");
    let refused =
        device.submit(QueueRole::Present, Submit { encoders: &mut [], waits: &[], signals: &[] });
    ctx.refused(refused, "a submit of no encoders to the present role");
    Ok(())
}

fn loss_is_returned_and_stays(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let buffer = ctx.buffer(&*device, 64, BufferUsage::COPY_DESTINATION, MemoryClass::Device);
    ctx.check(!device.is_lost(), "a new device is not lost");
    ctx.lose(&*device)?;
    ctx.check(device.is_lost(), "a lost device says so");
    let lost = device.create_buffer(&crate::BufferDesc {
        label: "conformance",
        size: 64,
        usage: BufferUsage::COPY_SOURCE,
        memory: MemoryClass::Device,
        sharing: crate::Sharing::Exclusive,
    });
    ctx.fails(lost, ErrorKind::DeviceLost, "a create after the loss");
    let encoder = device.create_encoder(QueueRole::Graphics, "conformance");
    ctx.fails(encoder.map(|_| ()), ErrorKind::DeviceLost, "an encoder after the loss");
    let lost =
        device.submit(QueueRole::Graphics, Submit { encoders: &mut [], waits: &[], signals: &[] });
    ctx.fails(lost, ErrorKind::DeviceLost, "a submit after the loss");
    ctx.fails(
        device.buffer_address(buffer),
        ErrorKind::DeviceLost,
        "a call on a live handle after the loss",
    );
    ctx.check(device.is_lost(), "the loss stays");
    let _ = device.info();
    let _ = device.capabilities();
    let _ = device.pipeline_stats();
    Ok(())
}

fn waits_return_at_once_after_a_loss(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let reached = ctx.point(&*device, 1);
    ctx.ok(device.signal_timeline(reached), "a host signal");
    let gate = ctx.held(&*device, QueueRole::Graphics, &mut []);
    ctx.lose(&*device)?;
    let start = Instant::now();
    let was = device.wait_timeline(reached, Duration::from_secs(60));
    ctx.check(matches!(was, Ok(true)), "a wait for a point reached before the loss returns true");
    let never = device.wait_timeline(gate.done, Duration::from_secs(60));
    ctx.fails(never, ErrorKind::DeviceLost, "a wait for a pending submit's point after the loss");
    // Both returned at once: well inside either timeout, however loaded the machine.
    ctx.check(start.elapsed() < Duration::from_secs(30), "the waits return at once");
    let unreached = TimelinePoint { timeline: reached.timeline, value: 2 };
    ctx.fails(
        device.wait_timeline(unreached, Duration::from_secs(60)),
        ErrorKind::DeviceLost,
        "a wait for a point never reached",
    );
    Ok(())
}

fn teardown_succeeds_after_a_loss(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let buffer = ctx.buffer(&*device, 64, BufferUsage::COPY_SOURCE, MemoryClass::Upload);
    let image = ctx.target(&*device);
    let sampler = ctx.ok(
        device.create_sampler(&crate::SamplerDesc {
            magnify: crate::Filter::Linear,
            minify: crate::Filter::Linear,
            mip: crate::Filter::Nearest,
            address_u: crate::AddressMode::Repeat,
            address_v: crate::AddressMode::Repeat,
        }),
        "a sampler",
    );
    let graphics = ctx.graphics(&*device, &[(Format::Rgba8Unorm, WriteMask::ALL)], 0, None);
    let compute = ctx.compute(&*device);
    let timeline = ctx.timeline(&*device, 0);
    let mut encoder = ctx.encoder(&*device, QueueRole::Graphics);
    clear(&mut *encoder, image, Some(graphics));
    let mut encoders: [Box<dyn Encoder>; 1] = [encoder];
    let _gate = ctx.held(&*device, QueueRole::Graphics, &mut encoders);
    ctx.lose(&*device)?;
    ctx.ok(device.destroy_buffer(buffer), "a buffer's destroy after the loss");
    ctx.ok(device.destroy_image(image), "a used image's destroy after the loss");
    ctx.ok(device.destroy_sampler(sampler), "a sampler's destroy after the loss");
    ctx.ok(
        device.destroy_graphics_pipeline(graphics),
        "a graphics pipeline's destroy after the loss",
    );
    ctx.ok(device.destroy_compute_pipeline(compute), "a compute pipeline's destroy after the loss");
    ctx.ok(device.destroy_timeline(timeline), "a timeline's destroy after the loss");
    ctx.ok(encoders[0].reset(), "a pending encoder's reset after the loss");
    let [encoder] = encoders;
    drop(encoder);
    drop(device);
    Ok(())
}

fn clean_run_sends_no_message(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let queue = QueueRole::Graphics;
    let bytes: Vec<u8> = (0..=255).collect();
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &bytes);
    let readback = ctx.buffer(&*device, 256, BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
    let mut encoder = ctx.encoder(&*device, queue);
    clear(&mut *encoder, ctx.target(&*device), Some(ctx.plain(&*device)));
    encoder.transition_buffers(&[
        buffer_to(source, BufferState::Undefined, BufferState::CopySource, queue),
        buffer_to(readback, BufferState::Undefined, BufferState::CopyDestination, queue),
    ]);
    encoder.copy_buffer(
        source,
        readback,
        &[crate::BufferCopy { source_offset: 0, destination_offset: 0, size: 256 }],
    );
    encoder.transition_buffers(&[buffer_to(
        readback,
        BufferState::CopyDestination,
        BufferState::HostRead,
        queue,
    )]);
    let done = ctx.point(&*device, 1);
    ctx.submit(&*device, queue, &mut [encoder], &[], &[done]);
    ctx.reached(&*device, done);
    ctx.check(ctx.read(&*device, readback, 0, 256) == bytes, "the copy's bytes are read back");
    ctx.check(ctx.messages() == 0, "a clean run sends no message");
    Ok(())
}
