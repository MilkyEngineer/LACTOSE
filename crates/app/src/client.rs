//! The client's prediction world and the timer root's frame around it (architecture
//! sections 9.2, 9.4 and 11; `crates/predict/README.md`, "The loop `app` writes").

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use lactose_core::{Error, Result, StableKey, Tick};
use lactose_ecs::PresentEventBatch;
use lactose_input::{PlayerInput, SampleHistory};
use lactose_net_api::StagedFrame;
use lactose_net_host::ClockEstimate;
use lactose_net_host::message::JoinAccepted;
use lactose_predict::{LocalPlayer, Predictor, Reconciled};
use lactose_task::{Executor, TickPacer};

use crate::game::Game;
use crate::policy::ErrorPolicy;
use crate::runtime::SolverExit;
use crate::save::Cooked;
use crate::world::WorldConfig;

/// A client's prediction world between runs: the predictor, its pacer, the lead it runs
/// ahead of the server by, the last clock estimate and the ticks the pacer granted since,
/// whether a clock resync is pending, the newest frame tick whose fold failed, the local
/// player's recent samples and the cooked content the fold builds bundles from. Built by
/// [`build_client`] on join; stepped by value from the timer root through
/// [`frame`](Self::frame); ended by [`shutdown`](Self::shutdown) with the session. The
/// replicated world (`WorldKind::Client`) is not here yet: its hook is `lactose-net`'s,
/// milestone 4 (`README.md`, "Not in this crate yet").
pub struct Client {
    predictor: Predictor,
    pacer: TickPacer,
    lead: u32,
    estimate: Option<Estimate>,
    pending_resync: bool,
    /// The newest staged frame tick whose fold `Cooked` failed: no content resync is made
    /// again for a frame at or before it.
    content_failed: Option<Tick>,
    history: SampleHistory,
    cooked: Arc<dyn Cooked>,
    solver: SolverExit,
    policy: ErrorPolicy,
}

/// The last clock estimate and the ticks the client's pacer granted since it was taken:
/// what the clock target advances from. Hidden plumbing, changeable.
struct Estimate {
    granted: u64,
    estimate: ClockEstimate,
}

/// Builds the prediction world on join, from the same `game` and `config` as a server:
/// `WorldKind::Prediction` at `Tick(accepted.tick.0 + lead)`, with every registration and
/// setup the server gets plus `lactose_predict::register_types` and the build check
/// `lactose_predict::check_prediction_reads`, then
/// `Predictor::new(simulation, LocalPlayer { player: accepted.player, entity }, DEFAULT_DEPTH)`,
/// and a pacer built at `Instant::now()`, whose first tick is due at once. `entity` is the
/// player's own entity as the server named it: a parameter until `JoinAccepted::entity`
/// lands (proposed with D122), then read from `accepted`, a signature change to list.
/// `lead` is the ticks the world runs ahead of the server by: the spike's choice at
/// milestone 1, the clock estimate's `lead_ticks` rounded up at milestone 4. Fails as the
/// builder, the check and `Predictor::new` fail, and as the solver pool fails.
pub fn build_client(
    executor: &Executor,
    game: &Game,
    config: WorldConfig,
    accepted: JoinAccepted,
    entity: StableKey,
    lead: u32,
    cooked: Arc<dyn Cooked>,
) -> Result<Client> {
    todo!("build the prediction world and its predictor")
}

impl Client {
    /// The prediction world's current tick.
    #[must_use]
    pub fn tick(&self) -> Tick {
        self.predictor.tick()
    }

    /// `Simulation::hash` of the prediction world: what the spike compares against the
    /// server's hash of the same tick, recorded as the server passed that tick, since the
    /// client stands [`lead`](Self::lead) ticks ahead of it.
    pub fn hash(&self) -> Result<u64> {
        self.predictor.simulation().hash()
    }

    /// Who the world predicts for.
    #[must_use]
    pub fn local(&self) -> LocalPlayer {
        self.predictor.local()
    }

    /// The ticks the world runs ahead of the server by: what [`build_client`] was given,
    /// until a [`clock`](Self::clock) estimate replaces it.
    #[must_use]
    pub fn lead(&self) -> u32 {
        self.lead
    }

    /// The predictor, between runs: its stats, ring and tolerance.
    #[must_use]
    pub fn predictor(&self) -> &Predictor {
        &self.predictor
    }

