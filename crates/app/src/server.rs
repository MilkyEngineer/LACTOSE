//! The server world, its store of saved regions and the timer root's frame around it
//! (architecture sections 5.1, 9.2, 9.3, 11 and 15.2; `crates/sim/README.md`, "The timer
//! root" and "Regions").

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use lactose_bundle::Baseline;
use lactose_core::{Error, Result, Tick};
use lactose_ecs::RegionId;
use lactose_net_api::{RegionRelevance, StagingFrame};
use lactose_persist::{Migrations, SchemaTable};
use lactose_sim::{
    AssetFailureEvent, LoadFailure, RegionBundle, RegionRequest, SessionFailure, Simulation,
    TickInput, TimeScale,
};
use lactose_task::{Executor, TickPacer};

use crate::game::Game;
use crate::policy::ErrorPolicy;
use crate::runtime::SolverExit;
use crate::save::{Cooked, SaveEncode, SavedRegion, SavedWorld};
use crate::world::{Start, WorldConfig};

/// A server world between runs: the simulation, its pacer, the schema table and the
/// migrations a save and a reload share, the cooked content a reload's table comes from,
/// the time scale the server decided, the baseline of every loaded region, and the store:
/// the saved state of every region that is not loaded ([`kept`](Self::kept)), with the
/// bundles queued for the next step. Built by [`build_server`]; stepped by value from the
/// timer root through [`step`](Self::step) or [`frame`](Self::frame); ended by
/// [`shutdown`](Self::shutdown), which drops the world and waits for its solver threads.
pub struct Server {
    simulation: Simulation,
    pacer: TickPacer,
    schemas: Arc<SchemaTable>,
    migrations: Migrations,
    cooked: Arc<dyn Cooked>,
    time_scale: TimeScale,
    pending_scale: Option<TimeScale>,
    baselines: BTreeMap<RegionId, Arc<Baseline>>,
    kept: BTreeMap<RegionId, Arc<SavedRegion>>,
    queued: BTreeMap<RegionId, RegionBundle>,
    solver: SolverExit,
    policy: ErrorPolicy,
}

/// Builds a server world from `game` and `config`, starting at `start`, over `cooked`: the
/// type table (`Game::types`), the schema table and the migrations, the solver pool and the
/// backend, every setup function, every registration function and every ordering edge in
/// the order `README.md` states ("Registration"), the input and session appliers, `config`
/// into the builder, and a pacer built at `Instant::now()`, whose first tick is due at
/// once. A `Loaded` start puts the saved tick and counter into the builder and every saved
/// region into the store; under `Directed` each is also queued for the first step, as
/// `Loader::bundle(&records, baseline, RegionMode::Simulated, table)` with `table` the
/// region's `Cooked::table` pinned through `Cooked::pin`, or `None`; under `Anchored`
/// nothing is queued, and the lifecycle's `Load` requests bring the regions back as
/// players join (`README.md`, "Saves and loads"). Fails as the builder fails (a
/// registration or setup error carries its crate's name; `log_retention` below the cap is
/// `InvalidData`), as the solver pool fails, as `Loader::bundle` fails for a region queued
/// here, and with `InvalidData` if `start` is `Loaded`, `config.regions` is `Anchored` and
/// a saved region is not in the atlas.
pub fn build_server(
    executor: &Executor,
    game: &Game,
    config: WorldConfig,
    start: Start,
    cooked: Arc<dyn Cooked>,
) -> Result<Server> {
    todo!("build the type table, the pool, the backend, the simulation and the store")
}

impl Server {
    /// The current tick: the last tick run, or the start tick before the first step.
    #[must_use]
    pub fn tick(&self) -> Tick {
        self.simulation.tick()
    }

    /// `Simulation::hash`: the world hash over every loaded region, what a headless run
    /// reports and a SOUR test records after every step. Fails as the hash fails, which
    /// the policy treats as a server failure: `Stop`.
    pub fn hash(&self) -> Result<u64> {
        self.simulation.hash()
    }

    /// The simulation, between runs.
    #[must_use]
    pub fn simulation(&self) -> &Simulation {
        &self.simulation
    }

    /// The simulation, between runs, for what the assembly's callers do there and nothing
    /// else: `lactose_input::player_entity` after a join, the gather state when a frame
    /// did not take it, the lifecycle's events. Everything written through
    /// `Simulation::world_mut` is an unjournaled edit (`crates/sim/README.md`).
    pub fn simulation_mut(&mut self) -> &mut Simulation {
        &mut self.simulation
    }

