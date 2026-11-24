//! What the integration tests share: a null device and the cooked shaders null takes for the
//! renderer's interface; keys, rows and passes; the end of a frame with an empty submit; a
//! sink that logs what it hands the renderer's, so that a test knows which row each index
//! of the region holds; the scatter's rule run on the host over a batch; and a world of each
//! kind with extract placed, run frame by frame as `app` runs it.

#![allow(dead_code, reason = "each test file uses part of the shared module")]

use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::ops::Range;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::Duration;

use lactose_asset_api::{AssetId, AssetRef};
use lactose_core::{NetId, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{ClaimedBy, RegionConfig, RegionId, RegionMode, World, WorldKind};
use lactose_math::render::RenderPosition;
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::Transform;
use lactose_present_api::{
    ExtractPass, InstanceRow, InstanceSink, MeshInstance, RenderTransform, SnapshotReader,
    extract_state, snapshot_channel,
};
use lactose_renderer::{
    ExtractSink, InstanceRecord, Opened, Renderer, ScatterBatch, ScatterEntry, StagingRow,
};
use lactose_rhi_api::{
    Capabilities, ConstantSlot, ConstantType, CookedShader, Device, DeviceRequest, Format, Message,
    MessageSink, QueueRole, ShaderStage, ShaderTarget, Submit, TimelineHandle, TimelinePoint,
};
use lactose_rhi_null::NullDevice;
use lactose_sim::{Simulation, SimulationBuilder, TickInput, TypeTable};
use lactose_task::{Executor, GraphBuilder, GraphNode, JobContext};

/// The longest a test waits for a point or its job.
pub const PATIENCE: Duration = Duration::from_secs(20);

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Silent;

impl MessageSink for Silent {
    fn message(&self, _: &Message<'_>) {}
}

/// A null device of the least capabilities.
pub fn null() -> Arc<dyn Device> {
    null_of(Capabilities::LEAST)
}

/// A null device of `capabilities`.
pub fn null_of(capabilities: Capabilities) -> Arc<dyn Device> {
    let request = DeviceRequest {
        adapter: None,
        surface: None,
        heap_slots: 0,
        samplers: 0,
        pipeline_cache: Vec::new(),
        messages: Arc::new(Silent),
    };
    Arc::new(NullDevice::new(request, capabilities).expect("a null device"))
}

/// The colour target's format every test uses.
pub const TARGET: Format = Format::Rgba8Unorm;

/// A cooked shader null takes: the smallest SPIR-V module null reads, with the reflection
/// the renderer's interface asks of `stage`.
pub fn null_shader(stage: ShaderStage) -> CookedShader {
    let (push_bytes, constants, outputs, workgroup) = match stage {
        ShaderStage::Compute => (32, Vec::new(), 0, Some([64, 1, 1])),
        ShaderStage::Vertex => (32, vec![ConstantSlot { id: 0, ty: ConstantType::Float }], 0, None),
        ShaderStage::Fragment => (0, Vec::new(), 1, None),
    };
    CookedShader {
        stage,
        target: ShaderTarget::SpirV,
        entry: "main".to_owned(),
        code: [0x0723_0203_u32, 0x0001_0000, 0, 1, 0]
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect(),
        heap: None,
        constants,
        push_bytes,
        inputs: 0,
        outputs,
        workgroup,
        varyings: Vec::new(),
    }
}

/// The three shaders of the interface, as null takes them.
pub fn null_shaders() -> [CookedShader; 3] {
    [ShaderStage::Compute, ShaderStage::Vertex, ShaderStage::Fragment].map(null_shader)
}

// ---- Keys, rows and passes ----

pub fn key(n: u64) -> StableKey {
    StableKey::Net(NetId::runtime(NonZeroU64::new(n).expect("non-zero")).expect("a key"))
}

/// A placement `x` metres along x from the origin cell's centre.
pub fn placed(x: f32) -> RenderTransform {
    RenderTransform {
        position: RenderPosition { cell: [0; 3], offset: [x, 0.0, 0.0] },
        rotation: [0.0, 0.0, 0.0, 1.0],
    }
}

/// The row of instance `n` at `x`.
pub fn row(n: u64, x: f32) -> InstanceRow {
    InstanceRow {
        key: key(n),
        mesh: AssetRef::new(AssetId::from_bits(n)),
        transform: placed(x),
        no_interpolate: false,
    }
}

/// One pass of a world, written through a sink of the test's own.
pub fn world_pass(
    renderer: &Renderer,
    kind: WorldKind,
    tick: Tick,
    full: bool,
    rows: &[InstanceRow],
) {
    let mut sink = renderer.sink();
    sink.begin(ExtractPass { kind, tick, full });
    sink.write(rows);
    sink.end();
}

/// The timeline a test ends its frames on, and its last value.
pub struct Ender {
    pub timeline: TimelineHandle,
    pub value: u64,
}

impl Ender {
    pub fn new(device: &dyn Device) -> Self {
        Self { timeline: device.create_timeline(0).expect("a timeline"), value: 0 }
    }

    /// Ends the prepared frame with a submit that declares nothing and signals the next
    /// point, and returns the point.
    pub fn end(&mut self, renderer: &mut Renderer) -> TimelinePoint {
        self.value += 1;
        let point = TimelinePoint { timeline: self.timeline, value: self.value };
        let submit = Submit { encoders: &mut [], waits: &[], signals: &[point] };
        renderer.device().submit(QueueRole::Graphics, submit).expect("an empty submit");
        renderer.submitted(point);
        point
    }
}

/// Opens a region for the next frame's writers, waiting for the point that frees one.
pub fn open_waiting(renderer: &mut Renderer) {
    loop {
        match renderer.open().expect("a timeline value") {
            Opened::Ready => return,
            Opened::Wait(point) => {
                assert!(renderer.device().wait_timeline(point, PATIENCE).expect("a wait"));
            }
            Opened::Unsubmitted => panic!("no frame of this test is left torn"),
        }
    }
}

/// Ends the prepared frame and opens the next region.
pub fn next_frame(renderer: &mut Renderer, ender: &mut Ender) {
    ender.end(renderer);
    open_waiting(renderer);
}

// ---- The region's log, and the scatter on the host ----

/// What a region holds at each index, as far as a test's writers told it: the row written
/// there, or `None` for a note. A writer appends under the log's lock with its call to the
/// renderer's writer, so the indices it reserves are the log's.
#[derive(Clone, Default)]
pub struct RegionLog(Arc<Mutex<Vec<Option<InstanceRow>>>>);

impl RegionLog {
    /// What the region holds, and a fresh log for the next one.
    pub fn take(&self) -> Vec<Option<InstanceRow>> {
        core::mem::take(&mut *lock(&self.0))
    }

    /// A sink that logs what it hands `sink`.
    pub fn sink(&self, sink: ExtractSink) -> LoggedSink {
        LoggedSink { sink, log: self.clone() }
    }

    /// A remote batch of `rows` through the renderer's writer, logged.
    pub fn remote(&self, renderer: &mut Renderer, rows: &[InstanceRow]) {
        let mut log = lock(&self.0);
        log.push(None);
        let writer = renderer.remote();
        writer.write(rows);
        log.extend(rows.iter().copied().map(Some));
        drop(writer);
        log.push(None);
    }

    /// One pass through a logged sink of the renderer's.
    pub fn pass(
        &self,
        renderer: &Renderer,
        pass: ExtractPass,
        removed: &[StableKey],
        rows: &[InstanceRow],
    ) {
        let mut sink = self.sink(renderer.sink());
        sink.begin(pass);
        sink.removed(removed);
        sink.write(rows);
        sink.end();
    }
}

/// An `ExtractSink` that logs each call before it returns.
pub struct LoggedSink {
    sink: ExtractSink,
    log: RegionLog,
}

impl InstanceSink for LoggedSink {
    fn begin(&mut self, pass: ExtractPass) {
        let mut log = lock(&self.log.0);
        self.sink.begin(pass);
        log.push(None);
    }

    fn removed(&mut self, keys: &[StableKey]) {
        let mut log = lock(&self.log.0);
        self.sink.removed(keys);
        log.extend(keys.iter().map(|_| None));
    }

    fn write(&self, rows: &[InstanceRow]) {
        let mut log = lock(&self.log.0);
        self.sink.write(rows);
        log.extend(rows.iter().copied().map(Some));
    }

    fn end(&mut self) {
        let mut log = lock(&self.log.0);
        self.sink.end();
        log.push(None);
    }
}

/// The scatter's rule run over `batch` on the host, dispatch by dispatch, each entry
/// reading the row `region` holds at its index: what a device leaves in the instance
/// buffer. `records` grows to the slots the batch names.
pub fn scatter(
    records: &mut Vec<InstanceRecord>,
    batch: ScatterBatch<'_>,
    region: &[Option<InstanceRow>],
) {
    for &range in batch.dispatches() {
        for entry in batch.run(range) {
            let slot = entry.slot.index() as usize;
            if records.len() <= slot {
                records.resize(slot + 1, InstanceRecord::from_bytes(&[0; 128]));
            }
            if entry.flags & ScatterEntry::FREE != 0 {
                records[slot].free();
            } else {
                let row = region[entry.row as usize].expect("an entry reads a row");
                records[slot].scatter(entry.flags, &StagingRow::of(&row));
            }
        }
    }
}

// ---- A world with extract placed ----

pub const REGION: RegionId = RegionId(1);
pub const PLAYER: PlayerId = PlayerId(1);

pub fn executor(workers: u32) -> Executor {
    Executor::builder().frame_workers(workers).background_workers(1).build().expect("an executor")
}

/// A world of `kind` as `app` builds it, with extract placed and one region.
pub fn simulation(kind: WorldKind) -> Simulation {
    let mut types = TypeTable::new();
    lactose_physics_api::register_types(&mut types).expect("physics registers");
    lactose_present_api::register_types(&mut types).expect("present registers");
    let mut simulation = SimulationBuilder::new(kind, types)
        .setup(lactose_present_api::setup)
        .systems(lactose_present_api::register_systems)
        .build()
        .expect("the world builds");
    let mode =
        if kind == WorldKind::Client { RegionMode::Replicated } else { RegionMode::Simulated };
    let region = RegionConfig { id: REGION, seed: 11, mode };
    simulation.world_mut().create_region(region).expect("a region");
    simulation
}

/// The transform at `x` along the simulation's x axis.
pub fn at(x: f64) -> Transform {
    Transform::at(Vec3::new(Scalar::from_f64(x), Scalar::ZERO, Scalar::ZERO))
}

/// Spawns instance `n` at `x`, claimed by the test's player, so that a prediction world's
/// extract writes it too.
pub fn spawn(world: &mut World, n: u64, x: f64) {
    let mesh = MeshInstance::new(AssetRef::new(AssetId::from_bits(n)));
    world.spawn(key(n), REGION, (at(x), mesh, ClaimedBy { player: PLAYER })).expect("a spawn");
}

/// Sets a row's transform to `x`, a write between ticks.
pub fn place(world: &mut World, n: u64, x: f64) {
    world.get_mut::<Transform>(key(n)).expect("a transform").translation.x = Scalar::from_f64(x);
}

/// A simulation with a sink installed, run frame by frame as `app` runs it.
pub struct Rig {
    simulation: Option<Simulation>,
    pub executor: Executor,
    pub reader: SnapshotReader,
}

impl Rig {
    /// The rig of `simulation`, with `sink` and a snapshot channel installed.
    pub fn new(simulation: Simulation, sink: Box<dyn InstanceSink>) -> Self {
        let mut simulation = simulation;
        let (writer, reader) = snapshot_channel();
        let state = extract_state(&mut simulation).expect("setup ran");
        drop(state.set_sink(Some(sink)));
        drop(state.set_snapshot(Some(writer)));
        Self { simulation: Some(simulation), executor: executor(2), reader }
    }

    pub fn simulation(&self) -> &Simulation {
        self.simulation.as_ref().expect("between runs")
    }

    pub fn world(&self) -> &World {
        self.simulation().world()
    }

    pub fn world_mut(&mut self) -> &mut World {
        self.simulation.as_mut().expect("between runs").world_mut()
    }

    /// Installs `sink` in place of the one installed, which makes the next pass full.
    pub fn set_sink(&mut self, sink: Box<dyn InstanceSink>) {
        let simulation = self.simulation.as_mut().expect("between runs");
        drop(extract_state(simulation).expect("setup ran").set_sink(Some(sink)));
    }

    pub fn tick(&self) -> Tick {
        self.simulation().tick()
    }

    pub fn step(&mut self) {
        let simulation = self.simulation.take().expect("between runs");
        let run = simulation.step(&self.executor, TickInput::default());
        self.simulation = Some(self.executor.block_on(run).expect("the step runs"));
    }

    /// The simulation side of a frame: the passes, then the events taken and published.
    pub fn frame(&mut self) {
        let simulation = self.simulation.take().expect("between runs");
        let run = simulation.run_passes(&self.executor);
        let mut simulation = self.executor.block_on(run).expect("the passes run");
        let events = simulation.world_mut().take_present_events();
        extract_state(&mut simulation).expect("setup ran").publish(events);
        self.simulation = Some(simulation);
    }
}

// ---- A job ----

type Body = Box<dyn FnOnce(&mut JobContext<'_>) + Send>;

/// One unit that runs a closure once.
struct Job(Mutex<Option<Body>>);

impl GraphNode for Job {
    fn name(&self) -> &'static str {
        "test::frame"
    }

    fn units(&self) -> u32 {
        1
    }

    fn run(&self, _units: Range<u32>, ctx: &mut JobContext<'_>) {
        let body = lock(&self.0).take().expect("one unit, run once");
        body(ctx);
    }
}

/// Runs `body` as a job of `executor`'s and returns what it returned. A job that has not
/// returned within [`PATIENCE`] aborts the process with a message, so that a wait inside
/// the frame fails the test at the bound rather than at nextest's kill.
pub fn in_job<R: Send + 'static>(
    executor: &Executor,
    body: impl FnOnce(&mut JobContext<'_>) -> R + Send + 'static,
) -> R {
    let (done, returned) = mpsc::channel();
    let body: Body = Box::new(move |ctx| {
        // The receiver is gone only where the test thread has aborted.
        let _ = done.send(body(ctx));
    });
    let mut builder = GraphBuilder::new();
    builder.add_node(Arc::new(Job(Mutex::new(Some(body)))));
    let run = executor.run_graph(builder.build().expect("one node"));
    match returned.recv_timeout(PATIENCE) {
        Ok(out) => {
            executor.block_on(run).expect("the job runs");
            out
        }
        Err(error) => {
            eprintln!("the test's job did not return: {error}");
            std::process::abort();
        }
    }
}

/// The written rows of a pass by key, the later of two rows of one key kept.
pub fn by_key(rows: &[InstanceRow]) -> BTreeMap<StableKey, InstanceRow> {
    rows.iter().map(|row| (row.key, *row)).collect()
}

/// Checks a `Result` the test expects to be `Ok`.
pub fn ok<T>(result: Result<T>) -> T {
    result.expect("the call succeeds")
}
