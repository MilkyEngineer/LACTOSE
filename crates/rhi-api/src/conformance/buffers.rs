//! Buffers and host access.

use std::sync::mpsc;

use super::kit::{
    Body, Ctx, Lead, Orders, PATIENCE, attachment, begin, buffer_to, compute_pass, image_to,
};
use crate::{
    BufferCopy, BufferHandle, BufferReach, BufferState, BufferUsage, Device, Encoder, ImageState,
    Load, MemoryClass, PassReach, QueueRole, Rule, Store, Submit,
};

type Applies = Result<(), &'static str>;

pub(super) const BODIES: &[(&str, Body)] = &[
    (
        "buffer_write_then_copy_then_read_returns_the_bytes",
        buffer_write_then_copy_then_read_returns_the_bytes,
    ),
    (
        "device_buffer_written_by_a_pass_is_read_after_it",
        device_buffer_written_by_a_pass_is_read_after_it,
    ),
    ("buffer_address_is_stable_and_distinct", buffer_address_is_stable_and_distinct),
    (
        "host_ranges_that_do_not_overlap_are_written_at_once",
        host_ranges_that_do_not_overlap_are_written_at_once,
    ),
    ("overlapping_host_access_is_refused", overlapping_host_access_is_refused),
    ("destroy_during_host_access_is_refused", destroy_during_host_access_is_refused),
    ("host_range_outside_the_buffer_is_refused", host_range_outside_the_buffer_is_refused),
    (
        "host_access_outside_the_memory_class_is_refused",
        host_access_outside_the_memory_class_is_refused,
    ),
    (
        "address_of_a_buffer_without_the_usage_is_refused",
        address_of_a_buffer_without_the_usage_is_refused,
    ),
    ("read_under_a_pending_writer_is_refused", read_under_a_pending_writer_is_refused),
    (
        "read_under_a_pending_transition_to_host_read_is_refused",
        read_under_a_pending_transition_to_host_read_is_refused,
    ),
    ("write_under_a_pending_writer_is_refused", write_under_a_pending_writer_is_refused),
    (
        "submit_writing_under_a_host_access_is_refused",
        submit_writing_under_a_host_access_is_refused,
    ),
    ("submit_racing_a_host_access_refuses_one", submit_racing_a_host_access_refuses_one),
    ("unflushed_range_at_a_submit_is_refused", unflushed_range_at_a_submit_is_refused),
    (
        "flush_and_invalidate_are_accepted_where_memory_is_coherent",
        flush_and_invalidate_are_accepted_where_memory_is_coherent,
    ),
    ("buffer_use_outside_its_usage_is_refused", buffer_use_outside_its_usage_is_refused),
];

/// Rounds of each race: enough for both orders to occur on most runs, few enough to be
/// quick. A race case asserts what holds in either order, so it never fails for want of one.
const ROUNDS: usize = 24;

const SIZE: u64 = 256;

fn pattern(seed: u8) -> Vec<u8> {
    (0..SIZE).map(|at| (at as u8).wrapping_mul(31).wrapping_add(seed)).collect()
}

/// Records a copy of `source`, from `Undefined`, into `destination`, from `Undefined`, of
/// the whole of both.
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

fn buffer_write_then_copy_then_read_returns_the_bytes(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let queue = QueueRole::Graphics;
    let bytes = pattern(1);
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &bytes);
    let readback = ctx.buffer(&*device, SIZE, BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
    let mut encoder = ctx.encoder(&*device, queue);
    copy(&mut *encoder, source, readback);
    encoder.transition_buffers(&[buffer_to(
        readback,
        BufferState::CopyDestination,
        BufferState::HostRead,
        queue,
    )]);
    let done = ctx.point(&*device, 1);
    ctx.submit(&*device, queue, &mut [encoder], &[], &[done]);
    ctx.reached(&*device, done);
    ctx.check(ctx.read(&*device, readback, 0, SIZE) == bytes, "the bytes written are read back");
    ctx.check(ctx.read(&*device, readback, 16, 8) == bytes[16..24], "a range reads its own bytes");
    Ok(())
}

