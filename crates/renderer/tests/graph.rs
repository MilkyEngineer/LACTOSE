//! The renderer's declaration through a render graph on null, frame by frame (`README.md`,
//! "A frame, by step", "Pipelines and shaders"): the plans a client's steady state takes and
//! the cache size it needs, the frame after `torn`, the scatter's queue where compute has a
//! queue of its own, the flushes a device whose host memory is not coherent needs, and
//! `Renderer::new`'s check of each cooked shader's reflection and of the batch's bound.
//! Adopted from reviewer-gpu's probes of renderer-impl round 1.

use std::sync::Arc;

use lactose_core::{ErrorKind, StableKey, Tick};
use lactose_ecs::WorldKind;
use lactose_input_host::LatestSample;
use lactose_math::render::RenderPosition;
use lactose_present_api::{ExtractPass, InstanceRow, InstanceSink};
use lactose_render_graph::{FrameDecl, GraphDesc, GraphStats, ImageImport, RenderGraph};
use lactose_renderer::{
    CameraLatch, FrameStats, FrameView, Opened, Projection, Renderer, RendererDesc, Shaders,
};
use lactose_rhi_api::{
    Capabilities, ConstantSlot, ConstantType, CookedShader, Extent, ImageDesc, ImageHandle,
    ImageMemory, ImageState, ImageUsage, QueueLayout, QueueRole, ShaderStage, Sharing, StateOn,
    TimelinePoint,
};
use lactose_task::JobContext;

use crate::common::{
    PATIENCE, TARGET, executor, in_job, key, null_of, null_shader, null_shaders, ok, row,
    world_pass,
};

const EXTENT: Extent = Extent { width: 64, height: 32 };

const PROJECTION: Projection = Projection { fov_y: 1.0, near: 0.1, far: 1000.0 };

/// A renderer with null's shaders, a target and a graph of `plans` plans, on a null of
/// `capabilities`.
struct Rig {
    renderer: Renderer,
    graph: RenderGraph,
    target: ImageHandle,
    input: LatestSample,
}

/// What one frame took: its stats, whether its plan was compiled for it, the queue its
/// scatter ran on, and the graph's counts after it.
#[derive(Debug)]
struct Taken {
    name: &'static str,
    stats: FrameStats,
    fresh: bool,
    scatter: Option<QueueRole>,
    graph: GraphStats,
}

impl Rig {
    fn new(capabilities: Capabilities, plans: u32) -> Self {
        let device = null_of(capabilities);
        let [scatter, vertex, fragment] = null_shaders();
        let shaders =
            Shaders { scatter: &scatter, vertex: &vertex, fragment: &fragment, target: TARGET };
        let desc = RendererDesc { instances: 8, rows: 40, regions: 2, shaders: Some(shaders) };
        let renderer = ok(Renderer::new(Arc::clone(&device), &desc));
        let image = ImageDesc {
            label: "target",
            format: TARGET,
            extent: EXTENT,
            mips: 1,
            layers: 1,
            usage: ImageUsage::COLOUR.union(ImageUsage::COPY_SOURCE),
            memory: ImageMemory::Own,
            sharing: Sharing::Exclusive,
        };
        let target = ok(device.create_image(&image));
        let graph = ok(RenderGraph::new(device, &GraphDesc { jobs: 2, plans }));
        Self { renderer, graph, target, input: LatestSample::new() }
    }

    /// Prepares, declares, records, latches and submits a frame, then waits for it and
    /// opens a region.
    fn frame(&mut self, name: &'static str, ctx: &mut JobContext<'_>) -> Taken {
        let stats = ok(self.renderer.prepare());
        let (fresh, scatter, recorded) = {
            let mut decl = FrameDecl::new(1);
            let target = decl.import_image(&ImageImport {
                label: "target",
                image: self.target,
                mips: 1,
                layers: 1,
                sharing: Sharing::Exclusive,
                initial: StateOn { state: ImageState::Undefined, queue: QueueRole::Graphics },
                last: StateOn { state: ImageState::CopySource, queue: QueueRole::Graphics },
                handoff: None,
                ready: None,
            });
            let view = FrameView { target, extent: EXTENT, fraction: 1.0 };
            let declared = ok(self.renderer.declare(&mut decl, &view));
            let compiled = ok(self.graph.compile(decl));
            let plan = compiled.plan();
            let on = |id| plan.passes().iter().find(|pass| pass.pass == id).map(|pass| pass.queue);
            let scatter = declared.scatter.and_then(on);
            (compiled.fresh(), scatter, ok(self.graph.record(compiled, ctx)))
        };
        let latch = CameraLatch {
            pivot: RenderPosition::default(),
            input: &self.input,
            boom: 4.0,
            projection: PROJECTION,
        };
        ok(self.renderer.latch(&latch));
        let point = ok(self.graph.submit(recorded));
        self.renderer.submitted(point);
        self.settle(point);
        Taken { name, stats, fresh, scatter, graph: self.graph.stats() }
    }

