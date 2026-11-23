//! Queues on null: each pass's queue and the frame's submits under both layouts; a transfer's
//! two halves and its wait; the waits of every submit that uses an import under another
//! queue's pending write or change, its `ready` point's among them, a `sampled` read's
//! included; a frame's change under an earlier frame's pending use; and a torn frame.
//!
//! A test holds a submit pending by a point the host owes, given as an import's `ready`, or
//! as its own submit's wait; null judges each submit against what is pending when it is
//! made, so a wait left out is refused only while the submit it should wait for is pending.

use lactose_render_graph::{
    BufferAccess, BufferImport, BufferUse, ComputePassDecl, FrameDecl, ImageImport, PassId,
    RenderPassDecl, TransientImage,
};
use lactose_rhi_api::{
    BufferReach, BufferState, BufferTransition, ComputePass, Device, HeapSlot, ImageState, Load,
    PassReach, QueueRole, Rule, Sharing, TimelinePoint,
};

use crate::common::{
    C, G, LAYOUTS, SCRATCH, buffer, clear, colour, compute_pipeline, define_buffer, device, draws,
    frame, fresh_buffer, graph, held, image, image_import, in_job, lines, on, reached, reads,
    several, submit_after, writes,
};

/// The lines that hold `from` to `to`, in a transition.
fn changes<S: core::fmt::Debug>(lines: &[String], from: S, to: S) -> usize {
    let change = format!("from: {from:?}, to: {to:?}");
    lines.iter().map(|line| line.matches(&change).count()).sum()
}

/// Under a set whose compute role has a queue of its own, a compute pass with no
/// graphics-queue dependency runs there and the frame takes a second submit; under one queue
/// every pass is the graphics role's and the frame is one submit. Each frame's point is
/// reached within a bounded wait: no submit waits for a point only a later submit to its
/// own queue signals.
#[test]
fn each_pass_runs_on_its_queue_and_each_queue_run_is_a_submit() {
    for queues in LAYOUTS {
        let device = device(queues);
        let handles = (
            buffer(&*device, "alone", Sharing::Exclusive),
            buffer(&*device, "fed", Sharing::Exclusive),
            image(&*device, "target", Sharing::Exclusive),
        );
        in_job(move |ctx| {
            let several = several(&*device);
            let mut graph = graph(&device, 2, 1);
            for _ in 0..3 {
                let mut decl = FrameDecl::new(1);
                let alone = decl.import_buffer(&fresh_buffer(
                    "alone",
                    handles.0,
                    C,
                    BufferState::ShaderWrite,
                ));
                let fed =
                    decl.import_buffer(&fresh_buffer("fed", handles.1, G, BufferState::ShaderRead));
                let target = decl.import_image(&image_import(
                    "target",
                    handles.2,
                    on(ImageState::Undefined, G),
                    on(ImageState::ColourAttachment, G),
                ));
                let independent = decl.compute_pass(
                    &ComputePassDecl {
                        label: "alone",
                        sampled: &[],
                        buffers: &writes(alone),
                        slots: &[],
                    },
                    |_| {},
                );
                decl.compute_pass(
                    &ComputePassDecl {
                        label: "feeds",
                        sampled: &[],
                        buffers: &writes(fed),
                        slots: &[],
                    },
                    |_| {},
                );
                decl.render_pass(
                    &RenderPassDecl {
                        buffers: &reads(fed),
                        ..draws("draws", &[colour(target, Load::DontCare)])
                    },
                    |_| {},
                );
                let compiled = graph.compile(decl).expect("a plan");
                let plan = compiled.plan();
                assert_eq!(plan.queues(), queues);
                for planned in plan.passes() {
                    let moved = several && planned.pass == independent;
                    assert_eq!(planned.queue, if moved { C } else { G });
                    assert_eq!(planned.submit, u32::from(several && planned.pass != independent));
                }
                assert_eq!(plan.submits(), if several { 2 } else { 1 });
                assert_eq!(plan.jobs(), if several { 3 } else { 2 });
                let recorded = graph.record(compiled, ctx).expect("a recorded frame");
                let point = graph.submit(recorded).expect("a frame");
                reached(&**graph.device(), point);
                let submitted = device.take_submitted();
                let roles: Vec<QueueRole> = submitted.iter().map(|(role, _)| *role).collect();
                assert_eq!(roles, if several { vec![C, G] } else { vec![G] });
                // Each submit's encoders are of its role.
                for (role, encoders) in &submitted {
                    assert!(encoders.iter().all(|encoder| encoder.queue == *role), "{submitted:?}");
                }
            }
        });
    }
}

