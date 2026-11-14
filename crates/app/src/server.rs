//! The server world, its store of saved regions and the timer root's frame around it
//! (architecture sections 5.1, 9.2, 9.3, 11 and 15.2; `crates/sim/README.md`, "The timer
//! root" and "Regions").

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use lactose_asset_api::PinnedTable;
use lactose_bundle::Baseline;
use lactose_core::{Error, ErrorKind, Result, Tick};
use lactose_ecs::{RegionId, RegionMode, WorldKind};
use lactose_net_api::{GatherState, RegionRelevance, StagingFrame};
use lactose_persist::{Container, ContainerHeader, Loader, Migrations, RegionRecords, SchemaTable};
use lactose_sim::{
    AssetFailureEvent, LoadFailure, RegionBundle, RegionControl, RegionExport, RegionRequest,
    RunError, SessionFailure, Simulation, TickInput, TimeScale,
};
use lactose_task::{Executor, TickPacer, WorkerClass};
use tracing::{debug, info, warn};

use crate::assemble;
use crate::game::Game;
use crate::policy::{ErrorPolicy, LOG_TARGET, Source, run_source};
use crate::runtime::{CATCH_UP_CAP, SolverExit, TICK_PERIOD};
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
    core: Core,
}

/// Everything of a [`Server`] but the simulation, so a run can move the simulation out and
/// put it back.
struct Core {
    pacer: TickPacer,
    schemas: Arc<SchemaTable>,
    migrations: Migrations,
    cooked: Arc<dyn Cooked>,
    time_scale: TimeScale,
    pending_scale: Option<TimeScale>,
    /// The scale the last step carried, `REAL_TIME` at build: the world's `TimeScale`
    /// mirror, kept here so the pacer is never read back from the world.
    carried: TimeScale,
    baselines: BTreeMap<RegionId, Arc<Baseline>>,
    kept: BTreeMap<RegionId, Arc<SavedRegion>>,
    queued: BTreeMap<RegionId, RegionBundle>,
    solver: SolverExit,
    policy: ErrorPolicy,
}

/// A run's failure as the runs inside this crate see it: [`ServerError`] and the tick the
/// world stood at when the run began, which `Headless` reports for a torn world.
pub(crate) struct Failed {
    pub(crate) error: Error,
    pub(crate) server: Option<Server>,
    pub(crate) tick: Tick,
}

impl From<Box<Failed>> for ServerError {
    fn from(failed: Box<Failed>) -> Self {
        let Failed { error, server, .. } = *failed;
        Self { error, server }
    }
}

