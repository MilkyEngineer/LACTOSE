//! The simulation: a world, its schedules and its region lifecycle, stepped one tick at a
//! time from the timer root (architecture sections 5.1 and 11).

use core::fmt;
use core::future::Future;
use core::num::NonZeroU64;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::collections::VecDeque;
use std::sync::Arc;

use lactose_core::{Error, ErrorKind, Result, Tick, xxh64};
use lactose_ecs::{
    Executor, LayoutConfig, LogCursor, RegionId, Schedule, ScheduleError, World, WorldBuilder,
    WorldKind,
};

use crate::hash::{Hashers, RegionDigest, TypeTable};
use crate::input::{InputApplier, SessionApplier, StagedState, TickInput};
use crate::region::{RegionControl, RegionExport, RegionLifecycle, RegionPolicy};
use crate::schedule::Registration;
use crate::systems::{NoInputs, NoSessions, NothingStaged, SAVE_COPY};
use crate::time::{CurrentTick, TICK_RATE_HZ, TickDelta, TimeScale};

/// The save pass's one stage.
const SAVE_STAGE: &str = "sim::save";

/// A setup function, boxed.
type Setup = Box<dyn FnOnce(&mut World) -> Result<()> + Send>;

/// A registration function, boxed.
type Register = Box<dyn FnOnce(&mut Registration) + Send>;

/// A run that resolves to `T`, or to the error with the simulation when nothing ran.
type Run<'a, T> = Pin<Box<dyn Future<Output = core::result::Result<T, RunError>> + Send + 'a>>;

/// Builds a [`Simulation`].
///
/// Defaults: the production layout, tick zero, a fresh `NetId` counter,
/// [`RegionControl::Directed`], a policy of activation radius 1, streaming radius 2 and
/// release radius 3, a structural-log retention of
/// [`DEFAULT_LOG_RETENTION`](Self::DEFAULT_LOG_RETENTION) ticks, no setup or
/// registration function, and hooks that do nothing.
pub struct SimulationBuilder {
    kind: WorldKind,
    types: TypeTable,
    layout: LayoutConfig,
    tick: Tick,
    next_net_id: NonZeroU64,
    control: RegionControl,
    policy: RegionPolicy,
    log_retention: u32,
    setups: Vec<Setup>,
    registrations: Vec<Register>,
    inputs: Option<Box<dyn InputApplier>>,
    sessions: Option<Box<dyn SessionApplier>>,
    staged: Option<Box<dyn StagedState>>,
}

impl SimulationBuilder {
    /// How many ticks of structural log a step keeps by default: two seconds at
    /// [`TICK_RATE_HZ`](crate::TICK_RATE_HZ).
    pub const DEFAULT_LOG_RETENTION: u32 = 120;

    /// A simulation of `kind` over the types of `types`.
    #[must_use]
    pub fn new(kind: WorldKind, types: TypeTable) -> Self {
        Self {
            kind,
            types,
            layout: LayoutConfig::default(),
            tick: Tick(0),
            next_net_id: NonZeroU64::MIN,
            control: RegionControl::Directed,
            policy: RegionPolicy { activation_radius: 1, streaming_radius: 2, release_radius: 3 },
            log_retention: Self::DEFAULT_LOG_RETENTION,
            setups: Vec::new(),
            registrations: Vec::new(),
            inputs: None,
            sessions: None,
            staged: None,
        }
    }

    /// The storage layout: SOUR's layout mode ([`harness::layout_mode`](crate::harness::layout_mode))
    /// or the production default.
    #[must_use]
    pub fn layout(mut self, layout: LayoutConfig) -> Self {
        self.layout = layout;
        self
    }

    /// The tick the world starts at: the server's tick on a client, the saved tick on a
    /// load ([`SavePoint::tick`]). The first step runs the tick after it.
    #[must_use]
    pub fn tick(mut self, tick: Tick) -> Self {
        self.tick = tick;
        self
    }

