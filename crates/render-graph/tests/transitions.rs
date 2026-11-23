//! Transitions on null: each derived from where the frame left the resource, the same-state
//! transition between two writers and across the frame's edge, the closing transition and
//! the release toward `handoff`, and the reach each pass begins with.

use lactose_render_graph::{
    BufferAccess, BufferUse, ComputePassDecl, CopyPassDecl, FrameDecl, RenderPassDecl,
};
use lactose_rhi_api::{BufferState, ImageRange, ImageState, ImageTransition, Load, Sharing};

use crate::common::{
    G, LAYOUTS, WHOLE, bring_buffer, bring_image, buffer, colour, compute_pipeline, device, draws,
    frame, fresh_buffer, graph, home, image, image_import, in_job, lines, on, reached, several,
    submit_after, writes,
};

/// The image and buffer transitions a frame's submits recorded, one entry each.
fn transitions(lines: &[String]) -> (usize, usize) {
    let images = lines.iter().map(|line| line.matches("ImageTransition {").count()).sum();
    let buffers = lines.iter().map(|line| line.matches("BufferTransition {").count()).sum();
    (images, buffers)
}

/// An image and a buffer taken through four states each, by passes of all three kinds, and
/// closed into their `last`: null accepts each transition only from the state the frame
/// left (`state`). The second frame starts each where the first left it. Under one queue
/// the frame records the transitions its uses imply and no others.
#[test]
fn each_transition_is_from_where_the_frame_left_the_resource() {
    for queues in LAYOUTS {
        let device = device(queues);
        let handles = (
            image(&*device, "image", Sharing::Exclusive),
            buffer(&*device, "buffer", Sharing::Exclusive),
            buffer(&*device, "out", Sharing::Exclusive),
        );
        let pipeline = compute_pipeline(&*device);
        in_job(move |ctx| {
            let mut graph = graph(&device, 2, 1);
            for at in 0..2 {
                let first = at == 0;
                let mut decl = FrameDecl::new(1);
                let image = decl.import_image(&image_import(
                    "image",
                    handles.0,
                    on(if first { ImageState::Undefined } else { ImageState::SampledRead }, G),
                    on(ImageState::SampledRead, G),
                ));
                let buffer = decl.import_buffer(&lactose_render_graph::BufferImport {
                    initial: on(
                        if first { BufferState::Undefined } else { BufferState::HostRead },
                        G,
                    ),
                    ..fresh_buffer("buffer", handles.1, G, BufferState::HostRead)
                });
                let out =
                    decl.import_buffer(&fresh_buffer("out", handles.2, G, BufferState::HostRead));
                decl.compute_pass(
                    &ComputePassDecl {
                        label: "write",
                        sampled: &[],
                        buffers: &writes(buffer),
                        slots: &[],
                    },
                    move |pass| {
                        pass.bind_pipeline(pipeline);
                        pass.push_address(0, buffer, 0);
                        pass.dispatch([1, 1, 1]);
                    },
                );
                let index = [BufferUse { buffer, access: BufferAccess::Index }];
                decl.render_pass(
                    &RenderPassDecl {
                        buffers: &index,
                        ..draws("draw", &[colour(image, Load::Clear([0.0; 4]))])
                    },
                    |_| {},
                );
                decl.copy_pass(
                    &CopyPassDecl {
                        label: "read image",
                        image_sources: &[image],
                        image_destinations: &[],
                        buffer_sources: &[],
                        buffer_destinations: &[out],
                    },
                    move |copy| copy.copy_image_to_buffer(image, out, &[WHOLE]),
                );
                decl.copy_pass(
                    &CopyPassDecl {
                        label: "write image",
                        image_sources: &[],
                        image_destinations: &[image],
                        buffer_sources: &[buffer],
                        buffer_destinations: &[],
                    },
                    move |copy| copy.copy_buffer_to_image(buffer, image, &[WHOLE]),
                );
                let point = frame(&mut graph, ctx, decl).expect("null accepts every transition");
                reached(&**graph.device(), point);
                let lines = lines(&device.take_submitted());
                if !several(&**graph.device()) {
                    // The image: to the attachment, the copy's source, its destination, and
                    // closed into `SampledRead`. The buffer: to the shader's write, the
                    // index, the copy's source, and closed into `HostRead`; `out` to the
                    // copy's destination and closed.
                    assert_eq!(transitions(&lines), (4, 6), "{lines:#?}");
                }
            }
        });
    }
}

