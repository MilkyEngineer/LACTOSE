//! Timelines, destroys and the races between calls.

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::kit::{Body, Ctx, HEAD, LONG, PATIENCE, WOKEN, buffer_to, clear, image_desc, woken};
use crate::{
    BufferCopy, BufferHandle, BufferState, BufferUsage, Device, Encoder, Format, ImageUsage,
    MemoryClass, QueuePath, QueueRole, Rule, Sharing, Submit, TimelineHandle, TimelinePoint,
};

type Applies = Result<(), &'static str>;

pub(super) const BODIES: &[(&str, Body)] = &[
    (
        "timeline_is_signalled_waited_and_read_by_the_host",
        timeline_is_signalled_waited_and_read_by_the_host,
    ),
    ("timeline_wait_times_out", timeline_wait_times_out),
    (
        "waits_are_woken_by_the_submit_that_reaches_them",
        waits_are_woken_by_the_submit_that_reaches_them,
    ),
    ("wait_is_woken_by_a_host_signal", wait_is_woken_by_a_host_signal),
    ("timeline_value_that_does_not_grow_is_refused", timeline_value_that_does_not_grow_is_refused),
    (
        "host_signal_above_a_pending_signal_is_refused",
        host_signal_above_a_pending_signal_is_refused,
    ),
    (
        "host_signal_below_a_promise_that_does_not_wait_for_it_is_refused",
        host_signal_below_a_promise_that_does_not_wait_for_it_is_refused,
    ),
    (
        "host_signal_below_a_pending_signal_releases_its_waiter",
        host_signal_below_a_pending_signal_releases_its_waiter,
    ),
    (
        "host_signal_below_a_promise_whose_wait_is_met_is_refused",
        host_signal_below_a_promise_whose_wait_is_met_is_refused,
    ),
    (
        "submit_signalling_no_more_than_it_waits_for_is_refused",
        submit_signalling_no_more_than_it_waits_for_is_refused,
    ),
    (
        "timeline_value_beyond_the_difference_limit_is_refused",
        timeline_value_beyond_the_difference_limit_is_refused,
    ),
    (
        "timeline_signalled_from_two_queues_without_a_wait_is_refused",
        timeline_signalled_from_two_queues_without_a_wait_is_refused,
    ),
    ("destroy_under_a_pending_submit_is_refused", destroy_under_a_pending_submit_is_refused),
    (
        "concurrent_resource_destroyed_while_another_queue_uses_it_is_refused",
        concurrent_resource_destroyed_while_another_queue_uses_it_is_refused,
    ),
    (
        "destroy_after_an_unobserved_submit_is_refused",
        destroy_after_an_unobserved_submit_is_refused,
    ),
    ("destroy_at_once_after_the_wait_is_accepted", destroy_at_once_after_the_wait_is_accepted),
    ("destroy_racing_a_submit_refuses_one", destroy_racing_a_submit_refuses_one),
    ("submits_racing_on_one_queue_keep_their_order", submits_racing_on_one_queue_keep_their_order),
    (
        "host_signals_racing_on_one_timeline_keep_it_growing",
        host_signals_racing_on_one_timeline_keep_it_growing,
    ),
    ("use_after_destruction_is_refused", use_after_destruction_is_refused),
    (
        "destroyed_after_recording_is_refused_at_submit",
        destroyed_after_recording_is_refused_at_submit,
    ),
    ("handle_of_another_device_is_refused", handle_of_another_device_is_refused),
];

const G: QueueRole = QueueRole::Graphics;
const C: QueueRole = QueueRole::Compute;
const SIZE: u64 = 256;
/// The difference no two values of a timeline may reach.
const LIMIT: u64 = 1 << 31;
/// Rounds of a race: enough for both orders on most runs.
const ROUNDS: usize = 24;

const fn at(timeline: TimelineHandle, value: u64) -> TimelinePoint {
    TimelinePoint { timeline, value }
}

fn submit(
    device: &dyn Device,
    role: QueueRole,
    encoders: &mut [Box<dyn Encoder>],
    waits: &[TimelinePoint],
    signals: &[TimelinePoint],
) -> lactose_core::Result<()> {
    device.submit(role, Submit { encoders, waits, signals })
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
    let source = ctx.upload(device, BufferUsage::EMPTY, &[4; SIZE as usize]);
    (source, ctx.buffer(device, SIZE, BufferUsage::COPY_DESTINATION, MemoryClass::Device))
}

