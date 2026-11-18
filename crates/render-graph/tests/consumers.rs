//! Compile-only: what `graph-impl`'s tests on null, the minimal renderer and the client's
//! frame write against this crate, type-checked and never run (every deciding body is
//! `todo!()`). They pin the surface the README promises: a change that breaks one is a
//! signature change to list.

#![allow(dead_code, unused_variables, missing_docs, clippy::too_many_arguments)]

use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use lactose_core::{Error, ErrorKind, Result};
use lactose_render_graph::{
    BufferAccess, BufferId, BufferImport, BufferUse, ColourUse, CompiledPlan, ComputePassDecl,
    CopyPassDecl, DepthUse, FrameDecl, GraphDesc, GraphKey, History, ImageId, ImageImport, PassId,
    RecordedFrame, RenderGraph, RenderPassDecl, TransientImage, TransientMemory,
};
use lactose_rhi_api::{
    BufferCopy, BufferHandle, BufferImageCopy, BufferState, ComputePipelineHandle, Device, Encoder,
    Extent, Format, GraphicsPipelineHandle, HeapSlot, ImageHandle, ImageRange, ImageState,
    ImageTransition, IndexedIndirectArgs, Load, QueuePath, QueueRole, Rect, Rule, Sharing, StateOn,
    TimelinePoint,
};
use lactose_task::JobContext;

const GRAPHICS: QueueRole = QueueRole::Graphics;

fn on<S>(state: S, queue: QueueRole) -> StateOn<S> {
    StateOn { state, queue }
}

fn colour(image: ImageId, load: Load<[f32; 4]>) -> ColourUse {
    ColourUse { image, mip: 0, layer: 0, load, local_read: false }
}

/// A render pass that draws into colour attachments and uses nothing else.
fn draws<'a>(label: &'a str, colour: &'a [ColourUse]) -> RenderPassDecl<'a> {
    RenderPassDecl { label, colour, depth: None, sampled: &[], buffers: &[], slots: &[] }
}

/// A buffer that stays on the graphics queue: imported from `Undefined` every frame, which
/// keeps its bytes, so the declaration is the same from the first frame on.
fn on_graphics<'a>(label: &'a str, buffer: BufferHandle, last: BufferState) -> BufferImport<'a> {
    BufferImport {
        label,
        buffer,
        sharing: Sharing::Exclusive,
        initial: on(BufferState::Undefined, GRAPHICS),
        last: on(last, GRAPHICS),
        ready: None,
    }
}

const SCRATCH: TransientImage<'static> = TransientImage {
    label: "scratch",
    format: Format::Rgba8Unorm,
    extent: Extent { width: 64, height: 64 },
    mips: 1,
    layers: 1,
};

const WHOLE_SCRATCH: BufferImageCopy = BufferImageCopy {
    buffer_offset: 0,
    mip: 0,
    layer: 0,
    rect: Rect { x: 0, y: 0, extent: SCRATCH.extent },
};

// ---- 1. `graph-impl`, on null: the proofs of its plan row, with no hook ----

