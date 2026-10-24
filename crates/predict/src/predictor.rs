//! The owner of the prediction world (architecture section 9.4): one [`Predictor`] per
//! client, built beside the replicated world, stepped ahead each frame with the local
//! player's inputs, reconciled against every frame the server sends, and rolled back and
//! re-simulated from the first difference.
//!
//! Runs take the predictor by value and hand it back, as `lactose_sim::Simulation`'s runs
//! do with the simulation, because a run shares the world with jobs on other workers; a
//! failure that left the world whole hands it back inside [`PredictError`].

use core::fmt;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::collections::{BTreeMap, BTreeSet};

use lactose_core::{ComponentKey, Error, ErrorKind, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{Executor, RegionId, World, WorldKind};
use lactose_net_api::StagedInbox;
use lactose_physics_api::Physics;
use lactose_reflect::{Reflect, TypeInfo};
use lactose_sim::{RunError, Simulation, StagedState, TICK_RATE_HZ, TickInput};

use crate::compare::PredictionTolerance;
use crate::fact::{Resolved, recorded};
use crate::hook::{PredictionHook, PredictionRecord, record};
use crate::reconcile::{PredictionStats, Reconciled};
use crate::ring::{Ring, Slot};
use crate::staged_region::StagedRegion;

mod rollback;

/// The ring depth a predictor starts with: sixty ticks, one second at the tick rate, a
/// placeholder and not a sized value. `app` replaces it with the frame re-simulation
/// budget divided by the per-tick cost it measures, under its own cap
/// ([`Predictor::set_depth`]), before it relies on a correction costing no lead: a
/// rollback to the oldest slot of an unsized ring may overrun the frame (D122).
pub const DEFAULT_DEPTH: u32 = 60;

/// How long a substitution the reconciliation acted on is remembered, so a repeat of the
/// report is skipped: while the ring holds its slot or for ten seconds after its tick,
/// whichever is longer, whatever the depth. The server repeats a substitution until it sees acknowledged
/// a packet that carried it (`crates/net-api/README.md`, `substituted`): every packet it
/// sends from the first carries it, and the client acknowledges every packet it stages, so
/// the repeats last about a round trip plus the client's send interval, longer only while
/// every packet from the client is lost. A repeat later than this reads as a first report
/// whose slot has left the ring, `TooOld`, never a rollback. Each entry is one `Tick`.
const REPEAT_WINDOW: u32 = 10 * TICK_RATE_HZ;

/// Who the prediction world predicts for: the player the server assigned on join
/// (`JoinAccepted::player`) and the player's own entity, the row the server's session
/// applier spawned or re-claimed with `ClaimedBy` and `StreamingAnchor`. The hook writes
/// `StreamingAnchor` beside the claim on that key whenever the key is live, so that
/// `lactose_input::player_entity` resolves in the prediction world; `ClaimedBy` alone does
/// not name it, since the player claims other entities too. Replaced between runs with
/// [`Predictor::set_local`] when the server names another entity.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LocalPlayer {
    /// The local player.
    pub player: PlayerId,
    /// The player's own entity, by origin key.
    pub entity: StableKey,
}

/// The prediction world and everything that rolls it back: the simulation it owns, the
/// ring of slots it can return to, what it has learned about the entities the local
/// player claims, and the counters the milestone 1 spike records.
///
/// Built once per session from a `WorldKind::Prediction` simulation; stepped, reconciled
/// and run by value from the timer root; dropped with the session. The README states the
/// loop `app` writes around it and the order its calls keep.
pub struct Predictor {
    simulation: Simulation,
    state: State,
}