/// An encoder that copies between a new pair of buffers, and the pair.
fn copier(
    ctx: &Ctx<'_>,
    device: &dyn Device,
    role: QueueRole,
) -> (Box<dyn Encoder>, BufferHandle, BufferHandle) {
    let (source, destination) = pair(ctx, device);
    let mut encoder = ctx.encoder(device, role);
    copy(&mut *encoder, source, destination);
    (encoder, source, destination)
}

fn timeline_is_signalled_waited_and_read_by_the_host(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let timeline = ctx.timeline(&*device, 5);
    ctx.check(
        ctx.ok(device.timeline_value(timeline), "a read") == 5,
        "a timeline starts at its value",
    );
    ctx.check(
        ctx.ok(device.wait_timeline(at(timeline, 5), Duration::ZERO), "a poll"),
        "a point at the value is reached",
    );
    ctx.check(
        ctx.ok(device.wait_timeline(at(timeline, 1), Duration::ZERO), "a poll"),
        "a point below it is reached",
    );
    ctx.check(
        !ctx.ok(device.wait_timeline(at(timeline, 6), Duration::ZERO), "a poll"),
        "a point above it is not",
    );
    ctx.ok(device.signal_timeline(at(timeline, 9)), "a host signal");
    ctx.check(ctx.ok(device.timeline_value(timeline), "a read") == 9, "the host's value is read");
    ctx.reached(&*device, at(timeline, 8));
    ctx.submit(&*device, G, &mut [], &[], &[at(timeline, 12)]);
    ctx.reached(&*device, at(timeline, 12));
    ctx.check(ctx.ok(device.timeline_value(timeline), "a read") >= 12, "a submit's signal is read");
    let (sender, receiver) = mpsc::channel();
    let signaller: &dyn Device = &*device;
    thread::scope(|scope| {
        scope.spawn(move || {
            let reached = signaller.wait_timeline(at(timeline, 20), PATIENCE);
            let _sent = sender.send(reached);
        });
        ctx.ok(device.signal_timeline(at(timeline, 20)), "a host signal another thread waits for");
        let reached = receiver
            .recv_timeout(PATIENCE)
            .unwrap_or_else(|_| ctx.fail("the waiter did not return"));
        ctx.check(ctx.ok(reached, "the other thread's wait"), "a waiter sees the host's signal");
    });
    Ok(())
}

fn timeline_wait_times_out(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let timeline = ctx.timeline(&*device, 0);
    let timeout = Duration::from_millis(30);
    let start = Instant::now();
    ctx.check(
        !ctx.ok(device.wait_timeline(at(timeline, 1), timeout), "a wait"),
        "an unreached point times out",
    );
    ctx.check(start.elapsed() >= timeout, "a wait does not return before its timeout");
    let gate = ctx.held(&*device, G, &mut []);
    ctx.check(
        !ctx.ok(device.wait_timeline(gate.done, timeout), "a wait"),
        "a held submit's point times out",
    );
    gate.open(ctx, &*device);
    Ok(())
}

fn timeline_value_that_does_not_grow_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let timeline = ctx.timeline(&*device, 5);
    ctx.refused(device.signal_timeline(at(timeline, 5)), "a host signal of the value");
    ctx.refused(device.signal_timeline(at(timeline, 3)), "a host signal below it");
    ctx.refused(
        submit(&*device, G, &mut [], &[], &[at(timeline, 5)]),
        "a submit's signal of the value",
    );
    ctx.refused(
        submit(&*device, G, &mut [], &[], &[at(timeline, 2)]),
        "a submit's signal below it",
    );
    let gate = ctx.point(&*device, 1);
    ctx.submit(&*device, G, &mut [], &[gate], &[at(timeline, 8)]);
    ctx.refused(
        submit(&*device, G, &mut [], &[], &[at(timeline, 8)]),
        "a signal of a value promised",
    );
    ctx.refused(
        submit(&*device, G, &mut [], &[], &[at(timeline, 7)]),
        "a signal below a value promised",
    );
    ctx.ok(device.signal_timeline(gate), "the gate's host signal");
    ctx.reached(&*device, at(timeline, 8));
    ctx.ok(device.signal_timeline(at(timeline, 9)), "a host signal that grows it");
    Ok(())
}