/// Two passes that write one import in one state, a colour target kept by the second and a
/// buffer two compute passes write, have the same-state transition between them, which null
/// requires (`state`).
#[test]
fn two_writers_in_one_state_have_a_transition_between_them() {
    for queues in LAYOUTS {
        let device = device(queues);
        let target = image(&*device, "target", Sharing::Exclusive);
        let written = buffer(&*device, "written", Sharing::Exclusive);
        let pipeline = compute_pipeline(&*device);
        in_job(move |ctx| {
            let queue = home(&*device);
            let mut graph = graph(&device, 2, 1);
            let mut decl = FrameDecl::new(1);
            let target = decl.import_image(&image_import(
                "target",
                target,
                on(ImageState::Undefined, G),
                on(ImageState::ColourAttachment, G),
            ));
            let written = decl.import_buffer(&fresh_buffer(
                "written",
                written,
                queue,
                BufferState::ShaderWrite,
            ));
            decl.render_pass(&draws("clear", &[colour(target, Load::Clear([0.0; 4]))]), |_| {});
            decl.render_pass(&draws("keep", &[colour(target, Load::Keep)]), |_| {});
            for label in ["first write", "second write"] {
                decl.compute_pass(
                    &ComputePassDecl { label, sampled: &[], buffers: &writes(written), slots: &[] },
                    move |pass| {
                        pass.bind_pipeline(pipeline);
                        pass.push_address(0, written, 0);
                        pass.dispatch([1, 1, 1]);
                    },
                );
            }
            let point = frame(&mut graph, ctx, decl).expect("null accepts both writes");
            reached(&**graph.device(), point);
            let lines = lines(&device.take_submitted());
            let same = |state: String| lines.iter().filter(|line| line.contains(&state)).count();
            let attachment = on(ImageState::ColourAttachment, G);
            assert_eq!(same(format!("from: {attachment:?}, to: {attachment:?}")), 1, "{lines:#?}");
            let write = on(BufferState::ShaderWrite, queue);
            assert_eq!(same(format!("from: {write:?}, to: {write:?}")), 1, "{lines:#?}");
        });
    }
}

/// One declaration, submitted for three frames, of an attachment kept with `Load::Keep` and a
/// buffer a compute pass writes, each imported in its writing state and left there: each
/// frame's first use is after the last frame's write, and null keeps a write across submits,
/// so without the same-state transition at the frame's edge it refuses the second frame
/// (`state`). Each lives on the queue its pass runs on, so no transfer stands in for it.
#[test]
fn a_write_in_one_state_across_the_frames_edge_has_its_transition() {
    for queues in LAYOUTS {
        let device = device(queues);
        let target = image(&*device, "target", Sharing::Exclusive);
        let written = buffer(&*device, "written", Sharing::Exclusive);
        let pipeline = compute_pipeline(&*device);
        let queue = home(&*device);
        bring_image(&*device, target, on(ImageState::ColourAttachment, G));
        bring_buffer(&*device, written, on(BufferState::ShaderWrite, queue));
        device.take_submitted();
        in_job(move |ctx| {
            let mut graph = graph(&device, 1, 1);
            for _ in 0..3 {
                let mut decl = FrameDecl::new(1);
                let attachment = on(ImageState::ColourAttachment, G);
                let target =
                    decl.import_image(&image_import("target", target, attachment, attachment));
                let write = on(BufferState::ShaderWrite, queue);
                let written = decl.import_buffer(&lactose_render_graph::BufferImport {
                    label: "written every frame",
                    buffer: written,
                    sharing: Sharing::Exclusive,
                    initial: write,
                    last: write,
                    ready: None,
                });
                decl.render_pass(&draws("keep", &[colour(target, Load::Keep)]), |_| {});
                decl.compute_pass(
                    &ComputePassDecl {
                        label: "write",
                        sampled: &[],
                        buffers: &writes(written),
                        slots: &[],
                    },
                    move |pass| {
                        pass.bind_pipeline(pipeline);
                        pass.push_address(0, written, 0);
                        pass.dispatch([1, 1, 1]);
                    },
                );
                let point = frame(&mut graph, ctx, decl).expect("null accepts the frame");
                reached(&**graph.device(), point);
                let lines = lines(&device.take_submitted());
                assert_eq!(transitions(&lines), (1, 1), "{lines:#?}");
            }
        });
    }
}

