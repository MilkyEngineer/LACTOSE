//! Frames through a render graph on null (`README.md`, "A frame, by step", "Regions"): the
//! first frames, with nothing extracted and with every slot freed in a region no writer
//! has used, submitted with nothing refused, so that no frame declares a read of what no
//! submit wrote (null's `undefined-read`); and frames held behind an import's `ready`
//! point, which leave no region free, so that `open` waits for the oldest and a writer that
//! runs anyway drops its rows. What the held frames read is the device tests'.

use std::sync::Arc;

use lactose_core::{Result, Tick};
use lactose_ecs::WorldKind;
use lactose_input_host::LatestSample;
use lactose_math::render::RenderPosition;
use lactose_render_graph::{FrameDecl, GraphDesc, ImageImport, RenderGraph};
use lactose_renderer::{
    CameraLatch, FrameStats, FrameView, Opened, Projection, Renderer, RendererDesc, ScatterEntry,
    Shaders,
};
use lactose_rhi_api::{
    Device, Extent, ImageDesc, ImageHandle, ImageMemory, ImageState, ImageUsage, QueueRole,
    Sharing, StateOn, TimelinePoint,
};
use lactose_task::JobContext;

use crate::common::{PATIENCE, TARGET, executor, in_job, null, null_shaders, ok, row, world_pass};

const EXTENT: Extent = Extent { width: 64, height: 32 };

const PROJECTION: Projection = Projection { fov_y: 1.0, near: 0.1, far: 1000.0 };

/// A renderer on null of `instances` slots and two regions, with null's shaders.
fn renderer(instances: u32, regions: u32) -> Renderer {
    let [scatter, vertex, fragment] = null_shaders();
    let shaders =
        Shaders { scatter: &scatter, vertex: &vertex, fragment: &fragment, target: TARGET };
    let desc = RendererDesc { instances, rows: 4 * instances + 8, regions, shaders: Some(shaders) };
    ok(Renderer::new(null(), &desc))
}

fn target(device: &dyn Device) -> ImageHandle {
    let desc = ImageDesc {
        label: "target",
        format: TARGET,
        extent: EXTENT,
        mips: 1,
        layers: 1,
        usage: ImageUsage::COLOUR.union(ImageUsage::COPY_SOURCE),
        memory: ImageMemory::Own,
        sharing: Sharing::Exclusive,
    };
    device.create_image(&desc).expect("a target")
}

/// What a test hands every frame.
struct Rig {
    renderer: Renderer,
    graph: RenderGraph,
    target: ImageHandle,
    input: LatestSample,
}

impl Rig {
    fn new(instances: u32, regions: u32) -> Self {
        let renderer = renderer(instances, regions);
        let device = Arc::clone(renderer.device());
        let target = target(&*device);
        let graph = RenderGraph::new(device, &GraphDesc { jobs: 2, plans: 4 }).expect("a graph");
        Self { renderer, graph, target, input: LatestSample::new() }
    }

    /// Prepares, declares, records, latches and submits a frame, its target held behind
    /// `held` if `Some`; returns the frame's stats and point. A frame the graph refuses is
    /// ended with `torn` and its error returned.
    fn frame(
        &mut self,
        ctx: &mut JobContext<'_>,
        held: Option<TimelinePoint>,
    ) -> Result<(FrameStats, TimelinePoint)> {
        let stats = self.renderer.prepare()?;
        let recorded = {
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
                ready: held,
            });
            let view = FrameView { target, extent: EXTENT, fraction: 1.0 };
            let declared = self.renderer.declare(&mut decl, &view)?;
            assert_eq!(declared.scatter.is_some(), stats.entries > 0);
            let compiled = self.graph.compile(decl);
            let recorded = compiled.and_then(|compiled| self.graph.record(compiled, ctx));
            match recorded {
                Ok(recorded) => recorded,
                Err(error) => {
                    self.renderer.torn();
                    return Err(error);
                }
            }
        };
        let latch = CameraLatch {
            pivot: RenderPosition::default(),
            input: &self.input,
            boom: 4.0,
            projection: PROJECTION,
        };
        self.renderer.latch(&latch)?;
        match self.graph.submit(recorded) {
            Ok(point) => {
                self.renderer.submitted(point);
                Ok((stats, point))
            }
            Err(error) => {
                self.renderer.torn();
                Err(error)
            }
        }
    }

    fn reached(&self, point: TimelinePoint) {
        let device = self.renderer.device();
        assert!(device.wait_timeline(point, PATIENCE).expect("a wait"), "{point:?} is reached");
    }

    fn open(&mut self) -> Opened {
        ok(self.renderer.open())
    }
}