fn host_signal_above_a_pending_signal_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let timeline = ctx.timeline(&*device, 0);
    let gate = ctx.point(&*device, 1);
    ctx.submit(&*device, G, &mut [], &[gate], &[at(timeline, 4)]);
    ctx.refused(device.signal_timeline(at(timeline, 5)), "a host signal above a pending signal");
    ctx.refused(
        device.signal_timeline(at(timeline, 4)),
        "a host signal of a pending signal's value",
    );
    ctx.ok(device.signal_timeline(gate), "the gate's host signal");
    ctx.reached(&*device, at(timeline, 4));
    ctx.ok(device.signal_timeline(at(timeline, 5)), "the host signal once the promise is kept");
    // A submit that waits on the timeline it signals: a host signal at or above its promise
    // is refused even though it would release that submit.
    let timeline = ctx.timeline(&*device, 0);
    ctx.submit(&*device, G, &mut [], &[at(timeline, 1)], &[at(timeline, 4)]);
    ctx.refused(
        device.signal_timeline(at(timeline, 5)),
        "a host signal above the promise of a submit it releases",
    );
    ctx.refused(
        device.signal_timeline(at(timeline, 4)),
        "a host signal of the promise of a submit it releases",
    );
    ctx.ok(device.signal_timeline(at(timeline, 1)), "the host signal the submit waits for");
    ctx.reached(&*device, at(timeline, 4));
    Ok(())
}

fn host_signal_below_a_promise_that_does_not_wait_for_it_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let timeline = ctx.timeline(&*device, 0);
    let gate = ctx.point(&*device, 1);
    ctx.submit(&*device, G, &mut [], &[gate], &[at(timeline, 4)]);
    ctx.refused(
        device.signal_timeline(at(timeline, 2)),
        "a host signal below a promise that waits elsewhere",
    );
    ctx.ok(device.signal_timeline(gate), "the gate's host signal");
    ctx.reached(&*device, at(timeline, 4));
    let other = ctx.timeline(&*device, 0);
    ctx.submit(&*device, G, &mut [], &[at(other, 3)], &[at(other, 6)]);
    ctx.ok(
        device.signal_timeline(at(other, 4)),
        "a host signal at or above the value the promise waits for",
    );
    ctx.reached(&*device, at(other, 6));
    Ok(())
}

fn host_signal_below_a_pending_signal_releases_its_waiter(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let timeline = ctx.timeline(&*device, 0);
    let (encoder, _, destination) = copier(ctx, &*device, G);
    let mut encoders = [encoder];
    ctx.submit(&*device, G, &mut encoders, &[at(timeline, 2)], &[at(timeline, 4)]);
    ctx.check(
        !ctx.ok(device.wait_timeline(at(timeline, 4), Duration::from_millis(20)), "a wait"),
        "a held submit",
    );
    ctx.refused_by(
        device.destroy_buffer(destination),
        Rule::Pending,
        "a destroy under the held submit",
    );
    ctx.ok(device.signal_timeline(at(timeline, 2)), "the host signal its wait is for");
    ctx.reached(&*device, at(timeline, 4));
    ctx.ok(device.destroy_buffer(destination), "the destroy once the promise is seen");
    Ok(())
}

/// A held submit waits for a value its timeline has reached, and promises one above the
/// host's: only the gate, on another timeline, holds it.
fn host_signal_below_a_promise_whose_wait_is_met_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let timeline = ctx.timeline(&*device, 3);
    let gate = ctx.point(&*device, 1);
    ctx.submit(&*device, G, &mut [], &[at(timeline, 2), gate], &[at(timeline, 6)]);
    ctx.refused(
        device.signal_timeline(at(timeline, 4)),
        "a host signal below a promise whose wait is met",
    );
    ctx.ok(device.signal_timeline(gate), "the gate's host signal");
    ctx.reached(&*device, at(timeline, 6));
    Ok(())
}

fn submit_signalling_no_more_than_it_waits_for_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let timeline = ctx.timeline(&*device, 0);
    ctx.refused(
        submit(&*device, G, &mut [], &[at(timeline, 5)], &[at(timeline, 5)]),
        "a signal of the value waited for",
    );
    ctx.refused(
        submit(&*device, G, &mut [], &[at(timeline, 5)], &[at(timeline, 4)]),
        "a signal below it",
    );
    ctx.check(
        ctx.ok(device.timeline_value(timeline), "a read") == 0,
        "the refusals changed nothing",
    );
    ctx.submit(&*device, G, &mut [], &[at(timeline, 5)], &[at(timeline, 6)]);
    ctx.ok(device.signal_timeline(at(timeline, 5)), "the host signal it waits for");
    ctx.reached(&*device, at(timeline, 6));
    Ok(())
}

