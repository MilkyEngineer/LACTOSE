//! What the headless binary and a SOUR test of the assembly write against this surface,
//! and the headless app's rows of the milestone 1 proof (`README.md`): a game crate's
//! registration, the binary's `main` (which `crates/app-headless` is, over the same sketch
//! game), the SOUR test that records a run under the runner, a save round trip, and the
//! tests of section 5. A change that breaks a sketch is a signature change to list.

#![forbid(unsafe_code)]
#![allow(missing_docs)]

#[path = "../../../tools/xtask/src/harness/protocol.rs"]
#[expect(
    clippy::wrong_self_convention,
    reason = "the runner's module as written; it is included, not owned, here"
)]
#[allow(
    unfulfilled_lint_expectations,
    reason = "the boundary lints these expectations name are not configured outside the boundary"
)]
#[allow(dead_code, reason = "the runner's module as written; a test uses what it needs")]
mod protocol;

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use lactose_app::{
    CATCH_UP_CAP, Cooked, CrateRegistration, ErrorPolicy, Game, Headless, HeadlessConfig,
    LOG_RETENTION, LOG_TARGET, LifecycleFailures, Response, RunReport, Server, Source, Start,
    StepOutput, TICK_PERIOD, Workers, WorldConfig, build_server, load,
};
use lactose_asset_api::{
    Asset, AssetId, AssetKind, AssetSet, AssetVersion, LoadPriority, PinnedTable, PublishedAssets,
    Readiness, UntypedHandle, VersionTable,
};
use lactose_bundle::Baseline;
use lactose_core::{Error, ErrorKind, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{ClaimedBy, LayoutConfig, Mut, RegionId, RegionMode, World, system};
use lactose_input::{AXIS_SCALE, Axis2, PlayerInput, Yaw, player_entity};
use lactose_math::{Scalar, Vec3};
use lactose_persist::Migrations;
use lactose_physics_api::{Collider, RigidBody, Shape, Transform, Velocity};
use lactose_reflect::component;
use lactose_sim::harness::{ChunkCapacity, layout_mode, layout_mode_of};
use lactose_sim::{
    PlayerSample, RegionBundle, RegionControl, Registration, SessionApplier, SessionEvent, Stage,
    StreamingAnchor, TickInput, TypeTable,
};
use lactose_task::{ChaosConfig, Executor};
use protocol::{Capacity, Layout, RunConfig, Session};

// ---- 1. A game crate's registration (architecture section 18) ----

/// Counts ticks; replicated, so a prediction system may read it.
#[component(key = "sketch::counter", persistent, replicated)]
pub struct Counter(#[field(id = 1)] pub u32);

#[system(name = "sketch::count", worlds = [server, prediction])]
fn count(counter: Mut<Counter>) {
    counter.0 = counter.0.wrapping_add(1);
}

/// The crate's generated `register_types`.
pub fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<Counter>()
}

/// The crate's generated `register_systems`.
pub fn register_systems(registration: &mut Registration) {
    registration.stage(Stage::Simulation).add(&COUNT);
}

/// The crate's `register_migrations`: no bump yet.
pub fn register_migrations(_: &mut Migrations) -> Result<()> {
    Ok(())
}

/// What the crate exposes: one `static`, function pointers only, nothing built before
/// `main`. Its `setup` would insert the crate's services; it has none, so `None`.
pub static SKETCH_SIM: CrateRegistration = CrateRegistration {
    name: "sketch-sim",
    types: register_types,
    setup: None,
    systems: register_systems,
    migrations: Some(register_migrations),
};

/// What the binary hands the assembly.
pub static GAME: Game =
    Game { name: "sketch", crates: &[SKETCH_SIM], order: None, sessions: Some(sessions) };

/// The game's session applier: a join spawns the player's counter, claimed; a leave keeps
/// it, so the sketch's counter outlives its player.
fn sessions() -> Box<dyn SessionApplier> {
    Box::new(Sessions)
}

struct Sessions;

impl SessionApplier for Sessions {
    fn apply(&mut self, world: &mut World, player: PlayerId, event: SessionEvent) -> Result<()> {
        match event {
            SessionEvent::Join { region } => {
                let key = StableKey::Net(world.allocate_net_id()?);
                world.spawn(key, region, (Counter(0), ClaimedBy { player }))
            }
            SessionEvent::Leave => Ok(()),
        }
    }
}

// ---- 2. The cooked content a headless run holds in memory ----

struct Store;

struct Blob;

impl Asset for Blob {
    const KIND: AssetKind = AssetKind::from_name("sketch::blob");
    const FORMAT_VERSION: u32 = 1;
}

impl PublishedAssets for Store {
    fn request(&self, _: AssetId, _: AssetVersion, _: LoadPriority) {}

    fn get(&self, id: AssetId, version: AssetVersion) -> Readiness<UntypedHandle> {
        Readiness::Ready(UntypedHandle::new(id, version, Blob))
    }
}

/// The version table of `region`'s one asset.
fn table_of(region: RegionId) -> VersionTable {
    [(AssetId::from_bits(region.0), AssetVersion::from_bits(1).expect("non-zero"))]
        .into_iter()
        .collect()
}

/// One region's cooked content: an empty baseline and one asset, built in memory. What
/// the asset store replaces at milestone 3.
struct InMemory {
    region: RegionId,
    baseline: Arc<Baseline>,
}

impl InMemory {
    fn new(region: RegionId) -> Result<Self> {
        let baseline = Arc::new(Baseline::from_rows(AssetSet::new(), [], [])?);
        Ok(Self { region, baseline })
    }

    /// The region's bundle in its cooked state, for the first frame.
    fn fresh(&self) -> Result<RegionBundle> {
        Ok(RegionBundle {
            seed: self.region.0,
            mode: RegionMode::Simulated,
            baseline: Arc::clone(&self.baseline),
            overlay: None,
            resources: Vec::new(),
            transient_resources: Vec::new(),
            assets: Some(self.pin(&table_of(self.region))?),
        })
    }
}

impl Cooked for InMemory {
    fn baseline(&self, content_hash: u64) -> Result<Arc<Baseline>> {
        if self.baseline.content_hash() == content_hash {
            Ok(Arc::clone(&self.baseline))
        } else {
            Err(Error::new(ErrorKind::NotFound, "no cooked baseline has that hash"))
        }
    }

    fn table(&self, region: RegionId) -> Option<VersionTable> {
        (region == self.region).then(|| table_of(region))
    }

    fn pin(&self, table: &VersionTable) -> Result<PinnedTable> {
        PinnedTable::pin(&Store, table)
            .ready()
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "the store has not published it"))
    }
}