/// A predictor without its simulation, which a run moves out and back.
struct State {
    local: LocalPlayer,
    ring: Ring,
    stats: PredictionStats,
    /// Whether the world held a `Physics` resource when the predictor was built: then
    /// every slot carries a physics snapshot and every restore restores it, and a step
    /// after the presence changed is refused.
    physics: bool,
    /// The region each entity the local player has claimed was last seen in, from the
    /// first fact that named it until the server's tombstone or release fact for it is
    /// reconciled, and replaced whole by each full frame's claimed spawns (never from the
    /// record, so an entity only the prediction world believes claimed is never
    /// re-spawned); never dropped for the ring's age, so an entity the prediction world
    /// despawned too early is re-spawned by its next fact however long the server left it
    /// unchanged. Kept across `resync` until the full frame that follows it, so an entry
    /// whose tombstone was lost outlives the server's claim by one full frame at most.
    known: BTreeMap<StableKey, RegionId>,
    /// The input ticks after a substituted one whose own sample is repaired once a frame
    /// at or after them is reconciled; a frame spans every tick since the previous one, so
    /// a repair is found by its tick, possibly in a later call.
    awaiting: BTreeSet<Tick>,
    /// The input ticks a call replaced by a substitute on the server's first report, so a
    /// repeat of the report is skipped wherever the ring stands; each is remembered while
    /// the ring holds its slot or for [`REPEAT_WINDOW`] ticks, whichever is longer, and
    /// all are forgotten by a full frame and by `resync`.
    substituted: BTreeSet<Tick>,
    /// The tick the newest full frame or `resync` restarted the ring at: a substitution
    /// or repair at or before it is skipped. Remembered as an entry of `substituted` is,
    /// so a repeat after the ring dropped the restart's slot is still skipped.
    restart: Option<Tick>,
    /// The facts compared at the tick their frame was popped at instead of their own, by
    /// that tick: each places its entity in a region the slot it would be compared at
    /// lacked and the popped tick loaded, and later facts about the same entity for a tick
    /// at or before it are folded in, so that each entity's facts at a tick are one
    /// statement. Kept while the ring holds the tick, since a
    /// re-simulation of it never re-applies them (`README.md`, "Reconciliation"); cleared
    /// by a full frame and by `resync`.
    kept: BTreeMap<Tick, Vec<Resolved>>,
    /// The components the record holds, ascending: the registry's replicated persistent
    /// ones but `Teleported`.
    recorded: Vec<ComponentKey>,
    /// What a dynamic body's `Transform` and `Velocity` may differ by and still agree.
    tolerance: PredictionTolerance,
}

impl State {
    fn join(self, simulation: Simulation) -> Predictor {
        Predictor { simulation, state: self }
    }

    /// The slot of `world`'s current tick, with its physics part if the world has physics
    /// and `physics` (a correction's re-take of a slot that had none passes `false`); a
    /// failed physics snapshot is counted.
    fn take(
        &mut self,
        world: &World,
        input: TickInput,
        frames: Vec<FrameRef>,
        physics: bool,
    ) -> Slot {
        let physics = physics && self.physics && world.service::<Physics>().is_some();
        let slot = Slot::take(world, self.local.player, &self.recorded, physics, input, frames);
        if physics && slot.physics.is_none() {
            self.stats.slots_without_physics += 1;
        }
        slot
    }

    /// Tells the world the ring's oldest tick.
    fn horizon(&self, world: &mut World) {
        if let Some(oldest) = self.ring.oldest() {
            world.set_rollback_horizon(oldest);
        }
    }

    /// What follows every step, predicted or re-simulated: the slot of the new tick taken
    /// with `input` and the frames the hook applied (appended, or replacing the held one
    /// when `replace`), the horizon moved; then the refusals a step resolves to when the
    /// hook did not run or the world's `Physics` presence changed.
    fn finish_step(&mut self, world: &mut World, input: TickInput, replace: bool) -> Result<()> {
        let tick = world.tick();
        let record = record(world)?;
        let ran = record.ran_at == Some(tick);
        let frames = core::mem::take(&mut record.stepped);
        self.stats.unechoed_spawns += core::mem::take(&mut record.unechoed);
        let slot = self.take(world, input, frames, true);
        if replace {
            self.ring.replace(slot);
        } else {
            self.ring.push(slot);
        }
        self.horizon(world);
        if !ran {
            return Err(invalid(format!(
                "the prediction hook did not run in the step for tick {}: a `dyn StagedState` \
                 was inserted over it, and `Predictor::resync` reinstalls it",
                tick.0
            )));
        }
        if world.service::<Physics>().is_some() != self.physics {
            return Err(invalid(
                "the prediction world's `Physics` presence changed since `Predictor::new`, so \
                 the ring would not restore it",
            ));
        }
        Ok(())
    }

