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
use std::collections::BTreeMap;

use lactose_core::{Error, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{Executor, RegionId};
use lactose_sim::{Simulation, TickInput};

use crate::reconcile::{PredictionStats, Reconciled};
use crate::ring::Ring;

/// The ring depth a predictor starts with: sixty ticks, one second at the tick rate, which
/// covers a 100 ms round trip's lead plus the redundancy window several times over. `app`
/// replaces it with the frame re-simulation budget divided by the per-tick cost it
/// measures, under its own cap ([`Predictor::set_depth`]).
pub const DEFAULT_DEPTH: u32 = 60;

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
    local: LocalPlayer,
    ring: Ring,
    stats: PredictionStats,
    /// Whether the world held a `Physics` resource when the predictor was built: then
    /// every slot carries a physics snapshot and every restore restores it, and a step
    /// after the presence changed is refused.
    physics: bool,
    /// The last region each entity the local player has claimed was seen in, for the
    /// entities some slot's record still holds or a popped frame still names: where a
    /// correction re-spawns an entity the prediction world despawned too early. Pruned to
    /// those keys after every ring change, so it is bounded by the ring's claimed entities.
    known: BTreeMap<StableKey, RegionId>,
}

impl Predictor {
    /// Takes ownership of `simulation`, which must be a `WorldKind::Prediction` world
    /// between runs (`InvalidData` otherwise) holding a `lactose_net_api::StagedInbox`
    /// (`NotFound` otherwise); on either refusal the simulation is dropped with the error,
    /// as a wrong build would be. Replaces whatever `dyn StagedState` service the world
    /// holds with a [`PredictionHook`](crate::PredictionHook) for `local`, inserts the
    /// record the hook fills beside it, takes the ring's first slot at the simulation's
    /// current tick (the join tick plus the lead `app` built it at, before any step), and
    /// tells the world its rollback horizon. `depth` is the ring depth, at least two
    /// (`InvalidData` otherwise). If the world holds a `lactose_physics_api::Physics`
    /// resource, every slot from now on carries a physics snapshot taken right after the
    /// world's and every restore restores it; a world without one rolls back its rows
    /// alone; a later change of that presence makes [`step`](Self::step) refuse.
    pub fn new(simulation: Simulation, local: LocalPlayer, depth: u32) -> Result<Self> {
        todo!()
    }

    /// The player and entity this world predicts for.
    #[must_use]
    pub fn local(&self) -> LocalPlayer {
        self.local
    }

    /// Replaces the player and entity between runs: from the next tick the hook keeps the
    /// claim and the anchor on the new entity and removes `StreamingAnchor` from the old
    /// one if it is live. A changed player takes effect with the rebuild that follows a
    /// re-join, since rows claimed under the old player go with the full frame's reload.
    /// Fails during a run.
    pub fn set_local(&mut self, local: LocalPlayer) -> Result<()> {
        todo!()
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
    /// [`step`](Self::step), which refuses when the hook did not run.
    pub fn simulation_mut(&mut self) -> &mut Simulation {
        &mut self.simulation
    }

    /// The rollback ring.
    #[must_use]
    pub fn ring(&self) -> &Ring {
        &self.ring
    }

    /// The counters since the predictor was built. Monotonic: `app` differences two
    /// readings for a rate.
    #[must_use]
    pub fn stats(&self) -> PredictionStats {
        self.stats
    }

    /// Sets the ring depth, at least two (`InvalidData` otherwise, and nothing changes).
    /// A smaller depth drops the oldest slots at once and moves the horizon forward. `app`
    /// derives it from the frame re-simulation budget divided by the per-tick cost it
    /// measured, under its configured cap (architecture section 9.4): a rollback to the
    /// oldest slot then fits the budget, which is what bounds re-simulation, since it does
    /// not count against the catch-up cap (D122).
    pub fn set_depth(&mut self, depth: u32) -> Result<()> {
        todo!()
    }

    /// Re-synchronises the prediction world to `tick`, between runs: `World::set_tick`,
    /// the ring emptied and a fresh first slot taken at `tick` with
    /// [`Ring::rebuilt_at`] set to it, the horizon moved there, the record cleared and the
    /// known regions kept. What `app` calls when [`Reconciled::resync`] is `Some`, with
    /// the tick it wants the world at (the server's tick plus the lead it runs ahead by);
    /// the frames still queued in the inbox are then from before the restart and are
    /// skipped rather than `TooOld`, so one resync is one resync. The full frame `app`
    /// then requests from the server rebuilds the world through the inbox and the
    /// `TickInput` as any full frame does. A `TickBudget::behind` from the pacer is a
    /// clock resync (`TickPacer::resync`), not this. Fails during a run, as `set_tick`
    /// does.
    pub fn resync(&mut self, tick: Tick) -> Result<()> {
        todo!()
    }

    /// Runs one tick ahead with `input`, which must carry the local player's sample in
    /// `inputs` with `input_tick` equal to the tick the step runs, `tick() + 1`
    /// (`InvalidData` with the predictor handed back before anything ran, since the input
    /// tick is what gives a predicted spawn the key the server derives), then takes the
    /// slot for the new tick: the world snapshot, the physics snapshot if the world has
    /// physics (a failed one leaves the slot without its physics part, logged and
    /// counted), the frames the hook popped in this step, `input` itself, and the record
    /// of every replicated component of every entity the local player has claimed. The
    /// oldest slot beyond the depth is dropped and the world's rollback horizon follows.
    /// Nothing is compared here: [`reconcile`](Self::reconcile) does that.
    ///
    /// After the slot is taken the step resolves to `InvalidData` with the predictor
    /// handed back if the hook did not run in it (a `dyn StagedState` inserted over it)
    /// or the world's `Physics` presence is not what it was at
    /// [`new`](Self::new); the ring is consistent, and nothing is reconciled until `app`
    /// repairs its world.
    ///
    /// The future is `Send` and is driven by the task that owns the predictor. It resolves
    /// to a [`PredictError`] with the predictor inside when the step failed before anything
    /// ran (`lactose_sim::RunError` with its simulation), and with `None` after
    /// `Cancelled`, when the executor shut down mid-run and the world is torn.
    pub fn step(self, executor: &Executor, input: TickInput) -> StepRun<'_> {
        todo!()
    }

    /// Reconciles every frame the hook popped since the last call, oldest first, as the
    /// README states: full frames rebuild and restart the ring; frames from before a
    /// restart are skipped; substituted ticks replace the ring's recorded samples and the
    /// first unsubstituted tick after them is repaired; each frame's claimed facts are
    /// compared against the slot of the frame's tick; from the oldest difference the
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
        todo!()
    }

    /// Runs the frame's passes on the prediction world (`Simulation::run_passes`: extract,
    /// then the gather if one is registered, which the prediction world's kind skips), once
    /// per frame after [`reconcile`](Self::reconcile), so extract never sees a tick a
    /// rollback discarded. Resolves as the simulation's run does.
    pub fn passes(self, executor: &Executor) -> PassesRun<'_> {
        todo!()
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
