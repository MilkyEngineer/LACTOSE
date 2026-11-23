//! Transients on null: culling, aliasing and its alias calls, the store actions, memoryless
//! where the device honours it and not where a local read loses it, the usage, the
//! placements' bounds, and the destroy of an evicted plan's images after its last frame.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lactose_render_graph::{
    BufferImport, CopyPassDecl, DepthUse, FrameDecl, PassId, RenderPassDecl, TransientImage,
    TransientMemory,
};
use lactose_rhi_api::{
    BufferState, Capabilities, Extent, Format, ImageDesc, ImageMemory, ImageState, ImageUsage,
    Load, Rule, Sharing,
};

use crate::common::{
    G, LAYOUTS, Logged, SCRATCH, WHOLE, buffer, colour, count, device, device_of, draws, frame,
    fresh_buffer, graph, held, image, image_import, in_job, lines, on, reached,
};

/// A copy of `image` whole into `out`, a pass of its own.
fn read_back(
    decl: &mut FrameDecl<'_>,
    label: &str,
    image: lactose_render_graph::ImageId,
    out: lactose_render_graph::BufferId,
) -> PassId {
    decl.copy_pass(
        &CopyPassDecl {
            label,
            image_sources: &[image],
            image_destinations: &[],
            buffer_sources: &[],
            buffer_destinations: &[out],
        },
        move |copy| copy.copy_image_to_buffer(image, out, &[WHOLE]),
    )
}

/// Two transients of one descriptor whose lives do not overlap share one placement; a
/// pass kept for its readback alone is kept and an unreachable one culled, its recorder
/// never called; and a read of the second before its first write is null's
/// `undefined-read`, returned by `submit` unchanged and leaving no frame.
#[test]
fn transients_alias_a_dead_pass_is_culled_and_a_read_before_the_write_is_refused() {
    for queues in LAYOUTS {
        let device = device(queues);
        let readbacks =
            [buffer(&*device, "a", Sharing::Exclusive), buffer(&*device, "b", Sharing::Exclusive)];
        let called = Arc::new(AtomicBool::new(false));
        let dead_called = Arc::clone(&called);
        in_job(move |ctx| {
            let mut graph = graph(&device, 4, 2);
            let mut decl = FrameDecl::new(1);
            let a = decl.transient_image(&SCRATCH);
            let b = decl.transient_image(&TransientImage { label: "scratch, again", ..SCRATCH });
            let unused = decl.transient_image(&TransientImage { label: "unused", ..SCRATCH });
            let out_a =
                decl.import_buffer(&fresh_buffer("a", readbacks[0], G, BufferState::HostRead));
            let out_b =
                decl.import_buffer(&fresh_buffer("b", readbacks[1], G, BufferState::HostRead));
            let first =
                decl.render_pass(&draws("first", &[colour(a, Load::Clear([0.0; 4]))]), |_| {});
            let read_a = read_back(&mut decl, "read a", a, out_a);
            // The breach under test: `b` is kept before any pass wrote it.
            let second = decl.render_pass(&draws("second", &[colour(b, Load::Keep)]), |_| {});
            let read_b = read_back(&mut decl, "read b", b, out_b);
            let dead = decl
                .render_pass(&draws("dead", &[colour(unused, Load::Clear([0.0; 4]))]), move |_| {
                    dead_called.store(true, Ordering::Relaxed)
                });

            let compiled = graph.compile(decl).expect("a plan");
            assert!(compiled.fresh());
            let plan = compiled.plan();
            assert!(plan.is_culled(dead) && !plan.is_culled(read_a) && !plan.is_culled(read_b));
            assert_eq!(plan.transient(unused), None);
            let kept: Vec<PassId> = plan.passes().iter().map(|planned| planned.pass).collect();
            assert_eq!(kept, [first, read_a, second, read_b]);
            assert!(matches!(plan.transient(a), Some(TransientMemory::Placed { .. })));
            assert_eq!(plan.transient(a), plan.transient(b));
            assert_eq!(plan.submits(), 1);

            let recorded = graph.record(compiled, ctx).expect("a recorded frame");
            let refused = graph.submit(recorded).expect_err("null refuses the read");
            assert_eq!(Rule::of(&refused), Some(Rule::UndefinedRead), "{refused}");
            assert_eq!(graph.last_frame(), None);
            assert!(device.take_submitted().is_empty());
        });
        assert!(!called.load(Ordering::Relaxed), "a culled pass's recorder ran");
    }
}