    /// A failed simulation run, with the predictor if its world survived.
    fn failed(self, failure: RunError) -> PredictError {
        PredictError { error: failure.error, predictor: failure.simulation.map(|s| self.join(s)) }
    }
}

type FrameRef = std::sync::Arc<lactose_net_api::StagedFrame>;

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidData, message.into())
}

/// Installs the hook over whatever `dyn StagedState` `world` holds, and a fresh record.
fn install(world: &mut World, local: LocalPlayer) {
    world.insert_service::<dyn StagedState>(Box::new(PredictionHook::new()));
    world.insert_service::<PredictionRecord>(Box::new(PredictionRecord::new(local)));
}

/// The tick after `tick`.
pub(crate) const fn next(tick: Tick) -> Tick {
    Tick(tick.0.wrapping_add(1))
}

/// The tick before `tick`.
pub(crate) const fn prev(tick: Tick) -> Tick {
    Tick(tick.0.wrapping_sub(1))
}

impl Predictor {
    /// Takes ownership of `simulation`, which must be a `WorldKind::Prediction` world
    /// between runs (`InvalidData` otherwise) holding a `lactose_net_api::StagedInbox`
    /// and a registry with [`StagedRegion`](crate::StagedRegion) in it, which
    /// [`register_types`](crate::register_types) registers (`NotFound` otherwise); on any
    /// refusal the simulation is dropped with the error,
    /// as a wrong build would be. Replaces whatever `dyn StagedState` service the world
    /// holds with a [`PredictionHook`](crate::PredictionHook), inserts the record the
    /// hook fills beside it, which holds `local` (the one copy), takes the ring's first
    /// slot at the simulation's current tick (the join tick plus the lead `app` built it
    /// at, before any step), and tells the world its rollback horizon. `depth` is the
    /// ring depth, at least two (`InvalidData` otherwise). If the world holds a
    /// `lactose_physics_api::Physics` resource, every slot from now on carries a physics
    /// snapshot taken right after the world's and every restore restores it; a world
    /// without one rolls back its rows alone; a later change of that presence makes
    /// [`step`](Self::step) refuse.
    pub fn new(mut simulation: Simulation, local: LocalPlayer, depth: u32) -> Result<Self> {
        if simulation.kind() != WorldKind::Prediction {
            return Err(invalid("a predictor owns a `WorldKind::Prediction` simulation"));
        }
        if depth < 2 {
            return Err(invalid("a ring holds the present and at least one tick to return to"));
        }
        let world = simulation.world_mut();
        if world.service::<StagedInbox>().is_none() {
            return Err(Error::new(
                ErrorKind::NotFound,
                "the prediction world holds no `StagedInbox`: `lactose_net_api::setup` inserts it",
            ));
        }
        if world.registry().get(StagedRegion::KEY).is_none() {
            return Err(Error::new(
                ErrorKind::NotFound,
                "the prediction world's registry lacks `StagedRegion`: \
                 `lactose_predict::register_types` registers it",
            ));
        }
        install(world, local);
        let mut state = State {
            local,
            ring: Ring::new(depth),
            stats: PredictionStats::default(),
            physics: world.service::<Physics>().is_some(),
            known: BTreeMap::new(),
            awaiting: BTreeSet::new(),
            substituted: BTreeSet::new(),
            restart: None,
            kept: BTreeMap::new(),
            recorded: world
                .registry()
                .iter()
                .filter(|info| recorded(info))
                .map(TypeInfo::key)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            tolerance: PredictionTolerance::DEFAULT,
        };
        let slot = state.take(world, TickInput::default(), Vec::new(), true);
        state.ring.push(slot);
        state.horizon(world);
        Ok(state.join(simulation))
    }

    /// The player and entity this world predicts for.
    #[must_use]
    pub fn local(&self) -> LocalPlayer {
        self.state.local
    }