    /// The counter the next runtime `NetId` is allocated from, from a save
    /// ([`SavePoint::next_net_id`]).
    #[must_use]
    pub fn next_net_id(mut self, counter: NonZeroU64) -> Self {
        self.next_net_id = counter;
        self
    }

    /// Who decides loads and modes. `Anchored` on a world that is not `Server`, with an
    /// empty atlas, or with a pinned region the atlas lacks, fails [`build`](Self::build)
    /// with `InvalidData`.
    #[must_use]
    pub fn regions(mut self, control: RegionControl) -> Self {
        self.control = control;
        self
    }

    /// The activation, streaming and release radii, inserted as the [`RegionPolicy`]
    /// resource. Anything but `release_radius >= streaming_radius > activation_radius`
    /// fails [`build`](Self::build) with `InvalidData`.
    #[must_use]
    pub fn policy(mut self, policy: RegionPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// How many ticks of the structural change log a step keeps. At the end of every step
    /// the entries older than that are trimmed: the step keeps a ring of one
    /// [`StructuralLog::cursor`](lactose_ecs::StructuralLog::cursor) per step and trims to
    /// the oldest with [`World::trim_structural_log`], which ignores a cursor made stale by
    /// a restore, a load or the stamp rebase, so those heal on their own. A consumer whose
    /// [`LogCursor`](lactose_ecs::LogCursor) has fallen further behind gets `None` from
    /// [`StructuralLog::since`](lactose_ecs::StructuralLog::since) and does a full pass, as
    /// the ECS already allows. No consumer trims the log itself. `ticks` must be at least the
    /// timer root's catch-up cap (`TickPacer`'s, `app`'s choice): a consumer that reads the log
    /// once per frame, such as the replication gather, otherwise finds its cursor stale after
    /// every catch-up frame and does a full pass each time. Not checked here, since the cap is
    /// `app`'s.
    #[must_use]
    pub fn log_retention(mut self, ticks: u32) -> Self {
        self.log_retention = ticks;
        self
    }

    /// Adds a setup function: it is called once at [`build`](Self::build), after this
    /// crate's own resources and services are inserted and before any registration
    /// function runs, with the world to insert into. One per crate that contributes
    /// resources or services: configuration resources (`Res<Gravity>`), the physics
    /// backend's service with the solver pool `app` built outside the boundary, `net`'s
    /// gather state, the presentation staging ring. Every resource and service a
    /// registered system names is inserted by a setup function; `build` fails with
    /// `InvalidData` carrying
    /// [`ScheduleError::MissingResource`](lactose_ecs::ScheduleError::MissingResource)
    /// otherwise. A world resource inserted here is configuration or session state and
    /// so `derived`; `build` refuses any other policy among the resource types the
    /// [`TypeTable`] knows (the README says why and what the check does not cover).
    #[must_use]
    pub fn setup(mut self, f: impl FnOnce(&mut World) -> Result<()> + Send + 'static) -> Self {
        self.setups.push(Box::new(f));
        self
    }

    /// Adds a registration function: it is called once at [`build`](Self::build), after
    /// every setup function and those registration functions added before it, with a
    /// [`Registration`] for this kind. One per crate that contributes systems (`game-sim`,
    /// `physics-rapier`, `anim`, `net`, `presentation`), and one of `app`'s own for the
    /// [`Registration::order`] edges between other crates' sets.
    #[must_use]
    pub fn systems(mut self, register: impl FnOnce(&mut Registration) + Send + 'static) -> Self {
        self.registrations.push(Box::new(register));
        self
    }

    /// The hook that writes players' inputs, inserted as the service resource
    /// `dyn InputApplier`. Without one, a frame's inputs are dropped and logged after
    /// their input ticks are set.
    #[must_use]
    pub fn inputs(mut self, applier: Box<dyn InputApplier>) -> Self {
        self.inputs = Some(applier);
        self
    }

    /// The hook that spawns, re-claims and despawns players' entities, inserted as the
    /// service resource `dyn SessionApplier`. Without one, a frame's sessions are dropped
    /// and logged.
    #[must_use]
    pub fn sessions(mut self, applier: Box<dyn SessionApplier>) -> Self {
        self.sessions = Some(applier);
        self
    }

    /// The hook that applies staged state, inserted as the service resource
    /// `dyn StagedState`. Without one, nothing is staged.
    #[must_use]
    pub fn staged_state(mut self, staged: Box<dyn StagedState>) -> Self {
        self.staged = Some(staged);
        self
    }

    /// Builds the world with the registry, the layout, the tick and the counter; inserts
    /// this crate's resources ([`TickDelta`](crate::TickDelta),
    /// [`CurrentTick`](crate::CurrentTick), [`TimeScale`](crate::TimeScale) at
    /// `REAL_TIME`, [`RegionPolicy`]) and services (the three hooks, the lifecycle, the
    /// frame and save slots); runs every setup function in order, then checks that no world
    /// resource of a registered type whose policy is not `derived` is present; runs every
    /// registration function in order; builds the tick schedule, the pass schedules and
    /// the save pass with the orders their sets belong to; and prepares each against the
    /// world, so that a schedule error surfaces here as `InvalidData` carrying the
    /// [`ScheduleError`](lactose_ecs::ScheduleError), never in a run.
    ///
    /// Fails with `InvalidData` as the setters say, as a setup function does, as
    /// [`WorldBuilder::build`](lactose_ecs::WorldBuilder::build) does, for a non-`derived`
    /// world resource, and for a schedule that does not build or prepare.
    pub fn build(self) -> Result<Simulation> {
        let invalid = |message: &'static str| Err(Error::new(ErrorKind::InvalidData, message));
        let policy = self.policy;
        if !(policy.release_radius >= policy.streaming_radius
            && policy.streaming_radius > policy.activation_radius)
        {
            return invalid(
                "the policy needs release_radius >= streaming_radius > activation_radius",
            );
        }
        if let RegionControl::Anchored { atlas, pinned } = &self.control {
            if self.kind != WorldKind::Server {
                return invalid("only a `Server` world is `Anchored`");
            }
            if atlas.is_empty() {
                return invalid("an `Anchored` world needs a non-empty atlas");
            }
            if pinned.iter().any(|&region| atlas.cell(region).is_none()) {
                return invalid("a pinned region is not in the atlas");
            }
        }

        let (registry, hashers, probes) = self.types.into_parts();
        let mut world = WorldBuilder::new(self.kind, Arc::new(registry))
            .layout(self.layout)
            .tick(self.tick)
            .next_net_id(self.next_net_id)
            .build()?;
        world.insert_resource(TickDelta::of_rate(TICK_RATE_HZ));
        world.insert_resource(CurrentTick::new(self.tick));
        world.insert_resource(TimeScale::REAL_TIME);
        world.insert_resource(policy);
        let inputs = self.inputs.unwrap_or_else(|| Box::new(NoInputs));
        let sessions = self.sessions.unwrap_or_else(|| Box::new(NoSessions));
        let staged = self.staged.unwrap_or_else(|| Box::new(NothingStaged));
        world.insert_service::<dyn InputApplier>(inputs);
        world.insert_service::<dyn SessionApplier>(sessions);
        world.insert_service::<dyn StagedState>(staged);
        world.insert_service(Box::new(LifecycleSlot(RegionLifecycle::new(self.control))));
        world.insert_service(Box::new(TickInputSlot(None)));
        world.insert_service(Box::new(SaveSlot(None)));

        for setup in self.setups {
            setup(&mut world)?;
        }
        for (key, present) in probes {
            if present(&world) {
                let name = world.registry().get(key).map_or("?", |info| info.name());
                tracing::warn!(
                    target: "curd",
                    resource = name,
                    "a world resource that is not `derived`: refused"
                );
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!(
                        "the world resource `{name}` is not `derived`: world resources are \
                         neither hashed nor saved, so simulation state is a component or a \
                         region resource"
                    ),
                ));
            }
        }