/// The frame above with `b` written first: each frame records the alias call before the
/// first use of each transient whose bytes the other takes, the frame before's included, and
/// stores what a later pass reads and nothing else.
#[test]
fn aliasing_transients_are_aliased_every_frame_and_stored_where_read_later() {
    for queues in LAYOUTS {
        let device = device(queues);
        let readbacks =
            [buffer(&*device, "a", Sharing::Exclusive), buffer(&*device, "b", Sharing::Exclusive)];
        let target = image(&*device, "target", Sharing::Exclusive);
        in_job(move |ctx| {
            let mut graph = graph(&device, 2, 1);
            for at in 0..3 {
                let mut decl = FrameDecl::new(1);
                let a = decl.transient_image(&SCRATCH);
                let b = decl.transient_image(&TransientImage { label: "b", ..SCRATCH });
                let c = decl.transient_image(&TransientImage { label: "c", ..SCRATCH });
                let initial =
                    if at == 0 { ImageState::Undefined } else { ImageState::ColourAttachment };
                let target = decl.import_image(&image_import(
                    "target",
                    target,
                    on(initial, G),
                    on(ImageState::ColourAttachment, G),
                ));
                let out_a =
                    decl.import_buffer(&fresh_buffer("a", readbacks[0], G, BufferState::HostRead));
                let out_b =
                    decl.import_buffer(&fresh_buffer("b", readbacks[1], G, BufferState::HostRead));
                decl.render_pass(&draws("a", &[colour(a, Load::Clear([0.0; 4]))]), |_| {});
                read_back(&mut decl, "read a", a, out_a);
                decl.render_pass(&draws("b", &[colour(b, Load::Clear([1.0; 4]))]), |_| {});
                read_back(&mut decl, "read b", b, out_b);
                // `c` is written and never read: an attachment of a pass kept for `target`.
                decl.render_pass(
                    &draws(
                        "c",
                        &[colour(target, Load::Clear([0.0; 4])), colour(c, Load::Clear([0.0; 4]))],
                    ),
                    |_| {},
                );
                let point = frame(&mut graph, ctx, decl).expect("a frame");
                reached(&**graph.device(), point);

                let lines = lines(&device.take_submitted());
                let begins: Vec<&String> =
                    lines.iter().filter(|line| line.starts_with("begin_render_pass")).collect();
                assert_eq!(begins.len(), 3, "{lines:#?}");
                assert_eq!(begins[0].matches("store: Store").count(), 1, "{}", begins[0]);
                assert_eq!(begins[1].matches("store: Store").count(), 1, "{}", begins[1]);
                // The target is an import, stored; `c` is read by no later pass.
                assert_eq!(begins[2].matches("store: Store").count(), 1, "{}", begins[2]);
                assert_eq!(begins[2].matches("store: DontCare").count(), 1, "{}", begins[2]);
                // `a`, `b` and `c` share bytes: each is aliased before its first use.
                assert_eq!(count(&lines, "alias"), 3, "{lines:#?}");
                let first = |what: &str| lines.iter().position(|line| line.starts_with(what));
                assert!(first("alias") < first("begin_render_pass"), "{lines:#?}");
            }
        });
    }
}

const DEPTH: TransientImage<'static> =
    TransientImage { label: "depth", format: Format::Depth32Float, ..SCRATCH };

fn depth(image: lactose_render_graph::ImageId, load: Load<f32>) -> Option<DepthUse> {
    Some(DepthUse { image, mip: 0, layer: 0, load })
}