/// Builds a server world from `game` and `config`, starting at `start`, over `cooked`: the
/// type table (`Game::types`), the schema table and the migrations, the solver pool and the
/// backend, every setup function, every registration function and every ordering edge in
/// the order `README.md` states ("Registration"), the input and session appliers, `config`
/// into the builder, and a pacer built at `Instant::now()`, whose first tick is due at
/// once. A `Loaded` start puts the saved tick and counter into the builder and every saved
/// region into the store; under `Directed` each is also queued for the first step, as
/// `Loader::bundle(&records, baseline, RegionMode::Simulated, table)` with `table` the
/// region's `Cooked::table` pinned through `Cooked::pin`, or `None` when it has none;
/// under `Anchored` each saved pinned region is queued the same way in
/// `RegionMode::Frozen`, with its table required, so it loads at the first step as it
/// would have stepped on unsaved, and the lifecycle's `Load` requests bring the others
/// back as players join (`README.md`, "Saves and loads"). Fails as the builder fails (a
/// registration or setup error carries its crate's name; `log_retention` below the cap is
/// `InvalidData`), as the solver pool fails, as `Cooked::pin` fails or `Loader::bundle`
/// fails for a region queued here, and, when `start` is `Loaded` and `config.regions` is
/// `Anchored`, with `InvalidData` if a saved region is not in the atlas and `NotFound` if a
/// saved pinned region has no `Cooked::table`.
pub fn build_server(
    executor: &Executor,
    game: &Game,
    config: WorldConfig,
    start: Start,
    cooked: Arc<dyn Cooked>,
) -> Result<Server> {
    let (types, schemas) = assemble::registry(game)?;
    let migrations = game.migrations()?;
    let mut kept = BTreeMap::new();
    let mut queued = BTreeMap::new();
    let mut resumed = None;
    if let Start::Loaded(saved) = start {
        match &config.regions {
            RegionControl::Directed => {
                let loader = Loader::new(&schemas, &migrations);
                for (&region, entry) in &saved.regions {
                    let table = cooked.table(region).map(|table| cooked.pin(&table)).transpose()?;
                    let bundle = loader.bundle(
                        &entry.records,
                        Arc::clone(&entry.baseline),
                        RegionMode::Simulated,
                        table,
                    )?;
                    queued.insert(region, bundle);
                }
            }
            RegionControl::Anchored { atlas, pinned } => {
                if let Some(region) = saved.regions.keys().find(|&&r| atlas.cell(r).is_none()) {
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        format!("the saved region {} is not in the atlas", region.0),
                    ));
                }
                // A pinned region runs whenever the server does: a resume that waited for
                // the lifecycle's request would run it a tick late, and diverge from the
                // world that was never saved. Queued here, it is never requested, so one
                // that could load no table would wait frozen with nothing to say so.
                let loader = Loader::new(&schemas, &migrations);
                for region in pinned {
                    let Some(entry) = saved.regions.get(region) else { continue };
                    let table = cooked.table(*region).ok_or_else(|| {
                        Error::new(
                            ErrorKind::NotFound,
                            format!("the saved pinned region {} has no cooked table", region.0),
                        )
                    })?;
                    let bundle = loader.bundle(
                        &entry.records,
                        Arc::clone(&entry.baseline),
                        RegionMode::Frozen,
                        Some(cooked.pin(&table)?),
                    )?;
                    queued.insert(*region, bundle);
                }
            }
        }
        resumed = Some((saved.tick, saved.next_net_id));
        kept = saved.regions;
    }
    let (mut builder, solver) = assemble::world(executor, game, &config, WorldKind::Server, types)?;
    if let Some(sessions) = game.sessions {
        builder = builder.sessions(sessions());
    }
    if let Some((tick, next_net_id)) = resumed {
        builder = builder.tick(tick).next_net_id(next_net_id);
    }
    let simulation = builder.build()?;
    let core = Core {
        pacer: TickPacer::new(Instant::now(), TICK_PERIOD, CATCH_UP_CAP),
        schemas: Arc::new(schemas),
        migrations,
        cooked,
        time_scale: TimeScale::REAL_TIME,
        pending_scale: None,
        carried: TimeScale::REAL_TIME,
        baselines: BTreeMap::new(),
        kept,
        queued,
        solver,
        policy: ErrorPolicy::MILESTONE_1,
    };
    Ok(Server { simulation, core })
}

/// `region`'s cooked table pinned, or `None`, logged at `warn`, while it has none or its pin
/// fails: a region answered by the store then loads `Frozen` and waits for
/// `TickInput::assets`, as a table not yet published is the store's ordinary transient case.
fn pinned_table(cooked: &dyn Cooked, region: RegionId) -> Option<PinnedTable> {
    let Some(table) = cooked.table(region) else {
        warn!(
            target: LOG_TARGET,
            region = region.0,
            "a saved region reloads with no table and waits frozen for one"
        );
        return None;
    };
    match cooked.pin(&table) {
        Ok(pinned) => Some(pinned),
        Err(error) => {
            warn!(
                target: LOG_TARGET,
                region = region.0,
                kind = ?error.kind(),
                %error,
                "a saved region's table cannot be pinned: it reloads with none and waits frozen"
            );
            None
        }
    }
}