    /// Replaces the player and entity between runs: from the next tick the hook keeps the
    /// claim and the anchor on the new entity and removes `StreamingAnchor` from the old
    /// one if it is live. A changed player takes effect with the rebuild that follows a
    /// re-join, since rows claimed under the old player go with the full frame's reload.
    /// Fails during a run.
    pub fn set_local(&mut self, local: LocalPlayer) -> Result<()> {
        record(self.simulation.world_mut())?.local = local;
        self.state.local = local;
        Ok(())
    }

    /// The prediction world's current tick: the newest slot's, which is the present.
    #[must_use]
    pub fn tick(&self) -> Tick {
        self.simulation.tick()
    }

    /// The simulation, for reading between runs.
    #[must_use]
    pub fn simulation(&self) -> &Simulation {
        &self.simulation
    }

    /// The simulation, for what `app` does between runs and nothing else: pushing frames
    /// into the world's `StagedInbox`, reading `World::regions` for the fold, taking the
    /// presentation events after [`reconcile`](Self::reconcile), and taking the
    /// lifecycle's asset failures. A write to simulation state here is an unjournaled
    /// edit the ring does not hold and a rollback undoes (`crates/sim/README.md`); a
    /// `dyn StagedState` inserted over the hook is caught by the next
    /// [`step`](Self::step), which refuses when the hook did not run, and repaired by
    /// [`resync`](Self::resync), which reinstalls it.
    pub fn simulation_mut(&mut self) -> &mut Simulation {
        &mut self.simulation
    }

    /// The rollback ring.
    #[must_use]
    pub fn ring(&self) -> &Ring {
        &self.state.ring
    }

    /// The counters since the predictor was built. Monotonic: `app` differences two
    /// readings for a rate.
    #[must_use]
    pub fn stats(&self) -> PredictionStats {
        self.state.stats
    }

    /// Sets the ring depth, at least two (`InvalidData` otherwise, and nothing changes).
    /// A smaller depth drops the oldest slots at once and moves the horizon forward. `app`
    /// derives it from the frame re-simulation budget divided by the per-tick cost it
    /// measured, under its configured cap (architecture section 9.4): a rollback to the
    /// oldest slot then fits the budget, which is what bounds re-simulation, since it does
    /// not count against the catch-up cap (D122).
    pub fn set_depth(&mut self, depth: u32) -> Result<()> {
        if depth < 2 {
            return Err(invalid("a ring holds the present and at least one tick to return to"));
        }
        self.state.ring.set_depth(depth);
        self.state.horizon(self.simulation.world_mut());
        Ok(())
    }

    /// Replaces the tolerance within which a dynamic body's `Transform` and `Velocity`
    /// agree with the server's (D124), from the next [`reconcile`](Self::reconcile) on. A
    /// predictor starts at [`PredictionTolerance::DEFAULT`].
    pub fn set_tolerance(&mut self, tolerance: PredictionTolerance) {
        self.state.tolerance = tolerance;
    }

    /// Re-synchronises the prediction world to `tick`, between runs: `World::set_tick`,
    /// the ring emptied and a fresh first slot taken at `tick` with
    /// [`Ring::rebuilt_at`] set to it, the horizon moved there, the record cleared and the
    /// hook reinstalled over whatever `dyn StagedState` the world holds (the repair for
    /// one inserted over it), and the known regions kept until the next full frame
    /// replaces them. What `app` calls when [`Reconciled::resync`] is `Some`, with the
    /// tick it wants the world at (the server's tick plus the lead it runs ahead by); the
    /// frames still queued in the inbox are then from the band before the restart, and are
    /// reconciled at the restart's slot rather than `TooOld`, so one resync is one resync
    /// and a tombstone, claim or release among them is not lost. The full frame `app`
    /// then requests from the server rebuilds the world through the inbox and the
    /// `TickInput` as any full frame does. A `TickBudget::behind` from the pacer is a
    /// clock resync (`TickPacer::resync`), not this. Fails during a run, as `set_tick`
    /// does.
    pub fn resync(&mut self, tick: Tick) -> Result<()> {
        let world = self.simulation.world_mut();
        world.set_tick(tick)?;
        install(world, self.state.local);
        let state = &mut self.state;
        state.ring.clear();
        let slot = state.take(world, TickInput::default(), Vec::new(), true);
        state.ring.push(slot);
        state.ring.rebuild_at(tick);
        state.horizon(world);
        state.awaiting.clear();
        state.substituted.clear();
        state.restart = Some(tick);
        state.kept.clear();
        state.stats.resyncs += 1;
        Ok(())
    }