    /// The predictor, between runs, for `set_depth`, `set_tolerance` and `set_local`;
    /// everything else written through it is an unjournaled edit the next rollback undoes.
    pub fn predictor_mut(&mut self) -> &mut Predictor {
        &mut self.predictor
    }

    /// The local player's last eight samples by tick (`REDUNDANT_SAMPLES`), newest last:
    /// what every input packet carries (architecture section 9.2). Filled by
    /// [`frame`](Self::frame) as it steps, and read by it for a tick it predicts again.
    #[must_use]
    pub fn history(&self) -> &SampleHistory {
        &self.history
    }

    /// Queues a staged frame the network delivered, between runs: `StagedInbox::push` on
    /// the prediction world's inbox. `false` when the inbox refused it (a tick not after
    /// the newest queued, or a frame from before an accepted full frame), which is logged
    /// at `debug` and enters nothing.
    #[must_use]
    pub fn push(&mut self, frame: Arc<StagedFrame>) -> bool {
        todo!("push into the inbox")
    }

    /// How the clock estimate reaches the timer root (architecture section 9.2), between
    /// runs: the pacer's scale becomes the world's `TimeScale` times
    /// `rate_thousandths / 1000`; `lead_ticks` rounded up becomes the lead; the estimate is
    /// kept for the clock target, with a count of the ticks the pacer grants from here on
    /// starting at zero; and `snap` marks a clock resync, which the next
    /// [`frame`](Self::frame) performs on entry and reports. The estimate is
    /// `lactose-net`'s to compute at milestone 4; the spike computes one from its round
    /// trip by hand.
    pub fn clock(&mut self, estimate: ClockEstimate) {
        todo!("apply the estimate to the pacer and the lead, keep it, mark a snap")
    }

    /// One frame of the timer root's client half, by value, in this order and nothing
    /// else. The clock target of a resync with a hint `h` (`Behind`'s `server`, or
    /// `TooOld`'s `frame`) is the last estimate's `server_tick` advanced by the ticks this
    /// client's pacer granted since the estimate was taken (the sum of every poll's budget,
    /// which the replicated scale and the rate already shape, so the target does not
    /// overshoot the server by a pause's length or the ticks slow motion did not run, as
    /// wall time would), or `h` if that is later or there is no estimate,
    /// plus the lead; with neither an estimate nor a hint, the world's own tick.
    ///
    /// 1. A pending clock resync (a `snap`, or the kept failure of the previous frame):
    ///    `pacer.resync(now)`, then `Predictor::resync` to the clock target with no hint.
    /// 2. The fold, from `StagedInbox::due(next)` for `next = tick() + 1`: the newest due
    ///    frame's `time_scale` that is `Some` goes into the input and to the pacer now,
    ///    before the poll, times the estimate's `rate_thousandths / 1000`, so a replicated
    ///    pause stops the ticks at once and an unpause makes them due; and the region
    ///    changes as `crates/net-api/README.md` states (`Irrelevant` removes any pending
    ///    `loaded` and `assets` entry and inserts `unloaded`; `Relevant` inserts `loaded`
    ///    through `RegionRelevance::to_bundle(baseline, None, table)` with `baseline` from
    ///    `Cooked::baseline` and `table` the message's table through `Cooked::pin`, or
    ///    `None`; `Assets` inserts the table pinned; a `full` frame first clears what
    ///    earlier due frames folded and puts every region of `World::regions` in
    ///    `unloaded`). A region whose baseline or pin `Cooked` cannot give is left out of
    ///    the fold. The client remembers the newest frame tick whose fold failed, and
    ///    resyncs once per failing frame tick: when the newest failing due frame is newer
    ///    than the remembered tick, the failure is logged (`Source::Cooked`, `Resync`), its
    ///    tick remembered, and unless step 1 resynced, the world is resynced at its own
    ///    tick with the pacer untouched: a content resync, reported so the full frame that
    ///    follows retries the region. A failing frame folded again (one that stays queued
    ///    through a pause) is left out again and resyncs nothing.
    /// 3. `pacer.poll(now)`, its budget's ticks added to the estimate's granted count.
    /// 4. One `Predictor::step` per due tick, with the fold's input in the first and an
    ///    empty one after. The local player's sample for `next` is the history's if it
    ///    holds one (a tick predicted before a backward resync is predicted again with the
    ///    sample the server was sent), else `sample(next)`, inserted; `sample` runs on the
    ///    task that drives the frame and must not block.
    /// 5. `Predictor::reconcile`, its wall time measured around the await.
    /// 6. `Predictor::passes`.
    /// 7. `take_present_events`.
    /// 8. If `Reconciled::resync` is `Some`: `pacer.resync(now)`, then `Predictor::resync`
    ///    to the clock target, with `Behind`'s `server` or `TooOld`'s `frame` as the hint.
    /// 9. If the budget was `behind`: `pacer.resync(now)`.
    ///
    /// Resolves with the client and a [`ClientOutput`], or with [`ClientError`]: the client
    /// kept when the predictor came back, with a clock resync marked for the next frame's
    /// step 1 (the policy's response is `Resync`); gone after `Cancelled`.
    pub fn frame<'a>(
        self,
        executor: &'a Executor,
        now: Instant,
        sample: &'a mut (dyn FnMut(Tick) -> PlayerInput + Send),
    ) -> ClientFrame<'a> {
        todo!("resync on entry, fold, poll, step, reconcile, run the passes, take the events")
    }

    /// Drops the world and waits for its solver threads: the predictor goes first, which
    /// drops the prediction world, `Physics` and its backend, which terminates the pool;
    /// then every `solver-<n>` thread's exit is waited for. Dropping a `Client` without
    /// this terminates the pool and does not wait. From the thread that owns the client,
    /// never from a worker or a solver thread.
    pub fn shutdown(self) {
        todo!("drop the predictor, then wait on the solver exit")
    }
}