    /// The schema table of the world's registry, built once at [`build_server`]: what every
    /// save and every relevance message of this world is written with.
    #[must_use]
    pub fn schemas(&self) -> &Arc<SchemaTable> {
        &self.schemas
    }

    /// The scale the pacer paces by, kept as `README.md` ("Time scale") states:
    /// `REAL_TIME` at build; a nonzero scale from the call that set it; zero from the step
    /// that carried it. After any step that carried a scale, by either path and from either
    /// origin, it is that scale, so between steps it equals the world's `TimeScale` mirror
    /// except after a nonzero [`set_time_scale`](Self::set_time_scale) whose step has not
    /// run, when it leads the mirror.
    #[must_use]
    pub fn time_scale(&self) -> TimeScale {
        self.time_scale
    }

    /// Decides the time scale (architecture section 11): `scale` is carried by the next
    /// step, by either path, as `TickInput::time_scale` in place of what the frame callback
    /// put there; a `scale` equal to [`time_scale`](Self::time_scale) with none pending is
    /// dropped. The pacer takes it by direction. Nonzero: the pacer paces by
    /// `thousandths / 1000` from the call and [`time_scale`](Self::time_scale) is `scale`
    /// at once, so a paused server has ticks due at its next [`frame`](Self::frame) and the
    /// first of them carries the unpause. Zero: the pacer and
    /// [`time_scale`](Self::time_scale) change only after the step that carried it, and a
    /// frame whose poll owes no tick runs that one tick for it, so the pause reaches the
    /// world before the ticks stop. The passes still run once per frame while paused.
    pub fn set_time_scale(&mut self, scale: TimeScale) {
        todo!("record the scale for the next step; a nonzero one reaches the pacer now")
    }

    /// `TickPacer::resync(now)`: forgets the ticks owed and makes the next fall due one
    /// period after `now`. What `Headless::run` does at its start, and what a loop does
    /// before its first [`frame`](Self::frame) after steps, a save or any other gap in
    /// which it did not poll, so the gap is not caught up at the cap.
    pub fn resync(&mut self, now: Instant) {
        self.pacer.resync(now);
    }

    /// The baseline of the bundle that loaded `region`, while it is loaded: what a save or a
    /// relevance message diffs its export against. `None` for a region that is not loaded,
    /// or was created through `Simulation::world_mut` with no bundle. The baseline moves
    /// into the region's [`SavedRegion`] when the region unloads, and a reload records the
    /// reloading bundle's.
    #[must_use]
    pub fn baseline(&self, region: RegionId) -> Option<&Arc<Baseline>> {
        self.baselines.get(&region)
    }

    /// The store: the saved state of every region that is not loaded, by region. A region
    /// enters it from a `Start::Loaded` save, or from the export of its unload, diffed; it
    /// leaves it in the step that loads the region again. Never a loaded region. What
    /// [`save_point`](Self::save_point) includes beside the loaded regions' exports, so no
    /// saved or unloaded region is lost between saves (`README.md`, "Saves and loads").
    #[must_use]
    pub fn kept(&self) -> &BTreeMap<RegionId, Arc<SavedRegion>> {
        &self.kept
    }

