//! The headless app (architecture section 2, "app-headless"): the executor, the IO pool
//! and one server world, stepped from a test or run at tick rate by a tool, with no
//! network, store or presentation. What the `app-headless` binary's `main` is, and what a
//! SOUR test of the assembly drives.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use lactose_core::{Error, ErrorKind, Result, Tick};
use lactose_io::{Io, IoConfig};
use lactose_sim::TickInput;
use lactose_task::{ExecutionLog, Executor, WorkerClass};
use tracing::debug;

use crate::game::Game;
use crate::policy::{ErrorPolicy, LOG_TARGET, Source};
use crate::runtime::{Workers, start_executor};
use crate::save::Cooked;
use crate::server::{Failed, Server, StepOutput, build_server};
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
    // Declared in drop order for an app dropped without `shutdown`: the world before the
    // IO pool, the IO pool before the executor (`crates/io/README.md`).
    server: Option<Server>,
    io: Io,
    executor: Executor,
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
        let HeadlessConfig { workers, world, start, saves } = config;
        let replay = workers.replay.is_some();
        let executor = start_executor(workers).inspect_err(startup)?;
        let platform = Arc::clone(executor.platform());
        let io = match Io::start(&*platform, IoConfig::for_platform(&*platform)) {
            Ok(io) => io,
            Err(error) => {
                startup(&error);
                executor.shutdown();
                return Err(error);
            }
        };
        let built = match saves {
            Some(saves) => executor.block_on(io.create_dir_all(saves)),
            None => Ok(()),
        }
        .and_then(|()| build_server(&executor, game, world, start, cooked));
        match built {
            Ok(server) => {
                Ok(Self { tick: server.tick(), executor, io, server: Some(server), replay })
            }
            Err(error) => {
                startup(&error);
                io.shutdown();
                executor.shutdown();
                Err(error)
            }
        }
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
        let server = self.server.as_ref().ok_or_else(torn)?;
        server.hash().inspect_err(|error| {
            ErrorPolicy::MILESTONE_1.decide(Source::Server { kept: true }, error);
        })
    }

    /// One tick with `frame`, driven from the calling thread (`Executor::block_on` over
    /// [`Server::step`]), no pass and no pacing: what `lactose_sim::harness::run_recorded`
    /// does for a simulation, for a SOUR test or a tool that records the hash after every
    /// tick. Returns what the step produced, its refusals included, so a test sees a
    /// refused bundle. On a kept failure the server stays and the error is returned; on a
    /// torn one `server()` is `None` afterwards and every later run fails with `Cancelled`.
    pub fn step(&mut self, frame: TickInput) -> Result<StepOutput> {
        let server = self.server.take().ok_or_else(torn)?;
        match self.executor.block_on(server.run_step(&self.executor, frame)) {
            Ok((server, output, _)) => {
                self.keep(server);
                Ok(output)
            }
            Err(failed) => Err(self.failed(failed)),
        }
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
        if self.replay {
            let error = Error::new(
                ErrorKind::Unsupported,
                "a paced run cannot follow a recorded execution log: replay is for step-driven runs",
            );
            ErrorPolicy::MILESTONE_1.decide(Source::Executor, &error);
            return Err(error);
        }
        let mut server = self.server.take().ok_or_else(torn)?;
        server.resync(Instant::now());
        let executor = self.executor.clone();
        let task = self
            .executor
            .spawn(WorkerClass::FrameCritical, timer_root(executor, server, ticks, frames));
        match self.executor.block_on(task) {
            Ok(Ok((server, mut report))) => {
                self.keep(server);
                report.hash = self.hash()?;
                Ok(report)
            }
            Ok(Err(failed)) => Err(self.failed(failed)),
            Err(error) => {
                // The task was cancelled with the executor, and the server with it.
                ErrorPolicy::MILESTONE_1.decide(Source::Executor, &error);
                Err(error)
            }
        }
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
        let server = self.server.take().ok_or_else(torn)?;
        let (server, saved) = match self.executor.block_on(server.save_point(&self.executor)) {
            Ok(done) => done,
            Err(failure) => {
                let failed =
                    Failed { error: failure.error, server: failure.server, tick: self.tick };
                return Err(self.failed(Box::new(failed)));
            }
        };
        let encoded = self.executor.block_on(server.encode_save(&self.executor, saved));
        self.keep(server);
        let written = encoded.and_then(|bytes| {
            self.executor.block_on(self.io.replace(slot, bytes)).map_err(|failed| failed.error)
        });
        written.inspect_err(|error| {
            ErrorPolicy::MILESTONE_1.decide(Source::Save, error);
        })
    }

    /// [`Server::shutdown`] (the world dropped, its solver threads waited for), then
    /// `Executor::execution_log` read if chaos recorded one (D112: read, not taken), then
    /// `Io::shutdown`, then `Executor::shutdown`, which joins the workers. After a run tore
    /// the world there is no server: `Server::shutdown` is skipped, and the torn world's
    /// solver pool is terminated when that world drops and not waited for. What a SOUR test
    /// writes the log from.
    pub fn shutdown(self) -> Option<ExecutionLog> {
        let Self { executor, io, server, .. } = self;
        if let Some(server) = server {
            server.shutdown();
        }
        let log = executor.execution_log();
        io.shutdown();
        executor.shutdown();
        log
    }

    /// The server back after a run, at its tick.
    fn keep(&mut self, server: Server) {
        self.tick = server.tick();
        self.server = Some(server);
    }

    /// A run's failure: the server back if it survived, else the tick it was torn at.
    fn failed(&mut self, failed: Box<Failed>) -> Error {
        let Failed { error, server, tick } = *failed;
        match server {
            Some(server) => self.keep(server),
            None => self.tick = tick,
        }
        error
    }
}

/// The timer root's loop for [`Headless::run`], on the task that owns the server: frames
/// at `Instant::now()` until `ticks` ticks have run, sleeping to each deadline between.
/// The report's hash is the caller's to fill.
async fn timer_root(
    executor: Executor,
    mut server: Server,
    ticks: u32,
    mut frames: Box<dyn FnMut(Tick) -> TickInput + Send>,
) -> core::result::Result<(Server, RunReport), Box<Failed>> {
    let mut report = RunReport { ticks: 0, frames: 0, behind: 0, hash: 0 };
    let mut issued = 0_u32;
    let mut bounded = move |tick: Tick| {
        (issued < ticks).then(|| {
            issued += 1;
            frames(tick)
        })
    };
    while report.ticks < ticks {
        let (stepped, output) = server.run_frame(&executor, Instant::now(), &mut bounded).await?;
        server = stepped;
        report.ticks += output.ticks;
        report.frames += 1;
        report.behind += u32::from(output.behind);
        for request in &output.requests {
            debug!(target: LOG_TARGET, ?request, "a region request the headless app leaves unanswered");
        }
        if report.ticks >= ticks {
            break;
        }
        if let Err(error) = executor.sleep_until(output.next_deadline).await {
            ErrorPolicy::MILESTONE_1.decide(Source::Executor, &error);
            let tick = server.tick();
            return Err(Box::new(Failed { error, server: Some(server), tick }));
        }
    }
    Ok((server, report))
}

/// The error of every run after one tore the server world.
fn torn() -> Error {
    Error::new(ErrorKind::Cancelled, "a run tore the server world: there is nothing to run")
}

/// A startup failure, logged as the policy decides: `Stop`.
fn startup(error: &Error) {
    ErrorPolicy::MILESTONE_1.decide(Source::Executor, error);
}