/// Two transients alias, the second starts undefined, and a read before its first write is
/// refused by null's rule, which `submit` returns unchanged. An unreachable pass is culled
/// and a pass whose only output is a readback is kept.
fn graph_impl_aliases_culls_and_is_refused(
    device: Arc<dyn Device>,
    ctx: &mut JobContext<'_>,
    readbacks: [BufferHandle; 2],
) -> Result<()> {
    let mut graph = RenderGraph::new(device, &GraphDesc { jobs: 4, plans: 2 })?;
    let mut decl = FrameDecl::new(1);
    let a = decl.transient_image(&SCRATCH);
    let b = decl.transient_image(&TransientImage { label: "scratch, again", ..SCRATCH });
    let unused = decl.transient_image(&TransientImage { label: "unused", ..SCRATCH });
    let out_a = decl.import_buffer(&on_graphics("readback a", readbacks[0], BufferState::HostRead));
    let out_b = decl.import_buffer(&on_graphics("readback b", readbacks[1], BufferState::HostRead));

    let first = decl.render_pass(&draws("first", &[colour(a, Load::Clear([0.0; 4]))]), |_| {});
    let read_a = decl.copy_pass(
        &CopyPassDecl {
            label: "read a",
            image_sources: &[a],
            image_destinations: &[],
            buffer_sources: &[],
            buffer_destinations: &[out_a],
        },
        move |copy| copy.copy_image_to_buffer(a, out_a, &[WHOLE_SCRATCH]),
    );
    // The breach under test: `b` is kept before any pass wrote it.
    let second = decl.render_pass(&draws("second", &[colour(b, Load::Keep)]), |_| {});
    let read_b = decl.copy_pass(
        &CopyPassDecl {
            label: "read b",
            image_sources: &[b],
            image_destinations: &[],
            buffer_sources: &[],
            buffer_destinations: &[out_b],
        },
        move |copy| copy.copy_image_to_buffer(b, out_b, &[WHOLE_SCRATCH]),
    );
    let dead = decl.render_pass(&draws("dead", &[colour(unused, Load::Clear([0.0; 4]))]), |_| {
        panic!("a culled pass's recorder is never called")
    });

    let compiled = graph.compile(decl)?;
    assert!(compiled.fresh());
    let plan: &CompiledPlan = compiled.plan();
    assert!(plan.is_culled(dead) && !plan.is_culled(read_a));
    assert_eq!(plan.transient(unused), None);
    let kept: Vec<PassId> = plan.passes().iter().map(|planned| planned.pass).collect();
    assert_eq!(kept, [first, read_a, second, read_b]);
    // Equal descriptors and disjoint lifetimes: one placement.
    assert!(matches!(plan.transient(a), Some(TransientMemory::Placed { .. })));
    assert_eq!(plan.transient(a), plan.transient(b));
    // One queue's frame is one submit, so the refusal below changes nothing.
    assert_eq!(plan.submits(), 1);

    let recorded = graph.record(compiled, ctx)?;
    let refused = graph.submit(recorded).expect_err("null refuses the read");
    assert_eq!(Rule::of(&refused), Some(Rule::UndefinedRead));
    assert_eq!(graph.last_frame(), None);
    Ok(())
}

/// Each input of the cache key, changed alone, forces a recompile; the same declaration
/// again does not; and the cap evicts.
fn graph_impl_key_inputs(device: Arc<dyn Device>, targets: [ImageHandle; 2]) -> Result<()> {
    let mut graph = RenderGraph::new(device, &GraphDesc { jobs: 1, plans: 2 })?;
    let declare = |views: u32, target: ImageHandle, initial: ImageState, label: &'static str| {
        let mut decl = FrameDecl::new(views);
        let target = decl.import_image(&ImageImport {
            label: "target",
            image: target,
            mips: 1,
            layers: 1,
            sharing: Sharing::Exclusive,
            initial: on(initial, GRAPHICS),
            last: on(ImageState::ColourAttachment, GRAPHICS),
            handoff: None,
            ready: None,
        });
        decl.render_pass(&draws(label, &[colour(target, Load::DontCare)]), |_| {});
        decl
    };

    let key: GraphKey = {
        let base = graph.compile(declare(1, targets[0], ImageState::Undefined, "draw"))?;
        assert!(base.fresh());
        base.plan().key()
    };
    // Another handle under the same declaration is the same plan.
    let again = graph.compile(declare(1, targets[1], ImageState::Undefined, "draw"))?;
    assert!(!again.fresh());
    assert_eq!(again.plan().key(), key);
    drop(again);

    for changed in [
        declare(2, targets[0], ImageState::Undefined, "draw"),
        declare(1, targets[0], ImageState::ColourAttachment, "draw"),
        declare(1, targets[0], ImageState::Undefined, "renamed"),
    ] {
        let frame = graph.compile(changed)?;
        assert!(frame.fresh());
        assert_ne!(frame.plan().key(), key);
    }
    let stats = graph.stats();
    assert_eq!((stats.compiles, stats.hits), (4, 1));
    assert_eq!((stats.plans, stats.evictions), (2, 2));
    Ok(())
}

