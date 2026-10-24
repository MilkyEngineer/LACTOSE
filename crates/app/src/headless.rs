//! The headless app (architecture section 2, "app-headless"): the executor and one server
//! world, stepped from a test or run at tick rate by a tool, with no network, store or
//! presentation. What the `app-headless` binary's `main` is, and what a SOUR test of the
//! assembly drives.

use std::path::PathBuf;

use lactose_core::{Result, Tick};
use lactose_sim::TickInput;
use lactose_task::{ExecutionLog, Executor};

use crate::game::Game;
use crate::runtime::Workers;
use crate::server::Server;
use crate::world::{Start, WorldConfig};

/// What a headless run is built from.
#[derive(Clone, Debug)]
pub struct HeadlessConfig {
    /// The executor: the defaults, or what SOUR's session asks for.
    pub workers: Workers,
    /// The world: [`WorldConfig::headless`], or that with SOUR's layout.
    pub world: WorldConfig,
    /// Fresh, or from a save.
    pub start: Start,
}

/// The headless app: the executor, the server world and the policy. Built by
/// [`start`](Self::start) on the thread that owns it, which drives every run through
/// `Executor::block_on` and is never a worker.
pub struct Headless {
    executor: Executor,
    server: Option<Server>,
    tick: Tick,
}

/// What [`Headless::run`] did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RunReport {
    /// Ticks run.
    pub ticks: u32,
    /// Frames run, the last one included; `frames <= ticks` unless time scale was zero.
    pub frames: u32,
    /// Frames whose pacer reported `behind`.
    pub behind: u32,
    /// The world hash after the last tick.
    pub hash: u64,
}

impl Headless {
    /// [`start_executor`](crate::start_executor) with `config.workers`, then
    /// [`build_server`](crate::build_server) with `config.world` and `config.start`. Fails
    /// as they fail, with the executor shut down again; the policy's response to a startup
    /// failure is `Stop`, so the binary's `main` returns it.
    pub fn start(game: &Game, config: HeadlessConfig) -> Result<Self> {
        todo!("start the executor and build the server")
    }

    /// The executor, for what a test spawns or sleeps beside the world.
    #[must_use]
    pub fn executor(&self) -> &Executor {
        &self.executor
    }

    /// The server world, between runs; `None` after a run tore it.
    #[must_use]
    pub fn server(&self) -> Option<&Server> {
        self.server.as_ref()
    }

    /// The server world, between runs; `None` after a run tore it.
    pub fn server_mut(&mut self) -> Option<&mut Server> {
        self.server.as_mut()
    }

    /// The current tick, or the tick the world was torn at.
    #[must_use]
    pub fn tick(&self) -> Tick {
        self.tick
    }

    /// `Server::hash`, or `Cancelled` after a run tore the world.
    pub fn hash(&self) -> Result<u64> {
        todo!("hash the server world")
    }

    /// One tick with `frame`, driven from the calling thread (`Executor::block_on` over
    /// [`Server::step`]), no pass and no pacing: what `lactose_sim::harness::run_recorded`
    /// does for a simulation, for a SOUR test or a tool that records the hash after every
    /// tick. On a kept failure the server stays and the error is returned; on a torn one
    /// `server()` is `None` afterwards and every later run fails with `Cancelled`.
    pub fn step(&mut self, frame: TickInput) -> Result<()> {
        todo!("block on one step")
    }

    /// The timer root for `ticks` ticks: a task spawned on a frame-critical worker that
    /// owns the server until it returns, driven from the calling thread. Each frame is
    /// [`Server::frame`] at `Instant::now()` with `frames` as its callback, wrapped to
    /// return `None` once `ticks` ticks have run so the last frame stops exactly there,
    /// then `Executor::sleep_until(next_deadline)`; what a frame produced is dropped
    /// (gathered rows) or logged (requests, exports), since the headless app has no packer
    /// and no store. Returns when `ticks` ticks have run, with the report, or with the first
    /// error as [`step`](Self::step) does. A zero time scale makes this wait forever: a
    /// tool pauses nothing it wants to finish.
    pub fn run(
        &mut self,
        ticks: u32,
        frames: Box<dyn FnMut(Tick) -> TickInput + Send>,
    ) -> Result<RunReport> {
        todo!("spawn the timer root and block on it")
    }

    /// A save of every loaded region to `slot`, from the calling thread, between runs: the
    /// save copy ([`Server::save_point`]), its encoding ([`Server::encode_save`]) and the
    /// slot write ([`write_slot`](crate::write_slot)), each blocked on in turn. The world
    /// is unchanged either way: a failed copy or encoding writes nothing, a failed write
    /// leaves the previous save in place. Returns the error for the binary's `main` to
    /// report; the policy's response to [`Source::Save`](crate::Source::Save) is `Log`,
    /// which is what a tool does with it.
    pub fn save(&mut self, slot: PathBuf) -> Result<()> {
        todo!("block on the save copy, the encoding and the slot write")
    }

    /// Drops the server world (which drops the physics backend and joins its solver pool),
    /// reads the execution log if chaos recorded one (D112: read, not taken), then
    /// `Executor::shutdown`, which joins the workers. What a SOUR test writes the log from.
    pub fn shutdown(self) -> Option<ExecutionLog> {
        todo!("drop the world, read the log, shut the executor down")
    }
}