fn timeline_value_beyond_the_difference_limit_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    // From the timeline's value.
    let timeline = ctx.timeline(&*device, 0);
    ctx.refused(device.signal_timeline(at(timeline, LIMIT)), "a host signal 2^31 above the value");
    ctx.refused(
        submit(&*device, G, &mut [], &[], &[at(timeline, LIMIT)]),
        "a submit's signal 2^31 above it",
    );
    ctx.refused(submit(&*device, G, &mut [], &[at(timeline, LIMIT)], &[]), "a wait 2^31 above it");
    ctx.ok(device.signal_timeline(at(timeline, LIMIT - 1)), "a host signal just within it");
    // From a pending submit's wait, which the timeline has met: a gate holds the submit.
    let value = (1 << 30) + 10;
    let timeline = ctx.timeline(&*device, value);
    let gate = ctx.gate(&*device);
    ctx.submit(&*device, G, &mut [], &[at(timeline, 1), gate.wait], &[gate.done]);
    ctx.refused(
        device.signal_timeline(at(timeline, 1 + LIMIT)),
        "a host signal 2^31 above a pending wait",
    );
    ctx.refused(
        submit(&*device, G, &mut [], &[], &[at(timeline, 1 + LIMIT)]),
        "a submit's signal 2^31 above it",
    );
    ctx.ok(device.signal_timeline(at(timeline, LIMIT)), "a host signal just within it");
    gate.open(ctx, &*device);
    // Below a pending submit's greater wait, which holds it.
    let timeline = ctx.timeline(&*device, value);
    let done = ctx.point(&*device, 1);
    ctx.submit(&*device, G, &mut [], &[at(timeline, LIMIT)], &[done]);
    ctx.refused(
        submit(&*device, G, &mut [], &[at(timeline, 0)], &[]),
        "a wait 2^31 below a pending wait",
    );
    ctx.ok(device.signal_timeline(at(timeline, LIMIT)), "the host signal the pending wait is for");
    ctx.reached(&*device, done);
    Ok(())
}

fn timeline_signalled_from_two_queues_without_a_wait_is_refused(ctx: &Ctx<'_>) -> Applies {
    if ctx.set.queues.compute != QueuePath::Own {
        return Err("compute shares the graphics queue");
    }
    let device = ctx.device();
    let timeline = ctx.timeline(&*device, 0);
    let gate = ctx.point(&*device, 1);
    ctx.submit(&*device, G, &mut [], &[gate], &[at(timeline, 2)]);
    ctx.refused(
        submit(&*device, C, &mut [], &[], &[at(timeline, 3)]),
        "a signal from another queue with no wait",
    );
    ctx.submit(&*device, C, &mut [], &[at(timeline, 2)], &[at(timeline, 3)]);
    ctx.ok(device.signal_timeline(gate), "the gate's host signal");
    ctx.reached(&*device, at(timeline, 3));
    Ok(())
}

fn destroy_under_a_pending_submit_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let (mut encoder, source, destination) = copier(ctx, &*device, G);
    let target = ctx.target(&*device);
    let pipeline = ctx.plain(&*device);
    let compute = ctx.compute(&*device);
    clear(&mut *encoder, target, Some(pipeline));
    super::kit::compute_pass(&mut *encoder, compute, crate::PassReach::NOTHING);
    encoder.end_pass();
    let gate = ctx.gate(&*device);
    let mut encoders = [encoder];
    ctx.submit(&*device, G, &mut encoders, &[gate.wait], &[gate.done]);
    ctx.refused(device.destroy_buffer(source), "a destroy of a buffer a pending submit reads");
    ctx.refused(
        device.destroy_buffer(destination),
        "a destroy of a buffer a pending submit writes",
    );
    ctx.refused(
        device.destroy_image(target),
        "a destroy of an image a pending submit renders into",
    );
    ctx.refused(
        device.destroy_graphics_pipeline(pipeline),
        "a destroy of a pipeline a pending submit binds",
    );
    ctx.refused(
        device.destroy_compute_pipeline(compute),
        "a destroy of a pipeline a pending submit binds",
    );
    ctx.refused(
        device.destroy_timeline(gate.wait.timeline),
        "a destroy of a timeline a pending submit waits for",
    );
    ctx.refused(
        device.destroy_timeline(gate.done.timeline),
        "a destroy of a timeline a pending submit signals",
    );
    gate.open(ctx, &*device);
    ctx.ok(device.destroy_buffer(source), "a destroy once the submit is seen complete");
    ctx.ok(device.destroy_buffer(destination), "a destroy");
    ctx.ok(device.destroy_image(target), "a destroy");
    ctx.ok(device.destroy_graphics_pipeline(pipeline), "a destroy");
    ctx.ok(device.destroy_compute_pipeline(compute), "a destroy");
    ctx.ok(device.destroy_timeline(gate.wait.timeline), "a destroy");
    ctx.ok(device.destroy_timeline(gate.done.timeline), "a destroy");
    Ok(())
}