/// What one client frame produced.
pub struct ClientOutput {
    /// New ticks predicted this frame, after the cap.
    pub ticks: u32,
    /// The cap was hit with ticks still owed: the pacer resynced its clock at once
    /// (architecture section 11), and the inputs for the skipped ticks are resent through
    /// the history.
    pub behind: bool,
    /// When the next tick falls due: what the caller sleeps until.
    pub next_deadline: Instant,
    /// What `Predictor::reconcile` did: the corrections, the re-simulated tick count and
    /// whether a resync was due. What the spike records.
    pub reconciled: Reconciled,
    /// The wall time `reconcile` took: divided by `reconciled.resimulated` for the rollback
    /// cost per tick the spike records (architecture section 22).
    pub rollback: Duration,
    /// `Some(tick)` if the world was resynced this frame, by step 1, 2 or 8 of
    /// [`Client::frame`], and now stands at `tick`: the caller requests a full frame from
    /// the server (`ResyncRequest`; reason `BehindCatchUp` at milestone 1, since the
    /// output does not say which step it was, "Not in this crate yet").
    pub resync: Option<Tick>,
    /// The presentation events delivered this frame and the cancellations, taken after the
    /// passes: presentation's at milestone 2, dropped by the spike.
    pub events: PresentEventBatch,
}

/// How a client run fails: the error, and the client when its world survived. `Some` when
/// `PredictError` handed the predictor back, so the caller keeps its world and the next
/// frame resyncs it; `None` after `Cancelled`. Converts into [`Error`].
pub struct ClientError {
    /// What failed.
    pub error: Error,
    /// The client, if its world survived the failure.
    pub client: Option<Client>,
}

impl fmt::Debug for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientError")
            .field("error", &self.error)
            .field("client", &self.client.is_some())
            .finish()
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, f)
    }
}

impl std::error::Error for ClientError {}

impl From<ClientError> for Error {
    fn from(failure: ClientError) -> Self {
        failure.error
    }
}

/// A run in flight, owning the client until it resolves.
type Run<'a, T> = Pin<Box<dyn Future<Output = core::result::Result<T, ClientError>> + Send + 'a>>;

/// One client frame in flight: resolves to the client and what the frame produced. See
/// [`Client::frame`].
#[must_use = "futures do nothing unless awaited"]
pub struct ClientFrame<'a> {
    run: Run<'a, (Client, ClientOutput)>,
}

impl Future for ClientFrame<'_> {
    type Output = core::result::Result<(Client, ClientOutput), ClientError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.run.as_mut().poll(cx)
    }
}