/// A compute pass, after a frame held pending, that writes a buffer the graphics queue left and one the
/// graphics queue takes after the frame: the crossings are at the frame's edges, where a
/// compute-role pass's dependencies are ("Queues"). An `Exclusive` buffer's crossing is a
/// release on the queue that had it, in a submit made first, and an acquire with equal
/// values on the other, in a submit that waits for it (`no-qfot` refuses an acquire whose
/// release is pending and not waited for); a `Concurrent` one's is one change, its submit
/// waiting all the same (`no-qfot` refuses a change under another queue's pending write).
#[test]
fn a_crossing_is_both_halves_of_a_transfer_or_one_change_and_a_wait() {
    for queues in LAYOUTS {
        for sharing in [Sharing::Exclusive, Sharing::Concurrent] {
            let device = device(queues);
            let coming = buffer(&*device, "coming", sharing);
            let going = buffer(&*device, "going", sharing);
            let hold = buffer(&*device, "hold", Sharing::Exclusive);
            let gate = held(&*device);
            define_buffer(&*device, coming, on(BufferState::ShaderRead, G));
            device.take_submitted();
            in_job(move |ctx| {
                let several = several(&*device);
                let mut graph = graph(&device, 1, 2);
                // A frame held pending, which every submit of the next waits for under
                // several queues: so the next frame's submits are pending as they are made.
                let mut decl = FrameDecl::new(1);
                let hold = decl.import_buffer(&BufferImport {
                    ready: Some(gate),
                    ..fresh_buffer("hold", hold, C, BufferState::ShaderWrite)
                });
                decl.compute_pass(
                    &ComputePassDecl {
                        label: "hold",
                        sampled: &[],
                        buffers: &writes(hold),
                        slots: &[],
                    },
                    |_| {},
                );
                let holding = frame(&mut graph, ctx, decl).expect("the held frame");
                device.take_submitted();

                let mut decl = FrameDecl::new(1);
                let coming = decl.import_buffer(&BufferImport {
                    label: "coming",
                    buffer: coming,
                    sharing,
                    initial: on(BufferState::ShaderRead, G),
                    last: on(BufferState::ShaderWrite, C),
                    ready: None,
                });
                let going = decl.import_buffer(&BufferImport {
                    sharing,
                    last: on(BufferState::ShaderRead, G),
                    ..fresh_buffer("going", going, C, BufferState::ShaderRead)
                });
                let all =
                    [coming, going].map(|buffer| BufferUse { buffer, access: BufferAccess::Write });
                decl.compute_pass(
                    &ComputePassDecl { label: "produce", sampled: &[], buffers: &all, slots: &[] },
                    |_| {},
                );
                let point = frame(&mut graph, ctx, decl).expect("null accepts the crossings");
                let submitted = device.take_submitted();
                let lines = lines(&submitted);
                let halves =
                    usize::from(several) * if sharing == Sharing::Exclusive { 2 } else { 1 };
                let came = changes(
                    &lines,
                    on(BufferState::ShaderRead, G),
                    on(BufferState::ShaderWrite, C),
                );
                let went = changes(
                    &lines,
                    on(BufferState::ShaderWrite, C),
                    on(BufferState::ShaderRead, G),
                );
                assert_eq!((came, went), (halves, halves), "{lines:#?}");
                if several && sharing == Sharing::Exclusive {
                    // A release on the graphics queue, the pass and a release on the compute
                    // queue, an acquire on the graphics queue.
                    let roles: Vec<QueueRole> = submitted.iter().map(|(role, _)| *role).collect();
                    assert_eq!(roles, [G, C, G], "{submitted:#?}");
                }
                graph.device().signal_timeline(gate).expect("the host's signal");
                reached(&**graph.device(), holding);
                reached(&**graph.device(), point);
            });
        }
    }
}