fn buffer_address_is_stable_and_distinct(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let sizes = [256, 1000, 4, 65536];
    let buffers: Vec<(BufferHandle, u64)> = sizes
        .iter()
        .map(|&size| {
            (ctx.buffer(&*device, size, BufferUsage::DEVICE_ADDRESS, MemoryClass::Device), size)
        })
        .collect();
    let ranges: Vec<(u64, u64)> = buffers
        .iter()
        .map(|&(buffer, size)| {
            let address = ctx.ok(device.buffer_address(buffer), "an address");
            ctx.check(
                ctx.ok(device.buffer_address(buffer), "an address") == address,
                "an address is stable",
            );
            (
                address,
                address
                    .checked_add(size)
                    .unwrap_or_else(|| ctx.fail("an address range that wraps")),
            )
        })
        .collect();
    for (at, a) in ranges.iter().enumerate() {
        for b in &ranges[at + 1..] {
            ctx.check(
                a.1 <= b.0 || b.1 <= a.0,
                "no byte of one live buffer has an address in another",
            );
        }
    }
    Ok(())
}

/// One thread's write is lent while another's, on a range that does not overlap it, runs
/// in full.
fn host_ranges_that_do_not_overlap_are_written_at_once(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let buffer = ctx.buffer(&*device, SIZE, BufferUsage::COPY_SOURCE, MemoryClass::Upload);
    let (lent, lent_seen) = mpsc::channel();
    let (other_done, other_seen) = mpsc::channel();
    let device: &dyn Device = &*device;
    std::thread::scope(|scope| {
        let first = scope.spawn(move || {
            device.write_buffer(buffer, 0, 128, &mut |bytes| {
                bytes.fill(1);
                lent.send(()).ok();
                other_seen.recv_timeout(PATIENCE).ok();
            })
        });
        let second = scope.spawn(move || {
            lent_seen.recv_timeout(PATIENCE).ok();
            let second = device.write_buffer(buffer, 128, 128, &mut |bytes| bytes.fill(2));
            other_done.send(()).ok();
            second
        });
        ctx.ok(
            second.join().unwrap_or_else(|_| ctx.fail("the second writer panicked")),
            "a write beside another",
        );
        ctx.ok(
            first.join().unwrap_or_else(|_| ctx.fail("the first writer panicked")),
            "the first write",
        );
    });
    ctx.ok(device.flush_buffer(buffer, 0, SIZE), "a flush");
    Ok(())
}

fn overlapping_host_access_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let buffer = ctx.buffer(&*device, SIZE, BufferUsage::COPY_SOURCE, MemoryClass::Upload);
    let mut inner = None;
    ctx.ok(
        device.write_buffer(buffer, 0, 128, &mut |bytes| {
            bytes.fill(1);
            inner = Some(device.write_buffer(buffer, 64, 128, &mut |bytes| bytes.fill(2)));
        }),
        "the outer write",
    );
    ctx.refused(
        inner.unwrap_or_else(|| ctx.fail("the fill was not called")),
        "a write overlapping one in progress",
    );
    ctx.ok(
        device.write_buffer(buffer, 64, 128, &mut |bytes| bytes.fill(2)),
        "the same write once the first returned",
    );
    // A read inside a read of the same range, of a readback the device filled.
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &pattern(6));
    let readback = ctx.buffer(&*device, SIZE, BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
    let mut encoder = ctx.encoder(&*device, QueueRole::Graphics);
    copy(&mut *encoder, source, readback);
    encoder.transition_buffers(&[buffer_to(
        readback,
        BufferState::CopyDestination,
        BufferState::HostRead,
        QueueRole::Graphics,
    )]);
    let done = ctx.point(&*device, 1);
    ctx.submit(&*device, QueueRole::Graphics, &mut [encoder], &[], &[done]);
    ctx.reached(&*device, done);
    ctx.ok(device.invalidate_buffer(readback, 0, SIZE), "an invalidate");
    let mut inner = None;
    ctx.ok(
        device.read_buffer(readback, 0, 128, &mut |_| {
            inner = Some(device.read_buffer(readback, 64, 128, &mut |_| {}));
        }),
        "the outer read",
    );
    ctx.refused(
        inner.unwrap_or_else(|| ctx.fail("the read was not called")),
        "a read overlapping one in progress",
    );
    // A flush or an invalidate of a range a host access in progress holds.
    let (mut flush, mut invalidate) = (None, None);
    ctx.ok(
        device.write_buffer(buffer, 0, 128, &mut |bytes| {
            bytes.fill(3);
            flush = Some(device.flush_buffer(buffer, 64, 128));
        }),
        "the write",
    );
    ctx.ok(device.flush_buffer(buffer, 0, 128), "the flush once the write returned");
    ctx.ok(
        device.read_buffer(readback, 0, 128, &mut |_| {
            invalidate = Some(device.invalidate_buffer(readback, 64, 128));
        }),
        "the read",
    );
    ctx.refused(
        flush.unwrap_or_else(|| ctx.fail("the fill was not called")),
        "a flush overlapping a write in progress",
    );
    ctx.refused(
        invalidate.unwrap_or_else(|| ctx.fail("the read was not called")),
        "an invalidate overlapping a read in progress",
    );
    Ok(())
}

