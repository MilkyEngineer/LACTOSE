//! What the proof's tests share: a game of one crate (a counter, a pass probe, a session
//! applier that spawns a claimed, anchored counter), cooked content for any region, a
//! stand-in clock, the run helpers and a `tracing` subscriber that keeps what `app` logs.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use lactose_app::{
    Cooked, CrateRegistration, FrameOutput, Game, Server, ServerError, Start, StepOutput,
    TICK_PERIOD, WorldConfig, build_server,
};
use lactose_asset_api::{
    Asset, AssetId, AssetKind, AssetSet, AssetVersion, LoadPriority, PinnedTable, PublishedAssets,
    Readiness, UntypedHandle, VersionTable,
};
use lactose_bundle::Baseline;
use lactose_core::{Error, ErrorKind, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{ClaimedBy, Executor, Mut, RegionId, RegionMode, Res, World, system};
use lactose_reflect::component;
use lactose_sim::{
    Pass, RegionAtlas, RegionBundle, RegionCell, RegionControl, Registration, SessionApplier,
    SessionEvent, Stage, StreamingAnchor, TickInput, TimeScale, TypeTable,
};

// ---- The game ----

/// Counts ticks on every entity that has it.
#[component(key = "proof::counter", persistent, replicated)]
pub struct Counter(#[field(id = 1)] pub u32);

#[system(name = "proof::count", worlds = [server, prediction])]
fn count(counter: Mut<Counter>) {
    counter.0 = counter.0.wrapping_add(1);
}

/// The passes the server world has run since the process started.
pub static PASSES: AtomicU32 = AtomicU32::new(0);

#[system(name = "proof::count_passes", worlds = [server])]
fn count_passes(_scale: Res<TimeScale>) {
    PASSES.fetch_add(1, Ordering::Relaxed);
}

/// The server world's `TimeScale` mirror at every tick it ran, in thousandths.
pub static SCALES: Mutex<Vec<u32>> = Mutex::new(Vec::new());

#[system(name = "proof::record_scale", worlds = [server])]
fn record_scale(scale: Res<TimeScale>) {
    SCALES.lock().unwrap_or_else(PoisonError::into_inner).push(scale.thousandths);
}

/// What [`SCALES`] recorded since the last call.
pub fn scales() -> Vec<u32> {
    std::mem::take(&mut *SCALES.lock().unwrap_or_else(PoisonError::into_inner))
}

/// The passes run since the process started.
pub fn passes() -> u32 {
    PASSES.load(Ordering::Relaxed)
}

fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<Counter>()
}

fn register_systems(registration: &mut Registration) {
    registration.stage(Stage::Simulation).add(&COUNT).add(&RECORD_SCALE);
    registration.pass(Pass::Extract).add(&COUNT_PASSES);
}

static PROOF: CrateRegistration = CrateRegistration {
    name: "proof",
    types: register_types,
    setup: None,
    systems: register_systems,
    migrations: None,
};

/// The game every proof test builds from.
pub static GAME: Game =
    Game { name: "proof", crates: &[PROOF], order: None, sessions: Some(sessions) };

fn sessions() -> Box<dyn SessionApplier> {
    Box::new(Sessions)
}

/// Spawns the player's counter, claimed and anchored, on `Join`; releases the claim and the
/// anchor on `Leave`, keeping the row.
struct Sessions;

impl SessionApplier for Sessions {
    fn apply(&mut self, world: &mut World, player: PlayerId, event: SessionEvent) -> Result<()> {
        match event {
            SessionEvent::Join { region } => {
                let key = StableKey::Net(world.allocate_net_id()?);
                let anchor = StreamingAnchor { priority: 0 };
                world.spawn(key, region, (Counter(0), ClaimedBy { player }, anchor))
            }
            SessionEvent::Leave => {
                let Some(key) = lactose_input::player_entity(world, player) else { return Ok(()) };
                world.remove::<StreamingAnchor>(key)?;
                world.remove::<ClaimedBy>(key)?;
                Ok(())
            }
        }
    }
}

// ---- The cooked content ----

/// A store that has published every asset.
struct Store;

struct Blob;

impl Asset for Blob {
    const KIND: AssetKind = AssetKind::from_name("proof::blob");
    const FORMAT_VERSION: u32 = 1;
}

impl PublishedAssets for Store {
    fn request(&self, _: AssetId, _: AssetVersion, _: LoadPriority) {}

    fn get(&self, id: AssetId, version: AssetVersion) -> Readiness<UntypedHandle> {
        Readiness::Ready(UntypedHandle::new(id, version, Blob))
    }
}

/// The version table of `region`'s one asset.
pub fn table_of(region: RegionId) -> VersionTable {
    [(AssetId::from_bits(region.0), AssetVersion::from_bits(1).expect("non-zero"))]
        .into_iter()
        .collect()
}

/// Cooked content for every region: one empty baseline, a table per region but those in
/// `unresolved`, and a count of the pins.
pub struct Content {
    pub baseline: Arc<Baseline>,
    pub unresolved: BTreeSet<RegionId>,
    pub pins: AtomicU32,
}

impl Content {
    pub fn new() -> Arc<Self> {
        Self::without(BTreeSet::new())
    }

    pub fn without(unresolved: BTreeSet<RegionId>) -> Arc<Self> {
        let baseline = Arc::new(Baseline::from_rows(AssetSet::new(), [], []).expect("empty"));
        Arc::new(Self { baseline, unresolved, pins: AtomicU32::new(0) })
    }

    /// `region`'s cooked bundle in `mode` over a fresh copy of the baseline (so a test can
    /// tell which bundle's baseline a region records), its table pinned.
    pub fn bundle(&self, region: RegionId, mode: RegionMode) -> RegionBundle {
        RegionBundle {
            seed: region.0,
            mode,
            baseline: Arc::new((*self.baseline).clone()),
            overlay: None,
            resources: Vec::new(),
            transient_resources: Vec::new(),
            assets: Some(self.pin(&table_of(region)).expect("published")),
        }
    }
}

impl Cooked for Content {
    fn baseline(&self, content_hash: u64) -> Result<Arc<Baseline>> {
        if self.baseline.content_hash() == content_hash {
            Ok(Arc::clone(&self.baseline))
        } else {
            Err(Error::new(ErrorKind::NotFound, "no cooked baseline has that hash"))
        }
    }

    fn table(&self, region: RegionId) -> Option<VersionTable> {
        (!self.unresolved.contains(&region)).then(|| table_of(region))
    }

    fn pin(&self, table: &VersionTable) -> Result<PinnedTable> {
        self.pins.fetch_add(1, Ordering::Relaxed);
        PinnedTable::pin(&Store, table)
            .ready()
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "the store has not published it"))
    }
}

