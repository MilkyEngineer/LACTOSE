//! What the integration tests share: worlds of each kind built as `app` builds them, with
//! extract placed and a snapshot channel whose reader a `Presentation` holds; a system that
//! emits a sound request for every row with a mesh, every tick; an event sink that records;
//! and a job to run the upload from.

#![allow(dead_code, reason = "each test file uses part of the shared module")]

use std::ops::Range;
use std::sync::{Arc, Mutex};

use lactose_asset_api::{AssetId, AssetRef};
use lactose_core::{NetId, PlayerId, StableKey, Tick};
use lactose_ecs::{
    ClaimedBy, LayoutConfig, PresentEmission, PresentEventKey, PresentEvents, RegionConfig,
    RegionId, RegionMode, StableKeyOf, World, WorldKind, system,
};
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::Transform;
use lactose_present_api::{
    AudioRequest, MeshInstance, PresentFrame, extract_state, snapshot_channel,
};
use lactose_presentation::{EventSink, PresentView, Presentation, Shown};
use lactose_sim::{Registration, Simulation, SimulationBuilder, Stage, TickInput, TypeTable};
use lactose_task::{Executor, GraphBuilder, GraphNode, JobContext};

// ---- The game ----

/// Emits one sound request for its row every tick: what a rollback re-emits.
#[system(name = "test::chirp", worlds = [server, prediction])]
pub fn chirp(present: PresentEvents, key: StableKeyOf, _mesh: &MeshInstance) {
    present.emit(*key, AudioRequest::at(AssetRef::new(AssetId::from_bits(9)), Vec3::ZERO));
}

pub fn types() -> TypeTable {
    let mut types = TypeTable::new();
    lactose_physics_api::register_types(&mut types).expect("physics registers");
    lactose_present_api::register_types(&mut types).expect("present registers");
    lactose_net_api::register_types(&mut types).expect("net registers");
    types
}

fn register_game(registration: &mut Registration) {
    registration.stage(Stage::Simulation).add(&CHIRP);
}

// ---- Keys and values ----

pub const REGION: RegionId = RegionId(1);
pub const PLAYER: PlayerId = PlayerId(1);

pub fn key(n: u64) -> StableKey {
    StableKey::Net(NetId::runtime(core::num::NonZeroU64::new(n).expect("non-zero")).expect("a key"))
}

pub fn scalar(value: f64) -> Scalar {
    Scalar::from_f64(value)
}

pub fn at(x: f64) -> Transform {
    Transform::at(Vec3::new(scalar(x), Scalar::ZERO, Scalar::ZERO))
}

pub fn mesh(n: u64) -> MeshInstance {
    MeshInstance::new(AssetRef::new(AssetId::from_bits(n)))
}

pub fn claim() -> ClaimedBy {
    ClaimedBy { player: PLAYER }
}

pub fn executor(workers: u32) -> Executor {
    Executor::builder().frame_workers(workers).background_workers(1).build().expect("an executor")
}

// ---- A world shown as `app` shows it ----

/// A world of one kind, with extract placed and a snapshot channel installed, run frame by
/// frame as `app` runs it, and the presentation schedule on the channel's reader.
pub struct Scene {
    simulation: Option<Simulation>,
    pub executor: Executor,
    pub presentation: Presentation,
}

impl Scene {
    /// A world of `kind` at `tick`, with one region: simulated, or replicated for `Client`.
    pub fn new(kind: WorldKind, tick: Tick, workers: u32) -> Self {
        let mut simulation = SimulationBuilder::new(kind, types())
            .layout(LayoutConfig::default())
            .tick(tick)
            .setup(lactose_present_api::setup)
            .systems(lactose_present_api::register_systems)
            .systems(register_game)
            .build()
            .expect("the world builds");
        let mode =
            if kind == WorldKind::Client { RegionMode::Replicated } else { RegionMode::Simulated };
        let config = RegionConfig { id: REGION, seed: 11, mode };
        simulation.world_mut().create_region(config).expect("a region");
        let (writer, reader) = snapshot_channel();
        let state = extract_state(&mut simulation).expect("setup ran");
        drop(state.set_snapshot(Some(writer)));
        Self {
            simulation: Some(simulation),
            executor: executor(workers),
            presentation: Presentation::new(reader),
        }
    }