/// The memory one frame under `set` gives a depth transient used by one pass and a colour
/// transient locally read in one, each frame submitted; and, in a second frame, a depth
/// candidate loaded with `Keep`, given memory and refused its read.
fn memoryless_frame(set: Capabilities) -> [Option<TransientMemory>; 2] {
    let device = device_of(set);
    let handle = image(&*device, "target", Sharing::Exclusive);
    in_job(move |ctx| {
        let mut graph = graph(&device, 1, 1);
        let mut decl = FrameDecl::new(1);
        let target = decl.import_image(&image_import(
            "target",
            handle,
            on(ImageState::Undefined, G),
            on(ImageState::ColourAttachment, G),
        ));
        let depth_image = decl.transient_image(&DEPTH);
        let local = decl.transient_image(&TransientImage { label: "local", ..SCRATCH });
        let colour_only = [colour(target, Load::Clear([0.0; 4]))];
        decl.render_pass(
            &RenderPassDecl {
                label: "depth",
                colour: &colour_only,
                depth: depth(depth_image, Load::Clear(1.0)),
                sampled: &[],
                buffers: &[],
                slots: &[],
            },
            |_| {},
        );
        let read = [
            colour(target, Load::Keep),
            lactose_render_graph::ColourUse {
                local_read: true,
                ..colour(local, Load::Clear([0.0; 4]))
            },
        ];
        decl.render_pass(
            &RenderPassDecl {
                label: "local read",
                colour: &read,
                depth: None,
                sampled: &[],
                buffers: &[],
                slots: &[],
            },
            |pass| pass.local_read(),
        );
        let compiled = graph.compile(decl).expect("a plan");
        let memory = [depth_image, local].map(|image| compiled.plan().transient(image));
        let recorded = graph.record(compiled, ctx).expect("a recorded frame");
        let point = graph.submit(recorded).expect("null accepts the frame");
        reached(&**graph.device(), point);

        // A candidate loaded with `Keep` is given memory, and its read is null's to refuse.
        let mut decl = FrameDecl::new(1);
        let target = decl.import_image(&image_import(
            "target",
            handle,
            on(ImageState::ColourAttachment, G),
            on(ImageState::ColourAttachment, G),
        ));
        let kept_image = decl.transient_image(&TransientImage { label: "kept", ..DEPTH });
        let colour_only = [colour(target, Load::Keep)];
        decl.render_pass(
            &RenderPassDecl {
                label: "keeps depth",
                colour: &colour_only,
                depth: depth(kept_image, Load::Keep),
                sampled: &[],
                buffers: &[],
                slots: &[],
            },
            |_| {},
        );
        let compiled = graph.compile(decl).expect("a plan");
        assert!(matches!(
            compiled.plan().transient(kept_image),
            Some(TransientMemory::Placed { .. })
        ));
        let recorded = graph.record(compiled, ctx).expect("a recorded frame");
        let refused = graph.submit(recorded).expect_err("null refuses the read of the kept depth");
        assert_eq!(Rule::of(&refused), Some(Rule::UndefinedRead), "{refused}");
        memory
    })
}

/// Memoryless is asked for where the device reports it honoured for the use, and nowhere
/// else: not under a set without it, and not for a colour use with a local read where the
/// device loses it there (`split`). Null refuses a memoryless attachment stored or kept
/// (`memoryless-store`) and a memoryless request in a pass with a local read where the set
/// does not report it survives (`split`), so each frame's submit is the proof of the rest.
#[test]
fn memoryless_only_where_the_device_honours_it() {
    let placed =
        |memory: Option<TransientMemory>| matches!(memory, Some(TransientMemory::Placed { .. }));
    let least = memoryless_frame(Capabilities::LEAST);
    assert!(placed(least[0]) && placed(least[1]), "{least:?}");
    let lazy =
        Capabilities { memoryless_colour: true, memoryless_depth: true, ..Capabilities::LEAST };
    let split = memoryless_frame(lazy);
    assert_eq!(split[0], Some(TransientMemory::Memoryless));
    assert!(placed(split[1]), "{split:?}");
    let survives = memoryless_frame(Capabilities { memoryless_survives_local_read: true, ..lazy });
    assert_eq!(survives, [Some(TransientMemory::Memoryless); 2]);
}

