//! Compile-only consumer sketches: what a game crate, `app`, the renderer, the
//! presentation schedule, `game-present`, the UI and the debug overlay write against this
//! crate, type-checked and never run (every deciding body is `todo!()`). They pin the
//! surface the README promises: a change that breaks one is a signature change to list.

#![allow(dead_code, unused_variables, missing_docs)]

use std::sync::{Arc, Mutex};

use lactose_asset_api::{AssetId, AssetRef};
use lactose_core::{FieldId, Result, StableKey};
use lactose_ecs::{
    Executor, Job, PresentEvent, PresentEvents, RegionId, ResMut, StableKeyOf, World, WorldKind,
    system,
};
use lactose_math::render::RenderPosition;
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::{Transform, Velocity};
use lactose_present_api::{
    AudioRequest, CameraRig, DebugDraw, DebugShape, Decal, EffectAsset, ExtractPass, ExtractState,
    InstanceRow, InstanceSink, Light, MaterialAsset, MeshAsset, MeshInstance, PresentFrame,
    RenderTransform, SnapshotReader, SoundAsset, VfxRequest, ViewModels, snapshot_channel,
};
use lactose_reflect::{Reflect, component, resource};
use lactose_sim::{Pass, Registration, Simulation, SimulationBuilder, Stage, TypeTable};

// ---- 1. A game crate: what it authors, and what its kernels ask of presentation ----

const CRATE_MESH: AssetRef<MeshAsset> = AssetRef::new(AssetId::from_bits(0x10));
const SCORCH: AssetRef<MaterialAsset> = AssetRef::new(AssetId::from_bits(0x11));
const FOOTSTEP: AssetRef<SoundAsset> = AssetRef::new(AssetId::from_bits(0x12));
const SPARKS: AssetRef<EffectAsset> = AssetRef::new(AssetId::from_bits(0x13));