    /// One tick, by value, with `frame` and nothing paced: for SOUR tests and tools through
    /// `Executor::block_on` from the thread that owns the server, and what
    /// [`frame`](Self::frame) runs per due tick. Before `Simulation::step`, in this order: a
    /// pending time scale replaces `frame.time_scale`; every bundle queued for this step
    /// goes into `frame.loaded`; and a `frame.loaded` entry for a region the store holds is
    /// replaced by the stored state, the entry lending its `mode` and its `assets`, the
    /// rest of it dropped and logged at `info` (`README.md`, "Saves and loads": a frame's
    /// entry never discards what was saved).
    /// After it: every export the lifecycle made is diffed against the region's
    /// [`baseline`](Self::baseline) into the store (`RegionRecords::from_export`; a failure
    /// there is this run's error, with the server kept, since it means the registry and the
    /// schema table disagree); the baseline of every bundle in `frame.loaded` the sync
    /// point accepted is recorded and its region leaves the store; every `Load` request for
    /// a kept region is answered by queuing its bundle for the next step; and the requests
    /// and the load, session and asset failures are taken into the [`StepOutput`], the
    /// failures also logged at `warn`. Resolves with the server at the next tick and the
    /// output, or with [`ServerError`]: the server kept when `Simulation::step` handed it
    /// back, gone otherwise.
    pub fn step(self, executor: &Executor, frame: TickInput) -> ServerStep<'_> {
        todo!("edit the frame, step the simulation once, keep the exports, take the outputs")
    }

    /// One frame of the timer root, by value: `pacer.poll(now)`, the budget's ticks taken
    /// as one when it owes none and a zero scale is pending; then one [`step`](Self::step)
    /// per tick with `frames(next)` as its input (`next` the tick about to run,
    /// `Tick(tick().0 + 1)`), its requests and failures appended to the output's; then
    /// `Simulation::run_passes` once, whether or not a tick ran; then `GatherState::take`
    /// into the output. `None` from `frames` ends the frame's stepping before that tick: it
    /// does not run, the ticks still due this frame are forgotten (the pacer counted them)
    /// and the passes run as usual; it is for ending a bounded run at its last tick and
    /// nothing else, since what it forgets is simulation time lost. `frames` runs on the
    /// task that drives the frame, a frame-critical worker, between steps, so what it
    /// drains (a jitter buffer, a session queue) is read in tick order and never
    /// concurrently with a step; it must not block. `behind` is reported, not acted on: the
    /// overload ladder is not in this crate yet. The caller sleeps until `next_deadline`
    /// (`Executor::sleep_until`) and calls again. Resolves with the server and the output,
    /// or with [`ServerError`] as [`step`](Self::step) does; a failure of the passes after
    /// the ticks ran hands the server back with the ticks kept, as `RunError` does.
    pub fn frame<'a>(
        self,
        executor: &'a Executor,
        now: Instant,
        frames: &'a mut (dyn FnMut(Tick) -> Option<TickInput> + Send),
    ) -> ServerFrame<'a> {
        todo!("poll the pacer, step the due ticks, run the passes, take the outputs")
    }

    /// The save copy of every region at a sync point, by value: `Simulation::save_point`,
    /// then one `WorkerClass::Background` task per exported region running
    /// `RegionRecords::from_export(&export, &baseline, &schemas)` with the region's
    /// [`baseline`](Self::baseline), joined in ascending region order, and beside them
    /// every kept region's [`SavedRegion`], shared. Resolves with the server, unchanged, and
    /// a [`SavedWorld`] at the save's tick, for [`encode_save`](Self::encode_save) or a
    /// `Start::Loaded`; or with [`ServerError`] with the server kept: a failed export or
    /// diff changes nothing, and a loaded region with no recorded baseline (one created
    /// through `Simulation::world_mut`) is `NotFound`.
    pub fn save_point(self, executor: &Executor) -> ServerSave<'_> {
        todo!("take the save copy, diff each export, add the kept regions")
    }

    /// The container bytes of `saved`, between runs: one `WorkerClass::Background` task per
    /// region running `RegionRecords::encode_section`, then
    /// `Container::assemble(&ContainerHeader::new(saved.tick, saved.next_net_id,
    /// schemas.hash()), sections)` once every task resolved, with this world's
    /// [`schemas`](Self::schemas) and the sections in ascending region order. Fails as
    /// `lactose-persist` fails; nothing is written by this call, so a failure loses
    /// nothing. The bytes go to `Io::replace` (`README.md`, "Saves and loads").
    pub fn encode_save(&self, executor: &Executor, saved: SavedWorld) -> SaveEncode {
        todo!("spawn one encode per region and assemble")
    }

    /// The relevance message for `region`, between runs, for a client whose set it enters
    /// (architecture section 9.3): `World::export_region(region)`, then
    /// `RegionRecords::from_export(&export, &baseline, &schemas)?.to_relevance(assets)` with
    /// the region's [`baseline`](Self::baseline) and `assets` the table the world holds for
    /// it (`World::region_assets(region).map(|pinned| pinned.table().clone())`), or `None`
    /// if none has entered, so the client builds the region over the assets the server
    /// simulates it with. On the calling thread at milestone 1, `executor` unused;
    /// milestone 4 moves the diff to a background task and shares one message between
    /// co-located clients, with this signature. Fails with `NotFound` for a region that is
    /// not loaded or whose baseline this world never recorded, and as the export and the
    /// diff fail.
    pub fn relevance(&self, executor: &Executor, region: RegionId) -> Result<RegionRelevance> {
        todo!("export the region and diff it")
    }

    /// Drops the world and waits for its solver threads: the simulation goes first, which
    /// drops `Physics` and its backend, which terminates the pool; then every `solver-<n>`
    /// thread's exit is waited for, so the threads are gone and not only told to go.
    /// Dropping a `Server` without this terminates the pool and does not wait. From the
    /// thread that owns the server, never from a worker or a solver thread.
    pub fn shutdown(self) {
        todo!("drop the simulation, then wait on the solver exit")
    }
}