/// A concurrent buffer is read by a held submit on graphics, then by a submit on compute
/// the caller sees complete: the second's completion does not free it.
fn concurrent_resource_destroyed_while_another_queue_uses_it_is_refused(ctx: &Ctx<'_>) -> Applies {
    if ctx.set.queues.compute != QueuePath::Own {
        return Err("compute shares the graphics queue");
    }
    let device = ctx.device();
    let usage = BufferUsage::COPY_SOURCE;
    let shared = ctx.buffer_shared(&*device, SIZE, usage, MemoryClass::Upload, Sharing::Concurrent);
    ctx.write(&*device, shared, 0, &[9; SIZE as usize]);
    let region = [BufferCopy { source_offset: 0, destination_offset: 0, size: SIZE }];
    let first = ctx.buffer(&*device, SIZE, BufferUsage::COPY_DESTINATION, MemoryClass::Device);
    // The change of state is seen complete first: a use on the other queue under a pending
    // one would be refused `no-qfot`.
    let mut setup = ctx.encoder(&*device, G);
    setup.transition_buffers(&[buffer_to(
        shared,
        BufferState::Undefined,
        BufferState::CopySource,
        G,
    )]);
    let set = ctx.point(&*device, 1);
    ctx.submit(&*device, G, &mut [setup], &[], &[set]);
    ctx.reached(&*device, set);
    let mut graphics = ctx.encoder(&*device, G);
    graphics.transition_buffers(&[buffer_to(
        first,
        BufferState::Undefined,
        BufferState::CopyDestination,
        G,
    )]);
    graphics.copy_buffer(shared, first, &region);
    let mut graphics = [graphics];
    let gate = ctx.held(&*device, G, &mut graphics);
    let second = ctx.buffer(&*device, SIZE, BufferUsage::COPY_DESTINATION, MemoryClass::Device);
    let mut compute = ctx.encoder(&*device, C);
    compute.transition_buffers(&[buffer_to(
        second,
        BufferState::Undefined,
        BufferState::CopyDestination,
        C,
    )]);
    compute.copy_buffer(shared, second, &region);
    let done = ctx.point(&*device, 1);
    ctx.submit(&*device, C, &mut [compute], &[], &[done]);
    ctx.reached(&*device, done);
    ctx.refused(
        device.destroy_buffer(shared),
        "a destroy while the other queue's submit is pending",
    );
    gate.open(ctx, &*device);
    ctx.ok(device.destroy_buffer(shared), "the destroy once both are seen complete");
    Ok(())
}

fn destroy_after_an_unobserved_submit_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let (encoder, source, _) = copier(ctx, &*device, G);
    let done = ctx.point(&*device, 1);
    ctx.submit(&*device, G, &mut [encoder], &[], &[done]);
    ctx.refused(device.destroy_buffer(source), "a destroy before the submit's point is seen");
    let seen = ctx.ok(device.timeline_value(done.timeline), "a read");
    if seen < done.value {
        ctx.reached(&*device, done);
    }
    ctx.ok(device.destroy_buffer(source), "the destroy once the point is seen");
    Ok(())
}