/// Runs `body` with a fresh rig of `instances` slots in a job of a fresh executor.
fn run(instances: u32, body: impl FnOnce(&mut Rig, &mut JobContext<'_>) + Send + 'static) {
    run_with(instances, 2, body);
}

/// The same, with `regions` regions.
fn run_with(
    instances: u32,
    regions: u32,
    body: impl FnOnce(&mut Rig, &mut JobContext<'_>) + Send + 'static,
) {
    let executor = executor(2);
    in_job(&executor, move |ctx| {
        let mut rig = Rig::new(instances, regions);
        body(&mut rig, ctx);
    });
    executor.shutdown();
}

#[test]
fn a_first_frame_with_nothing_extracted_is_submitted() {
    run(4, |rig, ctx| {
        let (stats, point) = rig.frame(ctx, None).expect("null refuses nothing");
        assert_eq!((stats.entries, stats.live), (0, 0));
        rig.reached(point);
        // And the frame after it, which has nothing again.
        assert_eq!(rig.open(), Opened::Ready);
        let (_, point) = rig.frame(ctx, None).expect("null refuses nothing");
        rig.reached(point);
    });
}

#[test]
fn a_frame_that_frees_every_slot_in_an_unused_region_is_submitted() {
    run_with(4, 3, |rig, ctx| {
        let rows = [row(1, 0.0), row(2, 1.0)];
        world_pass(&rig.renderer, WorldKind::Server, Tick(1), false, &rows);
        let (stats, point) = rig.frame(ctx, None).expect("null refuses nothing");
        assert_eq!(stats.live, 2);
        rig.reached(point);
        // A torn frame in the second region forgets every slot.
        assert_eq!(rig.open(), Opened::Ready);
        world_pass(&rig.renderer, WorldKind::Server, Tick(2), false, &rows);
        ok(rig.renderer.prepare());
        rig.renderer.torn();
        // The third region, which no writer has used: the batch is the run that frees every
        // slot ever given, which reads no row.
        assert_eq!(rig.open(), Opened::Ready);
        let (stats, point) = rig.frame(ctx, None).expect("null refuses nothing");
        assert_eq!((stats.live, stats.entries), (0, 2));
        assert!(
            rig.renderer.batch().entries().iter().all(|entry| entry.flags == ScatterEntry::FREE)
        );
        rig.reached(point);
        // And a frame that frees every slot by a full pass that writes nothing.
        assert_eq!(rig.open(), Opened::Ready);
        world_pass(&rig.renderer, WorldKind::Server, Tick(3), false, &rows);
        let (_, point) = rig.frame(ctx, None).expect("null refuses nothing");
        rig.reached(point);
        assert_eq!(rig.open(), Opened::Ready);
        world_pass(&rig.renderer, WorldKind::Server, Tick(4), true, &[]);
        let (stats, point) = rig.frame(ctx, None).expect("null refuses nothing");
        assert_eq!((stats.freed, stats.live, stats.entries), (2, 0, 2));
        rig.reached(point);
    });
}

#[test]
fn a_frame_after_a_torn_one_is_submitted() {
    run(4, |rig, ctx| {
        world_pass(&rig.renderer, WorldKind::Server, Tick(1), false, &[row(1, 0.0)]);
        ok(rig.renderer.prepare());
        rig.renderer.torn();
        // The torn frame's region is held until a frame is submitted after it.
        assert_eq!(rig.open(), Opened::Ready);
        world_pass(&rig.renderer, WorldKind::Server, Tick(2), true, &[row(1, 0.0)]);
        let (stats, point) = rig.frame(ctx, None).expect("null refuses nothing");
        assert_eq!((stats.allocated, stats.live), (1, 1));
        rig.reached(point);
        assert_eq!(rig.open(), Opened::Ready);
    });
}

#[test]
fn frames_held_in_flight_leave_no_region_to_open_until_the_oldest_point() {
    run(8, |rig, ctx| {
        let device = Arc::clone(rig.renderer.device());
        let held =
            TimelinePoint { timeline: device.create_timeline(0).expect("a timeline"), value: 1 };
        world_pass(&rig.renderer, WorldKind::Server, Tick(1), false, &[row(1, 0.0)]);
        let (_, first) = rig.frame(ctx, Some(held)).expect("null refuses nothing");
        // The other region is free, so the next frame's writers run while the first is held.
        assert_eq!(rig.open(), Opened::Ready);
        world_pass(&rig.renderer, WorldKind::Server, Tick(2), false, &[row(2, 0.0)]);
        let (_, second) = rig.frame(ctx, None).expect("null refuses nothing");
        // Both regions are in flight: no region opens, and a writer that runs anyway drops
        // its rows instead of writing under the first frame.
        assert_eq!(rig.open(), Opened::Wait(first));
        assert_eq!(rig.open(), Opened::Wait(first), "a second call opens nothing either");
        let late = [row(3, 0.0), row(4, 0.0), row(5, 0.0)];
        rig.renderer.remote().write(&late);
        assert!(!device.wait_timeline(first, core::time::Duration::ZERO).expect("a poll"));

        device.signal_timeline(held).expect("the host's point");
        rig.reached(first);
        rig.reached(second);
        assert_eq!(rig.open(), Opened::Ready);
        let stats = ok(rig.renderer.prepare());
        // The late rows found no region, and are counted by the next frame.s `prepare`.
        assert_eq!((stats.dropped, stats.remote_rows), (3, 0));
        assert_eq!(stats.live, 2);
    });
}

#[test]
fn rows_written_with_no_region_open_are_dropped_and_counted() {
    run(8, |rig, ctx| {
        let device = Arc::clone(rig.renderer.device());
        let held =
            TimelinePoint { timeline: device.create_timeline(0).expect("a timeline"), value: 1 };
        let (_, first) = rig.frame(ctx, Some(held)).expect("null refuses nothing");
        assert_eq!(rig.open(), Opened::Ready);
        let (_, _) = rig.frame(ctx, None).expect("null refuses nothing");
        assert_eq!(rig.open(), Opened::Wait(first));
        world_pass(&rig.renderer, WorldKind::Server, Tick(1), false, &[row(1, 0.0), row(2, 0.0)]);
        device.signal_timeline(held).expect("the host's point");
        rig.reached(first);
        assert_eq!(rig.open(), Opened::Ready);
        let stats = ok(rig.renderer.prepare());
        // The pass found no region: the kind is lost, and its rows written nowhere.
        assert!(stats.lost(WorldKind::Server));
        assert_eq!(stats.live, 0);
    });
}