/// The frames a headless run feeds: the region's bundle and player 1's join into it in the
/// first, nothing after. On a `Start::Loaded` run the first frame's entry meets the saved
/// region in the server's store: the saved state loads, and this entry lends it the mode
/// and the pinned table (`README.md`, "Saves and loads").
fn frames(cooked: Arc<InMemory>) -> Result<Box<dyn FnMut(Tick) -> TickInput + Send>> {
    let mut first = Some(cooked.fresh()?);
    let region = cooked.region;
    Ok(Box::new(move |_| {
        let mut input = TickInput::default();
        if let Some(bundle) = first.take() {
            input.loaded.insert(region, bundle);
            input.sessions.insert(PlayerId(1), SessionEvent::Join { region });
        }
        input
    }))
}

// ---- 3. `main` of `app-headless` ----

/// The binary's `main`, after it installs the crash hook and the stderr subscriber: the
/// executor's defaults, the headless world, `ticks` ticks at tick rate from a spawned timer
/// root, the report on stdout, an optional save, an orderly shutdown. `main` returns the
/// first error, which is the policy's `Stop`; a failed save is logged, as the policy says,
/// the run still reports, and the exit status is 2.
fn headless_main(ticks: u32, save_to: Option<PathBuf>) -> Result<ExitCode> {
    let cooked = Arc::new(InMemory::new(RegionId(1))?);
    let config = HeadlessConfig {
        workers: Workers::default(),
        world: WorldConfig::headless(),
        start: Start::Fresh,
        saves: save_to.as_deref().and_then(Path::parent).map(Path::to_path_buf),
    };
    let mut app = Headless::start(&GAME, config, Arc::clone(&cooked) as Arc<dyn Cooked>)?;
    let report: RunReport = app.run(ticks, frames(cooked)?)?;
    println!(
        "{} ticks in {} frames ({} behind), hash {:#018x}",
        report.ticks, report.frames, report.behind, report.hash
    );
    let mut exit = ExitCode::SUCCESS;
    if let Some(slot) = save_to {
        match app.save(slot.clone()) {
            Ok(()) => println!("saved to {}", slot.display()),
            // `Headless::save` logged it as the policy decides: `Log`, and the run reports.
            Err(error) => {
                eprintln!("the save failed: {error}");
                exit = ExitCode::from(2);
            }
        }
    }
    let _log = app.shutdown();
    Ok(exit)
}