        let mut registration = Registration::new(self.kind);
        for register in self.registrations {
            register(&mut registration);
        }
        let (mut tick_schedule, mut passes) = registration.build().map_err(schedule_error)?;
        let mut save = Schedule::builder(self.kind);
        save.stage(SAVE_STAGE, &[]).add(&SAVE_COPY);
        let mut save = save.build().map_err(schedule_error)?;
        tick_schedule.prepare(&world).map_err(schedule_error)?;
        for pass in passes.iter_mut().flatten() {
            pass.prepare(&world).map_err(schedule_error)?;
        }
        save.prepare(&world).map_err(schedule_error)?;

        // The ring starts with the log's end at build, so the first steps keep their entries
        // like every later one: trimmed only once `log_retention` steps have passed them.
        let built = world.structural_log().cursor();
        Ok(Simulation {
            world,
            core: Core {
                kind: self.kind,
                tick_schedule,
                passes,
                save,
                hashers,
                log_retention: self.log_retention,
                log_cursors: VecDeque::from([built]),
            },
        })
    }
}

/// A schedule error as the error a build or a run reports: `InvalidData` carrying it.
fn schedule_error(error: ScheduleError) -> Error {
    Error::new(ErrorKind::InvalidData, error.to_string()).with_source(error)
}

/// A world with its tick schedule, its pass schedules, its hashers and its region lifecycle:
/// what the timer root owns and runs.
///
/// Between runs the world is reachable for direct access: a snapshot, a restore, a
/// resource, a query. Every run ([`step`](Self::step), [`run_passes`](Self::run_passes),
/// [`save_point`](Self::save_point)) takes the simulation by value and hands it back when
/// it resolves, as [`Schedule::run_tick`] does with the world, because the run shares the
/// world with jobs on other workers; what fails before anything ran hands it back inside
/// the [`RunError`].
pub struct Simulation {
    world: World,
    /// Everything but the world, which a run moves out and back.
    core: Core,
}