/// A render pass's and a compute pass's reach hold the buffers declared `Read` and `Write`,
/// in `ShaderRead` and `ShaderWrite`, and not the one declared `Indirect`, which is brought
/// to its state alone: null refuses an address pushed for a buffer the pass does not
/// declare, or one in another state (`undeclared`, `state`).
#[test]
fn a_pass_reaches_exactly_its_read_and_written_buffers() {
    for queues in LAYOUTS {
        let device = device(queues);
        let handles = [
            buffer(&*device, "read", Sharing::Exclusive),
            buffer(&*device, "written", Sharing::Exclusive),
            buffer(&*device, "arguments", Sharing::Exclusive),
        ];
        let pipeline = compute_pipeline(&*device);
        let target = image(&*device, "target", Sharing::Exclusive);
        in_job(move |ctx| {
            let queue = home(&*device);
            let mut graph = graph(&device, 1, 1);
            let mut decl = FrameDecl::new(1);
            let [read, written, arguments] =
                [("read", 0), ("written", 1), ("arguments", 2)].map(|(label, at)| {
                    decl.import_buffer(&fresh_buffer(
                        label,
                        handles[at],
                        queue,
                        BufferState::HostRead,
                    ))
                });
            let target = decl.import_image(&image_import(
                "target",
                target,
                on(ImageState::Undefined, G),
                on(ImageState::ColourAttachment, G),
            ));
            // Every buffer is written first, so that the reads below read defined bytes.
            let all = [read, written, arguments]
                .map(|buffer| BufferUse { buffer, access: BufferAccess::Write });
            decl.compute_pass(
                &ComputePassDecl { label: "define", sampled: &[], buffers: &all, slots: &[] },
                |_| {},
            );
            let uses = [
                BufferUse { buffer: read, access: BufferAccess::Read },
                BufferUse { buffer: written, access: BufferAccess::Write },
                BufferUse { buffer: arguments, access: BufferAccess::Indirect },
            ];
            decl.compute_pass(
                &ComputePassDecl { label: "reach", sampled: &[], buffers: &uses, slots: &[] },
                move |pass| {
                    pass.bind_pipeline(pipeline);
                    pass.push_address(0, read, 0);
                    pass.push_address(8, written, 0);
                    pass.dispatch_indirect(arguments, 0);
                },
            );
            let read_only = [BufferUse { buffer: read, access: BufferAccess::Read }];
            decl.render_pass(
                &RenderPassDecl {
                    buffers: &read_only,
                    ..draws("draw", &[colour(target, Load::Clear([0.0; 4]))])
                },
                |_| {},
            );
            let point = frame(&mut graph, ctx, decl).expect("null accepts every reach");
            reached(&**graph.device(), point);
            let lines = lines(&device.take_submitted());
            let begins: Vec<&String> =
                lines.iter().filter(|line| line.starts_with("begin_")).collect();
            assert_eq!(begins.len(), 3, "{lines:#?}");
            let reach = |line: &str, buffer: usize, state: BufferState| {
                line.contains(&format!(
                    "BufferReach {{ buffer: {:?}, state: {state:?} }}",
                    handles[buffer]
                ))
            };
            assert_eq!(begins[1].matches("BufferReach {").count(), 2, "{}", begins[1]);
            assert!(reach(begins[1], 0, BufferState::ShaderRead), "{}", begins[1]);
            assert!(reach(begins[1], 1, BufferState::ShaderWrite), "{}", begins[1]);
            assert_eq!(begins[2].matches("BufferReach {").count(), 1, "{}", begins[2]);
            assert!(reach(begins[2], 0, BufferState::ShaderRead), "{}", begins[2]);
        });
    }
}