fn destroy_during_host_access_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let buffer = ctx.buffer(&*device, SIZE, BufferUsage::COPY_SOURCE, MemoryClass::Upload);
    let mut inner = None;
    ctx.ok(
        device.write_buffer(buffer, 0, 16, &mut |bytes| {
            bytes.fill(1);
            inner = Some(device.destroy_buffer(buffer));
        }),
        "the write",
    );
    ctx.refused(
        inner.unwrap_or_else(|| ctx.fail("the fill was not called")),
        "a destroy under a host write",
    );
    ctx.ok(device.destroy_buffer(buffer), "the destroy once the write returned");
    Ok(())
}

fn host_range_outside_the_buffer_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let buffer = ctx.buffer(&*device, 64, BufferUsage::COPY_SOURCE, MemoryClass::Upload);
    let mut called = false;
    let refused = device.write_buffer(buffer, 60, 8, &mut |_| called = true);
    ctx.refused(refused, "a write past the end");
    let refused = device.write_buffer(buffer, u64::MAX, 2, &mut |_| called = true);
    ctx.refused(refused, "a write whose end wraps");
    ctx.check(!called, "a refused write lends nothing");
    let refused = device.flush_buffer(buffer, 32, 64);
    ctx.refused(refused, "a flush past the end");
    Ok(())
}

fn host_access_outside_the_memory_class_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let readback = ctx.buffer(&*device, 64, BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
    let local = ctx.buffer(&*device, 64, BufferUsage::COPY_DESTINATION, MemoryClass::Device);
    for (buffer, what) in
        [(readback, "a write of a readback buffer"), (local, "a write of device memory")]
    {
        ctx.refused(device.write_buffer(buffer, 0, 16, &mut |bytes| bytes.fill(0)), what);
    }
    let upload = ctx.buffer(&*device, 64, BufferUsage::COPY_SOURCE, MemoryClass::Upload);
    for (buffer, what) in
        [(readback, "a flush of a readback buffer"), (local, "a flush of device memory")]
    {
        ctx.refused(device.flush_buffer(buffer, 0, 16), what);
    }
    for (buffer, what) in
        [(upload, "an invalidate of an upload buffer"), (local, "an invalidate of device memory")]
    {
        ctx.refused(device.invalidate_buffer(buffer, 0, 16), what);
    }
    Ok(())
}

fn address_of_a_buffer_without_the_usage_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let buffer = ctx.buffer(&*device, 64, BufferUsage::COPY_DESTINATION, MemoryClass::Device);
    ctx.refused(device.buffer_address(buffer), "the address of a buffer made without the usage");
    Ok(())
}