/// A simulation without its world.
struct Core {
    kind: WorldKind,
    tick_schedule: Schedule,
    /// Indexed by [`Pass`](crate::Pass); `None` for a pass with no system.
    passes: [Option<Schedule>; 2],
    /// The save pass: one stage holding `sim::save_copy`.
    save: Schedule,
    hashers: Hashers,
    log_retention: u32,
    /// The structural log's end at build and after each step, the last `log_retention + 1`
    /// of them, oldest first: the log is trimmed to the oldest.
    log_cursors: VecDeque<LogCursor>,
}

impl Core {
    /// Puts `world` back.
    fn join(self, world: World) -> Simulation {
        Simulation { world, core: self }
    }
}

impl Simulation {
    /// The kind of world.
    #[must_use]
    pub fn kind(&self) -> WorldKind {
        self.core.kind
    }

    /// The current tick: the last tick run, or the start tick before the first step.
    #[must_use]
    pub fn tick(&self) -> Tick {
        self.world.tick()
    }

    /// The world, for reading between runs.
    #[must_use]
    pub fn world(&self) -> &World {
        &self.world
    }

    /// The world, for direct access between runs: a snapshot or restore by
    /// `lactose-predict`, [`set_rollback_horizon`](World::set_rollback_horizon), a service
    /// hook replaced after build, a region created directly by a test. Everything written
    /// here is an unjournaled edit: `app` never writes simulation state this way, and the
    /// [`RegionPolicy`] is the one resource it changes here. Removing a resource or service
    /// a registered system names makes the next run fail with the simulation handed back.
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// The per-type hashers of this simulation's types.
    #[must_use]
    pub fn hashers(&self) -> &Hashers {
        &self.core.hashers
    }

    /// The world hash of `region` (architecture section 8.3): [`Hashers::world_hash`] over
    /// this world.
    pub fn world_hash(&self, region: RegionId) -> Result<u64> {
        self.core.hashers.world_hash(&self.world, region)
    }