/// An image left in its `last` on the graphics queue and handed off to the compute queue:
/// under several queues the frame records the release, which the test's acquire with equal
/// values meets in a submit that waits for the frame's point (`no-qfot` refuses an acquire
/// with no equal release); under one, no release, and the test's change is a transition.
#[test]
fn a_handoff_is_released_for_the_next_user_to_acquire() {
    for queues in LAYOUTS {
        let device = device(queues);
        let target = image(&*device, "target", Sharing::Exclusive);
        in_job(move |ctx| {
            let mut graph = graph(&device, 1, 1);
            let last = on(ImageState::ColourAttachment, G);
            let handoff = on(ImageState::SampledRead, lactose_rhi_api::QueueRole::Compute);
            let mut decl = FrameDecl::new(1);
            let image = decl.import_image(&lactose_render_graph::ImageImport {
                handoff: Some(handoff),
                ..image_import("target", target, on(ImageState::Undefined, G), last)
            });
            decl.render_pass(&draws("draw", &[colour(image, Load::Clear([0.0; 4]))]), |_| {});
            let point = frame(&mut graph, ctx, decl).expect("a frame");
            let lines = lines(&device.take_submitted());
            let release = format!("from: {last:?}, to: {handoff:?}");
            let released = lines.iter().filter(|line| line.contains(&release)).count();
            let several = several(&**graph.device());
            assert_eq!(released, usize::from(several), "{lines:#?}");

            let mut encoder =
                graph.device().create_encoder(handoff.queue, "next user").expect("an encoder");
            encoder.transition_images(&[ImageTransition {
                image: target,
                range: ImageRange { mip: 0, mips: 1, layer: 0, layers: 1 },
                from: last,
                to: handoff,
            }]);
            let (done, _acquire) =
                submit_after(&**graph.device(), handoff.queue, encoder, &[point]);
            reached(&**graph.device(), done);
        });
    }
}

/// The text of one transition from `from` to `to` in a logged line.
fn step<S: core::fmt::Debug>(from: S, to: S) -> String {
    format!("from: {from:?}, to: {to:?}")
}

/// A frame in which one resource takes two transitions in a row at one place.
#[derive(Clone, Copy, Debug)]
enum Chain {
    /// A target drawn, closed into `SampledRead`, then released toward the present role.
    ClosingChange,
    /// A target no pass uses, taken from `Undefined` into its `last`, then released.
    UnusedImage,
    /// An image sampled on the compute queue, acquired back on graphics, then released.
    ClosingAcquire,
    /// A buffer imported `Undefined` on graphics and read on compute: taken, then released.
    MovedBuffer,
    /// The same buffer, which no pass uses, left on compute.
    UnusedBuffer,
}