/// The same binary resuming from a slot: the save decoded before any world exists, its
/// tick and regions into the server's store through `Start::Loaded`, from which the first
/// step loads every one under `Directed`.
fn resume_main(slot: &[u8], ticks: u32) -> Result<RunReport> {
    let cooked = Arc::new(InMemory::new(RegionId(1))?);
    let saved = load(&GAME, slot, &*cooked)?;
    let config = HeadlessConfig {
        workers: Workers::default(),
        world: WorldConfig::headless(),
        start: Start::Loaded(saved),
        saves: None,
    };
    let mut app = Headless::start(&GAME, config, cooked)?;
    let report = app.run(ticks, Box::new(|_| TickInput::default()))?;
    app.shutdown();
    Ok(report)
}

// ---- 4. A SOUR test of the assembly ----

/// Steers the player's body along the ground by its input, keeping what gravity gave it,
/// so the input applier's writes reach the solver's state, which the world hash covers.
#[system(name = "sketch::steer", worlds = [server, prediction])]
fn steer(input: &PlayerInput, velocity: Mut<Velocity>) {
    let planar = input.world_movement() * Scalar::from_i32(4);
    velocity.linear = Vec3::new(planar.x, velocity.linear.y, planar.z);
}

fn register_steer(registration: &mut Registration) {
    registration.stage(Stage::Simulation).add(&STEER);
}

fn no_types(_: &mut TypeTable) -> Result<()> {
    Ok(())
}

/// What the SOUR run adds to the sketch: a system that steers player 1's body.
static SKETCH_BODY: CrateRegistration = CrateRegistration {
    name: "sketch-body",
    types: no_types,
    setup: None,
    systems: register_steer,
    migrations: None,
};

/// The sketch with a body: player 1 joins as a dynamic sphere, claimed and anchored, that
/// falls under gravity while its input steers it, so the solver pool (sized from the
/// workers SOUR's modes vary) and the input applier both enter the hash.
static STEERED_GAME: Game = Game {
    name: "sketch-steered",
    crates: &[SKETCH_SIM, SKETCH_BODY],
    order: None,
    sessions: Some(bodies),
};

fn bodies() -> Box<dyn SessionApplier> {
    Box::new(Bodies)
}

struct Bodies;

impl SessionApplier for Bodies {
    fn apply(&mut self, world: &mut World, player: PlayerId, event: SessionEvent) -> Result<()> {
        match event {
            SessionEvent::Join { region } => {
                let key = StableKey::Net(world.allocate_net_id()?);
                let start = Vec3::new(Scalar::ZERO, Scalar::from_i32(10), Scalar::ZERO);
                let row = (
                    Counter(0),
                    ClaimedBy { player },
                    StreamingAnchor { priority: 0 },
                    Transform::at(start),
                    Velocity::ZERO,
                    RigidBody::dynamic(),
                    Collider::new(Shape::sphere(Scalar::HALF)),
                );
                world.spawn(key, region, row)
            }
            SessionEvent::Leave => Ok(()),
        }
    }
}

