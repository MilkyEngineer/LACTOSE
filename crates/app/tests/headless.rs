//! Compile-only sketches of what the headless binary and a SOUR test of the assembly write
//! against this surface: a game crate's registration, the binary's `main`, the glue that
//! records a run, and a save round trip. Type-checked, never run (every deciding body in
//! the facade is `todo!()`); the tests at the end run what is real now. A change that
//! breaks a sketch is a signature change to list.

#![forbid(unsafe_code)]
#![allow(dead_code, unused_variables, missing_docs)]

#[path = "../../../tools/xtask/src/harness/protocol.rs"]
#[expect(
    clippy::wrong_self_convention,
    reason = "the runner's module as written; it is included, not owned, here"
)]
#[allow(
    unfulfilled_lint_expectations,
    reason = "the boundary lints these expectations name are not configured outside the boundary"
)]
mod protocol;

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use lactose_app::{
    CATCH_UP_CAP, Cooked, CrateRegistration, ErrorPolicy, Game, Headless, HeadlessConfig,
    LOG_RETENTION, LOG_TARGET, LifecycleFailures, Response, RunReport, Source, Start, StepOutput,
    TICK_PERIOD, Workers, WorldConfig, load,
};
use lactose_asset_api::{
    Asset, AssetId, AssetKind, AssetSet, AssetVersion, LoadPriority, PinnedTable, PublishedAssets,
    Readiness, UntypedHandle, VersionTable,
};
use lactose_bundle::Baseline;
use lactose_core::{Error, ErrorKind, Result, Tick};
use lactose_ecs::{Mut, RegionId, RegionMode, World, system};
use lactose_persist::Migrations;
use lactose_reflect::component;
use lactose_sim::harness::{ChunkCapacity, layout_mode};
use lactose_sim::{RegionBundle, RegionControl, Registration, Stage, TickInput, TypeTable};
use lactose_task::ChaosConfig;
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

/// The crate's generated `setup`: nothing to insert here, so the registration carries
/// `None`; shown for the shape.
pub fn setup(world: &mut World) -> Result<()> {
    Ok(())
}

/// The crate's generated `register_systems`.
pub fn register_systems(registration: &mut Registration) {
    registration.stage(Stage::Simulation).add(&COUNT);
}

/// The crate's `register_migrations`: no bump yet.
pub fn register_migrations(migrations: &mut Migrations) -> Result<()> {
    Ok(())
}

/// What the crate exposes: one `static`, function pointers only, nothing built before
/// `main`.
pub static SKETCH_SIM: CrateRegistration = CrateRegistration {
    name: "sketch-sim",
    types: register_types,
    setup: None,
    systems: register_systems,
    migrations: Some(register_migrations),
};

/// What the binary hands the assembly.
pub static GAME: Game = Game { name: "sketch", crates: &[SKETCH_SIM], order: None, sessions: None };

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

/// The frames a headless run feeds: the region's bundle in the first, nothing after. On a
/// `Start::Loaded` run the first frame's entry meets the saved region in the server's
/// store: the saved state loads, and this entry lends it the mode and the pinned table
/// (`README.md`, "Saves and loads").
fn frames(cooked: Arc<InMemory>) -> Box<dyn FnMut(Tick) -> TickInput + Send> {
    let mut first = true;
    Box::new(move |tick| {
        let mut input = TickInput::default();
        if std::mem::take(&mut first) {
            input.loaded.insert(cooked.region, cooked.fresh().expect("an empty baseline"));
        }
        input
    })
}

// ---- 3. `main` of `app-headless` ----

/// The binary's `main`: the executor's defaults, the headless world, `ticks` ticks at
/// tick rate from a spawned timer root, the hash on stdout, an orderly shutdown. `main`
/// returns the first error, which is the policy's `Stop`; a failed save is logged, as the
/// policy says, and the run still reports.
fn headless_main(ticks: u32, save_to: Option<PathBuf>) -> Result<()> {
    let cooked = Arc::new(InMemory::new(RegionId(1))?);
    let config = HeadlessConfig {
        workers: Workers::default(),
        world: WorldConfig::headless(),
        start: Start::Fresh,
        saves: save_to.as_deref().and_then(Path::parent).map(Path::to_path_buf),
    };
    let mut app = Headless::start(&GAME, config, Arc::clone(&cooked) as Arc<dyn Cooked>)?;
    let report: RunReport = app.run(ticks, frames(cooked))?;
    println!(
        "{} ticks in {} frames ({} behind), hash {:#018x}",
        report.ticks, report.frames, report.behind, report.hash
    );
    if let Some(slot) = save_to
        && let Err(error) = app.save(slot)
    {
        tracing::warn!(target: LOG_TARGET, %error, "the save failed; the run is reported");
    }
    let _log = app.shutdown();
    Ok(())
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

/// Runs `ticks` ticks under the runner's session, one `step` each, recording the hash
/// after every tick and the chaos log at the end: `lactose_sim::harness::run_recorded`'s
/// shape, over the assembly instead of a bare simulation. A refused bundle or session
/// event is the step's output and fails the test. Outside the runner the session is
/// production and nothing is recorded.
fn sour_run(test: &str, ticks: u32) -> Result<()> {
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
        chaos: config.chaos_seed.map(|seed| ChaosConfig {
            seed,
            shuffle_steals: true,
            inject_yields: true,
            force_split: true,
            record: true,
        }),
        ..Workers::default()
    };
    let world = WorldConfig { layout, regions: RegionControl::Directed, ..WorldConfig::headless() };
    let config_applied = HeadlessConfig { workers, world, start: Start::Fresh, saves: None };
    let cooked = Arc::new(InMemory::new(RegionId(1))?);
    let mut app = Headless::start(&GAME, config_applied, Arc::clone(&cooked) as Arc<dyn Cooked>)?;
    let applied = RunConfig {
        workers: config.workers.map(|_| {
            NonZeroU32::new(app.executor().frame_worker_count()).expect("at least one worker")
        }),
        layout: config.layout,
        chaos_seed: config.chaos_seed,
    };
    let mut recorder = session.recorder(test, &applied).map_err(protocol_error)?;
    let mut frames = frames(cooked);
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
    if let Some(log) = app.shutdown() {
        session.write_execution_log(test, log.as_bytes()).map_err(protocol_error)?;
    }
    Ok(())
}

fn protocol_error(error: protocol::ProtocolError) -> Error {
    Error::new(ErrorKind::InvalidData, error.to_string())
}

// ---- 5. What is real now ----

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