fn destroy_at_once_after_the_wait_is_accepted(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let mut encoder = ctx.encoder(&*device, G);
    for _ in 0..20 {
        let (source, destination) = pair(ctx, &*device);
        copy(&mut *encoder, source, destination);
        let gate = ctx.gate(&*device);
        let mut encoders = [encoder];
        ctx.submit(&*device, G, &mut encoders, &[gate.wait], &[gate.done]);
        let releaser: &dyn Device = &*device;
        thread::scope(|scope| {
            let release = scope.spawn(move || releaser.signal_timeline(gate.wait));
            // Bounded, so that a release that wakes no one fails the first round with a
            // message rather than each round at the wait's end.
            let began = Instant::now();
            ctx.reached(&*device, gate.done);
            ctx.check(began.elapsed() < WOKEN, "the release's host signal woke the wait");
            ctx.ok(device.destroy_buffer(source), "a destroy at once after the wait");
            ctx.ok(device.destroy_buffer(destination), "a destroy at once after the wait");
            ctx.ok(
                release.join().unwrap_or_else(|_| ctx.fail("the releaser panicked")),
                "the release",
            );
        });
        [encoder] = encoders;
        ctx.ok(encoder.reset(), "a reset at once after the wait");
    }
    Ok(())
}

/// The outcome's rule, if it is a refusal.
fn rule<T>(result: &lactose_core::Result<T>) -> Option<Rule> {
    result.as_ref().err().and_then(Rule::of)
}

fn destroy_racing_a_submit_refuses_one(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    for _ in 0..ROUNDS {
        let (encoder, source, _) = copier(ctx, &*device, G);
        let gate = ctx.gate(&*device);
        let mut encoders = [encoder];
        let racer: &dyn Device = &*device;
        let (destroy, submitted) = thread::scope(|scope| {
            let destroy = scope.spawn(move || racer.destroy_buffer(source));
            let encoders = &mut encoders;
            let submitted =
                scope.spawn(move || submit(racer, G, encoders, &[gate.wait], &[gate.done]));
            (
                destroy.join().unwrap_or_else(|_| ctx.fail("the destroyer panicked")),
                submitted.join().unwrap_or_else(|_| ctx.fail("the submitter panicked")),
            )
        });
        match (rule(&destroy), rule(&submitted)) {
            (Some(Rule::Pending), None) if submitted.is_ok() => {
                gate.open(ctx, &*device);
                ctx.ok(device.destroy_buffer(source), "the destroy once the submit is seen");
            }
            (None, Some(Rule::Destroyed)) if destroy.is_ok() => {}
            _ => ctx.fail(format_args!(
                "not exactly one refused: the destroy {destroy:?}, the submit {submitted:?}"
            )),
        }
    }
    Ok(())
}

/// Two threads submit to one queue at once, one of them held by its wait: whatever their
/// order, what the held one names is pending until its wait is reached.
fn submits_racing_on_one_queue_keep_their_order(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    for _ in 0..ROUNDS {
        let (held, held_source, _) = copier(ctx, &*device, G);
        let (free, free_source, _) = copier(ctx, &*device, G);
        let gate = ctx.gate(&*device);
        let done = ctx.point(&*device, 1);
        let (mut held, mut free) = ([held], [free]);
        let racer: &dyn Device = &*device;
        thread::scope(|scope| {
            let (held, free) = (&mut held, &mut free);
            let first = scope.spawn(move || submit(racer, G, held, &[gate.wait], &[gate.done]));
            let second = scope.spawn(move || submit(racer, G, free, &[], &[done]));
            ctx.ok(
                first.join().unwrap_or_else(|_| ctx.fail("a submitter panicked")),
                "the held submit",
            );
            ctx.ok(
                second.join().unwrap_or_else(|_| ctx.fail("a submitter panicked")),
                "the free submit",
            );
        });
        ctx.refused_by(
            device.destroy_buffer(held_source),
            Rule::Pending,
            "a destroy under the held submit",
        );
        ctx.check(
            !ctx.ok(device.wait_timeline(gate.done, Duration::ZERO), "a poll"),
            "the held submit is held",
        );
        gate.open(ctx, &*device);
        ctx.reached(&*device, done);
        ctx.ok(device.destroy_buffer(held_source), "the destroy once the wait is reached");
        ctx.ok(device.destroy_buffer(free_source), "the other's destroy once its point is seen");
    }
    Ok(())
}

