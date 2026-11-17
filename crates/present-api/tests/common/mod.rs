//! What the integration tests share: a small game (a walker that moves its row's transform
//! inside a tick, a chirper that emits a sound request every tick, a component the snapshot
//! holds), the build of a world of each kind with extract placed, a sink that records every
//! pass and checks the protocol of `InstanceSink`, and a rig that runs frames as `app` does.

#![allow(dead_code, reason = "each test file uses part of the shared module")]

use std::sync::{Arc, Mutex, MutexGuard};

use lactose_asset_api::{AssetId, AssetRef};
use lactose_core::{NetId, PlayerId, StableKey, Tick};
use lactose_ecs::{
    ClaimedBy, Executor, LayoutConfig, Mut, PresentEvents, RegionConfig, RegionId, RegionMode,
    StableKeyOf, World, WorldKind, system,
};
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::{Teleported, Transform};
use lactose_present_api::{
    AudioRequest, ExtractPass, ExtractState, InstanceRow, InstanceSink, MeshInstance,
    SnapshotReader, extract_state, snapshot_channel,
};
use lactose_reflect::component;
use lactose_sim::{Registration, Simulation, SimulationBuilder, Stage, TickInput, TypeTable};

// ---- The game ----

/// Moves the row's `Transform` along x by its value every tick, and writes nothing when it
/// is zero.
#[component(key = "test::walker", persistent)]
#[derive(PartialEq, Debug)]
pub struct Walker(#[field(id = 1)] pub Scalar);

/// Emits one sound request for its row every tick.
#[component(key = "test::chirper", persistent)]
#[derive(PartialEq, Debug)]
pub struct Chirper(#[field(id = 1)] pub u32);

/// A game component the UI shows: `extracted`, so a row with it is a presented row.
#[component(key = "test::shown", persistent, extracted)]
#[derive(PartialEq, Eq, Debug)]
pub struct Shown(#[field(id = 1)] pub u32);

#[system(name = "test::walk", worlds = [server, prediction])]
pub fn walk(walker: &Walker, transform: Mut<Transform>) {
    if walker.0 != Scalar::ZERO {
        transform.translation.x += walker.0;
    }
}

#[system(name = "test::chirp", worlds = [server, prediction])]
pub fn chirp(present: PresentEvents, key: StableKeyOf, _chirper: &Chirper) {
    present.emit(*key, AudioRequest::at(AssetRef::new(AssetId::from_bits(9)), Vec3::ZERO));
}

pub fn types() -> TypeTable {
    let mut types = TypeTable::new();
    lactose_physics_api::register_types(&mut types).expect("physics registers");
    lactose_present_api::register_types(&mut types).expect("present registers");
    types.register::<Walker>().expect("the game registers");
    types.register::<Chirper>().expect("the game registers");
    types.register::<Shown>().expect("the game registers");
    types
}

pub fn register_game(registration: &mut Registration) {
    registration.stage(Stage::Simulation).add(&WALK).add(&CHIRP);
}

// ---- Keys, regions, values ----

pub const REGION: RegionId = RegionId(1);
pub const OTHER_REGION: RegionId = RegionId(2);
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

/// The x of a row's transform.
pub fn x_of(world: &World, key: StableKey) -> Scalar {
    world.get::<Transform>(key).expect("a transform").translation.x
}

/// Sets a row's transform to `x`, a write between ticks.
pub fn place(world: &mut World, key: StableKey, x: f64) {
    world.get_mut::<Transform>(key).expect("a transform").translation.x = scalar(x);
}

/// Sets a row's `Teleported`, inserting it if the row has none.
pub fn teleport(world: &mut World, key: StableKey, tick: Tick) {
    match world.get_mut::<Teleported>(key) {
        Some(mut teleported) => teleported.0 = tick,
        None => world.insert(key, Teleported(tick)).expect("a live row"),
    }
}

// ---- Execution ----

pub fn executor(workers: u32) -> Executor {
    Executor::builder().frame_workers(workers).background_workers(1).build().expect("an executor")
}

/// A world of `kind` as `app` builds it, with the game, extract and both regions.
pub fn simulation(kind: WorldKind, layout: LayoutConfig, tick: Tick) -> Simulation {
    simulation_with(kind, layout, tick, None)
}

/// The same, keeping the structural log for `retention` ticks if `Some`.
pub fn simulation_with(
    kind: WorldKind,
    layout: LayoutConfig,
    tick: Tick,
    retention: Option<u32>,
) -> Simulation {
    let mut builder = SimulationBuilder::new(kind, types())
        .layout(layout)
        .tick(tick)
        .setup(lactose_present_api::setup)
        .systems(lactose_present_api::register_systems)
        .systems(register_game);
    if let Some(retention) = retention {
        builder = builder.log_retention(retention);
    }
    let mut simulation = builder.build().expect("the world builds");
    let mode =
        if kind == WorldKind::Client { RegionMode::Replicated } else { RegionMode::Simulated };
    for (id, seed) in [(REGION, 11), (OTHER_REGION, 12)] {
        simulation.world_mut().create_region(RegionConfig { id, seed, mode }).expect("a region");
    }
    simulation
}

// ---- The recording sink ----

/// What one pass gave the sink: its `begin`, its `removed` if called, and every row written,
/// sorted by key in `end`.
#[derive(Clone, PartialEq, Debug)]
pub struct PassLog {
    pub pass: ExtractPass,
    pub removed: Option<Vec<StableKey>>,
    pub rows: Vec<InstanceRow>,
    /// How many `write` calls the rows came in: layout and executor dependent.
    pub batches: usize,
}

impl PassLog {
    pub fn keys(&self) -> Vec<StableKey> {
        self.rows.iter().map(|row| row.key).collect()
    }

    pub fn row(&self, key: StableKey) -> Option<&InstanceRow> {
        self.rows.iter().find(|row| row.key == key)
    }

    /// Whether every row was written with `no_interpolate` set.
    pub fn all_flagged(&self) -> bool {
        self.rows.iter().all(|row| row.no_interpolate)
    }

    pub fn flagged(&self) -> Vec<StableKey> {
        self.rows.iter().filter(|row| row.no_interpolate).map(|row| row.key).collect()
    }
}

/// The passes a recording sink saw, shared with the test.
#[derive(Clone, Default)]
pub struct Recording(Arc<Mutex<Vec<PassLog>>>);

impl Recording {
    /// A sink that records into this.
    pub fn sink(&self) -> Box<dyn InstanceSink> {
        Box::new(RecordingSink { log: self.clone(), open: None, written: Mutex::default() })
    }

    /// Every pass recorded since the last take.
    pub fn take(&self) -> Vec<PassLog> {
        core::mem::take(&mut *self.lock())
    }

    /// The one pass recorded since the last take, or `None` if no sink method was called;
    /// fails if more than one was.
    pub fn one(&self) -> Option<PassLog> {
        let mut passes = self.take();
        assert!(passes.len() <= 1, "one pass at most: {passes:?}");
        passes.pop()
    }

    fn lock(&self) -> MutexGuard<'_, Vec<PassLog>> {
        self.0.lock().expect("no test panics while holding the log")
    }
}

/// A sink that checks the protocol (`begin`, `removed` at most once and before any row, writes
/// of non-empty batches with each key once, `end`) and records each pass.
struct RecordingSink {
    log: Recording,
    open: Option<PassLog>,
    /// The rows written in the open pass, and in how many batches.
    written: Mutex<(Vec<InstanceRow>, usize)>,
}

impl InstanceSink for RecordingSink {
    fn begin(&mut self, pass: ExtractPass) {
        assert!(self.open.is_none(), "`begin` inside a pass");
        self.open = Some(PassLog { pass, removed: None, rows: Vec::new(), batches: 0 });
    }

    fn removed(&mut self, keys: &[StableKey]) {
        let open = self.open.as_mut().expect("`removed` outside a pass");
        assert!(!open.pass.full, "`removed` on a full pass");
        assert!(open.removed.is_none(), "`removed` twice in a pass");
        assert!(self.written.get_mut().expect("unpoisoned").0.is_empty(), "`removed` after a row");
        assert!(!keys.is_empty(), "`removed` with no key");
        assert!(keys.is_sorted_by(|a, b| a < b), "`removed` ascending, each key once: {keys:?}");
        open.removed = Some(keys.to_vec());
    }

    fn write(&self, rows: &[InstanceRow]) {
        assert!(self.open.is_some(), "`write` outside a pass");
        assert!(!rows.is_empty(), "an empty batch");
        let mut written = self.written.lock().expect("unpoisoned");
        written.0.extend_from_slice(rows);
        written.1 += 1;
    }

    fn end(&mut self) {
        let mut open = self.open.take().expect("`end` outside a pass");
        let (mut rows, batches) = core::mem::take(self.written.get_mut().expect("unpoisoned"));
        open.batches = batches;
        rows.sort_by_key(|row| row.key);
        assert!(rows.windows(2).all(|pair| pair[0].key != pair[1].key), "a row written twice");
        open.rows = rows;
        self.log.lock().push(open);
    }
}

// ---- A world shown as `app` shows it ----

/// A simulation with a recording sink and a snapshot channel installed, run frame by frame
/// as `app` runs it: the frame's steps, `run_passes`, then the events taken and published.
pub struct Rig {
    simulation: Option<Simulation>,
    pub executor: Executor,
    pub recording: Recording,
    pub reader: SnapshotReader,
}

impl Rig {
    pub fn new(simulation: Simulation, executor: Executor) -> Self {
        let mut simulation = simulation;
        let recording = Recording::default();
        let (writer, reader) = snapshot_channel();
        let state = extract_state(&mut simulation).expect("setup ran");
        drop(state.set_sink(Some(recording.sink())));
        drop(state.set_snapshot(Some(writer)));
        Self { simulation: Some(simulation), executor, recording, reader }
    }

    pub fn simulation(&self) -> &Simulation {
        self.simulation.as_ref().expect("between runs")
    }

    pub fn simulation_mut(&mut self) -> &mut Simulation {
        self.simulation.as_mut().expect("between runs")
    }

    pub fn world(&self) -> &World {
        self.simulation().world()
    }

    pub fn world_mut(&mut self) -> &mut World {
        self.simulation_mut().world_mut()
    }

    pub fn state(&mut self) -> &mut ExtractState {
        extract_state(self.simulation_mut()).expect("setup ran")
    }

    pub fn tick(&self) -> Tick {
        self.simulation().tick()
    }

    pub fn hash(&self) -> u64 {
        self.simulation().hash().expect("the world hashes")
    }

    pub fn step(&mut self) {
        let simulation = self.simulation.take().expect("between runs");
        let run = simulation.step(&self.executor, TickInput::default());
        self.simulation = Some(self.executor.block_on(run).expect("the step runs"));
    }

    pub fn passes(&mut self) {
        let simulation = self.simulation.take().expect("between runs");
        let run = simulation.run_passes(&self.executor);
        self.simulation = Some(self.executor.block_on(run).expect("the passes run"));
    }

    /// The frame's events taken and published, as `app` does after the passes.
    pub fn publish(&mut self) {
        let events = self.world_mut().take_present_events();
        self.state().publish(events);
    }

    /// The simulation side of a frame with no step: the passes, then the publication. Returns
    /// what the pass gave the sink, if it called anything.
    pub fn frame(&mut self) -> Option<PassLog> {
        self.passes();
        self.publish();
        self.recording.one()
    }

    /// A frame after one step.
    pub fn tick_frame(&mut self) -> Option<PassLog> {
        self.step();
        self.frame()
    }
}
