//! The server world and the timer root's frame around it (architecture sections 5.1, 9.2,
//! 9.3 and 11; `crates/sim/README.md`, "The timer root").

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use lactose_asset_api::VersionTable;
use lactose_bundle::Baseline;
use lactose_core::{Error, Result, Tick};
use lactose_ecs::RegionId;
use lactose_net_api::{RegionRelevance, StagingFrame};
use lactose_persist::SchemaTable;
use lactose_sim::{RegionExport, RegionRequest, SavePoint, Simulation, TickInput, TimeScale};
use lactose_task::{Executor, TickPacer};

use crate::game::Game;
use crate::policy::ErrorPolicy;
use crate::save::SaveEncode;
use crate::world::{Start, WorldConfig};

/// A server world between runs: the simulation, its pacer, the schema table a save and a
/// relevance message share, the time scale the server decided, and the baseline of every
/// region it stepped a bundle for. Built by [`build_server`]; stepped by value from the
/// timer root through [`step`](Self::step) or [`frame`](Self::frame); dropped at shutdown,
/// which drops the physics backend and joins its solver pool.
pub struct Server {
    simulation: Simulation,
    pacer: TickPacer,
    schemas: Arc<SchemaTable>,
    time_scale: TimeScale,
    pending_scale: Option<TimeScale>,
    baselines: BTreeMap<RegionId, Arc<Baseline>>,
    policy: ErrorPolicy,
}

/// Builds a server world from `game` and `config`, starting at `start`: the one type table
/// (`Game::types`), the solver pool and the backend, every setup function, every
/// registration function and every ordering edge in the order `README.md` states
/// ("Registration"), the input and session appliers, `config` into the builder, and a
/// pacer whose first tick falls due now. Fails as the builder fails (a registration or
/// setup error carries its crate's name; `log_retention` below the cap is `InvalidData`),
/// as the solver pool fails, and with `InvalidData` if `start` is `Loaded` and `config.regions`
/// is `Anchored` but a saved region is not in the atlas. A `Loaded` start's bundles are
/// not loaded here: they go into the first frame, which [`Server::frame`] and
/// [`Server::step`] prepend to that frame's `loaded` (`README.md`, "Saving and loading").
pub fn build_server(
    executor: &Executor,
    game: &Game,
    config: WorldConfig,
    start: Start,
) -> Result<Server> {
    todo!("build the type table, the pool, the backend and the simulation")
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

    /// The time scale the server decided, as the pacer runs by it: `REAL_TIME` at build.
    #[must_use]
    pub fn time_scale(&self) -> TimeScale {
        self.time_scale
    }

    /// Decides the time scale from the next tick on (architecture section 11): the next
    /// step's `TickInput::time_scale` carries it, whichever path runs that step, and the
    /// pacer paces by `thousandths / 1000` from the next poll. Zero pauses: no tick falls
    /// due, the passes still run once per frame. What the frame callback put in
    /// `time_scale` is replaced, since the scale is the assembly's decision on the server.
    pub fn set_time_scale(&mut self, scale: TimeScale) {
        todo!("record the scale for the next step and the pacer")
    }

    /// The baseline of the last bundle this world stepped for `region` and did not refuse:
    /// what a save of the region is diffed against, and a relevance message built over.
    /// `None` for a region no accepted bundle has named.
    #[must_use]
    pub fn baseline(&self, region: RegionId) -> Option<&Arc<Baseline>> {
        self.baselines.get(&region)
    }

    /// One tick, by value, with `frame` and nothing else: for SOUR tests and tools, from
    /// the thread that owns the server through `Executor::block_on`, or from the timer
    /// root. Runs no pass. Applies a pending time scale and, on the first step of a `Loaded`
    /// start, prepends the save's bundles to `frame.loaded`; records the baseline of every
    /// bundle the sync point accepted; logs the lifecycle's load, session and asset failures
    /// and leaves its requests and exports for a frame to take. Resolves with the server at
    /// the next tick, or with [`ServerError`]: the server kept when `Simulation::step`
    /// handed it back, gone otherwise.
    pub fn step(self, executor: &Executor, frame: TickInput) -> ServerStep<'_> {
        todo!("step the simulation once")
    }

    /// One frame of the timer root, by value: `pacer.poll(now)`, then one
    /// [`step`](Self::step) per due tick with `frames(tick)` as its input (the tick the
    /// step runs, `Tick(self.tick().0 + 1)` at each call), then `Simulation::run_passes`
    /// once whether or not a tick ran, then the between-run takes into a [`FrameOutput`]:
    /// the gather's frame, the lifecycle's requests and exports. `None` from `frames` ends
    /// the frame's stepping before that tick: it does not run, the ticks still due this
    /// frame are forgotten (the pacer counted them), and the passes run as usual; what a
    /// bounded run returns at its last tick. The pacer's `behind` is reported, not acted
    /// on: the overload ladder is not in this crate yet. The caller sleeps until
    /// `next_deadline` (`Executor::sleep_until`) and calls again. Resolves with the server
    /// and the output, or with [`ServerError`] as [`step`](Self::step) does; a failure of
    /// the passes after the ticks ran hands the server back with the ticks kept, as
    /// `RunError` does.
    pub fn frame<'a>(
        self,
        executor: &'a Executor,
        now: Instant,
        frames: &'a mut (dyn FnMut(Tick) -> Option<TickInput> + Send),
    ) -> ServerFrame<'a> {
        todo!("poll the pacer, step the due ticks, run the passes, take the outputs")
    }

    /// The save copy of every loaded region at a sync point, by value:
    /// `Simulation::save_point`. Resolves with the server and the [`SavePoint`] the
    /// encoding takes ([`encode_save`](Self::encode_save)), or with [`ServerError`] with the
    /// server kept, since a failed export changes nothing.
    pub fn save_point(self, executor: &Executor) -> ServerSave<'_> {
        todo!("take the save copy")
    }

    /// The container bytes of `save`, between runs: one background task per region
    /// (`Executor::spawn`, `WorkerClass::Background`), each
    /// `RegionRecords::from_export(export, baseline, schemas)?.encode_section()?` with the
    /// region's [`baseline`](Self::baseline) and this world's [`schemas`](Self::schemas),
    /// then `Container::assemble(&ContainerHeader::of(&save, &schemas), sections)` once
    /// every task resolved. Fails with `NotFound` for a region whose baseline this world
    /// never recorded, and as `lactose-persist` fails; nothing is written by this call, so
    /// a failure loses nothing. The bytes go to [`write_slot`](crate::write_slot).
    pub fn encode_save(&self, executor: &Executor, save: SavePoint) -> SaveEncode {
        todo!("spawn one encode per region and assemble")
    }

    /// The relevance message for `region`, between runs, for a client whose set it enters
    /// (architecture section 9.3): `World::export_region`, then
    /// `RegionRecords::from_export(&export, baseline, schemas)?.to_relevance(assets)` with
    /// the region's [`baseline`](Self::baseline). `assets` is the region's resolved table,
    /// or `None` if it has none yet. On the calling thread at milestone 1; milestone 4
    /// moves the diff to a background task and shares one message between co-located
    /// clients. Fails with `NotFound` for a region that is not loaded or whose baseline this
    /// world never recorded, and as the export and the diff fail.
    pub fn relevance(
        &self,
        region: RegionId,
        assets: Option<VersionTable>,
    ) -> Result<RegionRelevance> {
        todo!("export the region and diff it")
    }
}