/// A `Device` buffer written through its address by a compute pass that declares it
/// `ShaderWrite` is then read through its address by a draw whose pass declares it
/// `ShaderRead`, and copied out and read by the host: a pass fills device memory as a copy
/// does. On null the case holds that the sequence is accepted, which it is not with the
/// pass's write dropped: the `ShaderRead` then reads contents left undefined. It compares no
/// bytes: device evidence that a pass writes through an address is the output case
/// `dispatch_writes_through_an_address`.
fn device_buffer_written_by_a_pass_is_read_after_it(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let queue = QueueRole::Graphics;
    let usage = BufferUsage::DEVICE_ADDRESS.union(BufferUsage::COPY_SOURCE);
    let filled = ctx.buffer(&*device, SIZE, usage, MemoryClass::Device);
    let readback = ctx.buffer(&*device, SIZE, BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
    let target = ctx.target(&*device);
    let mut encoder = ctx.encoder(&*device, queue);
    encoder.transition_buffers(&[buffer_to(
        filled,
        BufferState::Undefined,
        BufferState::ShaderWrite,
        queue,
    )]);
    let written = [BufferReach { buffer: filled, state: BufferState::ShaderWrite }];
    let compute = ctx.compute(&*device);
    compute_pass(&mut *encoder, compute, PassReach { slots: &[], buffers: &written });
    encoder.push_address(0, filled, 0);
    encoder.dispatch([1, 1, 1]);
    encoder.end_pass();
    encoder.transition_buffers(&[buffer_to(
        filled,
        BufferState::ShaderWrite,
        BufferState::ShaderRead,
        queue,
    )]);
    encoder.transition_images(&[image_to(
        target,
        ImageState::Undefined,
        ImageState::ColourAttachment,
        queue,
    )]);
    let read = [BufferReach { buffer: filled, state: BufferState::ShaderRead }];
    begin(
        &mut *encoder,
        &[attachment(target, Load::Clear([0.0; 4]), Store::Store, false)],
        PassReach { slots: &[], buffers: &read },
    );
    encoder.bind_graphics_pipeline(ctx.plain(&*device));
    encoder.push_address(0, filled, 0);
    encoder.draw(0..3, 0..1);
    encoder.end_pass();
    encoder.transition_buffers(&[
        buffer_to(filled, BufferState::ShaderRead, BufferState::CopySource, queue),
        buffer_to(readback, BufferState::Undefined, BufferState::CopyDestination, queue),
    ]);
    encoder.copy_buffer(
        filled,
        readback,
        &[BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }],
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
    ctx.read(&*device, readback, 0, SIZE);
    Ok(())
}

/// A readback buffer, written once by a submit seen complete, is read while a second
/// submit that writes it again is held pending, and invalidated while a third, which copies
/// into it with no transition of it, is.
fn read_under_a_pending_writer_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let queue = QueueRole::Graphics;
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &pattern(2));
    let readback = ctx.buffer(&*device, SIZE, BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
    let mut encoder = ctx.encoder(&*device, queue);
    copy(&mut *encoder, source, readback);
    encoder.transition_buffers(&[buffer_to(
        readback,
        BufferState::CopyDestination,
        BufferState::HostRead,
        queue,
    )]);
    let done = ctx.point(&*device, 1);
    ctx.submit(&*device, queue, &mut [encoder], &[], &[done]);
    ctx.reached(&*device, done);
    let mut writer = ctx.encoder(&*device, queue);
    writer.transition_buffers(&[buffer_to(
        readback,
        BufferState::HostRead,
        BufferState::CopyDestination,
        queue,
    )]);
    writer.copy_buffer(
        source,
        readback,
        &[BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }],
    );
    writer.transition_buffers(&[buffer_to(
        readback,
        BufferState::CopyDestination,
        BufferState::HostRead,
        queue,
    )]);
    let mut writers = [writer];
    let gate = ctx.held(&*device, queue, &mut writers);
    // An invalidate before the writer runs makes none of its writes visible.
    ctx.refused(
        device.invalidate_buffer(readback, 0, SIZE),
        "an invalidate under a pending writer",
    );
    let mut called = false;
    ctx.refused(
        device.read_buffer(readback, 0, SIZE, &mut |_| called = true),
        "a read under a pending writer",
    );
    ctx.check(!called, "a refused read lends nothing");
    gate.open(ctx, &*device);
    ctx.check(
        ctx.read(&*device, readback, 0, SIZE) == pattern(2),
        "the read once the writer is seen complete",
    );
    // A write is a change whether or not its submit transitions the buffer.
    let mut encoder = ctx.encoder(&*device, queue);
    encoder.transition_buffers(&[buffer_to(
        readback,
        BufferState::HostRead,
        BufferState::CopyDestination,
        queue,
    )]);
    let done = ctx.point(&*device, 1);
    ctx.submit(&*device, queue, &mut [encoder], &[], &[done]);
    ctx.reached(&*device, done);
    let mut copier = ctx.encoder(&*device, queue);
    copier.copy_buffer(
        source,
        readback,
        &[BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }],
    );
    let mut copiers = [copier];
    let gate = ctx.held(&*device, queue, &mut copiers);
    ctx.refused(
        device.invalidate_buffer(readback, 0, SIZE),
        "an invalidate under a pending copy that does not transition the buffer",
    );
    gate.open(ctx, &*device);
    Ok(())
}