/// The pacer's scale for `scale`.
fn pacer_scale(scale: TimeScale) -> f64 {
    f64::from(scale.thousandths) / 1000.0
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
        &self.core.schemas
    }

    /// The scale the pacer paces by, kept as `README.md` ("Time scale") states:
    /// `REAL_TIME` at build; a nonzero scale from the call that set it; zero from the step
    /// that carried it, or from the call that cancelled a pending nonzero scale while the
    /// world's mirror was already zero. After any step that carried a scale, by either path
    /// and from either origin, it is that scale. So between runs, with no scale pending, it
    /// equals the world's `TimeScale` mirror; with a nonzero scale pending it is that scale,
    /// ahead of the mirror; with a zero pending it is the scale the pacer still paces by
    /// until the step that carries the zero, which under [`frame`](Self::frame) is the
    /// frame's last.
    #[must_use]
    pub fn time_scale(&self) -> TimeScale {
        self.core.time_scale
    }

    /// Decides the time scale (architecture section 11): `scale` is carried by the next
    /// step, by either path, as `TickInput::time_scale` in place of what the frame callback
    /// put there; a `scale` equal to [`time_scale`](Self::time_scale) with none pending is
    /// dropped. The pacer takes it by direction. Nonzero: the pacer paces by
    /// `thousandths / 1000` from the call and [`time_scale`](Self::time_scale) is `scale`
    /// at once, so a paused server has ticks due at its next [`frame`](Self::frame) and the
    /// first of them carries the unpause. Zero: the pacer and
    /// [`time_scale`](Self::time_scale) change only after the step that carried it, a
    /// frame whose poll owes no tick runs that one tick for it, and the step that carries
    /// it is the frame's last, so the pause reaches the world before the ticks stop and no
    /// tick runs after it. A zero set while a nonzero scale is pending and the world's
    /// mirror is already zero (an unpause taken back before any step carried it) is no
    /// pause to carry: it cancels the pending scale and puts the pacer and
    /// [`time_scale`](Self::time_scale) back to zero, and no tick runs for it. The passes
    /// still run once per frame while paused.
    pub fn set_time_scale(&mut self, scale: TimeScale) {
        let core = &mut self.core;
        if scale == core.time_scale && core.pending_scale.is_none() {
            return;
        }
        if scale.is_paused() {
            let unpausing = core.pending_scale.is_some_and(|pending| !pending.is_paused());
            if unpausing && core.carried.is_paused() {
                core.pending_scale = None;
                core.pacer.set_time_scale(0.0);
                core.time_scale = scale;
                return;
            }
            core.pending_scale = Some(scale);
        } else {
            core.pending_scale = Some(scale);
            core.pacer.set_time_scale(pacer_scale(scale));
            core.time_scale = scale;
        }
    }

    /// `TickPacer::resync(now)`: forgets the ticks owed and makes the next fall due one
    /// period after `now`. What `Headless::run` does at its start, and what a loop does
    /// before its first [`frame`](Self::frame) after steps, a save or any other gap in
    /// which it did not poll, so the gap is not caught up at the cap.
    pub fn resync(&mut self, now: Instant) {
        self.core.pacer.resync(now);
    }

    /// The baseline of the bundle that loaded `region`, while it is loaded: what a save or a
    /// relevance message diffs its export against. `None` for a region that is not loaded,
    /// or was created through `Simulation::world_mut` with no bundle. The baseline moves
    /// into the region's [`SavedRegion`] when the region unloads, and a reload records the
    /// reloading bundle's.
    #[must_use]
    pub fn baseline(&self, region: RegionId) -> Option<&Arc<Baseline>> {
        self.core.baselines.get(&region)
    }

    /// The store: the saved state of every region that is not loaded, by region. A region
    /// enters it from a `Start::Loaded` save, or from the export of its unload, diffed; it
    /// leaves it in the step that loads the region again. Never a loaded region. What
    /// [`save_point`](Self::save_point) includes beside the loaded regions' exports, so no
    /// saved or unloaded region is lost between saves (`README.md`, "Saves and loads").
    #[must_use]
    pub fn kept(&self) -> &BTreeMap<RegionId, Arc<SavedRegion>> {
        &self.core.kept
    }

    /// One tick, by value, with `frame` and nothing paced: for SOUR tests and tools through
    /// `Executor::block_on` from the thread that owns the server, and what
    /// [`frame`](Self::frame) runs per due tick. Before `Simulation::step`, in this order: a
    /// pending time scale replaces `frame.time_scale`; every bundle queued for this step
    /// goes into `frame.loaded`; and a `frame.loaded` entry for a region the store holds is
    /// replaced by the stored state, the entry lending its `mode` and its `assets`, the
    /// rest of it dropped and logged at `info` (`README.md`, "Saves and loads": a frame's
    /// entry never discards what was saved).
    /// After it, the store is reconciled against the world the step left, never against the
    /// frame, since under `Anchored` a bundle can load and its region unload again in one
    /// step: every export the lifecycle made is diffed into the store
    /// (`RegionRecords::from_export`) against the baseline the region had when it was
    /// exported, its [`baseline`](Self::baseline) from before the step if it was loaded
    /// at the step's start (a `Directed` unload runs before the frame's bundles), else the
    /// baseline of this step's bundle that loaded it; the baseline of every bundle in
    /// `frame.loaded` the sync point accepted is recorded; every region loaded after the
    /// step (`World::regions`) leaves the store, and every region not loaded leaves the
    /// baselines; every `Load` request for a kept region is answered by queuing its bundle
    /// for the next step; and the requests and the load, session and asset failures are
    /// taken into the [`StepOutput`], the failures also logged at `warn`. A diff that fails
    /// (the registry and the schema table disagree, or the region has no recorded
    /// baseline, `NotFound`) loses that region's saved state, so after the rest of this
    /// bookkeeping is done for every other region the run fails with the first such error
    /// in ascending region order and the server gone (`server: None`): a server missing a
    /// region is never handed back to be saved. Resolves with the server at the next tick
    /// and the output, or with [`ServerError`]: the server kept when `Simulation::step`
    /// failed and handed it back, gone when it did not hand it back and after a failed
    /// diff.
    pub fn step(self, executor: &Executor, frame: TickInput) -> ServerStep<'_> {
        ServerStep {
            run: Box::pin(async move {
                let (server, output, _) = self.run_step(executor, frame).await?;
                Ok((server, output))
            }),
        }
    }

    /// One frame of the timer root, by value: `pacer.poll(now)`, preceded by
    /// `pacer.resync(now)` while [`time_scale`](Self::time_scale) is zero, the budget's ticks
    /// taken as one when it owes none and a zero scale is pending; then one [`step`](Self::step)
    /// per tick with `frames(next)` as its input (`next` the tick about to run,
    /// `Tick(tick().0 + 1)`), its requests and failures appended to the output's; then
    /// `Simulation::run_passes` once, whether or not a tick ran; then `GatherState::take`
    /// into the output. `None` from `frames` ends the frame's stepping before that tick: it
    /// does not run, the ticks still due this frame are forgotten (the pacer counted them)
    /// and the passes run as usual; it is for ending a bounded run at its last tick and
    /// nothing else, since what it forgets is simulation time lost. A step that carried a
    /// zero scale, pending or the callback's own, ends the frame's stepping after it the
    /// same way, so no tick runs with the world paused. `frames` runs on the
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
        ServerFrame {
            run: Box::pin(async move { Ok(self.run_frame(executor, now, frames).await?) }),
        }
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
        ServerSave { run: Box::pin(async move { Ok(self.run_save(executor).await?) }) }
    }

    /// The container bytes of `saved`, between runs: one `WorkerClass::Background` task per
    /// region running `RegionRecords::encode_section`, then
    /// `Container::assemble(&ContainerHeader::new(saved.tick, saved.next_net_id,
    /// schemas.hash()), sections)` once every task resolved, with this world's
    /// [`schemas`](Self::schemas) and the sections in ascending region order. Fails as
    /// `lactose-persist` fails; nothing is written by this call, so a failure loses
    /// nothing. The bytes go to `Io::replace` (`README.md`, "Saves and loads").
    pub fn encode_save(&self, executor: &Executor, saved: SavedWorld) -> SaveEncode {
        let header = ContainerHeader::new(saved.tick, saved.next_net_id, self.core.schemas.hash());
        let tasks: Vec<_> = saved
            .regions
            .into_values()
            .map(|region| {
                executor
                    .spawn(WorkerClass::Background, async move { region.records.encode_section() })
            })
            .collect();
        SaveEncode::new(Box::pin(async move {
            let mut sections = Vec::with_capacity(tasks.len());
            let mut failure = None;
            // Every task is awaited, so none outlives the future that spawned it.
            for task in tasks {
                match task.await.and_then(|section| section) {
                    Ok(section) => sections.push(section),
                    Err(error) => {
                        failure.get_or_insert(error);
                    }
                }
            }
            match failure {
                Some(error) => Err(error),
                None => Container::assemble(&header, sections),
            }
        }))
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
        // Milestone 4's background diff spawns here; milestone 1 diffs on this thread.
        let _ = executor;
        let baseline = self.core.baselines.get(&region).ok_or_else(|| no_baseline(region))?;
        let world = self.simulation.world();
        let export = RegionExport { region, rows: world.export_region(region)? };
        let assets = world.region_assets(region).map(|pinned| pinned.table().clone());
        Ok(RegionRecords::from_export(&export, baseline, &self.core.schemas)?.to_relevance(assets))
    }

    /// Drops the world and waits for its solver threads: the simulation goes first, which
    /// drops `Physics` and its backend, which terminates the pool; then every `solver-<n>`
    /// thread's exit is waited for, so the threads are gone and not only told to go.
    /// Dropping a `Server` without this terminates the pool and does not wait. From the
    /// thread that owns the server, never from a worker or a solver thread.
    pub fn shutdown(self) {
        let Self { simulation, core } = self;
        drop(simulation);
        core.solver.wait();
    }

    /// [`step`](Self::step), with the scale the step carried and the tick of a failure.
    pub(crate) async fn run_step(
        self,
        executor: &Executor,
        mut frame: TickInput,
    ) -> core::result::Result<(Self, StepOutput, Option<TimeScale>), Box<Failed>> {
        let Self { simulation, mut core } = self;
        let tick = simulation.tick();
        let stepped = Tick(tick.0.wrapping_add(1));
        let loaded_before: BTreeSet<RegionId> = simulation.world().regions().collect();
        if let Some(scale) = core.pending_scale {
            frame.time_scale = Some(scale);
        }
        let carried = frame.time_scale;
        // A step that fails before anything ran hands the queue back as it was.
        let queued = core.queued.clone();
        let mut refused = core.load_stored(&mut frame, stepped);
        let loaded: BTreeMap<RegionId, Arc<Baseline>> = frame
            .loaded
            .iter()
            .map(|(&region, bundle)| (region, Arc::clone(&bundle.baseline)))
            .collect();
        let mut simulation = match simulation.step(executor, frame).await {
            Ok(simulation) => simulation,
            Err(RunError { error, simulation }) => {
                let kept = simulation.is_some();
                core.policy.decide(run_source(&error, kept, server_source), &error);
                core.queued = queued;
                let server = simulation.map(|simulation| Self { simulation, core });
                return Err(Box::new(Failed { error, server, tick }));
            }
        };
        let lifecycle = simulation.lifecycle_mut();
        let requests = lifecycle.take_requests();
        let exports = lifecycle.take_exports();
        let mut loads = lifecycle.take_load_failures();
        let sessions = lifecycle.take_session_failures();
        let assets = lifecycle.take_asset_failures();

        let mut lost = BTreeMap::new();
        for export in exports {
            let baseline = if loaded_before.contains(&export.region) {
                core.baselines.get(&export.region)
            } else {
                loaded.get(&export.region)
            };
            let diffed = baseline.ok_or_else(|| no_baseline(export.region)).and_then(|baseline| {
                let records = RegionRecords::from_export(&export, baseline, &core.schemas)?;
                Ok(SavedRegion { records, baseline: Arc::clone(baseline) })
            });
            match diffed {
                Ok(saved) => {
                    core.kept.insert(export.region, Arc::new(saved));
                }
                Err(error) => {
                    lost.entry(export.region).or_insert(error);
                }
            }
        }
        let refusals: BTreeSet<RegionId> = loads.iter().map(|failure| failure.region).collect();
        for (region, baseline) in loaded {
            if !refusals.contains(&region) {
                core.baselines.insert(region, baseline);
            }
        }
        let loaded_after: BTreeSet<RegionId> = simulation.world().regions().collect();
        core.kept.retain(|region, _| !loaded_after.contains(region));
        core.baselines.retain(|region, _| loaded_after.contains(region));
        for request in &requests {
            match *request {
                RegionRequest::Load { region } => {
                    if let Some(failure) = core.answer(region, stepped) {
                        refused.push(failure);
                    }
                }
                RegionRequest::Cancel { region } => {
                    core.queued.remove(&region);
                }
                RegionRequest::Release { .. } => {}
            }
        }
        loads.append(&mut refused);
        let failures = LifecycleFailures { loads, sessions, assets };
        log_failures(&failures);
        if let Some(scale) = carried {
            core.pending_scale = None;
            core.carried = scale;
            core.pacer.set_time_scale(pacer_scale(scale));
            core.time_scale = scale;
        }
        if let Some((_, error)) = lost.into_iter().next() {
            core.policy.decide(Source::Server { kept: false }, &error);
            return Err(Box::new(Failed { error, server: None, tick }));
        }
        Ok((Self { simulation, core }, StepOutput { requests, failures }, carried))
    }

    /// [`frame`](Self::frame), with the tick of a failure.
    pub(crate) async fn run_frame(
        self,
        executor: &Executor,
        now: Instant,
        frames: &mut (dyn FnMut(Tick) -> Option<TickInput> + Send),
    ) -> core::result::Result<(Self, FrameOutput), Box<Failed>> {
        let mut server = self;
        if server.core.time_scale.is_paused() {
            // The pacer stopped with what it owed when the pause was carried, and a paused
            // pacer still pays out what it owes: forgetting it keeps the world paused, and
            // makes an unpause owe from the last paused frame on.
            server.core.pacer.resync(now);
        }
        let budget = server.core.pacer.poll(now);
        let pausing = server.core.pending_scale.is_some_and(TimeScale::is_paused);
        let due = if budget.ticks == 0 && pausing { 1 } else { budget.ticks };
        let mut ticks = 0;
        let mut requests = Vec::new();
        let mut failures = LifecycleFailures::default();
        for _ in 0..due {
            let Some(frame) = frames(Tick(server.tick().0.wrapping_add(1))) else { break };
            let (stepped, mut output, carried) = server.run_step(executor, frame).await?;
            server = stepped;
            ticks += 1;
            requests.append(&mut output.requests);
            failures.loads.append(&mut output.failures.loads);
            failures.sessions.append(&mut output.failures.sessions);
            failures.assets.append(&mut output.failures.assets);
            if carried.is_some_and(TimeScale::is_paused) {
                break;
            }
        }
        let Self { simulation, core } = server;
        let tick = simulation.tick();
        let mut simulation = match simulation.run_passes(executor).await {
            Ok(simulation) => simulation,
            Err(RunError { error, simulation }) => {
                let kept = simulation.is_some();
                core.policy.decide(run_source(&error, kept, server_source), &error);
                let server = simulation.map(|simulation| Self { simulation, core });
                return Err(Box::new(Failed { error, server, tick }));
            }
        };
        let gathered = simulation
            .world_mut()
            .service_mut::<GatherState>()
            .map_or_else(|| StagingFrame::new(tick), GatherState::take);
        if budget.behind {
            debug!(target: LOG_TARGET, tick = tick.0, ticks, "the frame is behind");
        }
        let output = FrameOutput {
            ticks,
            behind: budget.behind,
            next_deadline: budget.next_deadline,
            gathered,
            requests,
            failures,
        };
        Ok((Self { simulation, core }, output))
    }

    /// [`save_point`](Self::save_point), with the tick of a failure.
    async fn run_save(
        self,
        executor: &Executor,
    ) -> core::result::Result<(Self, SavedWorld), Box<Failed>> {
        let Self { simulation, core } = self;
        let tick = simulation.tick();
        let (simulation, point) = match simulation.save_point(executor).await {
            Ok(done) => done,
            Err(RunError { error, simulation }) => {
                let kept = simulation.is_some();
                core.policy.decide(run_source(&error, kept, server_source), &error);
                let server = simulation.map(|simulation| Self { simulation, core });
                return Err(Box::new(Failed { error, server, tick }));
            }
        };
        let server = Self { simulation, core };
        match server.diff_point(executor, point.regions).await {
            Ok(mut regions) => {
                for (&region, saved) in &server.core.kept {
                    regions.insert(region, Arc::clone(saved));
                }
                let saved =
                    SavedWorld { tick: point.tick, next_net_id: point.next_net_id, regions };
                Ok((server, saved))
            }
            Err(error) => {
                server.core.policy.decide(run_source(&error, true, server_source), &error);
                Err(Box::new(Failed { error, server: Some(server), tick }))
            }
        }
    }

    /// One background diff per export against the region's baseline, joined in region
    /// order; every baseline is checked before any task is spawned.
    async fn diff_point(
        &self,
        executor: &Executor,
        exports: Vec<RegionExport>,
    ) -> Result<BTreeMap<RegionId, Arc<SavedRegion>>> {
        let mut work = Vec::with_capacity(exports.len());
        for export in exports {
            let baseline = self
                .core
                .baselines
                .get(&export.region)
                .ok_or_else(|| no_baseline(export.region))?;
            work.push((export, Arc::clone(baseline)));
        }
        let tasks: Vec<_> = work
            .into_iter()
            .map(|(export, baseline)| {
                let schemas = Arc::clone(&self.core.schemas);
                let region = export.region;
                let task = executor.spawn(WorkerClass::Background, async move {
                    let records = RegionRecords::from_export(&export, &baseline, &schemas)?;
                    Ok(SavedRegion { records, baseline })
                });
                (region, task)
            })
            .collect();
        let mut regions = BTreeMap::new();
        let mut failure = None;
        // Every task is awaited, so none outlives the run that spawned it.
        for (region, task) in tasks {
            match task.await.and_then(|saved: Result<SavedRegion>| saved) {
                Ok(saved) => {
                    regions.insert(region, Arc::new(saved));
                }
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        failure.map_or(Ok(regions), Err)
    }
}

impl Core {
    /// The frame edits after the pending scale: every queued bundle into `frame.loaded`, and
    /// every `frame.loaded` entry for a kept or queued region replaced by the stored state,
    /// lending its mode and, when `Some`, its table. A stored state that cannot be bundled
    /// is a refusal at `stepped`, and the entry is dropped.
    fn load_stored(&mut self, frame: &mut TickInput, stepped: Tick) -> Vec<LoadFailure> {
        let mut refused = Vec::new();
        let mut queued = std::mem::take(&mut self.queued);
        let stored: Vec<RegionId> = frame
            .loaded
            .keys()
            .copied()
            .filter(|region| queued.contains_key(region) || self.kept.contains_key(region))
            .collect();
        for region in stored {
            let Some(entry) = frame.loaded.remove(&region) else { continue };
            info!(
                target: LOG_TARGET,
                region = region.0,
                tick = stepped.0,
                "a frame's bundle for a saved region loads the saved state"
            );
            let bundle = match queued.remove(&region) {
                Some(mut bundle) => {
                    bundle.mode = entry.mode;
                    if entry.assets.is_some() {
                        bundle.assets = entry.assets;
                    }
                    Ok(bundle)
                }
                None => self.bundle(region, entry.mode, entry.assets),
            };
            match bundle {
                Ok(bundle) => {
                    frame.loaded.insert(region, bundle);
                }
                Err(error) => refused.push(LoadFailure { tick: stepped, region, error }),
            }
        }
        frame.loaded.append(&mut queued);
        refused
    }

    /// The bundle of kept `region` in `mode`, over `assets` or else its cooked table pinned.
    fn bundle(
        &self,
        region: RegionId,
        mode: RegionMode,
        assets: Option<PinnedTable>,
    ) -> Result<RegionBundle> {
        let saved = self.kept.get(&region).ok_or_else(|| {
            Error::new(ErrorKind::NotFound, format!("region {} is not kept", region.0))
        })?;
        let assets = assets.or_else(|| pinned_table(&*self.cooked, region));
        Loader::new(&self.schemas, &self.migrations).bundle(
            &saved.records,
            Arc::clone(&saved.baseline),
            mode,
            assets,
        )
    }

    /// The server's answer to a `Load` of `region`: its bundle queued for the next step if
    /// it is kept, a refusal at `stepped` if it is kept and `Loader::bundle` fails (the
    /// lifecycle does not ask again: the caller's own entry brings it back), nothing if it
    /// is the caller's to answer.
    fn answer(&mut self, region: RegionId, stepped: Tick) -> Option<LoadFailure> {
        if !self.kept.contains_key(&region) {
            return None;
        }
        match self.bundle(region, RegionMode::Frozen, None) {
            Ok(bundle) => {
                self.queued.insert(region, bundle);
                None
            }
            Err(error) => Some(LoadFailure { tick: stepped, region, error }),
        }
    }
}

/// `Source::Server`, for [`run_source`].
fn server_source(kept: bool) -> Source {
    Source::Server { kept }
}

/// The error of a loaded region whose baseline this world never recorded.
fn no_baseline(region: RegionId) -> Error {
    Error::new(
        ErrorKind::NotFound,
        format!("region {} has no recorded baseline: it was not loaded through a bundle", region.0),
    )
}

/// Every refusal of a step at `warn`, with the region or player and the kind.
pub(crate) fn log_failures(failures: &LifecycleFailures) {
    for failure in &failures.loads {
        warn!(
            target: LOG_TARGET,
            tick = failure.tick.0,
            region = failure.region.0,
            kind = ?failure.error.kind(),
            error = %failure.error,
            "a region bundle was refused"
        );
    }
    for failure in &failures.sessions {
        warn!(
            target: LOG_TARGET,
            tick = failure.tick.0,
            player = ?failure.player,
            kind = ?failure.error.kind(),
            error = %failure.error,
            "a session event was refused"
        );
    }
    for failure in &failures.assets {
        warn!(
            target: LOG_TARGET,
            tick = failure.tick.0,
            failure = ?failure.failure,
            "an asset failure entered the world"
        );
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
    /// The session events the sync point refused (`take_session_failures`): a `Join` for
    /// a region the atlas lacks, or an event the game's applier failed.
    pub sessions: Vec<SessionFailure>,
    /// The asset failures the frame stamped (`take_asset_failures`).
    pub assets: Vec<AssetFailureEvent>,
}

impl LifecycleFailures {
    /// Whether nothing was refused.
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
    /// `loaded`. So the caller answers only the `Load`s whose region is not in
    /// [`Server::kept`] read after this step: a bundle it sends for a kept region once the
    /// server's own has loaded is refused as already loaded. A `Release` says the region's
    /// export is in the store now. Empty under `Directed`.
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
/// a `Gather` pass that no longer prepares, a save point's failed export or diff), so the
/// caller keeps its world; `None` after `Cancelled`, when the executor shut down during the
/// run and the world dropped with it, and after a step whose unload export could not be
/// diffed into the store, since that server has lost a region's saved state. Converts
/// into [`Error`], so `?` into a plain `Result` drops the server as a plain error would;
/// the policy's response is `Stop` either way.
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

#[cfg(test)]
mod tests {
    use std::sync::PoisonError;
    use std::sync::atomic::Ordering;

    use lactose_asset_api::VersionTable;

    use super::*;
    use crate::runtime::tests::SLOW_EXIT;

    /// Content with nothing cooked.
    struct Nothing;

    impl Cooked for Nothing {
        fn baseline(&self, _: u64) -> Result<Arc<Baseline>> {
            Err(Error::new(ErrorKind::NotFound, "nothing is cooked"))
        }

        fn table(&self, _: RegionId) -> Option<VersionTable> {
            None
        }

        fn pin(&self, _: &VersionTable) -> Result<PinnedTable> {
            Err(Error::new(ErrorKind::NotFound, "nothing is published"))
        }
    }

    static EMPTY: Game = Game { name: "empty", crates: &[], order: None, sessions: None };

    #[test]
    fn shutdown_waits_for_every_solver_thread_to_exit() {
        SLOW_EXIT.store(true, Ordering::Relaxed);
        let executor = Executor::builder().frame_workers(3).background_workers(1).build().unwrap();
        let server = build_server(
            &executor,
            &EMPTY,
            WorldConfig::headless(),
            Start::Fresh,
            Arc::new(Nothing),
        )
        .unwrap();
        let (threads, exited) = server.core.solver.counts();
        assert_eq!(threads, 2, "three frame workers less the one reserved");
        server.shutdown();
        assert_eq!(*exited.0.lock().unwrap_or_else(PoisonError::into_inner), threads);
        executor.shutdown();
    }
}