/// A use moved from one of a pass's lists to another is a key input: a copy pass that only
/// reads `b` writes nothing and is culled, and the same ids with `b` a destination make a
/// pass that writes an import and is kept. A hit would replay the culled plan.
fn graph_impl_key_tells_the_lists_apart(
    device: Arc<dyn Device>,
    images: [ImageHandle; 2],
) -> Result<()> {
    let mut graph = RenderGraph::new(device, &GraphDesc { jobs: 1, plans: 2 })?;
    let declare = |b_is_written: bool| {
        let mut decl = FrameDecl::new(1);
        let import = |label, image| ImageImport {
            label,
            image,
            mips: 1,
            layers: 1,
            sharing: Sharing::Exclusive,
            initial: on(ImageState::CopySource, GRAPHICS),
            last: on(ImageState::CopySource, GRAPHICS),
            handoff: None,
            ready: None,
        };
        let a = decl.import_image(&import("a", images[0]));
        let b = decl.import_image(&import("b", images[1]));
        let (sources, destinations): (&[ImageId], &[ImageId]) =
            if b_is_written { (&[a], &[b]) } else { (&[a, b], &[]) };
        let pass = decl.copy_pass(
            &CopyPassDecl {
                label: "p",
                image_sources: sources,
                image_destinations: destinations,
                buffer_sources: &[],
                buffer_destinations: &[],
            },
            |_| {},
        );
        (decl, pass)
    };

    let (reads, pass) = declare(false);
    let culled = graph.compile(reads)?;
    assert!(culled.plan().is_culled(pass));
    let (writes, pass) = declare(true);
    let kept = graph.compile(writes)?;
    assert!(kept.fresh());
    assert!(!kept.plan().is_culled(pass));
    assert_ne!(kept.plan().key(), culled.plan().key());
    Ok(())
}

/// The same-state transition across a frame's edge: a buffer a compute pass writes every
/// frame, imported in `ShaderWrite` and left there, is written by the second frame after
/// the first frame's write. Without the transition before the second frame's first use,
/// null refuses that frame's submit (`state`). The buffer lives on the queue the pass runs
/// on, so no transfer stands in for the transition under either layout.
fn graph_impl_writes_across_the_frame_edge(
    device: Arc<dyn Device>,
    ctx: &mut JobContext<'_>,
    pipeline: ComputePipelineHandle,
    written: BufferHandle,
) -> Result<()> {
    let several = device.capabilities().queues.compute == QueuePath::Own;
    let queue = if several { QueueRole::Compute } else { GRAPHICS };
    let mut graph = RenderGraph::new(device, &GraphDesc { jobs: 1, plans: 1 })?;
    let mut points = Vec::new();
    for _ in 0..2 {
        let mut decl = FrameDecl::new(1);
        let buffer = decl.import_buffer(&BufferImport {
            label: "written every frame",
            buffer: written,
            sharing: Sharing::Exclusive,
            initial: on(BufferState::ShaderWrite, queue),
            last: on(BufferState::ShaderWrite, queue),
            ready: None,
        });
        decl.compute_pass(
            &ComputePassDecl {
                label: "write",
                sampled: &[],
                buffers: &[BufferUse { buffer, access: BufferAccess::Write }],
                slots: &[],
            },
            move |pass| {
                pass.bind_pipeline(pipeline);
                pass.push_address(0, buffer, 0);
                pass.dispatch([1, 1, 1]);
            },
        );
        let compiled = graph.compile(decl)?;
        let recorded = graph.record(compiled, ctx)?;
        points.push(graph.submit(recorded)?);
    }
    assert!(graph.device().wait_timeline(points[1], Duration::from_secs(5))?);
    Ok(())
}

/// A copy pass's recorder writes each destination in one call: a second call into one
/// destination fails `record`, naming the pass, and nothing of the frame is recorded.
fn graph_impl_copy_writes_each_destination_once(
    device: Arc<dyn Device>,
    ctx: &mut JobContext<'_>,
    buffers: [BufferHandle; 3],
) -> Result<()> {
    let mut graph = RenderGraph::new(device, &GraphDesc { jobs: 1, plans: 1 })?;
    let mut decl = FrameDecl::new(1);
    let [first, second, out] = buffers;
    let first = decl.import_buffer(&on_graphics("first", first, BufferState::CopySource));
    let second = decl.import_buffer(&on_graphics("second", second, BufferState::CopySource));
    let out = decl.import_buffer(&on_graphics("out", out, BufferState::HostRead));
    let region = BufferCopy { source_offset: 0, destination_offset: 0, size: 16 };
    decl.copy_pass(
        &CopyPassDecl {
            label: "twice into out",
            image_sources: &[],
            image_destinations: &[],
            buffer_sources: &[first, second],
            buffer_destinations: &[out],
        },
        move |copy| {
            copy.copy_buffer(first, out, &[region]);
            copy.copy_buffer(second, out, &[region]);
        },
    );
    let compiled = graph.compile(decl)?;
    let breach = graph.record(compiled, ctx).err().expect("the second write is a breach");
    assert_eq!(breach.kind(), ErrorKind::InvalidData);
    Ok(())
}