/// A submit that only changes a buffer's state, here into `HostRead`, holds a host read
/// and an invalidate back as a writer does: its barrier makes the device's writes visible
/// to the host.
fn read_under_a_pending_transition_to_host_read_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let queue = QueueRole::Graphics;
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &pattern(2));
    let readback = ctx.buffer(&*device, SIZE, BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
    let mut encoder = ctx.encoder(&*device, queue);
    copy(&mut *encoder, source, readback);
    encoder.transition_buffers(&[buffer_to(
        readback,
        BufferState::CopyDestination,
        BufferState::HostRead,
        queue,
    )]);
    let done = ctx.point(&*device, 1);
    ctx.submit(&*device, queue, &mut [encoder], &[], &[done]);
    ctx.reached(&*device, done);
    let mut encoder = ctx.encoder(&*device, queue);
    encoder.transition_buffers(&[buffer_to(
        readback,
        BufferState::HostRead,
        BufferState::CopyDestination,
        queue,
    )]);
    ctx.submit(&*device, queue, &mut [encoder], &[], &[]);
    let mut mover = ctx.encoder(&*device, queue);
    mover.transition_buffers(&[buffer_to(
        readback,
        BufferState::CopyDestination,
        BufferState::HostRead,
        queue,
    )]);
    let mut movers = [mover];
    let gate = ctx.held(&*device, queue, &mut movers);
    let mut called = false;
    ctx.refused(
        device.read_buffer(readback, 0, SIZE, &mut |_| called = true),
        "a read while its move into HostRead is pending",
    );
    ctx.refused(
        device.invalidate_buffer(readback, 0, SIZE),
        "an invalidate while its move into HostRead is pending",
    );
    ctx.check(!called, "a refused read lends nothing");
    gate.open(ctx, &*device);
    ctx.check(
        ctx.read(&*device, readback, 0, SIZE) == pattern(2),
        "the read once the move is seen complete",
    );
    Ok(())
}

fn write_under_a_pending_writer_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let queue = QueueRole::Graphics;
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &pattern(3));
    let usage = BufferUsage::COPY_DESTINATION.union(BufferUsage::COPY_SOURCE);
    let upload = ctx.buffer(&*device, SIZE, usage, MemoryClass::Upload);
    let mut encoder = ctx.encoder(&*device, queue);
    copy(&mut *encoder, source, upload);
    let mut encoders = [encoder];
    let gate = ctx.held(&*device, queue, &mut encoders);
    let mut called = false;
    ctx.refused(
        device.write_buffer(upload, 0, 16, &mut |_| called = true),
        "a write under a pending writer",
    );
    ctx.check(!called, "a refused write lends nothing");
    gate.open(ctx, &*device);
    ctx.write(&*device, upload, 0, &[9; 16]);
    // A submit that only changes the buffer's state writes none of its bytes: a host write
    // under it is accepted, as a staging ring's writes under the frames in flight are.
    let mut mover = ctx.encoder(&*device, queue);
    mover.transition_buffers(&[buffer_to(
        upload,
        BufferState::CopyDestination,
        BufferState::CopySource,
        queue,
    )]);
    let mut movers = [mover];
    let gate = ctx.held(&*device, queue, &mut movers);
    ctx.write(&*device, upload, 16, &[7; 16]);
    gate.open(ctx, &*device);
    Ok(())
}

