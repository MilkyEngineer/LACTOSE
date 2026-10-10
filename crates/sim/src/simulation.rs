//! The simulation: a world, its schedules and its region lifecycle, stepped one tick at a
//! time from the timer root (architecture sections 5.1 and 11).

use core::future::Future;
use core::num::NonZeroU64;
use core::pin::Pin;
use core::task::{Context, Poll};

use lactose_core::{Result, Tick};
use lactose_ecs::{Executor, LayoutConfig, RegionId, Schedule, World, WorldKind};

use crate::hash::{Hashers, RegionDigest, TypeTable};
use crate::input::{InputApplier, StagedState, TickInput};
use crate::region::{RegionControl, RegionExport, RegionLifecycle, RegionPolicy};
use crate::schedule::Registration;
use crate::time::TimeScale;

/// A registration function, boxed.
type Register = Box<dyn FnOnce(&mut Registration) + Send>;

/// A run that hands the simulation back with `T`.
type Run<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Builds a [`Simulation`].
///
/// Defaults: the production layout, tick zero, a fresh `NetId` counter,
/// [`RegionControl::Directed`], a policy of activation radius 1 and streaming radius 2,
/// [`TimeScale::REAL_TIME`], no registration function, and hooks that do nothing.
pub struct SimulationBuilder {
    kind: WorldKind,
    types: TypeTable,
    layout: LayoutConfig,
    tick: Tick,
    next_net_id: NonZeroU64,
    control: RegionControl,
    policy: RegionPolicy,
    time_scale: TimeScale,
    registrations: Vec<Register>,
    inputs: Option<Box<dyn InputApplier>>,
    staged: Option<Box<dyn StagedState>>,
}

impl SimulationBuilder {
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

    /// The activation and streaming radii, inserted as the [`RegionPolicy`] resource. A
    /// streaming radius not greater than the activation radius fails
    /// [`build`](Self::build) with `InvalidData`.
    #[must_use]
    pub fn policy(mut self, policy: RegionPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// The initial time scale, inserted as the [`TimeScale`] resource.
    #[must_use]
    pub fn time_scale(mut self, scale: TimeScale) -> Self {
        self.time_scale = scale;
        self
    }

    /// Adds a registration function: it is called once at [`build`](Self::build), after
    /// those added before it, with a [`Registration`] for this kind. One per crate that
    /// contributes systems (`game-sim`, `physics-rapier`, `anim`, `net`, `presentation`).
    #[must_use]
    pub fn systems(mut self, register: impl FnOnce(&mut Registration) + Send + 'static) -> Self {
        self.registrations.push(Box::new(register));
        self
    }

    /// The hook that writes players' inputs, inserted as the service resource
    /// `dyn InputApplier`. Without one, a frame with inputs drops them and logs.
    #[must_use]
    pub fn inputs(mut self, applier: Box<dyn InputApplier>) -> Self {
        self.inputs = Some(applier);
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
    /// the resources ([`TickDelta`](crate::TickDelta), [`CurrentTick`](crate::CurrentTick),
    /// [`TimeScale`], [`RegionPolicy`]) and the services (the two hooks, the lifecycle, the
    /// frame slot); runs every registration function; builds the tick schedule, the pass
    /// schedules and the save pass; and prepares each against the world, so that a
    /// schedule error surfaces here as `InvalidData` carrying the
    /// [`ScheduleError`](lactose_ecs::ScheduleError), not in a step.
    ///
    /// Fails with `InvalidData` as the setters say, as
    /// [`WorldBuilder::build`](lactose_ecs::WorldBuilder::build) does, and for a schedule
    /// that does not build or prepare.
    pub fn build(self) -> Result<Simulation> {
        todo!()
    }
}

/// A world with its tick schedule, its pass schedules, its hashers and its region lifecycle:
/// what the timer root owns and steps.
///
/// Between steps the world is reachable for direct access: a snapshot, a restore, a
/// resource, a query. [`step`](Self::step) takes the simulation by value and hands it back
/// when the tick resolves, as [`Schedule::run_tick`] does with the world, because the run
/// shares the world with jobs on other workers.
pub struct Simulation {
    kind: WorldKind,
    world: World,
    tick_schedule: Schedule,
    /// Indexed by [`Pass`](crate::Pass); `None` for a pass with no stages.
    passes: [Option<Schedule>; 2],
    /// The save pass: one stage holding `sim::save_copy`.
    save: Schedule,
    hashers: Hashers,
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

    /// The world, for reading between steps.
    #[must_use]
    pub fn world(&self) -> &World {
        &self.world
    }

    /// The world, for direct access between steps: a snapshot or restore by
    /// `lactose-predict`, [`set_rollback_horizon`](World::set_rollback_horizon), a resource
    /// such as [`TimeScale`] or [`RegionPolicy`], a service hook replaced after build.
    /// Removing a resource or service a registered system names makes the next step fail.
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

    /// Runs one tick with `input` applied at its input sync point, then the passes, and
    /// hands the simulation back at the next tick. The README states what the step does and
    /// in what order; the signature states that it takes no clock and nothing but `input`.
    ///
    /// The future is `Send` and is driven by the `'static` task that owns the simulation.
    /// Errors, on none of which the simulation is handed back: kind `Cancelled` if the
    /// executor shut down first; kind `InvalidData` if a schedule no longer prepares against
    /// the world, which only a resource removed between steps can cause.
    pub fn step(self, executor: &Executor, input: TickInput) -> Step<'_> {
        todo!()
    }

    /// Copies every loaded region out at this sync point (architecture section 15.3): the
    /// save pass runs `sim::save_copy`, an exclusive system that forks one child per loaded
    /// region over [`World::export_region`], and hands back the simulation unchanged with
    /// the [`SavePoint`]. The same export an unload makes. Fails with `Unsupported` on a
    /// world that is not `Server` (a client never exports), and otherwise as a step does.
    pub fn save_point(self, executor: &Executor) -> SavePointRun<'_> {
        todo!()
    }
}

/// One tick in flight: resolves to the simulation at the next tick. See
/// [`Simulation::step`].
#[must_use = "futures do nothing unless awaited"]
pub struct Step<'a> {
    run: Run<'a, Simulation>,
}

impl Future for Step<'_> {
    type Output = Result<Simulation>;

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
    type Output = Result<(Simulation, SavePoint)>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        todo!()
    }
}

/// Everything a save holds beyond the records: what `lactose-persist` serialises with the
/// regions' overlays, and what it hands back to [`SimulationBuilder::tick`],
/// [`SimulationBuilder::next_net_id`] and, region by region through
/// [`TickInput::loaded`](crate::TickInput::loaded), on a load.
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

/// Where `sim::save_copy` leaves the copy. Plumbing: the implementation may reshape it.
pub(crate) struct SaveSlot(pub(crate) Option<SavePoint>);