    /// The hash of the whole simulation at this tick, what a SOUR test records: the tick
    /// and every loaded region's [`world_hash`](Self::world_hash) in ascending `RegionId`
    /// order, combined as the README states. A world with no region hashes its tick alone.
    pub fn hash(&self) -> Result<u64> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&self.world.tick().0.to_le_bytes());
        for region in self.world.regions() {
            bytes.extend_from_slice(&region.0.to_le_bytes());
            bytes.extend_from_slice(&self.world_hash(region)?.to_le_bytes());
        }
        Ok(xxh64(&bytes, 0))
    }

    /// The field-level digest of `region`: [`Hashers::digest`] over this world.
    pub fn digest(&self, region: RegionId) -> Result<RegionDigest> {
        self.core.hashers.digest(&self.world, region)
    }

    /// The region lifecycle: its requests, exports and failures.
    #[must_use]
    pub fn lifecycle(&self) -> &RegionLifecycle {
        // The slot's type is private to this crate, so nothing else can remove it.
        &self.world.service::<LifecycleSlot>().expect("the builder inserts the lifecycle").0
    }

    /// The region lifecycle, to take its outputs.
    pub fn lifecycle_mut(&mut self) -> &mut RegionLifecycle {
        &mut self.world.service_mut::<LifecycleSlot>().expect("the builder inserts the lifecycle").0
    }

    /// Runs one tick with `input` applied at its input sync point, trims the structural log
    /// to the retention, on a `Server` world takes and drops the presentation events so the
    /// queue does not grow (every other kind leaves them for `predict` or presentation),
    /// and hands the simulation back at the next tick. Nothing else: the passes are
    /// [`run_passes`](Self::run_passes). The README states what the sync point does and in
    /// what order; the signature states that a step takes no clock and nothing but `input`,
    /// which is consumed: a caller that may retry keeps a clone.
    ///
    /// The future is `Send` and is driven by the `'static` task that owns the simulation.
    /// It resolves to a [`RunError`] with the simulation inside if the tick schedule no
    /// longer prepares against the world (`InvalidData`), checked before anything runs; and
    /// with `None` after `Cancelled`, when the executor shut down mid-run and the world is
    /// torn.
    pub fn step(self, executor: &Executor, input: TickInput) -> Step<'_> {
        Step { run: Box::pin(self.run_step(executor, input)) }
    }

    #[expect(
        clippy::result_large_err,
        reason = "the future's output the facade fixed: a run hands back the simulation by \
                  value, once per tick"
    )]
    async fn run_step(
        mut self,
        executor: &Executor,
        input: TickInput,
    ) -> core::result::Result<Self, RunError> {
        if let Err(error) = self.core.tick_schedule.prepare(&self.world) {
            return Err(RunError::kept(schedule_error(error), self));
        }
        if matches!(self.core.kind, WorldKind::Server | WorldKind::Client) {
            let tick = self.world.tick();
            self.world.set_rollback_horizon(tick);
        }
        slot(&mut self.world).0 = Some(input);
        let Self { world, mut core } = self;
        let mut world =
            core.tick_schedule.run_tick(world, executor).await.map_err(RunError::lost)?;
        slot(&mut world).0 = None;

        let retention = usize::try_from(core.log_retention).unwrap_or(usize::MAX);
        core.log_cursors.push_back(world.structural_log().cursor());
        while core.log_cursors.len() > retention.saturating_add(1) {
            core.log_cursors.pop_front();
        }
        if let Some(&oldest) = core.log_cursors.front() {
            world.trim_structural_log(oldest);
        }
        if core.kind == WorldKind::Server {
            drop(world.take_present_events());
        }
        Ok(core.join(world))
    }

    /// Runs the passes of this frame: [`Pass::Extract`](crate::Pass::Extract) then
    /// [`Pass::Gather`](crate::Pass::Gather), each with [`Schedule::run_pass`] and only if
    /// a system was registered for it, both prepared before either runs so that a failure
    /// before anything ran hands the simulation back. The timer root calls it once per
    /// frame after the frame's ticks, paused or not, on the server and on both client
    /// worlds, so what was edited between ticks is still extracted and replicated and the
    /// prediction world's local player is extracted every frame; a gather therefore covers
    /// every tick since the last frame, up to the catch-up cap. `lactose-predict`'s
    /// re-simulation calls [`step`](Self::step) alone. Fails as a step does; `Gather` is
    /// prepared again once `Extract` has run, and if that fails the simulation comes back
    /// with `Extract`'s work done and `Gather`'s not.
    pub fn run_passes(self, executor: &Executor) -> PassRun<'_> {
        PassRun { run: Box::pin(self.run_passes_inner(executor)) }
    }

    #[expect(
        clippy::result_large_err,
        reason = "the future's output the facade fixed: a run hands back the simulation by \
                  value, once per tick"
    )]
    async fn run_passes_inner(
        mut self,
        executor: &Executor,
    ) -> core::result::Result<Self, RunError> {
        let world = &self.world;
        let prepared =
            self.core.passes.iter_mut().flatten().try_for_each(|pass| pass.prepare(world));
        if let Err(error) = prepared {
            return Err(RunError::kept(schedule_error(error), self));
        }
        let Self { mut world, mut core } = self;
        for index in 0..core.passes.len() {
            let Some(pass) = core.passes[index].as_mut() else { continue };
            // `Extract` may have removed what `Gather` names; checked again here, `run_pass`
            // would fail the same way with the world inside it.
            if let Err(error) = pass.prepare(&world) {
                return Err(RunError::kept(schedule_error(error), core.join(world)));
            }
            world = pass.run_pass(world, executor).await.map_err(RunError::lost)?;
        }
        Ok(core.join(world))
    }

    /// Copies every loaded region out at this sync point (architecture section 15.3): the
    /// save pass runs `sim::save_copy`, an exclusive system that forks one child per loaded
    /// region over [`World::export_region`], and hands back the simulation unchanged with
    /// the [`SavePoint`]. The same export an unload makes. On a world that is not `Server`
    /// it resolves at once to `Unsupported` with the simulation inside: a client never
    /// exports, and single-player runs a `Server` world in-process with a local client.
    /// Otherwise fails as a step does.
    pub fn save_point(self, executor: &Executor) -> SavePointRun<'_> {
        SavePointRun { run: Box::pin(self.run_save(executor)) }
    }

    #[expect(
        clippy::result_large_err,
        reason = "the future's output the facade fixed: a run hands back the simulation by \
                  value, once per tick"
    )]
    async fn run_save(
        mut self,
        executor: &Executor,
    ) -> core::result::Result<(Self, SavePoint), RunError> {
        if self.core.kind != WorldKind::Server {
            let error =
                Error::new(ErrorKind::Unsupported, "only a `Server` world exports a save point");
            return Err(RunError::kept(error, self));
        }
        if let Err(error) = self.core.save.prepare(&self.world) {
            return Err(RunError::kept(schedule_error(error), self));
        }
        let Self { world, mut core } = self;
        let mut world = core.save.run_pass(world, executor).await.map_err(RunError::lost)?;
        let copied = world
            .service_mut::<SaveSlot>()
            .expect("the builder inserts the save slot")
            .0
            .take()
            .expect("the save pass ran `sim::save_copy`");
        let simulation = core.join(world);
        match copied {
            Ok(save) => Ok((simulation, save)),
            // An export reads and changes nothing, so the simulation is as it was.
            Err(error) => Err(RunError::kept(error, simulation)),
        }
    }
}