fn submit_writing_under_a_host_access_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let queue = QueueRole::Graphics;
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &pattern(4));
    let upload = ctx.buffer(&*device, SIZE, BufferUsage::COPY_DESTINATION, MemoryClass::Upload);
    let mut encoder = ctx.encoder(&*device, queue);
    copy(&mut *encoder, source, upload);
    let mut encoders = [encoder];
    let mut refused = None;
    ctx.ok(
        device.write_buffer(upload, 0, 16, &mut |bytes| {
            bytes.fill(0);
            refused = Some(
                device.submit(queue, Submit { encoders: &mut encoders, waits: &[], signals: &[] }),
            );
        }),
        "the host write",
    );
    ctx.refused(
        refused.unwrap_or_else(|| ctx.fail("the fill was not called")),
        "a submit that writes under a host write",
    );
    ctx.ok(device.flush_buffer(upload, 0, 16), "a flush");
    let done = ctx.point(&*device, 1);
    ctx.submit(&*device, queue, &mut encoders, &[], &[done]);
    ctx.reached(&*device, done);
    // A pass that declares the buffer `ShaderWrite` writes it as a copy does.
    let pipeline = ctx.compute(&*device);
    let usage = BufferUsage::DEVICE_ADDRESS;
    let reached = ctx.buffer(&*device, SIZE, usage, MemoryClass::Upload);
    let mut encoder = ctx.encoder(&*device, queue);
    encoder.transition_buffers(&[buffer_to(
        reached,
        BufferState::Undefined,
        BufferState::ShaderWrite,
        queue,
    )]);
    let reach = [BufferReach { buffer: reached, state: BufferState::ShaderWrite }];
    compute_pass(&mut *encoder, pipeline, PassReach { slots: &[], buffers: &reach });
    encoder.dispatch([1, 1, 1]);
    encoder.end_pass();
    let mut encoders = [encoder];
    let mut refused = None;
    ctx.ok(
        device.write_buffer(reached, 0, 16, &mut |bytes| {
            bytes.fill(0);
            refused = Some(
                device.submit(queue, Submit { encoders: &mut encoders, waits: &[], signals: &[] }),
            );
        }),
        "the host write",
    );
    ctx.refused(
        refused.unwrap_or_else(|| ctx.fail("the fill was not called")),
        "a submit whose pass writes the buffer under a host write",
    );
    Ok(())
}

/// A submit that copies into an upload buffer races a write of it whose fill stays until
/// the submit has returned: exactly one of the two is refused. A round led by the write
/// submits once the fill has begun; one led by the submit writes once it has returned.
fn submit_racing_a_host_access_refuses_one(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let queue = QueueRole::Graphics;
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &pattern(5));
    let mut orders = Orders::new(["the write first", "the submit first"]);
    for lead in Lead::rounds(ROUNDS) {
        let upload = ctx.buffer(&*device, SIZE, BufferUsage::COPY_DESTINATION, MemoryClass::Upload);
        let mut encoder = ctx.encoder(&*device, queue);
        copy(&mut *encoder, source, upload);
        let mut encoders = [encoder];
        let gate = ctx.gate(&*device);
        let (returned, returned_seen) = mpsc::channel::<()>();
        let (filling, filling_seen) = mpsc::channel::<()>();
        let device: &dyn Device = &*device;
        let encoders = &mut encoders;
        let mut submit = move || {
            if lead == Lead::First {
                filling_seen.recv_timeout(PATIENCE).ok();
            }
            let submit = device
                .submit(queue, Submit { encoders, waits: &[gate.wait], signals: &[gate.done] });
            returned.send(()).ok();
            submit
        };
        let write = move || {
            device.write_buffer(upload, 0, SIZE, &mut |bytes| {
                filling.send(()).ok();
                bytes.fill(1);
                returned_seen.recv_timeout(PATIENCE).ok();
            })
        };
        let (write, submit) = if lead == Lead::Second {
            let submit = submit();
            (write(), submit)
        } else {
            std::thread::scope(|scope| {
                let (write, submit) = if matches!(lead, Lead::Free { second_first: true }) {
                    let submit = scope.spawn(submit);
                    (scope.spawn(write), submit)
                } else {
                    let write = scope.spawn(write);
                    (write, scope.spawn(submit))
                };
                (
                    write.join().unwrap_or_else(|_| ctx.fail("the writer panicked")),
                    submit.join().unwrap_or_else(|_| ctx.fail("the submitter panicked")),
                )
            })
        };
        let order = match (&write, &submit) {
            (Ok(()), Err(error)) if Rule::of(error) == Some(Rule::HostOverlap) => 0,
            (Err(error), Ok(())) if Rule::of(error) == Some(Rule::Pending) => 1,
            _ => ctx.fail(format_args!(
                "not exactly one refused: the write {write:?}, the submit {submit:?}"
            )),
        };
        orders.reached(ctx, lead, order);
        if submit.is_ok() {
            gate.open(ctx, device);
        } else {
            ctx.ok(device.flush_buffer(upload, 0, SIZE), "a flush");
        }
    }
    orders.report(ctx);
    Ok(())
}