// ---- Worlds and runs ----

/// An executor with `workers` frame-critical workers and one background worker.
pub fn executor(workers: u32) -> Executor {
    Executor::builder().frame_workers(workers).background_workers(1).build().expect("executor")
}

/// A server of [`GAME`] over `content`.
pub fn server(
    executor: &Executor,
    config: WorldConfig,
    start: Start,
    content: &Arc<Content>,
) -> Server {
    build_server(executor, &GAME, config, start, Arc::clone(content) as Arc<dyn Cooked>)
        .expect("the server builds")
}

/// One step, which must succeed, with the store's invariant checked after it.
pub fn step(executor: &Executor, server: Server, frame: TickInput) -> (Server, StepOutput) {
    let (server, output) = executor.block_on(server.step(executor, frame)).expect("the step");
    assert_disjoint(&server);
    (server, output)
}

/// One step that may fail.
pub fn try_step(
    executor: &Executor,
    server: Server,
    frame: TickInput,
) -> core::result::Result<(Server, StepOutput), Box<ServerError>> {
    executor.block_on(server.step(executor, frame)).map_err(Box::new)
}

/// No region is both loaded and kept.
pub fn assert_disjoint(server: &Server) {
    for region in server.simulation().world().regions() {
        assert!(!server.kept().contains_key(&region), "region {} is loaded and kept", region.0);
    }
}