/// The frame slot of a world the builder made.
fn slot(world: &mut World) -> &mut TickInputSlot {
    world.service_mut::<TickInputSlot>().expect("the builder inserts the frame slot")
}

/// How a run fails: the error, and the simulation when the world survived. `Some` when the
/// kind check or [`Schedule::prepare`] failed before the run started, so the caller keeps
/// its world; when the `Gather` pass no longer prepares after `Extract` ran (the simulation
/// then holds what `Extract` did); and when a save point's export failed, which changes
/// nothing. `None` after `Cancelled`, when the executor shut down during the run and the
/// world dropped with it. The frame a step was given is not handed back either way.
/// Converts into [`Error`], so `?` into a `lactose_core::Result` drops the simulation as a
/// plain error would.
pub struct RunError {
    /// What failed.
    pub error: Error,
    /// The simulation, if the failure came before anything ran.
    pub simulation: Option<Simulation>,
}

impl fmt::Debug for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunError")
            .field("error", &self.error)
            .field("simulation", &self.simulation.is_some())
            .finish()
    }
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, f)
    }
}

impl std::error::Error for RunError {}

impl From<RunError> for Error {
    fn from(failure: RunError) -> Self {
        failure.error
    }
}

impl RunError {
    /// A failure before anything ran: the simulation goes back.
    fn kept(error: Error, simulation: Simulation) -> Self {
        Self { error, simulation: Some(simulation) }
    }