/// The two queue paths: under a capability set whose compute role has a queue of its own, a
/// compute pass with no graphics-queue dependency runs there and the frame takes a second
/// submit; under one queue every pass is the graphics role's and the frame is one submit.
fn graph_impl_queue_paths(
    device: Arc<dyn Device>,
    alone: BufferHandle,
    fed: BufferHandle,
    target: ImageHandle,
) -> Result<()> {
    let several = device.capabilities().queues.compute == QueuePath::Own;
    let mut graph = RenderGraph::new(device, &GraphDesc { jobs: 2, plans: 1 })?;
    let mut decl = FrameDecl::new(1);
    let alone = decl.import_buffer(&BufferImport {
        label: "alone",
        buffer: alone,
        sharing: Sharing::Exclusive,
        initial: on(BufferState::Undefined, QueueRole::Compute),
        last: on(BufferState::ShaderWrite, QueueRole::Compute),
        ready: None,
    });
    let fed = decl.import_buffer(&on_graphics("fed", fed, BufferState::ShaderRead));
    let target = decl.import_image(&ImageImport {
        label: "target",
        image: target,
        mips: 1,
        layers: 1,
        sharing: Sharing::Exclusive,
        initial: on(ImageState::Undefined, GRAPHICS),
        last: on(ImageState::ColourAttachment, GRAPHICS),
        handoff: None,
        ready: None,
    });
    let writes = |buffer| [BufferUse { buffer, access: BufferAccess::Write }];
    let independent = decl.compute_pass(
        &ComputePassDecl { label: "alone", sampled: &[], buffers: &writes(alone), slots: &[] },
        |_| {},
    );
    let feeds = decl.compute_pass(
        &ComputePassDecl { label: "feeds", sampled: &[], buffers: &writes(fed), slots: &[] },
        |_| {},
    );
    let draws = decl.render_pass(
        &RenderPassDecl {
            label: "draws",
            colour: &[colour(target, Load::DontCare)],
            depth: None,
            sampled: &[],
            buffers: &[BufferUse { buffer: fed, access: BufferAccess::Read }],
            slots: &[],
        },
        |_| {},
    );

    let compiled = graph.compile(decl)?;
    let plan = compiled.plan();
    assert_eq!(plan.queues(), graph.device().capabilities().queues);
    for planned in plan.passes() {
        let moved = several && planned.pass == independent;
        assert_eq!(planned.queue, if moved { QueueRole::Compute } else { GRAPHICS });
        assert_eq!(planned.submit, u32::from(several && planned.pass != independent));
    }
    assert_eq!(plan.submits(), if several { 2 } else { 1 });
    assert_eq!(plan.jobs(), if several { 3 } else { 2 });
    Ok(())
}