/// The test's own graphics submit, held pending, writes a `Concurrent` buffer and changes a
/// `Concurrent` image, and signals the point each import names as its `ready`. The frame's
/// two compute-queue submits, each followed by a graphics one, each use both, one of them
/// sampled through a declared slot: each waits for the point, not the first alone (`no-qfot`
/// refuses a use under another queue's pending write or change, and a wait in an earlier
/// submit orders no later one).
#[test]
fn every_submit_that_uses_an_import_waits_for_its_ready_point() {
    for queues in LAYOUTS {
        let device = device(queues);
        let written = buffer(&*device, "written", Sharing::Concurrent);
        let sampled = image(&*device, "sampled", Sharing::Concurrent);
        let outs = [
            buffer(&*device, "out", Sharing::Exclusive),
            buffer(&*device, "out, again", Sharing::Exclusive),
        ];
        let target = image(&*device, "target", Sharing::Exclusive);
        let pipeline = compute_pipeline(&*device);
        let slot = HeapSlot(1);
        let gate = held(&*device);
        // The test's write and change, on the graphics queue, pending until the gate opens.
        let mut encoder = device.create_encoder(G, "the test's write").expect("an encoder");
        clear(&mut *encoder, sampled, on(ImageState::SampledRead, G));
        encoder.transition_buffers(&[BufferTransition {
            buffer: written,
            from: on(BufferState::Undefined, G),
            to: on(BufferState::ShaderWrite, G),
        }]);
        encoder.begin_compute_pass(&ComputePass {
            label: "the test's write",
            reach: PassReach {
                slots: &[],
                buffers: &[BufferReach { buffer: written, state: BufferState::ShaderWrite }],
            },
        });
        encoder.end_pass();
        // Held until the job below has opened the gate and seen the frame through.
        let (ready, _writer) = submit_after(&*device, G, encoder, &[gate]);
        device.write_heap_slot(slot, sampled).expect("a slot");
        device.take_submitted();
        in_job(move |ctx| {
            let several = several(&*device);
            let mut graph = graph(&device, 1, 1);
            let mut decl = FrameDecl::new(1);
            let written = decl.import_buffer(&BufferImport {
                label: "written",
                buffer: written,
                sharing: Sharing::Concurrent,
                initial: on(BufferState::ShaderWrite, G),
                last: on(BufferState::ShaderRead, C),
                ready: Some(ready),
            });
            let sampled = decl.import_image(&ImageImport {
                sharing: Sharing::Concurrent,
                ready: Some(ready),
                ..image_import(
                    "sampled",
                    sampled,
                    on(ImageState::SampledRead, G),
                    on(ImageState::SampledRead, G),
                )
            });
            let outs = outs.map(|out| {
                decl.import_buffer(&fresh_buffer("out", out, C, BufferState::ShaderWrite))
            });
            let target = decl.import_image(&image_import(
                "target",
                target,
                on(ImageState::Undefined, G),
                on(ImageState::ColourAttachment, G),
            ));
            let mut passes: Vec<PassId> = Vec::new();
            for (at, out) in outs.into_iter().enumerate() {
                let uses = [
                    BufferUse { buffer: written, access: BufferAccess::Read },
                    BufferUse { buffer: out, access: BufferAccess::Write },
                ];
                let label = ["sample", "sample again"][at];
                passes.push(decl.compute_pass(
                    &ComputePassDecl { label, sampled: &[sampled], buffers: &uses, slots: &[slot] },
                    move |pass| {
                        pass.bind_pipeline(pipeline);
                        pass.push_address(0, written, 0);
                        pass.dispatch([1, 1, 1]);
                    },
                ));
                // A graphics pass after each: the second compute submit is not the frame's
                // last, which waits for the other queue's last submit.
                let label = ["between", "after"][at];
                decl.render_pass(&draws(label, &[colour(target, Load::Clear([0.0; 4]))]), |_| {});
            }
            let compiled = graph.compile(decl).expect("a plan");
            assert_eq!(compiled.plan().submits(), if several { 4 } else { 1 });
            let recorded = graph.record(compiled, ctx).expect("a recorded frame");
            let point = graph.submit(recorded).expect("null accepts every use");
            graph.device().signal_timeline(gate).expect("the host's signal");
            reached(&**graph.device(), point);
        });
    }
}