/// A game component the UI shows: `extracted`, so a row that has it is a presented row and
/// the snapshot holds it.
#[component(key = "sketch::health", persistent, replicated, extracted)]
pub struct Health(#[field(id = 1)] pub u32);

/// Section 6.1's `footsteps`, as the proposed amendment writes it: the request says where
/// the sound is, so presentation needs no row of the walker.
#[system(name = "sketch::footsteps", worlds = [server, prediction])]
fn footsteps(present: PresentEvents, key: StableKeyOf, at: &Transform, vel: &Velocity) {
    if vel.linear.length() > Scalar::ONE {
        present.emit(*key, AudioRequest::at(FOOTSTEP, at.translation));
    }
}

/// An effect, and a shape for the overlay that lasts until the next tick is shown.
#[system(name = "sketch::impacts", worlds = [server, prediction])]
fn impacts(present: PresentEvents, key: StableKeyOf, at: &Transform, health: &Health) {
    if health.0 == 0 {
        present.emit(*key, VfxRequest::at(SPARKS, at.translation, at.rotation));
        present.emit(*key, DebugShape::sphere(at.translation, Scalar::ONE, 0xff00_00ff));
    }
}

/// What a server spawn or a cooked baseline puts on entities: a player the camera follows,
/// a lamp, a mark on the floor. All four components go through the ordinary bundle path.
fn author(world: &mut World, region: RegionId, keys: [StableKey; 3]) -> Result<()> {
    let [player, lamp, mark] = keys;
    let eye = Vec3::new(Scalar::ZERO, Scalar::from_f64(1.5), Scalar::ZERO);
    world.spawn(
        player,
        region,
        (
            Transform::IDENTITY,
            Velocity::ZERO,
            MeshInstance::new(CRATE_MESH),
            CameraRig::new(eye, Scalar::from_f64(4.0)),
            Health(100),
        ),
    )?;
    let warm = Vec3::new(Scalar::ONE, Scalar::from_f64(0.8), Scalar::from_f64(0.6));
    let lamp_light = Light::point(warm, Scalar::from_f64(800.0), Scalar::from_f64(12.0));
    world.spawn(lamp, region, (Transform::at(eye), lamp_light))?;
    world.spawn(mark, region, (Transform::IDENTITY, Decal::new(SCORCH, Vec3::ONE)))
}

fn register_game(registration: &mut Registration) {
    registration.stage(Stage::Post).add(&FOOTSTEPS).add(&IMPACTS);
}

// ---- 2. `app`: the build of a world of any kind, the outputs, the frame ----

fn types() -> Result<TypeTable> {
    let mut types = TypeTable::new();
    lactose_physics_api::register_types(&mut types)?;
    lactose_present_api::register_types(&mut types)?;
    types.register::<Health>()?;
    Ok(types)
}

/// The same three calls for every kind: a dedicated server installs no output and its
/// passes do nothing.
fn build(kind: WorldKind) -> Result<Simulation> {
    SimulationBuilder::new(kind, types()?)
        .setup(lactose_present_api::setup)
        .systems(lactose_present_api::register_systems)
        .systems(register_game)
        .build()
}

/// A world that is shown gets a sink from the renderer and the writing half of a channel
/// whose reading half goes to the presentation schedule.
fn show(simulation: &mut Simulation, sink: Box<dyn InstanceSink>) -> SnapshotReader {
    let (writer, reader) = snapshot_channel();
    let extract = extract_state(simulation);
    let replaced_sink = extract.set_sink(Some(sink));
    let replaced_writer = extract.set_snapshot(Some(writer));
    reader
}

fn extract_state(simulation: &mut Simulation) -> &mut ExtractState {
    simulation.world_mut().service_mut::<ExtractState>().expect("`setup` inserted it")
}

/// Once per frame, after the frame's ticks: the passes run extract, then the events of
/// those ticks and the snapshot extract wrote are published in one call.
fn frame(executor: &Executor, simulation: Simulation) -> Simulation {
    let mut simulation = executor.block_on(simulation.run_passes(executor)).expect("the passes");
    let events = simulation.world_mut().take_present_events();
    extract_state(&mut simulation).publish(events);
    simulation
}

/// A world that stops being shown gives its outputs back.
fn hide(simulation: &mut Simulation) {
    let extract = extract_state(simulation);
    let sink = extract.set_sink(None);
    let writer = extract.set_snapshot(None);
}

// ---- 3. The renderer, and a test that counts: a sink ----

/// What a sink that counts keeps, shared with the test that reads it.
#[derive(Default)]
struct Counted {
    passes: Vec<ExtractPass>,
    removed: Vec<StableKey>,
    rows: Vec<InstanceRow>,
}

/// A sink over shared state: the renderer's is a handle on its staging ring and slot table
/// in the same way, one handle per world it is installed in.
struct CountingSink {
    counted: Arc<Mutex<Counted>>,
    /// Rows arrive from several jobs at once, through `&self`.
    staged: Mutex<Vec<InstanceRow>>,
}

impl InstanceSink for CountingSink {
    fn begin(&mut self, pass: ExtractPass) {
        // `pass.kind` tells a client's two worlds apart; on a full pass, whatever this
        // sink holds for that kind and is not written before `end` is gone.
        self.counted.lock().expect("no holder panicked").passes.push(pass);
    }

    fn removed(&mut self, keys: &[StableKey]) {
        self.counted.lock().expect("no holder panicked").removed.extend_from_slice(keys);
    }

    fn write(&self, rows: &[InstanceRow]) {
        self.staged.lock().expect("no holder panicked").extend_from_slice(rows);
    }

    fn end(&mut self) {
        // Batches arrive in no meaningful order: what is kept is ordered by key here.
        let staged = self.staged.get_mut().expect("no holder panicked");
        staged.sort_by_key(|row| row.key);
        self.counted.lock().expect("no holder panicked").rows.append(staged);
    }
}

/// The remote upload of the presentation schedule produces the same row, outside any
/// pass and through the renderer's own writer, never through a sink: sampled, already
/// interpolated, so `no_interpolate`.
fn remote_row(key: StableKey, mesh: MeshInstance, sampled: &Transform) -> InstanceRow {
    InstanceRow {
        key,
        mesh: mesh.mesh,
        transform: RenderTransform::from_sim(sampled),
        no_interpolate: true,
    }
}

/// A pass system of a test's own, in place of the crate's: the same call.
#[system(name = "sketch::extract_twice", worlds = [server], exclusive)]
fn extract_twice(world: &mut World, job: Job, state: ResMut<ExtractState>) {
    let world: &World = world;
    state.extract(world, &mut job);
    state.extract(world, &mut job);
}

fn register_test_pass(registration: &mut Registration) {
    registration.pass(Pass::Extract).add(&EXTRACT_TWICE);
}

// ---- 4. The presentation schedule: a frame from the snapshot ----

/// The camera pivot, lights and decals, and the frame's events. `alpha` is how far the
/// frame is between the pair's two ticks, which is presentation's to compute.
fn present(reader: &mut SnapshotReader, local: StableKey, alpha: Scalar, debug: &mut DebugDraw) {
    let Some(frame) = reader.acquire() else {
        return;
    };
    let (older, newer) = (frame.older(), frame.newer());
    let span = (older.tick(), newer.tick(), newer.kind(), newer.cut());

    // The camera: the rig beside the row's two transforms.
    if let (Some(pair), Some(rig)) = (frame.transform(local), newer.get::<CameraRig>(local)) {
        let shown = if pair.no_interpolate {
            pair.current
        } else {
            Transform::new(
                pair.previous.translation.lerp(pair.current.translation, alpha),
                pair.previous.rotation.slerp(pair.current.rotation, alpha),
            )
        };
        let pivot = RenderPosition::from_sim(rig.pivot_of(&shown));
        let rest_length = rig.boom;
    }

    // Every presented row of a kind, by key, with the transform it is placed by.
    for (key, light) in newer.rows::<Light>() {
        let placed = frame.transform(key).map(|pair| RenderTransform::from_sim(&pair.current));
    }
    for (key, decal) in newer.rows::<Decal>() {
        let placed = frame.transform(key);
    }
    let lights = newer.rows::<Light>().len();

    // The events of the ticks since the previous frame, each once.
    for emission in &frame.events().emitted {
        let about = emission.key.origin;
        if let Some(audio) = emission.get::<AudioRequest>() {
            let at = RenderPosition::from_sim(audio.position);
        } else if let Some(vfx) = emission.get::<VfxRequest>() {
            let at = RenderPosition::from_sim(vfx.position);
        } else if let Some(shape) = emission.get::<DebugShape>() {
            debug.shape(&shape);
        }
    }
    for cancelled in &frame.events().cancelled {
        let stop = cancelled.kind == AudioRequest::KIND;
    }
}

// ---- 5. `game-present` and the UI: a view-model ----

/// A view-model: `derived`, so it may hold a raw float and no lint file is needed to
/// declare it.
#[resource(key = "sketch::health_bar", derived)]
pub struct HealthBar {
    #[field(id = 1)]
    pub fraction: f32,
    #[field(id = 2)]
    pub shown: u32,
}

/// `game-present`, every frame: from an extracted game component to what the UI shows.
fn fill(frame: &PresentFrame<'_>, local: StableKey, views: &mut ViewModels) {
    match frame.newer().get::<Health>(local) {
        Some(health) => {
            let shown = health.0.min(100);
            let previous = views.set(HealthBar { fraction: 0.5, shown });
        }
        None => {
            let removed = views.remove::<HealthBar>();
        }
    }
}

/// The UI: by type where it knows the type, by key and field ID where a cooked layout
/// names them.
fn bind(views: &ViewModels) -> Result<()> {
    let typed: Option<HealthBar> = views.get::<HealthBar>();
    if let Some(view) = views.get_erased(HealthBar::KEY) {
        let name = view.info().name();
        let fraction: &[u8] = view.info().field_bytes(view.bytes(), FieldId(1))?;
    }
    for view in views {
        let key = view.info().key();
    }
    Ok(())
}

// ---- 6. The debug overlay: the frame's segments ----

fn overlay(debug: &mut DebugDraw) {
    let origin = RenderPosition::from_sim(Vec3::ZERO);
    debug.line(origin, RenderPosition::from_sim(Vec3::ONE), 0x00ff_00ff);
    debug.shape(&DebugShape::aabb(Vec3::ZERO, Vec3::ONE, 0xffff_00ff));
    debug.shape(&DebugShape::line(Vec3::ZERO, Vec3::ONE, 0xffff_ffff));
    for line in debug.lines() {
        let segment = (line.from, line.to, line.color);
    }
    debug.clear();
}