/// An encoder is reset only after its frame's point: with the device made to lag, a second
/// frame takes encoders of its own, and once the first frame's point is reached a third
/// reuses them. `frame` declares an import whose `ready` is `held`, a point of a timeline
/// of the test's that the host owes, so both frames stay pending until the test signals it.
fn graph_impl_pools_under_lag(
    graph: &mut RenderGraph,
    ctx: &mut JobContext<'_>,
    held: TimelinePoint,
    mut frame: impl FnMut(&mut RenderGraph, &mut JobContext<'_>) -> Result<TimelinePoint>,
) -> Result<()> {
    let first = frame(graph, ctx)?;
    let after_one = graph.stats().encoders;
    let second = frame(graph, ctx)?;
    assert_eq!(graph.stats().encoders, 2 * after_one);
    assert_eq!(first.timeline, second.timeline);
    assert!(first.value < second.value);
    assert_eq!(graph.last_frame(), Some(second));

    // The test lets the device go: the host signals the point it owes before any wait, and
    // a point reached covers every earlier frame.
    graph.device().signal_timeline(held)?;
    assert!(graph.device().wait_timeline(second, Duration::from_secs(5))?);
    frame(graph, ctx)?;
    assert_eq!(graph.stats().encoders, 2 * after_one);
    Ok(())
}

// ---- 2. The minimal renderer of decision 5.4 (`renderer`, `renderer-impl`) ----

struct MinimalRenderer {
    ring: BufferHandle,
    ring_offset: u64,
    rows: u32,
    instances: BufferHandle,
    indices: BufferHandle,
    arguments: BufferHandle,
    camera: BufferHandle,
    scatter: ComputePipelineHandle,
    triangles: GraphicsPipelineHandle,
    textures: Vec<HeapSlot>,
    extent: Extent,
    clear: [f32; 4],
}

/// The shape of the renderer's pass-provider trait (architecture section 12.5): a provider
/// adds passes to the frame's declaration, and their recorders borrow it for the frame.
trait PassProvider: Send + Sync {
    fn declare<'f>(&'f self, decl: &mut FrameDecl<'f>, target: ImageId);
}

impl PassProvider for MinimalRenderer {
    /// The staging ring read by the scatter, which writes the persistent instance buffer
    /// through its address; then the instanced triangles, indexed and indirect, reading the
    /// instances and the latched camera.
    fn declare<'f>(&'f self, decl: &mut FrameDecl<'f>, target: ImageId) {
        let ring = decl.import_buffer(&on_graphics("ring", self.ring, BufferState::ShaderRead));
        let instances =
            decl.import_buffer(&on_graphics("instances", self.instances, BufferState::ShaderRead));
        let indices = decl.import_buffer(&on_graphics("indices", self.indices, BufferState::Index));
        let arguments =
            decl.import_buffer(&on_graphics("arguments", self.arguments, BufferState::Indirect));
        let camera =
            decl.import_buffer(&on_graphics("camera", self.camera, BufferState::ShaderRead));
        // Used by one pass as its depth attachment: memoryless where the device reports it.
        let depth = decl.transient_image(&TransientImage {
            label: "depth",
            format: Format::Depth32Float,
            extent: self.extent,
            mips: 1,
            layers: 1,
        });

        decl.compute_pass(
            &ComputePassDecl {
                label: "scatter",
                sampled: &[],
                buffers: &[
                    BufferUse { buffer: ring, access: BufferAccess::Read },
                    BufferUse { buffer: instances, access: BufferAccess::Write },
                ],
                slots: &[],
            },
            move |pass| {
                pass.bind_pipeline(self.scatter);
                pass.push_address(0, ring, self.ring_offset);
                pass.push_address(8, instances, 0);
                pass.push_bytes(16, &self.rows.to_le_bytes());
                pass.dispatch([self.rows.div_ceil(64), 1, 1]);
            },
        );
        decl.render_pass(
            &RenderPassDecl {
                label: "triangles",
                colour: &[colour(target, Load::Clear(self.clear))],
                depth: Some(DepthUse { image: depth, mip: 0, layer: 0, load: Load::Clear(0.0) }),
                sampled: &[],
                buffers: &[
                    BufferUse { buffer: instances, access: BufferAccess::Read },
                    BufferUse { buffer: camera, access: BufferAccess::Read },
                    BufferUse { buffer: indices, access: BufferAccess::Index },
                    BufferUse { buffer: arguments, access: BufferAccess::Indirect },
                ],
                slots: &self.textures,
            },
            move |pass| {
                pass.bind_pipeline(self.triangles);
                pass.bind_index_buffer(indices, 0);
                pass.push_address(0, instances, 0);
                pass.push_address(8, camera, 0);
                if let Some(&texture) = self.textures.first() {
                    pass.push_slot(16, texture);
                }
                pass.draw_indexed_indirect(arguments, 0, 1, IndexedIndirectArgs::BYTES);
            },
        );
    }
}