    fn settle(&mut self, point: TimelinePoint) {
        let device = Arc::clone(self.renderer.device());
        assert!(ok(device.wait_timeline(point, PATIENCE)), "{point:?} is reached");
        assert_eq!(ok(self.renderer.open()), Opened::Ready);
    }
}

/// The frames a client meets: the first, with nothing; two spawned; three rounds of the
/// steady state, a frame with no tick, a tick that moves an instance and spawns one, and a
/// tick that despawns it alone; a full pass of nothing, which frees every slot, and a frame
/// with the table empty; a spawn; a torn frame, the frame after it and one with no tick.
fn steady_state(capabilities: Capabilities, plans: u32) -> Vec<Taken> {
    let executor = executor(2);
    let taken = in_job(&executor, move |ctx| {
        let mut rig = Rig::new(capabilities, plans);
        let mut taken = Vec::new();
        let mut tick = 0;
        // A frame after a `Server` pass of `rows` that removes `removed`, if any.
        let mut frame =
            |rig: &mut Rig, name, pass: Option<(bool, &[InstanceRow])>, removed: &[StableKey]| {
                tick += 1;
                if let Some((full, rows)) = pass {
                    let mut sink = rig.renderer.sink();
                    sink.begin(ExtractPass { kind: WorldKind::Server, tick: Tick(tick), full });
                    sink.removed(removed);
                    sink.write(rows);
                    sink.end();
                }
                taken.push(rig.frame(name, ctx));
            };
        frame(&mut rig, "first", None, &[]);
        frame(&mut rig, "spawn two", Some((false, &[row(1, 0.0), row(2, 1.0)])), &[]);
        for n in 10..13 {
            frame(&mut rig, "no tick", None, &[]);
            frame(&mut rig, "moved", Some((false, &[row(1, n as f32), row(n, 0.0)])), &[]);
            frame(&mut rig, "despawned", Some((false, &[])), &[key(n)]);
        }
        frame(&mut rig, "every slot freed", Some((true, &[])), &[]);
        frame(&mut rig, "empty table", None, &[]);
        frame(&mut rig, "spawn again", Some((false, &[row(3, 0.0)])), &[]);
        world_pass(&rig.renderer, WorldKind::Server, Tick(100), false, &[row(3, 2.0)]);
        ok(rig.renderer.prepare());
        rig.renderer.torn();
        assert_eq!(ok(rig.renderer.open()), Opened::Ready);
        frame(&mut rig, "after torn", Some((true, &[row(3, 2.0)])), &[]);
        frame(&mut rig, "no tick after torn", None, &[]);
        taken
    });
    executor.shutdown();
    taken
}

fn report(taken: &[Taken]) {
    for t in taken {
        eprintln!(
            "{:20} entries {} live {} fresh {:5} compiles {} hits {} plans {}",
            t.name,
            t.stats.entries,
            t.stats.live,
            t.fresh,
            t.graph.compiles,
            t.graph.hits,
            t.graph.plans
        );
    }
}

/// The steady state alternates three plans, a frame with no tick, a tick with rows and a
/// tick that only despawns, and each compiles once: the first frame, a frame that frees
/// every slot, one with the table empty and the frame after `torn` each take a plan of
/// their own, seven in all. A cache of three holds the alternation; one of two compiles
/// every frame of it (`README.md`, "The scatter").
#[test]
fn the_steady_state_alternates_three_plans_and_each_edge_takes_one() {
    // The second and third rounds of the steady state.
    let steady = 5..11;
    let roomy = steady_state(Capabilities::LEAST, 16);
    report(&roomy);
    let last = &roomy[roomy.len() - 1].graph;
    assert_eq!((last.compiles, last.plans), (7, 7));
    let fresh = |taken: &[Taken]| taken[steady.clone()].iter().filter(|t| t.fresh).count();
    assert_eq!(fresh(&roomy), 0, "the alternation hits after its first round");
    let three = steady_state(Capabilities::LEAST, 3);
    assert_eq!(fresh(&three), 0, "a cache of three holds the alternation");
    let two = steady_state(Capabilities::LEAST, 2);
    report(&two);
    assert_eq!(fresh(&two), steady.len(), "a cache of two holds none of it");
}

/// The frame after `torn` imports the instance buffer from `Undefined`: its declaration is
/// not the steady state's, which the cache holds, so it compiles a plan of its own, and the
/// frame after it hits the steady state's again.
#[test]
fn the_frame_after_torn_imports_the_instances_from_undefined() {
    let taken = steady_state(Capabilities::LEAST, 16);
    let of = |name| taken.iter().find(|t| t.name == name).expect("the frame");
    assert!(!of("moved").fresh, "the steady state's plan is cached");
    assert!(of("after torn").fresh, "the frame after torn hit the steady state's plan");
    assert!(!of("no tick after torn").fresh);
}