/// `chain`'s frame on a fresh device of `queues`, the point it returned, and its lines. On a
/// device whose compute role has a queue of its own, the two transitions of the chain, in
/// order.
fn chain_frame(
    chain: Chain,
    queues: lactose_rhi_api::QueueLayout,
) -> (lactose_core::Result<lactose_rhi_api::TimelinePoint>, Vec<String>, [String; 2]) {
    use lactose_render_graph::{BufferImport, ImageImport};
    use lactose_rhi_api::QueueRole::{Compute as C, Present as P};
    let device = device(queues);
    let target = image(&*device, "target", Sharing::Exclusive);
    let data = buffer(&*device, "data", Sharing::Exclusive);
    let out = buffer(&*device, "out", Sharing::Exclusive);
    let pipeline = compute_pipeline(&*device);
    if matches!(chain, Chain::ClosingAcquire) {
        bring_image(&*device, target, on(ImageState::SampledRead, G));
    }
    let sampled = on(ImageState::SampledRead, G);
    let expected = match chain {
        Chain::ClosingChange => [
            step(on(ImageState::ColourAttachment, G), sampled),
            step(sampled, on(ImageState::SampledRead, P)),
        ],
        Chain::UnusedImage => [
            step(on(ImageState::Undefined, G), on(ImageState::ColourAttachment, G)),
            step(on(ImageState::ColourAttachment, G), on(ImageState::SampledRead, P)),
        ],
        Chain::ClosingAcquire => [
            step(on(ImageState::SampledRead, C), sampled),
            step(sampled, on(ImageState::SampledRead, P)),
        ],
        Chain::MovedBuffer | Chain::UnusedBuffer => [
            step(on(BufferState::Undefined, G), on(BufferState::ShaderRead, G)),
            step(on(BufferState::ShaderRead, G), on(BufferState::ShaderRead, C)),
        ],
    };
    let logged = std::sync::Arc::clone(&device);
    let point = in_job(move |ctx| {
        let mut graph = graph(&device, 1, 1);
        let mut decl = FrameDecl::new(1);
        let handed = |initial, last| ImageImport {
            handoff: Some(on(ImageState::SampledRead, P)),
            ..image_import("target", target, initial, last)
        };
        let data_import = BufferImport {
            label: "data",
            buffer: data,
            sharing: Sharing::Exclusive,
            initial: on(BufferState::Undefined, G),
            last: on(BufferState::ShaderRead, C),
            ready: None,
        };
        match chain {
            Chain::ClosingChange => {
                let image = decl.import_image(&handed(on(ImageState::Undefined, G), sampled));
                decl.render_pass(&draws("draw", &[colour(image, Load::Clear([0.0; 4]))]), |_| {});
            }
            Chain::UnusedImage => {
                decl.import_image(&handed(
                    on(ImageState::Undefined, G),
                    on(ImageState::ColourAttachment, G),
                ));
            }
            Chain::ClosingAcquire => {
                let image = decl.import_image(&handed(sampled, sampled));
                let out =
                    decl.import_buffer(&fresh_buffer("out", out, C, BufferState::ShaderWrite));
                decl.compute_pass(
                    &ComputePassDecl {
                        label: "sample",
                        sampled: &[image],
                        buffers: &writes(out),
                        slots: &[],
                    },
                    move |pass| {
                        pass.bind_pipeline(pipeline);
                        pass.dispatch([1, 1, 1]);
                    },
                );
            }
            Chain::MovedBuffer => {
                let data = decl.import_buffer(&data_import);
                decl.compute_pass(
                    &ComputePassDecl {
                        label: "read",
                        sampled: &[],
                        buffers: &crate::common::reads(data),
                        slots: &[],
                    },
                    move |pass| {
                        pass.bind_pipeline(pipeline);
                        pass.dispatch([1, 1, 1]);
                    },
                );
            }
            Chain::UnusedBuffer => {
                decl.import_buffer(&data_import);
            }
        }
        let point = frame(&mut graph, ctx, decl);
        if let Ok(point) = point {
            reached(&**graph.device(), point);
        }
        point
    });
    (point, lines(&logged.take_submitted()), expected)
}

/// A resource that takes two transitions in a row, a change or an acquire and then a release,
/// or a take from `Undefined` and then a release, takes them in two transition calls, in that
/// order, never in one: a device makes one call one barrier, whose elements are not ordered
/// with one another. The logging device refuses a submit with a call that names one resource
/// twice, as it does for every test (`tests/common`). Under one queue no chain arises.
#[test]
fn no_transition_call_names_a_resource_twice() {
    for queues in LAYOUTS {
        for chain in [
            Chain::ClosingChange,
            Chain::UnusedImage,
            Chain::ClosingAcquire,
            Chain::MovedBuffer,
            Chain::UnusedBuffer,
        ] {
            let (point, lines, expected) = chain_frame(chain, queues);
            assert!(point.is_ok(), "{queues:?}, {chain:?}: {point:?}\n{lines:#?}");
            if queues == lactose_rhi_api::QueueLayout::SEVERAL {
                let first = lines.iter().rposition(|line| line.contains(&expected[0]));
                let second = lines.iter().rposition(|line| line.contains(&expected[1]));
                assert!(
                    matches!((first, second), (Some(first), Some(second)) if first < second),
                    "{chain:?}: the chain as two calls, in order\n{lines:#?}"
                );
            }
        }
    }
}