/// A recorder is called once, by the one job that owns it: it may consume what it captured
/// and hold a `&mut` for the frame, and the borrow ends when `record` returns.
fn recorder_owns_what_it_captures(
    graph: &mut RenderGraph,
    ctx: &mut JobContext<'_>,
    target: ImageHandle,
    list: Vec<(GraphicsPipelineHandle, u32)>,
) -> Result<u32> {
    let mut drawn = 0u32;
    let mut decl = FrameDecl::new(1);
    let target = decl.import_image(&ImageImport {
        label: "target",
        image: target,
        mips: 1,
        layers: 1,
        sharing: Sharing::Exclusive,
        initial: on(ImageState::Undefined, GRAPHICS),
        last: on(ImageState::ColourAttachment, GRAPHICS),
        handoff: None,
        ready: None,
    });
    decl.render_pass(&draws("list", &[colour(target, Load::DontCare)]), |pass| {
        for (pipeline, vertices) in list {
            pass.bind_pipeline(pipeline);
            pass.draw(0..vertices, 0..1);
            drawn += vertices;
        }
    });
    let compiled = graph.compile(decl)?;
    let recorded = graph.record(compiled, ctx)?;
    graph.submit(recorded)?;
    Ok(drawn)
}

/// Passes from other providers reach the graph the same way, in the order the renderer
/// calls them, which is the frame's order.
fn declare_all<'f>(
    providers: &'f [Box<dyn PassProvider>],
    decl: &mut FrameDecl<'f>,
    target: ImageId,
) {
    for provider in providers {
        provider.declare(decl, target);
    }
}

/// A device test's readback (`renderer-impl`): the target copied to a buffer the host
/// reads after the frame's point.
fn renderer_readback(
    graph: &mut RenderGraph,
    ctx: &mut JobContext<'_>,
    renderer: &MinimalRenderer,
    target: ImageHandle,
    readback: BufferHandle,
    pixels: &mut Vec<u8>,
) -> Result<()> {
    let mut decl = FrameDecl::new(1);
    let target = decl.import_image(&ImageImport {
        label: "target",
        image: target,
        mips: 1,
        layers: 1,
        sharing: Sharing::Exclusive,
        initial: on(ImageState::Undefined, GRAPHICS),
        last: on(ImageState::CopySource, GRAPHICS),
        handoff: None,
        ready: None,
    });
    let out = decl.import_buffer(&on_graphics("readback", readback, BufferState::HostRead));
    renderer.declare(&mut decl, target);
    let region =
        BufferImageCopy { rect: Rect { x: 0, y: 0, extent: renderer.extent }, ..WHOLE_SCRATCH };
    decl.copy_pass(
        &CopyPassDecl {
            label: "readback",
            image_sources: &[target],
            image_destinations: &[],
            buffer_sources: &[],
            buffer_destinations: &[out],
        },
        move |copy| copy.copy_image_to_buffer(target, out, &[region]),
    );

    let compiled = graph.compile(decl)?;
    let recorded = graph.record(compiled, ctx)?;
    let done = graph.submit(recorded)?;
    let device = graph.device();
    assert!(device.wait_timeline(done, Duration::from_secs(5))?);
    let len = u64::from(renderer.extent.width * renderer.extent.height * 4);
    device.invalidate_buffer(readback, 0, len)?;
    device.read_buffer(readback, 0, len, &mut |bytes| pixels.extend_from_slice(bytes))
}

// ---- 3. The client's frame (`frame-design`, `frame-impl`) ----

/// What `lactose-app-client` wires. `lactose-app` never names this crate (decision 5.11):
/// it drives the render side through a trait of `frame-design`'s, whose shape is below.
struct ClientRender {
    graph: RenderGraph,
    renderer: MinimalRenderer,
    recorded: Option<RecordedFrame>,
}

trait RenderSide: Send + Sync {
    /// Compile and record, in a job of the frame.
    fn record(
        &mut self,
        ctx: &mut JobContext<'_>,
        target: ImageHandle,
        read_done: Option<TimelinePoint>,
    ) -> Result<()>;

    /// Latch and submit: the frame's point, which the present thread's submit waits for.
    fn latch_and_submit(&mut self, camera: &[u8; 64]) -> Result<TimelinePoint>;
}

impl RenderSide for ClientRender {
    fn record(
        &mut self,
        ctx: &mut JobContext<'_>,
        target: ImageHandle,
        read_done: Option<TimelinePoint>,
    ) -> Result<()> {
        let mut decl = FrameDecl::new(1);
        // One of the offscreen targets of decision 5.6: taken back from `Undefined` once the
        // present thread's last read of it is done, and handed on to the present role.
        let target = decl.import_image(&ImageImport {
            label: "target",
            image: target,
            mips: 1,
            layers: 1,
            sharing: Sharing::Exclusive,
            initial: on(ImageState::Undefined, GRAPHICS),
            last: on(ImageState::ColourAttachment, GRAPHICS),
            handoff: Some(on(ImageState::SampledRead, QueueRole::Present)),
            ready: read_done,
        });
        self.renderer.declare(&mut decl, target);
        let compiled = self.graph.compile(decl)?;
        self.recorded = Some(self.graph.record(compiled, ctx)?);
        Ok(())
    }