/// A frame loading `regions` fresh, `Simulated`.
pub fn load(content: &Content, regions: impl IntoIterator<Item = u64>) -> TickInput {
    let mut frame = TickInput::default();
    for region in regions {
        frame
            .loaded
            .insert(RegionId(region), content.bundle(RegionId(region), RegionMode::Simulated));
    }
    frame
}

/// A frame with one session event.
pub fn session(player: u32, event: SessionEvent) -> TickInput {
    let mut frame = TickInput::default();
    frame.sessions.insert(PlayerId(player), event);
    frame
}

/// The loaded regions.
pub fn loaded(server: &Server) -> BTreeSet<u64> {
    server.simulation().world().regions().map(|region| region.0).collect()
}

/// The kept regions.
pub fn kept(server: &Server) -> BTreeSet<u64> {
    server.kept().keys().map(|region| region.0).collect()
}

/// The world's `TimeScale` mirror.
pub fn mirror(server: &Server) -> TimeScale {
    *server.simulation().world().resource::<TimeScale>().expect("the mirror")
}

/// A 5 by 5 grid of regions `100 + y * 5 + x`.
pub fn grid() -> RegionAtlas {
    let mut atlas = RegionAtlas::new();
    for y in 0..5 {
        for x in 0..5 {
            atlas
                .insert(RegionId(100 + y * 5 + x), RegionCell { x: x as i32, y: y as i32 })
                .expect("cell");
        }
    }
    atlas
}

/// The headless configuration under `Anchored` over [`grid`] with `pinned`.
pub fn anchored(pinned: impl IntoIterator<Item = u64>) -> WorldConfig {
    WorldConfig {
        regions: RegionControl::Anchored {
            atlas: grid(),
            pinned: pinned.into_iter().map(RegionId).collect(),
        },
        ..WorldConfig::headless()
    }
}

/// The instants a test drives a pacer by: `base` and whole periods after it.
#[derive(Clone, Copy)]
pub struct Clock {
    base: Instant,
}

impl Clock {
    /// A clock based on now, which is at or after any pacer's build instant.
    pub fn new() -> Self {
        Self { base: Instant::now() }
    }

    /// `periods` tick periods after the base.
    pub fn at(self, periods: u32) -> Instant {
        self.base + TICK_PERIOD * periods
    }
}

/// One frame at `now`, which must succeed.
pub fn frame(
    executor: &Executor,
    server: Server,
    now: Instant,
    frames: &mut (dyn FnMut(Tick) -> Option<TickInput> + Send),
) -> (Server, FrameOutput) {
    let (server, output) =
        executor.block_on(server.frame(executor, now, frames)).expect("the frame");
    assert_disjoint(&server);
    (server, output)
}

// ---- What `app` logs ----

/// One event at target `app`.
#[derive(Clone, Debug)]
pub struct Logged {
    pub level: tracing::Level,
    pub message: String,
}

/// Installs a subscriber that keeps every event at target `app`, for the rest of the
/// process (nextest runs each test in its own).
pub fn capture() -> Arc<Mutex<Vec<Logged>>> {
    let kept = Arc::new(Mutex::new(Vec::new()));
    tracing::subscriber::set_global_default(Capture(Arc::clone(&kept))).expect("one per process");
    kept
}

/// The events kept so far at `level` whose message contains `text`.
pub fn logged(kept: &Mutex<Vec<Logged>>, level: tracing::Level, text: &str) -> usize {
    let kept = kept.lock().unwrap_or_else(PoisonError::into_inner);
    kept.iter().filter(|event| event.level == level && event.message.contains(text)).count()
}

struct Capture(Arc<Mutex<Vec<Logged>>>);

struct Message(String);

impl tracing::field::Visit for Message {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        use std::fmt::Write as _;
        let _ = write!(self.0, "{}={value:?} ", field.name());
    }
}

impl tracing::Subscriber for Capture {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        metadata.target() == lactose_app::LOG_TARGET
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let mut message = Message(String::new());
        event.record(&mut message);
        let logged = Logged { level: *event.metadata().level(), message: message.0 };
        self.0.lock().unwrap_or_else(PoisonError::into_inner).push(logged);
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}
