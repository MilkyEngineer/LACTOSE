//! The simulation: a world, its schedules and its region lifecycle, stepped one tick at a
//! time from the timer root (architecture sections 5.1 and 11).

use core::fmt;
use core::future::Future;
use core::num::NonZeroU64;
use core::pin::Pin;
use core::task::{Context, Poll};

use lactose_core::{Error, Result, Tick};
use lactose_ecs::{Executor, LayoutConfig, RegionId, Schedule, World, WorldKind};

use crate::hash::{Hashers, RegionDigest, TypeTable};
use crate::input::{InputApplier, SessionApplier, StagedState, TickInput};
use crate::region::{RegionControl, RegionExport, RegionLifecycle, RegionPolicy};
use crate::schedule::Registration;

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
        todo!()
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

    /// Who decides loads and modes. `Anchored` on a world that is not `Server`, or with an
    /// empty atlas, fails [`build`](Self::build) with `InvalidData`.
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
    /// the entries older than that are trimmed; a consumer whose
    /// [`LogCursor`](lactose_ecs::LogCursor) has fallen further behind gets `None` from
    /// [`StructuralLog::since`](lactose_ecs::StructuralLog::since) and does a full pass, as
    /// the ECS already allows. No consumer trims the log itself.
    #[must_use]
    pub fn log_retention(mut self, ticks: u32) -> Self {
        self.log_retention = ticks;
        self
    }

    /// Adds a setup function: it is called once at [`build`](Self::build), after this
    /// crate's own resources and services are inserted and before any registration
    /// function runs, with the world to insert into. One per crate that contributes
    /// resources or services: configuration resources (`Res<Gravity>`), the physics
    /// backend's service, `net`'s gather state, the presentation staging ring. Every
    /// resource and service a registered system names is inserted by a setup function;
    /// `build` fails with `InvalidData` carrying
    /// [`ScheduleError::MissingResource`](lactose_ecs::ScheduleError::MissingResource)
    /// otherwise. A world resource inserted here is configuration or session state and
    /// so `derived`; `build` refuses any other policy (the README says why).
    #[must_use]
    pub fn setup(mut self, f: impl FnOnce(&mut World) -> Result<()> + Send + 'static) -> Self {
        self.setups.push(Box::new(f));
        self
    }

    /// Adds a registration function: it is called once at [`build`](Self::build), after
    /// every setup function and those registration functions added before it, with a
    /// [`Registration`] for this kind. One per crate that contributes systems (`game-sim`,
    /// `physics-rapier`, `anim`, `net`, `presentation`).
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

    /// The hook that spawns and despawns players' entities, inserted as the service
    /// resource `dyn SessionApplier`. Without one, a frame's sessions are dropped and
    /// logged.
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
    /// resource whose policy is not `derived` is present; runs every registration function
    /// in order; builds the tick schedule, the pass schedules and the save pass; and
    /// prepares each against the world, so that a schedule error surfaces here as
    /// `InvalidData` carrying the [`ScheduleError`](lactose_ecs::ScheduleError), never in a
    /// step.
    ///
    /// Fails with `InvalidData` as the setters say, as a setup function does, as
    /// [`WorldBuilder::build`](lactose_ecs::WorldBuilder::build) does, for a non-`derived`
    /// world resource, and for a schedule that does not build or prepare.
    pub fn build(self) -> Result<Simulation> {
        todo!()
    }
}

/// A world with its tick schedule, its pass schedules, its hashers and its region lifecycle:
/// what the timer root owns and steps.
///
/// Between runs the world is reachable for direct access: a snapshot, a restore, a
/// resource, a query. Every run ([`step`](Self::step), [`run_passes`](Self::run_passes),
/// [`save_point`](Self::save_point)) takes the simulation by value and hands it back when
/// it resolves, as [`Schedule::run_tick`] does with the world, because the run shares the
/// world with jobs on other workers; what fails before anything ran hands it back inside
/// the [`RunError`].
pub struct Simulation {
    kind: WorldKind,
    world: World,
    tick_schedule: Schedule,
    /// Indexed by [`Pass`](crate::Pass); `None` for a pass with no system.
    passes: [Option<Schedule>; 2],
    /// The save pass: one stage holding `sim::save_copy`.
    save: Schedule,
    hashers: Hashers,
    log_retention: u32,
}