/// Transients of three sizes, two of them alive at once: each placement lies inside its
/// block at an offset of its alignment (null's `bounds` refuses one that does not, at the
/// image's creation), two that are alive at once share no byte, and one that begins after
/// both ended takes the lowest bytes that fit.
#[test]
fn placements_are_aligned_inside_their_block_and_apart_while_alive() {
    let device = device(lactose_rhi_api::QueueLayout::ONE);
    let readback = buffer(&*device, "out", Sharing::Exclusive);
    in_job(move |ctx| {
        let mut graph = graph(&device, 1, 1);
        let mut decl = FrameDecl::new(1);
        let small = TransientImage { label: "small", ..SCRATCH };
        let large = TransientImage {
            label: "large",
            extent: Extent { width: 256, height: 256 },
            ..SCRATCH
        };
        let late =
            TransientImage { label: "late", extent: Extent { width: 32, height: 32 }, ..SCRATCH };
        let descs = [small, large, late];
        let [a, b, c] = descs.map(|desc| decl.transient_image(&desc));
        let out = decl.import_buffer(&fresh_buffer("out", readback, G, BufferState::HostRead));
        // `a` and `b` are alive together, from the first pass to the copy; `c` after both.
        decl.render_pass(&draws("a", &[colour(a, Load::Clear([0.0; 4]))]), |_| {});
        decl.render_pass(&draws("b", &[colour(b, Load::Clear([0.0; 4]))]), |_| {});
        decl.copy_pass(
            &CopyPassDecl {
                label: "read a and b",
                image_sources: &[a, b],
                image_destinations: &[],
                buffer_sources: &[],
                buffer_destinations: &[out],
            },
            move |copy| copy.copy_image_to_buffer(a, out, &[WHOLE]),
        );
        decl.render_pass(&draws("c", &[colour(c, Load::Clear([0.0; 4]))]), |_| {});
        let c_out = lactose_rhi_api::BufferImageCopy {
            rect: lactose_rhi_api::Rect { x: 0, y: 0, extent: late.extent },
            ..WHOLE
        };
        decl.copy_pass(
            &CopyPassDecl {
                label: "read c",
                image_sources: &[c],
                image_destinations: &[],
                buffer_sources: &[],
                buffer_destinations: &[out],
            },
            move |copy| copy.copy_image_to_buffer(c, out, &[c_out]),
        );
        let compiled = graph.compile(decl).expect("a plan");
        let placed: Vec<(u32, u64, u64)> = [a, b, c]
            .iter()
            .map(|&image| match compiled.plan().transient(image) {
                Some(TransientMemory::Placed { block, offset, size }) => (block, offset, size),
                other => panic!("{other:?}"),
            })
            .collect();
        for (desc, &(_, offset, size)) in descs.iter().zip(&placed) {
            let usage = ImageUsage::COLOUR.union(ImageUsage::COPY_SOURCE);
            let placement = graph
                .device()
                .image_placement(&ImageDesc {
                    label: desc.label,
                    format: desc.format,
                    extent: desc.extent,
                    mips: 1,
                    layers: 1,
                    usage,
                    memory: ImageMemory::Own,
                    sharing: Sharing::Exclusive,
                })
                .expect("a placement");
            assert_eq!(size, placement.size, "{}", desc.label);
            assert_eq!(offset % placement.alignment, 0, "{}", desc.label);
        }
        let [(_, a_at, a_size), (_, b_at, b_size), (_, c_at, _)] = placed[..] else {
            unreachable!()
        };
        assert!(a_at + a_size <= b_at || b_at + b_size <= a_at, "{placed:?}");
        assert_eq!(c_at, 0, "{placed:?}");
        let recorded = graph.record(compiled, ctx).expect("a recorded frame");
        let point = graph.submit(recorded).expect("a frame");
        reached(&**graph.device(), point);
    });
}

/// An evicted plan's images and block are destroyed after its last frame completes and not
/// before: with the frame held pending, the eviction destroys nothing, and once its point is
/// reached the next `compile` destroys them, none refused (`pending`).
#[test]
fn an_evicted_plans_transients_go_after_its_last_frame() {
    for queues in LAYOUTS {
        let device = device(queues);
        let readback = buffer(&*device, "out", Sharing::Exclusive);
        let gate = held(&*device);
        in_job(move |ctx| {
            let mut graph = graph(&device, 1, 1);
            let declare = |label: &'static str, ready| {
                let mut decl = FrameDecl::new(1);
                let a = decl.transient_image(&SCRATCH);
                let out = decl.import_buffer(&BufferImport {
                    ready,
                    ..fresh_buffer("out", readback, G, BufferState::HostRead)
                });
                decl.render_pass(&draws(label, &[colour(a, Load::Clear([0.0; 4]))]), |_| {});
                read_back(&mut decl, "read", a, out);
                decl
            };
            let destroyed = |device: &Logged| {
                device.destroyed().iter().filter(|(kind, _)| *kind != "timeline").count()
            };
            let pending = frame(&mut graph, ctx, declare("held", Some(gate))).expect("a frame");
            // The eviction of the held frame's plan.
            let compiled = graph.compile(declare("other", None)).expect("a plan");
            assert_eq!(graph.stats().evictions, 1);
            drop(compiled);
            assert_eq!(destroyed(&device), 0, "{:?}", device.destroyed());
            graph.device().signal_timeline(gate).expect("the host's signal");
            reached(&**graph.device(), pending);
            let again = frame(&mut graph, ctx, declare("other", None)).expect("a frame");
            reached(&**graph.device(), again);
            // The held plan's one image and its block.
            assert_eq!(destroyed(&device), 2, "{:?}", device.destroyed());
            drop(graph);
            let all = device.destroyed();
            assert!(all.iter().all(|&(_, accepted)| accepted), "{all:?}");
            assert_eq!(destroyed(&device), 4, "{all:?}");
        });
    }
}