    fn latch_and_submit(&mut self, camera: &[u8; 64]) -> Result<TimelinePoint> {
        let recorded = self
            .recorded
            .take()
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "no frame is recorded"))?;
        let device = self.graph.device();
        // One camera buffer is shown. The renderer keeps one for each frame in flight: the
        // host does not write a range a pending submit reads.
        device.write_buffer(self.renderer.camera, 0, 64, &mut |bytes| {
            bytes.copy_from_slice(camera);
        })?;
        device.flush_buffer(self.renderer.camera, 0, 64)?;
        self.graph.submit(recorded)
    }
}

/// The graph and a recorded frame wait in a world's resource between the frame's steps.
fn as_resources(render: ClientRender, recorded: RecordedFrame) -> [Box<dyn Any + Send + Sync>; 2] {
    [Box::new(render), Box::new(recorded)]
}

/// The frame drives the render side through the `dyn` object, from a job.
fn frame(
    side: &mut dyn RenderSide,
    ctx: &mut JobContext<'_>,
    target: ImageHandle,
    read_done: Option<TimelinePoint>,
    camera: &[u8; 64],
) -> Result<TimelinePoint> {
    side.record(ctx, target, read_done)?;
    side.latch_and_submit(camera)
}

/// The present thread's half of the hand-off, on its own encoder, in a submit that waits
/// for the frame's point: the acquire where the present role has a queue of its own and
/// the graph recorded the release, and the whole change where one queue serves both.
fn present_thread_reads(encoder: &mut dyn Encoder, target: ImageHandle) {
    encoder.transition_images(&[ImageTransition {
        image: target,
        range: ImageRange::FIRST,
        from: on(ImageState::ColourAttachment, GRAPHICS),
        to: on(ImageState::SampledRead, QueueRole::Present),
    }]);
}

// ---- 4. The history ping-pong ----

/// A pass that writes this frame's half and samples the last frame's, through the slot its
/// owner keeps for each half. From the second frame on the declaration, and so the plan,
/// is the same every frame.
fn history_frame(
    graph: &mut RenderGraph,
    ctx: &mut JobContext<'_>,
    history: &mut History<(ImageHandle, HeapSlot)>,
    pipeline: GraphicsPipelineHandle,
) -> Result<TimelinePoint> {
    let sampled = on(ImageState::SampledRead, GRAPHICS);
    let half = |label, image, initial| ImageImport {
        label,
        image,
        mips: 1,
        layers: 1,
        sharing: Sharing::Exclusive,
        initial,
        last: sampled,
        handoff: None,
        ready: None,
    };
    let mut decl = FrameDecl::new(1);
    let (write, _) = history.current();
    let current = decl.import_image(&half("history", write, on(ImageState::Undefined, GRAPHICS)));
    let previous = history
        .previous()
        .map(|(image, slot)| (decl.import_image(&half("history, previous", image, sampled)), slot));
    let read: Vec<ImageId> = previous.iter().map(|&(image, _)| image).collect();
    let slots: Vec<HeapSlot> = previous.iter().map(|&(_, slot)| slot).collect();
    decl.render_pass(
        &RenderPassDecl {
            label: "accumulate",
            colour: &[colour(current, Load::DontCare)],
            depth: None,
            sampled: &read,
            buffers: &[],
            slots: &slots,
        },
        move |pass| {
            pass.bind_pipeline(pipeline);
            if let Some((_, slot)) = previous {
                pass.push_slot(0, slot);
            }
            pass.draw(0..3, 0..1);
        },
    );
    let compiled = graph.compile(decl)?;
    let recorded = graph.record(compiled, ctx)?;
    let point = graph.submit(recorded)?;
    // Only a frame that was submitted wrote its half.
    history.advance();
    Ok(point)
}

/// Ids are plain numbers in their declaration, in declaration order.
fn ids_count_in_declaration_order(image: ImageId, buffer: BufferId, pass: PassId) -> [u32; 3] {
    [image.index(), buffer.index(), pass.index()]
}