/// What one frame of the timer root produced, for what runs between frames.
pub struct FrameOutput {
    /// New ticks run this frame, after the cap.
    pub ticks: u32,
    /// The cap was hit with ticks still owed: the overload ladder's input.
    pub behind: bool,
    /// When the next tick falls due: what the caller sleeps until.
    pub next_deadline: Instant,
    /// The replication gather's frame since the last take (`GatherState::take`): the
    /// packer's. Empty on a world with no changed rows; dropped by the headless app.
    pub gathered: StagingFrame,
    /// Regions the lifecycle asked for this frame: the asset store's to answer through a
    /// later frame's `loaded`. Empty under `Directed`.
    pub requests: Vec<RegionRequest>,
    /// Regions the lifecycle unloaded this frame, copied out: `lactose-persist`'s to diff
    /// for a save-on-unload, or to drop.
    pub exports: Vec<RegionExport>,
}

/// How a server run fails: the error, and the server when its world survived. `Some`
/// when the run's `RunError` handed the simulation back (a failure before anything ran,
/// a `Gather` pass that no longer prepares, a failed export), so the caller keeps its
/// world; `None` after `Cancelled`, when the executor shut down during the run and the
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

/// One tick in flight: resolves to the server at the next tick. See [`Server::step`].
#[must_use = "futures do nothing unless awaited"]
pub struct ServerStep<'a> {
    run: Run<'a, Server>,
}

impl Future for ServerStep<'_> {
    type Output = core::result::Result<Server, ServerError>;

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

/// A save copy in flight: resolves to the server and the copy. See [`Server::save_point`].
#[must_use = "futures do nothing unless awaited"]
pub struct ServerSave<'a> {
    run: Run<'a, (Server, SavePoint)>,
}

impl Future for ServerSave<'_> {
    type Output = core::result::Result<(Server, SavePoint), ServerError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.run.as_mut().poll(cx)
    }
}