fn unflushed_range_at_a_submit_is_refused(ctx: &Ctx<'_>) -> Applies {
    if ctx.set.coherent_host_memory {
        return Err("host memory is coherent");
    }
    let device = ctx.device();
    let queue = QueueRole::Graphics;
    let source = ctx.buffer(&*device, SIZE, BufferUsage::COPY_SOURCE, MemoryClass::Upload);
    ctx.ok(device.write_buffer(source, 0, SIZE, &mut |bytes| bytes.fill(4)), "a host write");
    let readback = ctx.buffer(&*device, SIZE, BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
    let mut encoder = ctx.encoder(&*device, queue);
    copy(&mut *encoder, source, readback);
    let mut encoders = [encoder];
    let refused =
        device.submit(queue, Submit { encoders: &mut encoders, waits: &[], signals: &[] });
    ctx.refused(refused, "a copy from a written range not flushed");
    ctx.ok(device.flush_buffer(source, 0, SIZE), "the flush");
    let done = ctx.point(&*device, 1);
    ctx.submit(&*device, queue, &mut encoders, &[], &[done]);
    ctx.reached(&*device, done);
    Ok(())
}

fn flush_and_invalidate_are_accepted_where_memory_is_coherent(ctx: &Ctx<'_>) -> Applies {
    if !ctx.set.coherent_host_memory {
        return Err("host memory is not coherent");
    }
    let device = ctx.device();
    let queue = QueueRole::Graphics;
    let bytes = pattern(6);
    let source = ctx.upload(&*device, BufferUsage::EMPTY, &bytes);
    ctx.ok(device.flush_buffer(source, 0, SIZE), "a second flush");
    let readback = ctx.buffer(&*device, SIZE, BufferUsage::COPY_DESTINATION, MemoryClass::Readback);
    let mut encoder = ctx.encoder(&*device, queue);
    copy(&mut *encoder, source, readback);
    encoder.transition_buffers(&[buffer_to(
        readback,
        BufferState::CopyDestination,
        BufferState::HostRead,
        queue,
    )]);
    let done = ctx.point(&*device, 1);
    ctx.submit(&*device, queue, &mut [encoder], &[], &[done]);
    ctx.reached(&*device, done);
    ctx.ok(device.invalidate_buffer(readback, 0, SIZE), "an invalidate");
    ctx.check(ctx.read(&*device, readback, 0, SIZE) == bytes, "the bytes read back");
    Ok(())
}

fn buffer_use_outside_its_usage_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let queue = QueueRole::Graphics;
    let source = ctx.buffer(&*device, SIZE, BufferUsage::COPY_DESTINATION, MemoryClass::Upload);
    ctx.write(&*device, source, 0, &pattern(7));
    let destination =
        ctx.buffer(&*device, SIZE, BufferUsage::COPY_DESTINATION, MemoryClass::Device);
    let mut encoder = ctx.encoder(&*device, queue);
    copy(&mut *encoder, source, destination);
    let refused =
        device.submit(queue, Submit { encoders: &mut [encoder], waits: &[], signals: &[] });
    ctx.refused(refused, "a copy from a buffer made without `COPY_SOURCE`");
    let plain = ctx.buffer(&*device, SIZE, BufferUsage::COPY_DESTINATION, MemoryClass::Device);
    let pipeline = ctx.compute(&*device);
    let mut encoder = ctx.encoder(&*device, queue);
    encoder.transition_buffers(&[buffer_to(
        plain,
        BufferState::Undefined,
        BufferState::ShaderWrite,
        queue,
    )]);
    let reach = [BufferReach { buffer: plain, state: BufferState::ShaderWrite }];
    compute_pass(&mut *encoder, pipeline, PassReach { slots: &[], buffers: &reach });
    encoder.dispatch([1, 1, 1]);
    encoder.end_pass();
    let refused =
        device.submit(queue, Submit { encoders: &mut [encoder], waits: &[], signals: &[] });
    ctx.refused(refused, "a pass that declares a buffer made without `DEVICE_ADDRESS`");
    Ok(())
}