/// Player 1's sample at `tick`: the stick forward, the yaw turning a little every tick.
fn sample(tick: Tick) -> PlayerInput {
    let movement = Axis2 { x: 0, y: AXIS_SCALE };
    PlayerInput { movement, yaw: Yaw(tick.0.wrapping_mul(97) as u16), ..PlayerInput::default() }
}

/// `frames`, with player 1's sample in every tick's frame.
fn steered_frames(cooked: Arc<InMemory>) -> Result<Box<dyn FnMut(Tick) -> TickInput + Send>> {
    let mut frames = frames(cooked)?;
    Ok(Box::new(move |tick| {
        let mut input = frames(tick);
        let bytes = sample(tick).to_sample_bytes().to_vec();
        let sample = PlayerSample { input_tick: tick, substituted: false, sample: bytes };
        input.inputs.insert(PlayerId(1), sample);
        input
    }))
}

/// Runs `ticks` ticks of [`STEERED_GAME`] under the runner's session, one `step` each,
/// recording the hash after every tick and the chaos log at the end:
/// `lactose_sim::harness::run_recorded`'s shape, over the assembly instead of a bare
/// simulation. A refused bundle or session event is the step's output and fails the test;
/// so does a body the input did not steer or the solver did not move, which every mode
/// would agree on. Outside the runner the session is production and nothing is recorded.
fn recorded_run(test: &str, ticks: u32) -> Result<()> {
    let session = Session::from_env().map_err(protocol_error)?;
    let config = *session.config();
    let layout = match config.layout {
        Layout::Production => layout_mode(ChunkCapacity::Production, 0)?,
        Layout::Perturbed { capacity: Capacity::Rows(n), seed } => {
            layout_mode(ChunkCapacity::Rows(n), seed)?
        }
        Layout::Perturbed { capacity: Capacity::Bytes(n), seed } => {
            layout_mode(ChunkCapacity::Bytes(n), seed)?
        }
    };
    let workers = Workers {
        frame_workers: config.workers.map(NonZeroU32::get),
        chaos: config.chaos_seed.map(chaos),
        ..Workers::default()
    };
    let world = WorldConfig { layout, regions: RegionControl::Directed, ..WorldConfig::headless() };
    let config_applied = HeadlessConfig { workers, world, start: Start::Fresh, saves: None };
    let cooked = Arc::new(InMemory::new(RegionId(1))?);
    let cooked_dyn = Arc::clone(&cooked) as Arc<dyn Cooked>;
    let mut app = Headless::start(&STEERED_GAME, config_applied, cooked_dyn)?;
    // What the assembly built, as its world and executor report it, never the session's
    // request: a world the assembly built under another layout fails the runner's check.
    let world = app.server().expect("a started app has its server").simulation().world();
    let applied = RunConfig {
        workers: config.workers.map(|_| {
            NonZeroU32::new(app.executor().frame_worker_count()).expect("at least one worker")
        }),
        layout: applied_layout(world.layout())?,
        chaos_seed: app.executor().chaos_config().map(|chaos| chaos.seed),
    };
    let mut recorder = session.recorder(test, &applied).map_err(protocol_error)?;
    let mut frames = steered_frames(cooked)?;
    for _ in 0..ticks {
        let next = Tick(app.tick().0 + 1);
        let out: StepOutput = app.step(frames(next))?;
        if !out.failures.is_empty() {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("refused at {next:?}: {out:?}"),
            ));
        }
        recorder.record(app.tick().0, app.hash()?).map_err(protocol_error)?;
    }
    steered(&app, Tick(ticks))?;
    if let Some(log) = app.shutdown() {
        session.write_execution_log(test, log.as_bytes()).map_err(protocol_error)?;
    }
    Ok(())
}