/// Two waiters on two timelines are woken by the one submit that reaches both points: a
/// backend wakes every waiter a submit concerns, not one of them.
fn waits_are_woken_by_the_submit_that_reaches_them(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let points = [ctx.point(&*device, 1), ctx.point(&*device, 1)];
    let device: &dyn Device = &*device;
    let returned = thread::scope(|scope| {
        let waiters = points
            .map(|point| scope.spawn(move || (device.wait_timeline(point, LONG), Instant::now())));
        thread::sleep(HEAD);
        let woke = Instant::now();
        ctx.submit(device, G, &mut [], &[], &points);
        waiters.map(|waiter| {
            let (reached, at) = waiter.join().unwrap_or_else(|_| ctx.fail("a waiter panicked"));
            (reached, at.saturating_duration_since(woke))
        })
    });
    for (reached, after) in returned {
        ctx.check(matches!(reached, Ok(true)), "a waiter's point is reached");
        if after >= WOKEN {
            ctx.fail(format_args!("a waiter returned {after:?} after the submit that reached it"));
        }
    }
    Ok(())
}

fn wait_is_woken_by_a_host_signal(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let point = ctx.point(&*device, 1);
    let (reached, after) = woken(
        || device.wait_timeline(point, LONG),
        || {
            ctx.ok(device.signal_timeline(point), "the host signal");
        },
    );
    ctx.check(matches!(reached, Ok(true)), "the waiter's point is reached");
    if after >= WOKEN {
        ctx.fail(format_args!("the wait returned {after:?} after the host signal"));
    }
    Ok(())
}

/// Two host signals race on one timeline. On null the two holds of a host signal are a few
/// instructions apart, so the case meets the two orders one after the other, and a signal
/// landing between the other's holds is held by null's fixed interleaving of it; a device
/// backend's driver call widens that window, and its own unit tests fix it ("Threads").
fn host_signals_racing_on_one_timeline_keep_it_growing(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    for round in 0..ROUNDS as u64 {
        let timeline = ctx.timeline(&*device, round);
        let (low, high) = (at(timeline, round + 3), at(timeline, round + 5));
        let racer: &dyn Device = &*device;
        let (low_signal, high_signal) = thread::scope(|scope| {
            let low_signal = scope.spawn(move || racer.signal_timeline(low));
            let high_signal = scope.spawn(move || racer.signal_timeline(high));
            (
                low_signal.join().unwrap_or_else(|_| ctx.fail("a signaller panicked")),
                high_signal.join().unwrap_or_else(|_| ctx.fail("a signaller panicked")),
            )
        });
        for result in [&low_signal, &high_signal] {
            if result.is_err() && rule(result) != Some(Rule::Timeline) {
                ctx.fail(format_args!(
                    "a racing host signal failed otherwise than by `timeline`: {result:?}"
                ));
            }
        }
        let expected = match (low_signal.is_ok(), high_signal.is_ok()) {
            (_, true) => high.value,
            (true, false) => low.value,
            (false, false) => ctx.fail("both racing host signals were refused"),
        };
        ctx.check(
            ctx.ok(device.timeline_value(timeline), "a read") == expected,
            "the timeline ends at the greater accepted value",
        );
        ctx.ok(device.destroy_timeline(timeline), "a destroy");
    }
    Ok(())
}

fn use_after_destruction_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let usage = BufferUsage::COPY_SOURCE.union(BufferUsage::DEVICE_ADDRESS);
    let buffer = ctx.buffer(&*device, SIZE, usage, MemoryClass::Upload);
    ctx.ok(device.destroy_buffer(buffer), "a destroy");
    ctx.refused(device.destroy_buffer(buffer), "a second destroy");
    ctx.refused(device.buffer_address(buffer), "an address");
    ctx.refused(device.write_buffer(buffer, 0, 4, &mut |_| {}), "a host write");
    ctx.refused(device.read_buffer(buffer, 0, 4, &mut |_| {}), "a host read");
    ctx.refused(device.flush_buffer(buffer, 0, 4), "a flush");
    let image = ctx.sampled(&*device);
    ctx.ok(device.destroy_image(image), "a destroy");
    ctx.refused(device.destroy_image(image), "a second destroy");
    ctx.refused(device.image_memoryless(image), "a memoryless query");
    ctx.refused(device.write_heap_slot(crate::HeapSlot(0), image), "a heap write");
    let timeline = ctx.timeline(&*device, 0);
    ctx.ok(device.destroy_timeline(timeline), "a destroy");
    ctx.refused(device.timeline_value(timeline), "a read");
    ctx.refused(device.signal_timeline(at(timeline, 1)), "a host signal");
    ctx.refused(device.wait_timeline(at(timeline, 1), Duration::ZERO), "a wait");
    ctx.refused(submit(&*device, G, &mut [], &[at(timeline, 1)], &[]), "a submit's wait");
    let pipeline = ctx.plain(&*device);
    ctx.ok(device.destroy_graphics_pipeline(pipeline), "a destroy");
    ctx.refused(device.destroy_graphics_pipeline(pipeline), "a second destroy");
    let compute = ctx.compute(&*device);
    ctx.ok(device.destroy_compute_pipeline(compute), "a destroy");
    ctx.refused(device.destroy_compute_pipeline(compute), "a second destroy");
    Ok(())
}