/// On a device whose compute role has a queue of its own, every scatter stays on the
/// graphics queue, that of the frame which frees every slot (extent 0, no draw) included.
#[test]
fn every_scatter_stays_on_graphics_under_several_queues() {
    let capabilities = Capabilities { queues: QueueLayout::SEVERAL, ..Capabilities::LEAST };
    let taken = steady_state(capabilities, 16);
    let mut scattered = 0;
    for t in &taken {
        assert_eq!(t.scatter.is_some(), t.stats.entries > 0, "{}", t.name);
        if let Some(queue) = t.scatter {
            assert_eq!(queue, QueueRole::Graphics, "{}", t.name);
            scattered += 1;
        }
    }
    let freed = taken.iter().find(|t| t.name == "every slot freed").expect("the frame");
    assert_eq!((freed.stats.live, freed.scatter), (0, Some(QueueRole::Graphics)));
    assert_eq!(scattered, 10);
}

/// The same frames on a null whose host memory is not coherent, which refuses a submit
/// that reads a range written and not flushed (null's `unflushed`): every buffer a frame
/// writes from the host is flushed after its write.
#[test]
fn frames_on_a_null_whose_host_memory_is_not_coherent_flush_their_writes() {
    let capabilities = Capabilities { coherent_host_memory: false, ..Capabilities::LEAST };
    let taken = steady_state(capabilities, 16);
    assert_eq!(taken.len(), 16);
    assert!(taken.iter().any(|t| t.stats.entries > 0));
}

/// `Renderer::new` holds each cooked shader's reflection to the interface: one push size
/// that differs from the interface's, one workgroup, one constant, one output or input, one
/// stage, and each is refused with `InvalidData`; the fragment push block's other allowed
/// size is accepted. Null's pipelines read no push size, so the push sizes are held by the
/// check alone.
#[test]
fn new_holds_each_shader_reflection() {
    type Change = fn(&mut CookedShader);
    let cases: [(&str, usize, Change, bool); 18] = [
        ("fragment push 32", 2, |s| s.push_bytes = 32, true),
        ("fragment push 16", 2, |s| s.push_bytes = 16, false),
        ("fragment push 64", 2, |s| s.push_bytes = 64, false),
        ("vertex push 0", 1, |s| s.push_bytes = 0, false),
        ("vertex push 64", 1, |s| s.push_bytes = 64, false),
        ("scatter push 0", 0, |s| s.push_bytes = 0, false),
        ("scatter workgroup 32", 0, |s| s.workgroup = Some([32, 1, 1]), false),
        ("scatter workgroup 64x2", 0, |s| s.workgroup = Some([64, 2, 1]), false),
        ("scatter workgroup none", 0, |s| s.workgroup = None, false),
        ("vertex constant int", 1, |s| s.constants[0].ty = ConstantType::Int, false),
        ("vertex constant missing", 1, |s| s.constants.clear(), false),
        (
            "vertex constant extra",
            1,
            |s| s.constants.push(ConstantSlot { id: 1, ty: ConstantType::Float }),
            false,
        ),
        (
            "scatter constant",
            0,
            |s| s.constants.push(ConstantSlot { id: 0, ty: ConstantType::Float }),
            false,
        ),
        ("fragment outputs 0b11", 2, |s| s.outputs = 0b11, false),
        ("fragment outputs 0b10", 2, |s| s.outputs = 0b10, false),
        ("fragment input", 2, |s| s.inputs = 1, false),
        ("vertex as fragment", 2, |s| *s = null_shader(ShaderStage::Vertex), false),
        ("vertex workgroup", 1, |s| s.workgroup = Some([64, 1, 1]), false),
    ];
    let mut wrong = Vec::new();
    for (name, at, change, accepted) in cases {
        let mut shaders = null_shaders();
        change(&mut shaders[at]);
        let [scatter, vertex, fragment] = &shaders;
        let shaders = Shaders { scatter, vertex, fragment, target: TARGET };
        let desc = RendererDesc { instances: 4, rows: 16, regions: 2, shaders: Some(shaders) };
        let made = Renderer::new(null_of(Capabilities::LEAST), &desc);
        let right = match &made {
            Ok(_) => accepted,
            Err(error) => !accepted && error.kind() == ErrorKind::InvalidData,
        };
        if !right {
            wrong.push((name, made.err()));
        }
    }
    assert!(wrong.is_empty(), "{wrong:?}");
}

/// `Renderer::new` refuses a description whose batch bound, `instances + 2 * rows`, is past
/// `u32::MAX`, with `InvalidData` and before anything else that could refuse it: without
/// shaders, where null would otherwise refuse the entries buffer with `ResourceExhausted`, and
/// with a shader whose reflection `new` refuses, which it checks after the bound.
#[test]
fn new_refuses_a_batch_bound_past_u32_max_first() {
    let mut shaders = null_shaders();
    shaders[2].push_bytes = 16;
    let [scatter, vertex, fragment] = &shaders;
    let wrong = Shaders { scatter, vertex, fragment, target: TARGET };
    for shaders in [None, Some(wrong)] {
        let desc = RendererDesc { instances: u32::MAX, rows: 4, regions: 2, shaders };
        let error = Renderer::new(null_of(Capabilities::LEAST), &desc).err().expect("refused");
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{error:?}");
        assert!(error.message().contains("u32::MAX"), "the batch bound refused it: {error:?}");
    }
}