/// Player 1's body after `last`: it holds the last tick's sample, the input applier's
/// write, and it has fallen and moved along the ground, the solver's.
fn steered(app: &Headless, last: Tick) -> Result<()> {
    let world = app.server().expect("the server").simulation().world();
    let missing = |what: &str| Error::new(ErrorKind::NotFound, format!("player 1's {what}"));
    let key = player_entity(world, PlayerId(1)).ok_or_else(|| missing("entity"))?;
    if world.get::<PlayerInput>(key) != Some(&sample(last)) {
        return Err(Error::new(ErrorKind::InvalidData, "the last sample was not applied"));
    }
    let at = world.get::<Transform>(key).ok_or_else(|| missing("transform"))?.translation;
    let ground = at.x * at.x + at.z * at.z;
    if at.y >= Scalar::from_i32(10) || ground <= Scalar::ONE {
        return Err(Error::new(ErrorKind::InvalidData, format!("the body did not move: {at:?}")));
    }
    Ok(())
}

fn protocol_error(error: protocol::ProtocolError) -> Error {
    Error::new(ErrorKind::InvalidData, error.to_string())
}

/// The layout a world reports, as the runner spells it; one no SOUR mode builds fails.
fn applied_layout(layout: &LayoutConfig) -> lactose_core::Result<Layout> {
    Ok(match layout_mode_of(layout)? {
        (ChunkCapacity::Production, _) => Layout::Production,
        (ChunkCapacity::Rows(n), seed) => Layout::Perturbed { capacity: Capacity::Rows(n), seed },
        (ChunkCapacity::Bytes(n), seed) => Layout::Perturbed { capacity: Capacity::Bytes(n), seed },
    })
}

fn chaos(seed: u64) -> ChaosConfig {
    ChaosConfig { seed, shuffle_steals: true, inject_yields: true, force_split: true, record: true }
}

#[test]
fn a_recorded_headless_run() {
    recorded_run("lactose_app::headless::a_recorded_headless_run", 120).unwrap();
}

// ---- 5. The headless app's proof ----

const TICKS: u32 = 30;

/// A headless app of the sketch game with `workers` frame workers.
fn app(workers: Workers, start: Start, saves: Option<PathBuf>) -> (Headless, Arc<InMemory>) {
    let cooked = Arc::new(InMemory::new(RegionId(1)).expect("the content"));
    let config = HeadlessConfig { workers, world: WorldConfig::headless(), start, saves };
    let app = Headless::start(&GAME, config, Arc::clone(&cooked) as Arc<dyn Cooked>)
        .expect("the app starts");
    (app, cooked)
}

fn with_workers(frame_workers: u32) -> Workers {
    Workers {
        frame_workers: Some(frame_workers),
        background_workers: Some(1),
        ..Workers::default()
    }
}

/// `ticks` steps with the sketch's frames, then the hash.
fn stepped(app: &mut Headless, cooked: &Arc<InMemory>, ticks: u32) -> u64 {
    let mut frames = frames(Arc::clone(cooked)).expect("the frames");
    for _ in 0..ticks {
        let next = Tick(app.tick().0 + 1);
        let out = app.step(frames(next)).expect("the step");
        assert!(out.failures.is_empty(), "{out:?}");
    }
    app.hash().expect("the hash")
}