    /// Runs one tick ahead with `input`, which must carry the local player's sample in
    /// `inputs` with `input_tick` equal to the tick the step runs, `tick() + 1`
    /// (`InvalidData` with the predictor handed back before anything ran, since the input
    /// tick is what gives a predicted spawn the key the server derives), then takes the
    /// slot for the new tick: the world snapshot, the physics snapshot if the world has
    /// physics (a failed one leaves the slot without its physics part, logged and
    /// counted), the frames the hook popped in this step, `input` itself, and the record
    /// of every replicated persistent component but `Teleported` of every entity the local
    /// player has claimed. The
    /// oldest slot beyond the depth is dropped and the world's rollback horizon follows.
    /// Nothing is compared here: [`reconcile`](Self::reconcile) does that.
    ///
    /// After the slot is taken the step resolves to `InvalidData` with the predictor
    /// handed back if the hook did not run in it (a `dyn StagedState` inserted over it)
    /// or the world's `Physics` presence is not what it was at
    /// [`new`](Self::new); the ring is consistent, and nothing is reconciled until `app`
    /// calls [`resync`](Self::resync), which reinstalls the hook.
    ///
    /// The future is `Send` and is driven by the task that owns the predictor. It resolves
    /// to a [`PredictError`] with the predictor inside when the step failed before anything
    /// ran (`lactose_sim::RunError` with its simulation), and with `None` after
    /// `Cancelled`, when the executor shut down mid-run and the world is torn.
    pub fn step(self, executor: &Executor, input: TickInput) -> StepRun<'_> {
        StepRun { run: Box::pin(self.run_step(executor, input)) }
    }

    #[expect(
        clippy::result_large_err,
        reason = "the run's public output: a failure hands the predictor back by value"
    )]
    async fn run_step(
        mut self,
        executor: &Executor,
        input: TickInput,
    ) -> core::result::Result<Self, PredictError> {
        let tick = next(self.tick());
        let player = self.state.local.player;
        if input.inputs.get(&player).map(|sample| sample.input_tick) != Some(tick) {
            let error = invalid(format!(
                "the step for tick {} carries no sample of the local player for that input tick",
                tick.0
            ));
            return Err(PredictError { error, predictor: Some(self) });
        }
        match record(self.simulation.world_mut()) {
            Ok(record) => record.replay = None,
            Err(error) => return Err(PredictError { error, predictor: Some(self) }),
        }
        let kept = input.clone();
        let Self { simulation, mut state } = self;
        let mut simulation = match simulation.step(executor, input).await {
            Ok(simulation) => simulation,
            Err(failure) => return Err(state.failed(failure)),
        };
        state.stats.ticks_predicted += 1;
        let finished = state.finish_step(simulation.world_mut(), kept, false);
        let predictor = state.join(simulation);
        match finished {
            Ok(()) => Ok(predictor),
            Err(error) => Err(PredictError { error, predictor: Some(predictor) }),
        }
    }

    /// Reconciles every frame the hook popped since the last call, oldest first, as the
    /// README states: full frames rebuild and restart the ring; frames popped before the
    /// newest full frame are skipped, and the facts of frames for ticks before the restart,
    /// exact rows aside, are compared at the restart's slot; substituted ticks replace the
    /// ring's recorded samples and the first unsubstituted tick after them is repaired;
    /// each other frame's claimed facts are compared against the slot of the frame's tick
    /// (one that places an entity in a region that slot lacks, at the slot of the tick the
    /// frame was popped at, where it is kept for later calls),
    /// a dynamic body's `Transform` and `Velocity` within the tolerance
    /// ([`set_tolerance`](Self::set_tolerance)); from the oldest difference the
    /// world is restored, the facts at that tick are written stamped, the physics is told
    /// what was corrected, and every later tick is re-simulated from the ring's inputs
    /// with its frames replayed, comparing again at every tick a later frame has facts
    /// for. The world ends at the present tick it started at, and the ring holds the
    /// re-simulated slots. Nothing to reconcile resolves at once with an empty
    /// [`Reconciled`].
    ///
    /// The future is `Send`. It resolves to a [`PredictError`] with the predictor inside
    /// when a restore or a re-simulated step refused, the world then at the tick the
    /// failure left it and the ring trimmed to it, and with `None` after `Cancelled`. A
    /// frame that must be re-simulated without one of its slots (older than the ring) is
    /// not an error: it is [`Reconciled::resync`].
    pub fn reconcile(self, executor: &Executor) -> ReconcileRun<'_> {
        ReconcileRun { run: Box::pin(self.run_reconcile(executor)) }
    }

    /// Runs the frame's passes on the prediction world (`Simulation::run_passes`: extract,
    /// then the gather if one is registered, which the prediction world's kind skips), once
    /// per frame after [`reconcile`](Self::reconcile), so extract never sees a tick a
    /// rollback discarded. Resolves as the simulation's run does.
    pub fn passes(self, executor: &Executor) -> PassesRun<'_> {
        PassesRun { run: Box::pin(self.run_passes(executor)) }
    }

    #[expect(
        clippy::result_large_err,
        reason = "the run's public output: a failure hands the predictor back by value"
    )]
    async fn run_passes(self, executor: &Executor) -> core::result::Result<Self, PredictError> {
        let Self { simulation, state } = self;
        match simulation.run_passes(executor).await {
            Ok(simulation) => Ok(state.join(simulation)),
            Err(failure) => Err(state.failed(failure)),
        }
    }
}