/// A frame's compute submit is held pending behind its import's `ready`, and its graphics
/// submit, the last, shares nothing with it: the frame's point is not reached until the
/// compute submit has completed, because the last submit waits for each other queue's last
/// ("Submission").
#[test]
fn a_frames_point_waits_for_every_queue() {
    for queues in LAYOUTS {
        let device = device(queues);
        let out = buffer(&*device, "out", Sharing::Exclusive);
        let target = image(&*device, "target", Sharing::Exclusive);
        let gate = held(&*device);
        in_job(move |ctx| {
            let several = several(&*device);
            let mut graph = graph(&device, 1, 1);
            let mut decl = FrameDecl::new(1);
            let out = decl.import_buffer(&BufferImport {
                ready: Some(gate),
                ..fresh_buffer("out", out, C, BufferState::ShaderWrite)
            });
            let target = decl.import_image(&image_import(
                "target",
                target,
                on(ImageState::Undefined, G),
                on(ImageState::ColourAttachment, G),
            ));
            decl.compute_pass(
                &ComputePassDecl { label: "held", sampled: &[], buffers: &writes(out), slots: &[] },
                |_| {},
            );
            decl.render_pass(&draws("apart", &[colour(target, Load::Clear([0.0; 4]))]), |_| {});
            let compiled = graph.compile(decl).expect("a plan");
            assert_eq!(compiled.plan().submits(), if several { 2 } else { 1 });
            let recorded = graph.record(compiled, ctx).expect("a recorded frame");
            let point = graph.submit(recorded).expect("null accepts the frame");
            let value = graph.device().timeline_value(point.timeline).expect("a value");
            assert!(value < point.value, "the frame's point waits for its held compute submit");
            graph.device().signal_timeline(gate).expect("the host's signal");
            reached(&**graph.device(), point);
        });
    }
}

/// After a frame held pending, a compute pass and a graphics pass read a `Concurrent` buffer
/// in its initial state, independent of each other, and the buffer's change to its `last`
/// follows the graphics read, in a submit that is not the frame's last: it waits for the
/// compute submit that read it (`no-qfot` refuses a change under another queue's pending
/// use), though that submit neither wrote nor changed it.
#[test]
fn a_change_after_another_queues_read_in_the_frame_waits_for_it() {
    for queues in LAYOUTS {
        let device = device(queues);
        let shared = buffer(&*device, "shared", Sharing::Concurrent);
        let hold = buffer(&*device, "hold", Sharing::Exclusive);
        let outs = [
            buffer(&*device, "out", Sharing::Exclusive),
            buffer(&*device, "out, again", Sharing::Exclusive),
        ];
        let target = image(&*device, "target", Sharing::Exclusive);
        let gate = held(&*device);
        define_buffer(&*device, shared, on(BufferState::ShaderRead, G));
        in_job(move |ctx| {
            let several = several(&*device);
            let mut graph = graph(&device, 1, 2);
            let mut decl = FrameDecl::new(1);
            let hold = decl.import_buffer(&BufferImport {
                ready: Some(gate),
                ..fresh_buffer("hold", hold, C, BufferState::ShaderWrite)
            });
            decl.compute_pass(
                &ComputePassDecl {
                    label: "hold",
                    sampled: &[],
                    buffers: &writes(hold),
                    slots: &[],
                },
                |_| {},
            );
            let holding = frame(&mut graph, ctx, decl).expect("the held frame");

            let mut decl = FrameDecl::new(1);
            let shared = decl.import_buffer(&BufferImport {
                label: "shared",
                buffer: shared,
                sharing: Sharing::Concurrent,
                initial: on(BufferState::ShaderRead, G),
                last: on(BufferState::CopySource, G),
                ready: None,
            });
            let outs = outs.map(|out| {
                decl.import_buffer(&fresh_buffer("out", out, C, BufferState::ShaderWrite))
            });
            let target = decl.import_image(&image_import(
                "target",
                target,
                on(ImageState::Undefined, G),
                on(ImageState::ColourAttachment, G),
            ));
            let read = [
                BufferUse { buffer: shared, access: BufferAccess::Read },
                BufferUse { buffer: outs[0], access: BufferAccess::Write },
            ];
            decl.compute_pass(
                &ComputePassDecl {
                    label: "read on compute",
                    sampled: &[],
                    buffers: &read,
                    slots: &[],
                },
                |_| {},
            );
            decl.render_pass(
                &RenderPassDecl {
                    buffers: &reads(shared),
                    ..draws("read on graphics", &[colour(target, Load::Clear([0.0; 4]))])
                },
                |_| {},
            );
            decl.compute_pass(
                &ComputePassDecl {
                    label: "later",
                    sampled: &[],
                    buffers: &writes(outs[1]),
                    slots: &[],
                },
                |_| {},
            );
            decl.render_pass(&draws("last", &[colour(target, Load::Clear([0.0; 4]))]), |_| {});
            let compiled = graph.compile(decl).expect("a plan");
            assert_eq!(compiled.plan().submits(), if several { 4 } else { 1 });
            let recorded = graph.record(compiled, ctx).expect("a recorded frame");
            let point = graph.submit(recorded).expect("null accepts the change");
            graph.device().signal_timeline(gate).expect("the host's signal");
            reached(&**graph.device(), holding);
            reached(&**graph.device(), point);
        });
    }
}