/// A directory of this process's own under the system's temporary directory, emptied.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lactose-app-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn a_headless_run_is_a_function_of_its_frames() {
    let (mut by_step, cooked) = app(with_workers(2), Start::Fresh, None);
    let stepped_hash = stepped(&mut by_step, &cooked, TICKS);
    by_step.shutdown();

    let (mut by_run, cooked) = app(with_workers(2), Start::Fresh, None);
    let report = by_run.run(TICKS, frames(cooked).expect("the frames")).expect("the run");
    assert_eq!(report.ticks, TICKS);
    assert!(report.frames >= TICKS / CATCH_UP_CAP);
    assert_eq!(report.hash, stepped_hash, "the paced run hashes as the stepped one");
    assert_eq!(by_run.tick(), Tick(TICKS));
    by_run.shutdown();

    for workers in [1, 3] {
        let (mut other, cooked) = app(with_workers(workers), Start::Fresh, None);
        assert_eq!(stepped(&mut other, &cooked, TICKS), stepped_hash, "{workers} workers");
        other.shutdown();
    }
}

#[test]
fn a_run_on_a_replay_executor_is_unsupported_before_any_tick() {
    let chaos = Workers { chaos: Some(chaos(7)), ..with_workers(2) };
    let (mut recorded, cooked) = app(chaos, Start::Fresh, None);
    stepped(&mut recorded, &cooked, 3);
    let log = recorded.shutdown().expect("chaos recorded a log");
    let replay = Workers { replay: Some(log), ..with_workers(2) };
    let (mut replaying, cooked) = app(replay, Start::Fresh, None);
    let error = replaying.run(5, frames(cooked).expect("the frames")).expect_err("refused");
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert_eq!(replaying.tick(), Tick(0), "before any tick");
    replaying.shutdown();
}