    pub fn world(&self) -> &World {
        self.simulation.as_ref().expect("between runs").world()
    }

    pub fn world_mut(&mut self) -> &mut World {
        self.simulation.as_mut().expect("between runs").world_mut()
    }

    pub fn tick(&self) -> Tick {
        self.simulation.as_ref().expect("between runs").tick()
    }

    pub fn step(&mut self) {
        let simulation = self.simulation.take().expect("between runs");
        let run = simulation.step(&self.executor, TickInput::default());
        self.simulation = Some(self.executor.block_on(run).expect("the step runs"));
    }

    /// The simulation side of a frame: the passes, then the frame's events taken and
    /// published, as `app` does. Returns the keys published: emitted, then cancelled.
    pub fn publish(&mut self) -> (Vec<PresentEventKey>, Vec<PresentEventKey>) {
        let simulation = self.simulation.take().expect("between runs");
        let run = simulation.run_passes(&self.executor);
        let mut simulation = self.executor.block_on(run).expect("the passes run");
        let events = simulation.world_mut().take_present_events();
        let emitted = events.emitted.iter().map(|emission| emission.key).collect();
        let cancelled = events.cancelled.clone();
        extract_state(&mut simulation).expect("setup ran").publish(events);
        self.simulation = Some(simulation);
        (emitted, cancelled)
    }

    /// A step, then the publication.
    pub fn tick_publish(&mut self) {
        self.step();
        self.publish();
    }

    /// The schedule's frame at `view`, with the events into `events`.
    pub fn frame<'s>(
        &'s mut self,
        view: &PresentView<'_>,
        events: &mut dyn EventSink,
    ) -> Option<Shown<'s>> {
        self.presentation.frame(view, events)
    }
}

/// A view with no focus and no remote rows, at `fraction`.
pub fn view(fraction: f32) -> PresentView<'static> {
    PresentView { fraction, focus: None, remote: None }
}

// ---- The sinks ----

/// Every event the schedule delivered, in delivery order, with the frame it came in.
#[derive(Default, Debug)]
pub struct Delivered {
    pub frame: u32,
    pub emitted: Vec<(u32, PresentEventKey)>,
    pub cancelled: Vec<(u32, PresentEventKey)>,
}

impl EventSink for Delivered {
    fn emitted(&mut self, emission: &PresentEmission) {
        assert!(emission.get::<AudioRequest>().is_some(), "the chirp's request");
        self.emitted.push((self.frame, emission.key));
    }

    fn cancelled(&mut self, key: PresentEventKey) {
        self.cancelled.push((self.frame, key));
    }
}

/// A sink for a frame whose events the test does not look at.
pub struct Ignored;

impl EventSink for Ignored {
    fn emitted(&mut self, _emission: &PresentEmission) {}

    fn cancelled(&mut self, _key: PresentEventKey) {}
}

/// The x of the row's transform the frame shows at `fraction`.
pub fn shown_x(frame: &PresentFrame<'_>, key: StableKey, fraction: f32) -> f64 {
    let shown = lactose_presentation::interpolate(frame, key, fraction).expect("a transform");
    shown.translation.x.to_f64()
}

// ---- A job ----

type Body = Box<dyn FnOnce(&mut JobContext<'_>) + Send>;

/// One unit that runs a closure once.
struct Job(Mutex<Option<Body>>);

impl GraphNode for Job {
    fn name(&self) -> &'static str {
        "test::job"
    }

    fn units(&self) -> u32 {
        1
    }

    fn run(&self, _units: Range<u32>, ctx: &mut JobContext<'_>) {
        let body = self.0.lock().expect("unpoisoned").take().expect("one unit, run once");
        body(ctx);
    }
}

/// Runs `body` as a job of `executor`'s, as the frame runs the schedule's upload, and returns
/// once it and every job it forked have finished.
pub fn in_job(executor: &Executor, body: impl FnOnce(&mut JobContext<'_>) + Send + 'static) {
    let mut builder = GraphBuilder::new();
    builder.add_node(Arc::new(Job(Mutex::new(Some(Box::new(body))))));
    let graph = builder.build().expect("one node");
    executor.block_on(executor.run_graph(graph)).expect("the job runs");
}
