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
use crate::save::Cooked;
use crate::world::WorldConfig;

/// A client's prediction world between runs: the predictor, its pacer, the lead it runs
/// ahead of the server by, the last clock estimate, the local player's recent samples and
/// the cooked content the fold builds bundles from. Built by [`build_client`] on join;
/// stepped by value from the timer root through [`frame`](Self::frame); dropped with the
/// session. The replicated world (`WorldKind::Client`) is not here yet: its hook is
/// `lactose-net`'s, milestone 4 (`README.md`, "Not in this crate yet").
pub struct Client {
    predictor: Predictor,
    pacer: TickPacer,
    lead: u32,
    estimate: Option<ClockEstimate>,
    history: SampleHistory,
    cooked: Arc<dyn Cooked>,
    policy: ErrorPolicy,
}

/// Builds the prediction world on join, from the same `game` and `config` as a server:
/// `WorldKind::Prediction` at `Tick(accepted.tick.0 + lead)`, with every registration and
/// setup the server gets plus `lactose_predict::register_types` and the build check
/// `lactose_predict::check_prediction_reads`, then
/// `Predictor::new(simulation, LocalPlayer { player: accepted.player, entity }, DEFAULT_DEPTH)`,
/// and a pacer whose first tick falls due now. `entity` is the player's own entity as the
/// server named it: a parameter until `JoinAccepted::entity` lands (proposed with D122),
/// then read from `accepted`, a signature change to list. `lead` is the ticks the world
/// runs ahead of the server by: the spike's choice at milestone 1, the clock estimate's
/// `lead_ticks` rounded up at milestone 4. Fails as the builder, the check and
/// `Predictor::new` fail, and as the solver pool fails.
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
    /// server's for the same tick when it looks for divergence.
    pub fn hash(&self) -> Result<u64> {
        self.predictor.simulation().hash()
    }

    /// Who the world predicts for.
    #[must_use]
    pub fn local(&self) -> LocalPlayer {
        self.predictor.local()
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
    /// [`frame`](Self::frame) as it steps.
    #[must_use]
    pub fn history(&self) -> &SampleHistory {
        &self.history
    }

    /// Queues a staged frame the network delivered, between runs: `StagedInbox::push` on
    /// the prediction world's inbox. `false` when the inbox refused it (a tick not after
    /// the newest queued, or a frame from before an accepted full frame), which is logged
    /// and enters nothing.
    #[must_use]
    pub fn push(&mut self, frame: Arc<StagedFrame>) -> bool {
        todo!("push into the inbox")
    }

    /// How the clock estimate reaches the timer root (architecture section 9.2): the pacer's
    /// scale becomes the replicated time scale times `rate_thousandths / 1000`; `lead_ticks`
    /// rounded up becomes the lead; and if `snap` is set the pacer resyncs at `now` and the
    /// predictor resyncs to `server_tick + lead`, which the next [`frame`](Self::frame)
    /// reports in [`ClientOutput::resync`] so a full frame is requested. The estimate is
    /// `lactose-net`'s to compute (named only at milestone 1; the spike computes one from
    /// its round trip by hand).
    pub fn clock(&mut self, now: Instant, estimate: ClockEstimate) {
        todo!("apply the estimate to the pacer, the lead and, on snap, the predictor")
    }

    /// One frame of the timer root's client half, by value, in the order
    /// `crates/predict/README.md` states: the due frames' time scale and region changes
    /// folded into the first new tick's `TickInput` (`StagedInbox::due`; a `Relevant`
    /// region through `RegionRelevance::to_bundle` with the cooked baseline and table, an
    /// `Irrelevant` one into `unloaded`, `Assets` pinned into `assets`; a full frame puts
    /// every loaded region in `unloaded` first); `pacer.poll(now)`; one `Predictor::step`
    /// per due tick with `sample(tick).to_player_sample(tick)` as the local player's input,
    /// the sample kept in the history; `Predictor::reconcile`, timed; `Predictor::passes`;
    /// the presentation events taken; a resync if the reconciliation asks for one
    /// (`pacer.resync(now)`, then `Predictor::resync` to the last estimate's `server_tick`
    /// plus the lead, or without an estimate to `Behind`'s `server` plus the lead or, for
    /// `TooOld`, the world's own tick); `pacer.resync(now)` alone if the budget was
    /// `behind`. Resolves
    /// with the client and a [`ClientOutput`], or with [`ClientError`]: the client kept when
    /// the predictor came back (the policy's response is `Resync`, and the output of the
    /// next frame reports it), gone after `Cancelled`.
    pub fn frame<'a>(
        self,
        executor: &'a Executor,
        now: Instant,
        sample: &'a mut (dyn FnMut(Tick) -> PlayerInput + Send),
    ) -> ClientFrame<'a> {
        todo!("fold, poll, step, reconcile, run the passes, take the events, resync")
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
    /// `Some(tick)` if the world was resynced this frame and now stands at `tick`: the
    /// caller requests a full frame from the server (`ResyncRequest`, reason
    /// `BehindCatchUp` for both of the predictor's reasons at milestone 1).
    pub resync: Option<Tick>,
    /// The presentation events delivered this frame and the cancellations, taken after the
    /// passes: presentation's at milestone 2, dropped by the spike.
    pub events: PresentEventBatch,
}

/// How a client run fails: the error, and the client when its world survived. `Some` when
/// `PredictError` handed the predictor back, so the caller keeps its world and resyncs;
/// `None` after `Cancelled`. Converts into [`Error`].
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