/// The lifecycle's refusals of one step, taken after it: a SOUR test, the store's driver
/// and the milestone 4 binary read them here; the step that reported them has carried on.
#[derive(Debug, Default)]
pub struct LifecycleFailures {
    /// The bundles the sync point refused (`RegionLifecycle::take_load_failures`), a bundle
    /// the server could not build for a kept region included, as a `LoadFailure` at the
    /// step's tick with `Loader::bundle`'s error.
    pub loads: Vec<LoadFailure>,
    /// The session events the applier refused (`take_session_failures`).
    pub sessions: Vec<SessionFailure>,
    /// The asset failures the frame stamped (`take_asset_failures`).
    pub assets: Vec<AssetFailureEvent>,
}

impl LifecycleFailures {
    /// Whether nothing was refused. Real, not `todo!()`: it is a value.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.loads.is_empty() && self.sessions.is_empty() && self.assets.is_empty()
    }
}

/// What one step produced beside the next tick: what a step-driven run reads per tick,
/// and what each step of a frame adds to the [`FrameOutput`].
#[derive(Debug, Default)]
pub struct StepOutput {
    /// The lifecycle's requests this step, in the order issued, every one. A `Load` of a
    /// region the store holds is answered by the server itself in the next step and is
    /// still reported, so the caller can resolve the region's table and release it later;
    /// a `Load` of any other region is the caller's to answer through a later frame's
    /// `loaded`; a `Release` says the region's export is in the store now. Empty under
    /// `Directed`.
    pub requests: Vec<RegionRequest>,
    /// The lifecycle's refusals this step.
    pub failures: LifecycleFailures,
}

/// What one frame of the timer root produced, for what runs between frames.
pub struct FrameOutput {
    /// New ticks run this frame, after the cap; one when a pending pause ran its tick.
    pub ticks: u32,
    /// The cap was hit with ticks still owed: the overload ladder's input.
    pub behind: bool,
    /// When the next tick falls due: what the caller sleeps until.
    pub next_deadline: Instant,
    /// The replication gather's frame since the last take (`GatherState::take`): the
    /// packer's. Empty on a world with no changed rows; dropped by the headless app.
    pub gathered: StagingFrame,
    /// Every step's requests this frame, in step order (`StepOutput::requests`).
    pub requests: Vec<RegionRequest>,
    /// Every step's refusals this frame, in step order (`StepOutput::failures`).
    pub failures: LifecycleFailures,
}

/// How a server run fails: the error, and the server when its world survived. `Some`
/// when the run's `RunError` handed the simulation back (a failure before anything ran,
/// a `Gather` pass that no longer prepares, a failed export or diff), so the caller keeps
/// its world; `None` after `Cancelled`, when the executor shut down during the run and the
/// world dropped with it. Converts into [`Error`], so `?` into a plain `Result` drops the
/// server as a plain error would; the policy's response is `Stop` either way.
pub struct ServerError {
    /// What failed.
    pub error: Error,
    /// The server, if its world survived the failure.
    pub server: Option<Server>,
}

impl fmt::Debug for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerError")
            .field("error", &self.error)
            .field("server", &self.server.is_some())
            .finish()
    }
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, f)
    }
}

impl std::error::Error for ServerError {}

impl From<ServerError> for Error {
    fn from(failure: ServerError) -> Self {
        failure.error
    }
}

/// A run in flight, owning the server until it resolves.
type Run<'a, T> = Pin<Box<dyn Future<Output = core::result::Result<T, ServerError>> + Send + 'a>>;

/// One tick in flight: resolves to the server at the next tick and what the step produced.
/// See [`Server::step`].
#[must_use = "futures do nothing unless awaited"]
pub struct ServerStep<'a> {
    run: Run<'a, (Server, StepOutput)>,
}

impl Future for ServerStep<'_> {
    type Output = core::result::Result<(Server, StepOutput), ServerError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.run.as_mut().poll(cx)
    }
}

/// One frame in flight: resolves to the server and what the frame produced. See
/// [`Server::frame`].
#[must_use = "futures do nothing unless awaited"]
pub struct ServerFrame<'a> {
    run: Run<'a, (Server, FrameOutput)>,
}

impl Future for ServerFrame<'_> {
    type Output = core::result::Result<(Server, FrameOutput), ServerError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.run.as_mut().poll(cx)
    }
}

/// A save copy in flight: resolves to the server and every region's saved state. See
/// [`Server::save_point`].
#[must_use = "futures do nothing unless awaited"]
pub struct ServerSave<'a> {
    run: Run<'a, (Server, SavedWorld)>,
}

impl Future for ServerSave<'_> {
    type Output = core::result::Result<(Server, SavedWorld), ServerError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.run.as_mut().poll(cx)
    }
}