/// A frame's compute pass reads a `Concurrent` buffer while its submit is held pending; the
/// next frame's render pass writes it, a change of state on the graphics queue under the
/// compute queue's pending use, which waits for it (`no-qfot`) with no word from the
/// caller.
#[test]
fn a_frame_waits_for_an_earlier_frames_pending_use_before_a_change() {
    for queues in LAYOUTS {
        let device = device(queues);
        let shared = buffer(&*device, "shared", Sharing::Concurrent);
        let out = buffer(&*device, "out", Sharing::Exclusive);
        let target = image(&*device, "target", Sharing::Exclusive);
        let gate = held(&*device);
        define_buffer(&*device, shared, on(BufferState::ShaderRead, G));
        in_job(move |ctx| {
            let mut graph = graph(&device, 1, 1);
            let read = on(BufferState::ShaderRead, G);
            let mut decl = FrameDecl::new(1);
            let shared_id = decl.import_buffer(&BufferImport {
                label: "shared",
                buffer: shared,
                sharing: Sharing::Concurrent,
                initial: read,
                last: read,
                ready: None,
            });
            let out = decl.import_buffer(&BufferImport {
                ready: Some(gate),
                ..fresh_buffer("out", out, C, BufferState::ShaderWrite)
            });
            let uses = [
                BufferUse { buffer: shared_id, access: BufferAccess::Read },
                BufferUse { buffer: out, access: BufferAccess::Write },
            ];
            decl.compute_pass(
                &ComputePassDecl { label: "read", sampled: &[], buffers: &uses, slots: &[] },
                |_| {},
            );
            let first = frame(&mut graph, ctx, decl).expect("the reading frame");

            let mut decl = FrameDecl::new(1);
            let shared = decl.import_buffer(&BufferImport {
                label: "shared",
                buffer: shared,
                sharing: Sharing::Concurrent,
                initial: read,
                last: on(BufferState::ShaderWrite, G),
                ready: None,
            });
            let target = decl.import_image(&image_import(
                "target",
                target,
                on(ImageState::Undefined, G),
                on(ImageState::ColourAttachment, G),
            ));
            decl.render_pass(
                &RenderPassDecl {
                    buffers: &writes(shared),
                    ..draws("write", &[colour(target, Load::Clear([0.0; 4]))])
                },
                |_| {},
            );
            let second = frame(&mut graph, ctx, decl).expect("null accepts the change");
            graph.device().signal_timeline(gate).expect("the host's signal");
            reached(&**graph.device(), first);
            reached(&**graph.device(), second);
        });
    }
}