#[test]
fn a_save_through_a_file_resumes_where_the_original_goes_on() {
    let dir = scratch("save");
    let slot = dir.join("slot.sav");
    let (mut original, cooked) = app(with_workers(2), Start::Fresh, Some(dir.clone()));
    stepped(&mut original, &cooked, TICKS);
    original.save(slot.clone()).expect("the save");
    let bytes = std::fs::read(&slot).expect("the slot");
    let saved = load(&GAME, &bytes, &*cooked).expect("the load");
    assert_eq!(saved.tick, Tick(TICKS));
    assert_eq!(saved.regions.len(), 1);

    // The resumed world's first step loads the saved region from its store.
    let (mut resumed, _) = app(with_workers(2), Start::Loaded(saved), None);
    assert_eq!(resumed.tick(), Tick(TICKS));
    let later = |app: &mut Headless| {
        for _ in 0..TICKS {
            let out = app.step(TickInput::default()).expect("the step");
            assert!(out.failures.is_empty(), "{out:?}");
        }
        app.hash().expect("the hash")
    };
    assert_eq!(later(&mut resumed), later(&mut original));
    resumed.shutdown();
    original.shutdown();

    // A corrupt section fails at `load`, before any world exists.
    let mut corrupt = bytes;
    let last = corrupt.len() - 1;
    corrupt[last] ^= 0xff;
    assert!(load(&GAME, &corrupt, &*cooked).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_saved_world_resumes_with_no_file() {
    let executor =
        Executor::builder().frame_workers(2).background_workers(1).build().expect("executor");
    let cooked = Arc::new(InMemory::new(RegionId(1)).expect("the content"));
    let build = |start| {
        let cooked = Arc::clone(&cooked) as Arc<dyn Cooked>;
        build_server(&executor, &GAME, WorldConfig::headless(), start, cooked).expect("the server")
    };
    let step = |server: Server, frame| {
        let (server, out) = executor.block_on(server.step(&executor, frame)).expect("the step");
        assert!(out.failures.is_empty(), "{out:?}");
        server
    };
    let mut original = build(Start::Fresh);
    let mut frames = frames(Arc::clone(&cooked)).expect("the frames");
    for _ in 0..TICKS {
        let next = Tick(original.tick().0 + 1);
        original = step(original, frames(next));
    }
    let (mut original, saved) =
        executor.block_on(original.save_point(&executor)).expect("the save point");
    let mut resumed = build(Start::Loaded(saved));
    for _ in 0..TICKS {
        original = step(original, TickInput::default());
        resumed = step(resumed, TickInput::default());
    }
    assert_eq!(resumed.hash().expect("the hash"), original.hash().expect("the hash"));
    resumed.shutdown();
    original.shutdown();
    executor.shutdown();
}

#[cfg(unix)]
#[test]
fn a_failed_replace_leaves_the_previous_file() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = scratch("replace");
    let slot = dir.join("slot.sav");
    let (mut app, cooked) = app(with_workers(2), Start::Fresh, Some(dir.clone()));
    stepped(&mut app, &cooked, 3);
    app.save(slot.clone()).expect("the first save");
    let first = std::fs::read(&slot).expect("the slot");
    for _ in 0..3 {
        app.step(TickInput::default()).expect("the step");
    }
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).expect("read-only");
    let failed = app.save(slot.clone());
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("writable");
    assert!(failed.is_err(), "a directory that refuses the new file fails the save");
    assert_eq!(std::fs::read(&slot).expect("the slot"), first, "the previous save is intact");
    assert!(app.server().is_some(), "a failed save leaves the world");
    app.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_chaos_runs_log_is_read_at_shutdown() {
    let chaos = Workers { chaos: Some(chaos(11)), ..with_workers(2) };
    let (mut app, cooked) = app(chaos, Start::Fresh, None);
    stepped(&mut app, &cooked, 5);
    let log = app.shutdown().expect("the log");
    assert!(!log.as_bytes().is_empty());
}

#[test]
fn the_timer_root_constants_hold() {
    assert_eq!(CATCH_UP_CAP, 4);
    const { assert!(LOG_RETENTION >= CATCH_UP_CAP) };
    assert_eq!(TICK_PERIOD, Duration::from_nanos(16_666_666));
    assert_eq!(WorldConfig::headless().log_retention, LOG_RETENTION);
    assert!(matches!(WorldConfig::headless().regions, RegionControl::Directed));
}

#[test]
fn the_milestone_1_policy_is_the_table() {
    let policy = ErrorPolicy::MILESTONE_1;
    let error = Error::new(ErrorKind::InvalidData, "any");
    for source in [Source::Executor, Source::Load] {
        assert_eq!(policy.respond(source, &error), Response::Stop, "{source:?}");
    }
    for kept in [false, true] {
        assert_eq!(policy.respond(Source::Server { kept }, &error), Response::Stop);
    }
    assert_eq!(policy.respond(Source::Prediction { kept: true }, &error), Response::Resync);
    assert_eq!(policy.respond(Source::Prediction { kept: false }, &error), Response::Stop);
    assert_eq!(policy.respond(Source::Save, &error), Response::Log);
    // Architecture section 14: a client-only asset failure requests a resync.
    assert_eq!(policy.respond(Source::Cooked, &error), Response::Resync);
    // The milestone 1 table reads no error kind: a disk-full save is logged like any other.
    let full = Error::new(ErrorKind::StorageFull, "the disk");
    assert_eq!(policy.respond(Source::Save, &full), Response::Log);
}

#[test]
fn the_log_target_is_no_module_path_prefix() {
    // `EnvFilter` matches a directive's target by prefix: the target must not be one that
    // a `lactose_*` module path starts with.
    assert_eq!(LOG_TARGET, "app");
    assert!(!"lactose_app".starts_with(LOG_TARGET));
}

#[test]
fn no_refusal_is_empty() {
    assert!(LifecycleFailures::default().is_empty());
    assert!(StepOutput::default().failures.is_empty());
}

#[test]
fn a_registration_is_a_plain_value() {
    let game: Game = GAME;
    assert_eq!(game.crates.len(), 1);
    assert_eq!(game.crates[0].name, "sketch-sim");
    assert!(game.crates[0].setup.is_none());
    assert!(game.crates[0].migrations.is_some());
}

// The binary's `main` and the resume sketch: type-checked here, run by the binary.
const _: fn(u32, Option<PathBuf>) -> Result<ExitCode> = headless_main;
const _: fn(&[u8], u32) -> Result<RunReport> = resume_main;