impl Simulation {
    /// The kind of world.
    #[must_use]
    pub fn kind(&self) -> WorldKind {
        self.kind
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
        &self.hashers
    }

    /// The world hash of `region` (architecture section 8.3): [`Hashers::world_hash`] over
    /// this world.
    pub fn world_hash(&self, region: RegionId) -> Result<u64> {
        todo!()
    }

    /// The hash of the whole simulation at this tick, what a SOUR test records: the tick
    /// and every loaded region's [`world_hash`](Self::world_hash) in ascending `RegionId`
    /// order, combined as the README states. A world with no region hashes its tick alone.
    pub fn hash(&self) -> Result<u64> {
        todo!()
    }

    /// The field-level digest of `region`: [`Hashers::digest`] over this world.
    pub fn digest(&self, region: RegionId) -> Result<RegionDigest> {
        todo!()
    }

    /// The region lifecycle: its requests, exports and failures.
    #[must_use]
    pub fn lifecycle(&self) -> &RegionLifecycle {
        todo!()
    }

    /// The region lifecycle, to take its outputs.
    pub fn lifecycle_mut(&mut self) -> &mut RegionLifecycle {
        todo!()
    }

    /// Runs one tick with `input` applied at its input sync point, trims the structural log
    /// to the retention, and hands the simulation back at the next tick. Nothing else: the
    /// passes are [`run_passes`](Self::run_passes). The README states what the sync point
    /// does and in what order; the signature states that a step takes no clock and nothing
    /// but `input`.
    ///
    /// The future is `Send` and is driven by the `'static` task that owns the simulation.
    /// It resolves to a [`RunError`] with the simulation inside if a schedule no longer
    /// prepares against the world (`InvalidData`), checked before anything runs; and with
    /// `None` after `Cancelled`, when the executor shut down mid-run and the world is torn.
    pub fn step(self, executor: &Executor, input: TickInput) -> Step<'_> {
        todo!()
    }

    /// Runs the passes of this frame: [`Pass::Extract`](crate::Pass::Extract) then
    /// [`Pass::Gather`](crate::Pass::Gather), each with [`Schedule::run_pass`] and only if
    /// a system was registered for it, then on a `Server` world takes and drops the
    /// presentation events so the queue does not grow (every other kind leaves them for
    /// `predict` or presentation). The timer root calls it once per frame after the frame's
    /// ticks, paused or not, so what was edited between ticks is still extracted and
    /// replicated; `lactose-predict`'s re-simulation never calls it. Fails as a step does.
    pub fn run_passes(self, executor: &Executor) -> PassRun<'_> {
        todo!()
    }

    /// Copies every loaded region out at this sync point (architecture section 15.3): the
    /// save pass runs `sim::save_copy`, an exclusive system that forks one child per loaded
    /// region over [`World::export_region`], and hands back the simulation unchanged with
    /// the [`SavePoint`]. The same export an unload makes. On a world that is not `Server`
    /// it resolves at once to `Unsupported` with the simulation inside: a client never
    /// exports, and single-player runs a `Server` world in-process with a local client.
    /// Otherwise fails as a step does.
    pub fn save_point(self, executor: &Executor) -> SavePointRun<'_> {
        todo!()
    }
}

/// How a run fails: the error, and the simulation when nothing ran. `Some` when the kind
/// check or [`Schedule::prepare`] failed before the run started, so the caller keeps its
/// world; `None` after `Cancelled`, when the executor shut down during the run and the
/// world dropped with it. Converts into [`Error`], so `?` into a `lactose_core::Result`
/// drops the simulation as a plain error would.
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

/// One tick in flight: resolves to the simulation at the next tick. See
/// [`Simulation::step`].
#[must_use = "futures do nothing unless awaited"]
pub struct Step<'a> {
    run: Run<'a, Simulation>,
}

impl Future for Step<'_> {
    type Output = core::result::Result<Simulation, RunError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        todo!()
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

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        todo!()
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

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        todo!()
    }
}

/// Everything a save holds beyond the records: what `lactose-persist` serialises with the
/// regions' overlays, and what it hands back to [`SimulationBuilder::tick`],
/// [`SimulationBuilder::next_net_id`] and, region by region through
/// [`TickInput::loaded`](crate::TickInput::loaded), on a load. World resources are not in
/// it: they are configuration or session state, never saved.
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
pub(crate) struct SaveSlot(pub(crate) Option<SavePoint>);