    /// A failure of the run itself, after which the world is gone.
    fn lost(error: Error) -> Self {
        Self { error, simulation: None }
    }
}

/// One tick in flight: resolves to the simulation at the next tick. See
/// [`Simulation::step`].
#[must_use = "futures do nothing unless awaited"]
pub struct Step<'a> {
    run: Run<'a, Simulation>,
}

impl Future for Step<'_> {
    type Output = core::result::Result<Simulation, RunError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.run.as_mut().poll(cx)
    }
}

/// A frame's passes in flight: resolves to the simulation unchanged in tick. See
/// [`Simulation::run_passes`].
#[must_use = "futures do nothing unless awaited"]
pub struct PassRun<'a> {
    run: Run<'a, Simulation>,
}

impl Future for PassRun<'_> {
    type Output = core::result::Result<Simulation, RunError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.run.as_mut().poll(cx)
    }
}

/// A save copy in flight: resolves to the simulation and the copy. See
/// [`Simulation::save_point`].
#[must_use = "futures do nothing unless awaited"]
pub struct SavePointRun<'a> {
    run: Run<'a, (Simulation, SavePoint)>,
}

impl Future for SavePointRun<'_> {
    type Output = core::result::Result<(Simulation, SavePoint), RunError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.run.as_mut().poll(cx)
    }
}

/// Everything a save holds beyond the records: what `lactose-persist` serialises with the
/// regions' overlays, and what it hands back to [`SimulationBuilder::tick`],
/// [`SimulationBuilder::next_net_id`] and, region by region through
/// [`TickInput::loaded`](crate::TickInput::loaded), on a load. World resources are not in
/// it: they are configuration or session state, never saved. Nor is session state: a save
/// carries no players, held joins or requests, so on a load every player joins again.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SavePoint {
    /// The tick the copy was taken at.
    pub tick: Tick,
    /// The runtime `NetId` counter (architecture section 7), so RNG streams and ordering
    /// survive a load.
    pub next_net_id: NonZeroU64,
    /// Every loaded region, in ascending `RegionId` order.
    pub regions: Vec<RegionExport>,
}

/// The frame the next step applies, held as a service resource so the sync point systems
/// can take it. Plumbing: the implementation may reshape it.
pub(crate) struct TickInputSlot(pub(crate) Option<TickInput>);

/// The lifecycle, held as a service resource under a type no other crate can name, so no
/// system can take it. Plumbing: the implementation may reshape it.
pub(crate) struct LifecycleSlot(pub(crate) RegionLifecycle);

/// Where `sim::save_copy` leaves the copy. Plumbing: the implementation may reshape it.
pub(crate) struct SaveSlot(pub(crate) Option<Result<SavePoint>>);