fn destroyed_after_recording_is_refused_at_submit(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    let (encoder, source, _) = copier(ctx, &*device, G);
    ctx.ok(device.destroy_buffer(source), "a destroy after the recording");
    ctx.refused(
        submit(&*device, G, &mut [encoder], &[], &[]),
        "the submit of a destroyed buffer's copy",
    );
    let target = ctx.target(&*device);
    let pipeline = ctx.plain(&*device);
    let mut encoder = ctx.encoder(&*device, G);
    clear(&mut *encoder, target, Some(pipeline));
    ctx.ok(device.destroy_graphics_pipeline(pipeline), "a destroy after the recording");
    ctx.refused(
        submit(&*device, G, &mut [encoder], &[], &[]),
        "the submit of a destroyed pipeline's draw",
    );
    let mut encoder = ctx.encoder(&*device, G);
    clear(&mut *encoder, target, None);
    ctx.ok(device.destroy_image(target), "a destroy after the recording");
    ctx.refused(
        submit(&*device, G, &mut [encoder], &[], &[]),
        "the submit of a destroyed image's pass",
    );
    Ok(())
}

fn handle_of_another_device_is_refused(ctx: &Ctx<'_>) -> Applies {
    let device = ctx.device();
    // The same kinds made on `device` first: where a backend numbers each device's handles
    // alike, each foreign handle lands on a live index and is refused by more than its
    // index.
    let own_buffer = ctx.buffer(&*device, SIZE, BufferUsage::COPY_SOURCE, MemoryClass::Upload);
    let own_image = ctx
        .image(&*device, &image_desc(Format::Rgba8Unorm, super::kit::EXTENT, ImageUsage::SAMPLED));
    let own_timeline = ctx.timeline(&*device, 0);
    let own_pipeline = ctx.compute(&*device);
    let other = ctx.device();
    let buffer = ctx.buffer(&*other, SIZE, BufferUsage::COPY_SOURCE, MemoryClass::Upload);
    let image = ctx
        .image(&*other, &image_desc(Format::Rgba8Unorm, super::kit::EXTENT, ImageUsage::SAMPLED));
    let timeline = ctx.timeline(&*other, 0);
    let pipeline = ctx.compute(&*other);
    ctx.refused(device.destroy_buffer(buffer), "another device's buffer");
    ctx.refused(device.write_buffer(buffer, 0, 4, &mut |_| {}), "another device's buffer");
    ctx.refused(device.destroy_image(image), "another device's image");
    ctx.refused(device.write_heap_slot(crate::HeapSlot(0), image), "another device's image");
    ctx.refused(device.signal_timeline(at(timeline, 1)), "another device's timeline");
    ctx.refused(submit(&*device, G, &mut [], &[], &[at(timeline, 1)]), "another device's timeline");
    ctx.refused(device.destroy_compute_pipeline(pipeline), "another device's pipeline");
    let (_, destination) = pair(ctx, &*device);
    let mut encoder = ctx.encoder(&*device, G);
    copy(&mut *encoder, buffer, destination);
    ctx.refused(submit(&*device, G, &mut [encoder], &[], &[]), "another device's buffer recorded");
    ctx.ok(device.destroy_buffer(own_buffer), "the device's own buffer");
    ctx.ok(device.destroy_image(own_image), "the device's own image");
    ctx.ok(device.destroy_timeline(own_timeline), "the device's own timeline");
    ctx.ok(device.destroy_compute_pipeline(own_pipeline), "the device's own pipeline");
    Ok(())
}