/// A buffer a compute pass alone uses, declared on the graphics role at both ends, crosses
/// to the compute queue and back in every frame where that role has a queue of its own,
/// and not where it shares the graphics queue.
#[test]
fn a_buffer_declared_on_graphics_and_written_on_compute_goes_out_and_back() {
    for queues in LAYOUTS {
        let device = device(queues);
        let handle = buffer(&*device, "out and back", Sharing::Exclusive);
        in_job(move |ctx| {
            let several = several(&*device);
            let mut graph = graph(&device, 1, 1);
            for _ in 0..2 {
                let mut decl = FrameDecl::new(1);
                let buffer = decl.import_buffer(&fresh_buffer(
                    "out and back",
                    handle,
                    G,
                    BufferState::ShaderWrite,
                ));
                decl.compute_pass(
                    &ComputePassDecl {
                        label: "write",
                        sampled: &[],
                        buffers: &writes(buffer),
                        slots: &[],
                    },
                    |_| {},
                );
                let point = frame(&mut graph, ctx, decl).expect("a frame");
                reached(&**graph.device(), point);
                let lines = lines(&device.take_submitted());
                let across = lines
                    .iter()
                    .map(|line| {
                        line.matches("queue: Compute")
                            .count()
                            .min(line.matches("queue: Graphics").count())
                    })
                    .sum::<usize>();
                assert_eq!(across, if several { 4 } else { 0 }, "{lines:#?}");
            }
        });
    }
}

/// A frame refused after its first submit is torn: the next `submit` first makes the torn
/// frame's last submit in its place, which waits for the torn frame's compute submit, held
/// pending, and signals the torn frame's point; the next frame, graphics work alone, waits
/// for that point, so it is not reached until the hold is let go, and its point is above
/// the torn one. Under one queue the frame is one submit, and its refusal changes nothing.
#[test]
fn a_torn_frame_is_finished_by_the_next_submit() {
    for queues in LAYOUTS {
        let device = device(queues);
        let written = buffer(&*device, "written", Sharing::Exclusive);
        let target = image(&*device, "target", Sharing::Exclusive);
        let gate = held(&*device);
        in_job(move |ctx| {
            let several = several(&*device);
            let mut graph = graph(&device, 1, 3);
            let declare = |torn: bool, compute: bool| {
                let mut decl = FrameDecl::new(1);
                if compute {
                    let written = decl.import_buffer(&BufferImport {
                        ready: torn.then_some(gate),
                        ..fresh_buffer("written", written, C, BufferState::ShaderWrite)
                    });
                    decl.compute_pass(
                        &ComputePassDecl {
                            label: "write",
                            sampled: &[],
                            buffers: &writes(written),
                            slots: &[],
                        },
                        |_| {},
                    );
                }
                let target = decl.import_image(&image_import(
                    "target",
                    target,
                    on(ImageState::Undefined, G),
                    on(ImageState::ColourAttachment, G),
                ));
                let scratch =
                    decl.transient_image(&TransientImage { label: "never written", ..SCRATCH });
                // The breach that tears: a transient kept before any pass wrote it.
                let load = if torn { Load::Keep } else { Load::Clear([0.0; 4]) };
                decl.render_pass(
                    &draws("draw", &[colour(target, Load::Clear([0.0; 4])), colour(scratch, load)]),
                    |_| {},
                );
                decl
            };
            let before = frame(&mut graph, ctx, declare(false, true)).expect("a frame");
            reached(&**graph.device(), before);
            let refused =
                frame(&mut graph, ctx, declare(true, true)).expect_err("null refuses the read");
            assert_eq!(Rule::of(&refused), Some(Rule::UndefinedRead), "{refused}");
            assert_eq!(graph.last_frame(), Some(before));
            let after = frame(&mut graph, ctx, declare(false, false)).expect("the frame after");
            let torn = TimelinePoint { value: before.value + 1, ..before };
            if several {
                assert!(after.value > torn.value, "{before:?} {after:?}");
                let value = graph.device().timeline_value(after.timeline).expect("a value");
                assert!(value < torn.value, "the torn frame's held submit is waited for");
            } else {
                assert_eq!(after.value, torn.value, "{before:?} {after:?}");
            }
            graph.device().signal_timeline(gate).expect("the host's signal");
            reached(&**graph.device(), after);
            reached(&**graph.device(), torn);
        });
    }
}
