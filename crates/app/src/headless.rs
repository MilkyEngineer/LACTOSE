//! The headless app (architecture section 2, "app-headless"): the executor, the IO pool
//! and one server world, stepped from a test or run at tick rate by a tool, with no
//! network, store or presentation. What the `app-headless` binary's `main` is, and what a
//! SOUR test of the assembly drives.

use std::path::PathBuf;
use std::sync::Arc;

use lactose_core::{Result, Tick};
use lactose_io::Io;
use lactose_sim::TickInput;
use lactose_task::{ExecutionLog, Executor};

use crate::game::Game;
use crate::runtime::Workers;
use crate::save::Cooked;
use crate::server::{Server, StepOutput};
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
    /// The save directory, created at start with `Io::create_dir_all`; `None` for a run
    /// that never saves, when a [`save`](Headless::save) into a directory that does not
    /// exist fails as `Io::replace` says (`NotFound`).
    pub saves: Option<PathBuf>,
}

/// The headless app: the executor, the IO pool, the server world and the policy. Built by
/// [`start`](Self::start) on the thread that owns it, which drives every run through
/// `Executor::block_on` and is never a worker.
pub struct Headless {
    executor: Executor,
    io: Io,
    server: Option<Server>,
    tick: Tick,
    replay: bool,
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
    /// [`start_executor`](crate::start_executor) with `config.workers`; then
    /// `Io::start(&platform, IoConfig::for_platform(&platform))` with `config.workers`'
    /// platform, or `StdPlatform` when it is `None`; then `Io::create_dir_all` of
    /// `config.saves` when `Some`, blocked on; then [`build_server`](crate::build_server)
    /// with `config.world`, `config.start` and `cooked`. Fails as they fail, with what was
    /// started shut down again in reverse; the policy's response to a startup failure is
    /// `Stop`, so the binary's `main` returns it.
    pub fn start(game: &Game, config: HeadlessConfig, cooked: Arc<dyn Cooked>) -> Result<Self> {
        todo!("start the executor and the IO pool, create the save directory, build the server")
    }

    /// The executor, for what a test spawns or sleeps beside the world.
    #[must_use]
    pub fn executor(&self) -> &Executor {
        &self.executor
    }

    /// The IO pool, for what a test or a tool reads or lists beside the world.
    #[must_use]
    pub fn io(&self) -> &Io {
        &self.io
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
    /// tick. Returns what the step produced, its refusals included, so a test sees a
    /// refused bundle. On a kept failure the server stays and the error is returned; on a
    /// torn one `server()` is `None` afterwards and every later run fails with `Cancelled`.
    pub fn step(&mut self, frame: TickInput) -> Result<StepOutput> {
        todo!("block on one step")
    }

    /// The timer root for `ticks` ticks: [`Server::resync`] at `Instant::now()` first, so
    /// the gap since the last frame (steps, a save, the start) is not caught up at the cap;
    /// then a task spawned on a frame-critical worker that owns the server until it
    /// returns, driven from the calling thread. Each frame is [`Server::frame`] at
    /// `Instant::now()` with `frames` as its callback, wrapped to return `None` once
    /// `ticks` ticks have run so the last frame stops exactly there, then
    /// `Executor::sleep_until(next_deadline)`; what a frame produced is dropped (gathered
    /// rows) or logged (requests, refusals), since the headless app has no packer and no
    /// store. Returns when `ticks` ticks have run, with the report, or with the first error
    /// as [`step`](Self::step) does. Refuses with `Unsupported`, before anything runs, an
    /// executor built with `Workers::replay`: a paced run interleaves steps and passes by
    /// wall time, which no recorded log can follow, so replay is for step-driven runs. A
    /// zero time scale makes this wait forever: a tool pauses nothing it wants to finish.
    pub fn run(
        &mut self,
        ticks: u32,
        frames: Box<dyn FnMut(Tick) -> TickInput + Send>,
    ) -> Result<RunReport> {
        todo!("resync the pacer, spawn the timer root and block on it")
    }

    /// A save of every region, loaded or kept, to `slot`, from the calling thread, between
    /// runs: [`Server::save_point`], [`Server::encode_save`] and `Io::replace(slot, bytes)`,
    /// each blocked on in turn. A failed save copy is `Source::Server`, `Stop`: the error
    /// is returned and the server is as it was. A failed encode or replace is
    /// `Source::Save`, `Log`: the error is returned, the world is unchanged, and the slot
    /// holds the previous save unless `Io::replace` failed after its rename, as its
    /// contract states (`crates/io/README.md`, "The atomic replace"); the bytes a
    /// `ReplaceFailed` hands back are dropped, since a tool retries by saving again.
    pub fn save(&mut self, slot: PathBuf) -> Result<()> {
        todo!("block on the save copy, the encoding and the replace")
    }

    /// [`Server::shutdown`] (the world dropped, its solver threads waited for), then
    /// `Executor::execution_log` read if chaos recorded one (D112: read, not taken), then
    /// `Io::shutdown`, then `Executor::shutdown`, which joins the workers. After a run tore
    /// the world there is no server: `Server::shutdown` is skipped, and the torn world's
    /// solver pool is terminated when that world drops and not waited for. What a SOUR test
    /// writes the log from.
    pub fn shutdown(self) -> Option<ExecutionLog> {
        todo!("shut the server down, read the log, shut the pool and the executor down")
    }
}