/// How a run fails: the error, and the predictor when its world survived. `Some` when the
/// failure came before anything ran (an input without the local player's sample, a
/// simulation run refused by `Schedule::prepare`, a restore refused), or after a step
/// whose hook did not run, or during a re-simulation (the world then at the tick the
/// failure left it), so the caller keeps its world; `None` after `Cancelled`, when the
/// executor shut down during a run and the world dropped with it. Converts into
/// [`Error`], so `?` into a `lactose_core::Result` drops the predictor as a plain error
/// would; a caller that wants to keep it matches.
pub struct PredictError {
    /// What failed.
    pub error: Error,
    /// The predictor, if its world survived the failure.
    pub predictor: Option<Predictor>,
}

impl fmt::Debug for PredictError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PredictError")
            .field("error", &self.error)
            .field("predictor", &self.predictor.is_some())
            .finish()
    }
}

impl fmt::Display for PredictError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, f)
    }
}

impl std::error::Error for PredictError {}

impl From<PredictError> for Error {
    fn from(failure: PredictError) -> Self {
        failure.error
    }
}

/// A run in flight, owning the predictor until it resolves.
type Run<'a, T> = Pin<Box<dyn Future<Output = core::result::Result<T, PredictError>> + Send + 'a>>;

/// One predicted tick in flight: resolves to the predictor at the next tick. See
/// [`Predictor::step`].
#[must_use = "futures do nothing unless awaited"]
pub struct StepRun<'a> {
    run: Run<'a, Predictor>,
}

impl Future for StepRun<'_> {
    type Output = core::result::Result<Predictor, PredictError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.run.as_mut().poll(cx)
    }
}

/// A reconciliation in flight: resolves to the predictor at the tick it started at, with
/// what it did. See [`Predictor::reconcile`].
#[must_use = "futures do nothing unless awaited"]
pub struct ReconcileRun<'a> {
    run: Run<'a, (Predictor, Reconciled)>,
}

impl Future for ReconcileRun<'_> {
    type Output = core::result::Result<(Predictor, Reconciled), PredictError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.run.as_mut().poll(cx)
    }
}

/// A frame's passes in flight: resolves to the predictor unchanged in tick. See
/// [`Predictor::passes`].
#[must_use = "futures do nothing unless awaited"]
pub struct PassesRun<'a> {
    run: Run<'a, Predictor>,
}

impl Future for PassesRun<'_> {
    type Output = core::result::Result<Predictor, PredictError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.run.as_mut().poll(cx)
    }
}
