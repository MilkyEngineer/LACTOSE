# The assembly (`lactose-app`)

What turns every crate's registration into an executor, a solver pool and the worlds, and
then runs them: the timer root, the owner of the error policy, the place the `dyn`
resources are wired, the client's loop around the prediction world, the frame of a world
that is shown, the server's store of saved regions, and the save path from the sync point
to the slot. The thin binaries link it: `app-headless` at milestone 1, `app-client` at
milestone 2 with a local world that is shown, and `app-server` at milestone 4, when
`app-client` gains its replicated world.

| | |
|---|---|
| Tier | assembly |
| Determinism boundary | outside |
| Unsafe | none, `#![forbid(unsafe_code)]` |

**Status: implemented, but for the shown frame, which is a facade (below).** Every other
public signature is the facade's (D131), unchanged, and its body is implemented; the proof
below runs. Every sentence under
"Behaviour the signatures cannot express" is one a dependent crate may rely on and the
implementation must meet. Architecture sections 2, 3, 4, 5.1, 5.2, 8.3, 9.2, 9.3, 9.7,
11, 14, 15.2, 15.3, 16, 18, 19, 20 and 22 are the design; where this file and the
architecture disagree, the architecture wins and this file is wrong. Decision D131
records the choices those sections left open. `crates/sim/README.md`,
`crates/task/README.md`, `crates/predict/README.md`, `crates/net-api/README.md`,
`crates/net-host/README.md`, `crates/persist/README.md`, `crates/io/README.md`,
`crates/physics-api/README.md`, `crates/physics-rapier/README.md`,
`crates/input/README.md` and `crates/anim/README.md` are the contracts this crate builds on
and never restates: what they say about runs, the sync point, the pacer, the predictor's
loop, staged frames and the fold, the container, the atomic replace, the solver pool and
the registration functions holds here unchanged.

**Status of the shown frame: facade.** Module `shown`, `WorldConfig::shown`,
`Source::Render` and `ErrorPolicy::MILESTONE_2` are D162's, which adds to D131's surface.
It changes no function and no field D131 fixed, and one type, by adding to it: `Source`,
an enum that is not `#[non_exhaustive]`, gains the variant `Render`, so a `match` that
names every variant gains an arm, and none is written outside this crate. Every signature
of theirs is final and every body that decides anything is `todo!()`, which lane
`frame-impl` writes. The blocks from "The shown frame" to "What D162 asks of other
crates" are the contract `frame-impl` meets, and "Proof for the shown frame" says what is
held now. Architecture sections 3, 5.1, 5.3, 8.1, 8.3, 11, 16 and 19 and the milestone 2
plan's decisions 5.1, 5.5, 5.6, 5.11, 5.16 and 5.18 are their design. They build on
`crates/present-api/README.md` and
`crates/rhi-api/README.md`, which this crate names, and, behind the two objects the binary
hands it, on `crates/render-graph/README.md`, `crates/renderer/README.md` and
`crates/platform/README.md`, which it may not name (`crate-tiers.toml`,
`[server_denylist]`; decision 5.11).

D142 changed no signature of this crate: `build_server` and `load` do what the bullet
"Binds" under "Saves and loads" states, and "Proof for D142" names the tests that hold it.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `app-headless`, the milestone 1 netcode spike,
the SOUR test of the assembly, `app-client` from milestone 2 and, from milestone 4,
`app-server` and `net` are written against this surface. `tests/headless.rs` holds the
binary's `main` (which `crates/app-headless` is) and the SOUR test of the assembly,
`tests/spike.rs` is a compile-only sketch of the spike's loop, and `tests/shown.rs` is one
of the client binary's wiring and of `frame-impl`'s tests: a change that breaks one is a
signature change to list.

## Surface

| Item | Purpose |
|---|---|
| `CrateRegistration`, `Game` | What a game crate contributes, as function pointers (its generated `register_types`, `setup`, `register_systems`, `register_migrations`), and the game as the binary hands it over: every crate in call order, the game's own ordering edges, its session applier |
| `Workers`, `start_executor` | What the assembly decides for the executor beyond the builder's defaults (SOUR's workers and chaos modes, a replay, the platform), and the one place an executor is built |
| `CATCH_UP_CAP`, `LOG_RETENTION`, `TICK_PERIOD` | The timer root's constants: four ticks per frame, the structural-log retention every world is built with, the tick period every pacer is built with |
| `WorldConfig`, `Start` | What the assembly decides for one world (layout, region control, policy, physics, retention), and where a server world starts: fresh, or from a saved world |
| `build_server`, `Server` | A server world between runs: the simulation, its pacer, the schema table, the time scale, the baseline of every loaded region, the store of every saved region that is not loaded (`kept`); `step` for an unpaced tick, `frame` for one frame of the timer root, `resync` for the pacer, `save_point` and `encode_save` for a save, `relevance` for a region's relevance message, `shutdown` to drop the world and wait for its solver threads |
| `ServerStep`, `ServerFrame`, `ServerSave`, `StepOutput`, `LifecycleFailures`, `FrameOutput`, `ServerError` | The server's runs in flight; what a step produced (the lifecycle's requests and its load, session and asset refusals); what a frame produced (ticks, `behind`, the next deadline, the gathered frame, every step's requests and refusals); and how a run fails with the server kept or gone |
| `build_client`, `Client` | A client's prediction world between runs: the predictor, its pacer, the lead, the sample history; `push` for a staged frame, `clock` for an estimate, `frame` for one frame of the timer root's client half, `shutdown` |
| `ClientFrame`, `ClientOutput`, `ClientError` | The client's frame in flight, what it produced (ticks, `behind`, the next deadline, the reconciliation, the rollback's wall time, a resync, the presentation events), and how it fails |
| `ErrorPolicy`, `Source`, `Response`, `LOG_TARGET` | The policy as a table from where an error came from to what the assembly does with it; the `tracing` target everything here logs at, `app` |
| `Cooked`, `SavedRegion`, `SavedWorld`, `load` | Where cooked content comes from until the asset store exists; one region's saved state between loads; every region's saved state at one tick, which is what a container decodes to and what a save point yields; the load path from the slot's bytes to a `Start::Loaded` |
| `SaveEncode` | A save being encoded on background tasks; the bytes then go to `lactose_io::Io::replace`, which nothing here wraps |
| `install_crash_hook` | The panic hook every engine binary installs first in `main`: a text report to stderr, then to a file in the directory the binary names, then the abort ("The crash hook" below) |
| `HeadlessConfig`, `Headless`, `RunReport` | The headless app: the executor, the IO pool and one server world, stepped from a test or run at tick rate by a tool, saved to a slot, shut down in order |
| `WorldConfig::shown`, `RECORD_RESERVATION` (facade) | The configuration of a world that is shown, and the physics reservation a frame's record is left with while the step blocks: one, until `record-stall` reports |
| `JOIN_MARGIN`, `SHED_DEPTH` (facade) | How long before the next tick's deadline a paced frame stops waiting for the frame before it, 2 ms; and the count of finished pictures still on their way to the display, taken as a present call begins, at which the frame after that call is shed: three, as `reviewer-gpu` measured it on the M1. `frame-pace` reads both |
| `build_shown`, `Shown` (facade) | A shown world between runs: a `Server` built by `build_server`'s path with extract's outputs installed, and the render side driven beside it; `step` for one tick and its frame, joined; `frame` for one paced frame; `flush` to join the frame in flight; `stop` to end the render side and hand both back; `server` and `server_mut` for what a `Server` offers by reference; `stats` |
| `RenderSide`, `PresentSide`, `PresentLane` (facade) | The two traits the assembly drives the render side through, which name no backend (decision 5.11): the object the binary builds over the device, the graph, the renderer and the presentation schedule; the state of the present thread, over the swapchain; and the lane to it |
| `TARGETS`, `TargetIndex`, `Region`, `FrameTime`, `Draw`, `KindCounts`, `FrameCounts`, `Submitted`, `Finished`, `PresentReport` (facade) | What crosses those traits, all plain data: the two offscreen targets by index; whether a region is open, or the point that frees one; the frame's time and fraction; what a frame draws into; a drawn frame's counts by world kind; what a submit left; what one present call reads, and what it did |
| `ShownStep`, `ShownFrame`, `ShownFlush`, `ShownStop`, `Rendered`, `ShownOutput`, `ShownStats`, `Stopped`, `ShownError`, `ShownKept` (facade) | The shown world's runs in flight; what a frame did on the render side (drew, hid, waited for a region, was busy, was torn); what a paced frame produced; the counters since the build; what `stop` hands back; and how a run fails, with what survived it |
| `Source::Render`, `ErrorPolicy::MILESTONE_2` (facade) | The render side as a source of errors, and the table a shown world is built with: a lost device stops the run, and any other error tears the frame or skips the present |

Hidden plumbing, which the implementation may reshape: the solver pool's construction and
the exit count a world waits on (`runtime::solver_pool`, `runtime::SolverExit`); the
pending time scale, the scale the last step carried, the bundles queued for the next step
and the migrations inside `Server`; the clock estimate with the ticks granted since it was
taken, the lead, the pending resync, the newest frame tick whose fold failed and the tick
the last frame's steps started from inside `Client`; the frame in flight or the idle render
side, the snapshot's reading half, the present role's queue path, the present call in
flight, each target's last read and the flags of the frame's state inside `Shown`
(`shown::Flight`, `shown::Side`); the bit inside `ErrorPolicy` that says which milestone's
table it is.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| `app-headless` (milestone 1) | Start an executor, an IO pool and a server world from the game's registrations, run N ticks at tick rate, report the hash, save, shut down in order; `main` returns the first fatal error | `Headless::{start, run, hash, save, shutdown}`, `RunReport`; the policy's `Stop` is `main`'s `Err` |
| | The same from a save, losing no saved region | `load` before any world exists, `Start::Loaded`, the store ("Saves and loads" below) |
| Every engine binary | A panic reported as text to stderr and to a file, and the process aborted, in every profile | `install_crash_hook`, first in `main` ("The crash hook" below) |
| A SOUR test of the assembly | Build from the session's workers, layout and chaos; one unpaced tick at a time with the hash after each and every refusal visible; the chaos log at the end | `Workers::{frame_workers, chaos}`, `WorldConfig::layout`, `Headless::{step, hash, shutdown}`, `StepOutput::failures`; the glue is `tests/headless.rs`, section 4 |
| The milestone 1 netcode spike, server side | Step the server once per frame with the sessions and inputs its jitter buffer released; take the gathered frame for its packer; the player's entity after a join; a full frame for the join over the table the server simulates with | `Server::frame` with a frame callback, `FrameOutput::gathered`, `Server::simulation` with `lactose_input::player_entity`, `Server::relevance` and `Server::time_scale` into a `StagedFrame::resync` |
| The spike, client side | Build the prediction world on `JoinAccepted`; queue staged frames; one frame with a scripted sample; the last eight samples to send; a resync to request; the rollback cost and correction count to record | `build_client`, `Client::{push, frame, history}`, `ClientOutput::{reconciled, rollback, resync}` |
| | Pace by its own round-trip estimate until `net` computes one | `Client::clock` with a `ClockEstimate` it fills by hand |
| Game crates (milestone 5; the sketches now) | Register once and be built into every world kind, with no code of theirs running before `main` and nothing a dev-profile restart cannot redo | `CrateRegistration` as a `static` of function pointers; `Game::crates` in call order; `Game::sessions` for the player's entity |
| | Order their own sets against another crate's, the engine's included, without owning a stage list | `Game::order`; a game crate names `lactose_input::sets::APPLY`, `lactose_physics_api::sets::WRITE_BACK` and `lactose_anim::sets::ADVANCE` directly, since it depends on those crates |
| `persist` | Its pure calls placed on threads: one diff per exported region and one encode per region on background workers, the assembly once they resolve; the loader run before any world exists with the registry the world will have; the records' baseline by hash; the records of an unloaded region kept until it loads again; the saved world's binds read from every region's records and given to the builder before any region loads (D142) | `Server::save_point`, `Server::encode_save`, `load`, `Cooked::baseline`, `Server::baseline`, `Server::kept`, `build_server` with `Start::Loaded` |
| `io` | The pool started once by `app` with the binary's platform, the save directory created at startup, the slot written as one `Io::replace`, the pool shut down before the executor | `Headless::{start, save, shutdown}`; "Saves and loads" below |
| The store's driver (`asset` at milestone 3, `net` at milestone 4) | Every `Load` the lifecycle issues, told which ones the server answers itself; the region's table resolvable for either; a `Release` when a region's export is in the store | `StepOutput::requests`, `FrameOutput::requests`, `Server::kept`; "Saves and loads" below |
| `asset` (milestone 3) | A stand-in with the store's shape, replaced by its traits with a listed signature change | `Cooked`; "Not in this crate yet" |
| `lactose-app-client` (milestone 2; `frame-impl`) | Build a local world that is shown from the game's registrations and two objects of its own, with no backend named below it; run one paced frame from its root; know what each frame did | `build_shown` with `WorldConfig::shown()`, a `Box<dyn RenderSide>` and a `PresentLane`; `Shown::frame`, `ShownOutput`, `Shown::stats`; "The shown frame", "What `lactose-app-client` wires" |
| | Write its render side over the graph and the renderer, and its present side over the swapchain, with every contract stated | `RenderSide`, `PresentSide` and the values they pass; "The render side", "The targets and the present thread"; `tests/shown.rs`, sections 1 and 2 |
| | End with frames in flight, and after a lost device, in the order `lactose-platform` gives, with no `?` able to drop the render side on its root's worker | `Shown::stop`, `Stopped`, `ShownError::kept`, which a caller matches since the error does not convert; "Errors", "Shutdown" |
| The fixed-frame mode of decision 5.1 | One tick and one frame at a time, with a time, an extent and a sample of its own; every frame drawn; the last target's bytes and the frame's values to dump | `Shown::step` with a `FrameTime` and an extent; `Rendered::Drew::point` to wait for; `Stopped::render` taken back by `Any`; `tests/shown.rs`, section 4 |
| `frame-impl`'s tests of criterion 3 | A render side with no device that logs and blocks where the test says; a shown world and the same world unshown, stepped alike; the hash after each tick | `RenderSide` and `PresentSide` implemented by a test; `Shown::step`, `Shown::server`, `Server::step`; "Determinism of a shown world"; `tests/shown.rs`, section 5 |
| `lactose-renderer`, `lactose-render-graph` and `lactose-presentation`, behind the binary's object | A job's context for the record and for the presentation schedule's forks, with no lock and no `Sync` asked of the object; the sink installed and replaced at the right frames; one extent a frame; the target's last read as a point; no pass of extract between `prepare` and the next open region; no submit behind a present on a worker; the drop off the workers, the graph first | `RenderSide::{open, present, draw}` take `&mut JobContext`, lent by `Executor::run_job`; "The frame's three states", "Counts, torn frames and resets"; `Draw`; "The shown frame"; "The targets and the present thread"; `Stopped::render` |
| `lactose-platform`'s present thread, and the `swapchain` lane | The state its thread owns, declared by the frame; one call in flight; a last call that drops the swapchain; no host signal owed | `PresentSide` as the `S` of `present_thread`, `PresentLane`; "The targets and the present thread"; `Shown::stop` |
| `measure-m2` | What to time and count for `frame-pace` and `record-stall`, what to record beside the counts, and the reservation to vary | `ShownOutput`, `Rendered` with `queued`, `held`, `shed` and `Busy`, `Shown::stats`, `PresentReport::waiting`; `JOIN_MARGIN` and `SHED_DEPTH`; `WorldConfig::physics` beside `RECORD_RESERVATION`; "Pacing" |
| `game-sim` (the mover; `frame-impl`) | What to write so that the local world moves, with the binary declaring nothing a world registers | "The mover of the local world" |
| `app-server`, `net`, and `app-client`'s replicated world (milestone 4) | The places named now and the signatures that will change | `Client` gains the replicated world; `build_client` loses `entity`; `Workers::platform` already takes the platform crate's `ClientPlatform`, from milestone 2; `Server::relevance` already takes the executor; a shown client's two worlds; "Not in this crate yet" |
| The error policy (architecture section 19) | One place that decides, as a value a test can read | `ErrorPolicy::MILESTONE_1`, `ErrorPolicy::MILESTONE_2`, `Source`, `Response`; "Errors" below |

## The registration a game crate writes

A game crate exposes its generated functions, and nothing runs before `main` (architecture
section 18). It may name neither this crate nor `lactose-persist` (`crate-tiers.toml`), so it
writes no `CrateRegistration` (`game/game-sim` is the one there is):

```rust
use lactose_core::Result;
use lactose_sim::{Registration, SessionApplier, TypeTable};

pub fn register_types(types: &mut TypeTable) -> Result<()> { Ok(()) }
pub fn register_systems(registration: &mut Registration) {}
pub fn sessions() -> Box<dyn SessionApplier> { todo!() }
```

The binary holds the registration value, lists the crates and hands the game to the
assembly (`tests/headless.rs`, section 3, is the headless binary's `main` in full, with the
cooked content it builds in memory). A binary declares nothing a world registers, so it
passes no setup and no migration function (architecture section 8.1, D152); where a game
crate's registration value and its migrations are declared is decided before milestone 5
(`docs/verification.md`):

```rust
use std::sync::Arc;

use lactose_app::{
    Cooked, CrateRegistration, Game, Headless, HeadlessConfig, Start, Workers, WorldConfig,
};

static GAME_SIM: CrateRegistration = CrateRegistration {
    name: "game-sim",
    types: game_sim::register_types,
    setup: None,
    systems: game_sim::register_systems,
    migrations: None,
};

static GAME: Game = Game {
    name: "sketch",
    crates: &[GAME_SIM],   // in call order; game-ai and game-content follow at milestone 5
    order: None,           // the game's own `Registration::order` edges
    sessions: Some(game_sim::sessions), // `None` drops and logs every session event
};

fn main() -> lactose_core::Result<()> {
    let cooked: Arc<dyn Cooked> = Arc::new(InMemory::new()?);
    let config = HeadlessConfig {
        workers: Workers::default(),
        world: WorldConfig::headless(),
        start: Start::Fresh,
        saves: Some("saves".into()),
    };
    let mut app = Headless::start(&GAME, config, cooked)?;
    let report = app.run(600, Box::new(|_tick| lactose_sim::TickInput::default()))?;
    println!("{:#018x}", report.hash);
    app.shutdown();
    Ok(())
}
```

## Behaviour the signatures cannot express

**Registration** (architecture sections 5.2, 6.1 and 18; `crates/sim/README.md`, "The
registration a game crate writes")

- Every `TypeTable` of the process is built by `Game::types`, a fresh one per world and per
  load, in this order and no other: `TypeTable::new()`; `lactose_input::register_types`,
  `lactose_net_api::register_types`, `lactose_physics_api::register_types`,
  `lactose_anim::register_types`, `lactose_predict::register_types`; then every
  `Game::crates` entry's `types` in list order. Every world kind is built from the same
  function of the same list, the engine's prediction-only and client-only types included,
  so the protocol hash of architecture section 9.1 is the same on the client and the
  server and a save written by one world of the process loads in any other. Registration
  order changes no hash (`crates/sim/README.md`, "The world hash"), so the order is for the
  log and the error message alone: a failing `types` fails `Game::types` with the crate's
  `name` in the message.
- Every world, whatever its kind, gets the same setup functions in this order:
  `lactose_net_api::setup`; `lactose_physics_api::setup(world, backend, config.physics)`
  with the backend built for this world ("The executor and the solver pool" below); then
  every crate's `setup` that is `Some`, in list order. The same registration functions in
  this order: `lactose_input::register_systems`, `lactose_net_api::register_systems`,
  `lactose_physics_api::register_systems`, `lactose_anim::register_systems`; every crate's
  `systems` in list order; `Game::order` if `Some`; then the assembly's own ordering
  function, which holds the edges between engine crates' sets and is empty at milestone 1
  (no two engine crates place an exclusive system in one stage or pass). The edges a game
  needs against the engine's sets are the game's own, in `Game::order`: an exclusive
  `Stage::Input` set after `lactose_input::sets::APPLY`, an exclusive `Stage::Post` set
  against `lactose_physics_api::sets::WRITE_BACK`, a parameter writer before
  `lactose_anim::sets::ADVANCE`. `StageBuilder::add` skips a system whose `worlds` exclude
  the kind, so one set of functions serves every world.
- Every world gets `SimulationBuilder::inputs(Box::new(PlayerInputApplier::new()))`. A
  server world gets `sessions(game.sessions())` when `Game::sessions` is `Some`, and the
  builder's default otherwise. A prediction world gets
  `check(lactose_predict::check_prediction_reads)` and no session applier; its staged-state
  hook is the predictor's (`Predictor::new` installs it). Nothing else is added: the
  replicated world's hook is `lactose-net`'s and the replicated world is not built here
  yet.
- `WorldConfig` goes into the builder as `layout`, `regions`, `policy` and `log_retention`;
  `physics` goes to the backend's setup. `log_retention` below `CATCH_UP_CAP` is refused at
  build with `InvalidData`, since the builder cannot check it and this crate owns the cap.
  `build_client` refuses `regions` other than `Directed` with `InvalidData` before anything
  is built: a prediction world loads the regions the frames name, in the mode its fold gives
  them ("The client's frame" below). `lactose-sim`'s builder refuses an `Anchored` world
  that is not a `Server` as well; `build_client`'s check refuses it earlier, before the
  solver pool starts, with its own message. A `Start::Loaded` sets `tick` and `next_net_id`
  from the saved world; `Start::Fresh` leaves the builder's defaults.
- A function pointer is the registration's shape because the dev profile's restart rebuilds
  the worlds from the same functions with nothing to re-create, because every world of a
  process is built from the same set, and because nothing a registration holds can reach a
  save. A crate that needs state at registration keeps it in a resource its `setup`
  inserts.

**The executor and the solver pool** (architecture sections 5.1 and 16; D111)

- `start_executor` is `Executor::builder()` with each `Some` field of `Workers` applied
  (`frame_workers`, `background_workers`, `chaos`, `replay`, `platform`), then `build`. A
  binary builds one executor; every world of the process runs on it. `Executor` is a
  handle: `Headless` holds one, a `Server` or `Client` holds none and is handed `&Executor`
  at every run, so the executor outlives every world it steps.
- `Workers::replay` is for step-driven runs: a recorded log is followable only by the same
  sequence of runs, and a paced run interleaves steps and passes by wall time.
  `Headless::run` refuses an executor built with a replay (`Unsupported`, before anything
  runs); `Headless::step` and the SOUR glue replay as `lactose_sim::harness` does.
- Each world gets its own solver pool and its own backend, built beside the world by
  `build_server` or `build_client`: `rayon::ThreadPoolBuilder` with
  `executor.frame_worker_count() - config.physics.reserved_workers` threads, at least one,
  named `solver-<n>`, never `use_current_thread`, with a `start_handler` that enters
  `ThreadClass::PhysicsSolver` on each thread before it takes work (the deterministic
  floating-point state, and the scheduling class best effort: a refusal is logged at `warn`
  and the thread runs on) and an `exit_handler` that counts the threads as they exit;
  `lactose_physics_rapier::backend(pool)`, which enters the class again on every thread, for
  a pool built without this handler (entering the class a thread is already in is harmless):
  the handler adds entry before any job, independent of the backend. A one-worker executor
  (SOUR's workers mode) gets a one-thread pool. Two worlds in one process (the spike) have
  two pools; milestone 4 decides whether a client's two worlds share one. The count is what
  `Server::shutdown` and `Client::shutdown` wait on ("Shutdown" below): rayon's
  `ThreadPool::drop` terminates the pool and returns before its threads have exited.
- A `Platform` is the binary's: `Workers::platform` is `None` for the headless binary and
  `StdPlatform` is used, for the executor and the IO pool alike; `app-client` passes the
  platform crate's `ClientPlatform` from milestone 2, whose topology is `StdPlatform`'s
  (`crates/platform/README.md`, "`ClientPlatform`").
- A shown world is built with `WorldConfig::shown()` (facade), which is `headless()` with
  `physics.reserved_workers` set to `RECORD_RESERVATION`: the frame-critical workers that
  stay awake while the tick's physics step blocks on the solver pool, which is what the
  record of the frame before has left in the overlap (architecture sections 5.3 and 16).
  It is one, which is `lactose-physics-api`'s own default, until the result file
  `record-stall` reports at one and at two (the milestone 2 plan, decision 5.16; D153). A
  follow-up commit changes the constant and nothing else if the file shows it wrong. The
  solver pool's size follows it, as above.

**The timer root** (architecture sections 5.1 and 11; `crates/sim/README.md`, "The timer
root")

- Every pacer is `TickPacer::new(now, TICK_PERIOD, CATCH_UP_CAP)` with `now` the instant
  the world is built, so its first tick is due at that instant: a poll at the build instant
  yields one tick. `Server::resync(now)` is `TickPacer::resync(now)`, after which the next
  tick is due one period after `now` and nothing is owed; `Headless::run` calls it at its
  start, and a loop calls it before its first `frame` after any gap in which it did not
  poll (steps, a save), since the pacer otherwise catches the gap up at the cap with
  `behind` set. `CATCH_UP_CAP` is four in every shipped configuration and `LOG_RETENTION`
  (120) is at least it, pinned at compile time.
- `Server::frame(executor, now, frames)` does the following, in this order, and nothing
  else: `pacer.poll(now)`, preceded by `pacer.resync(now)` while `time_scale()` is zero
  (a pacer stopped with a tick owed still pays it out, which would run a tick on a paused
  world, and an unpause owes from the last paused frame on), the budget's ticks taken as
  one when it owes none and a zero scale is pending ("Time scale" below); for each tick,
  `frames(next)` with `next` the tick about to run (`Tick(tick().0 + 1)`), and
  `Server::step` with what it returns, its requests and refusals appended to the
  output's, or an end to the frame's stepping on
  `None`, the due ticks not run being forgotten (the pacer counted them) and nothing else
  changing, and the same end after a step that carried a zero scale, pending or the
  callback's own, so no tick runs with the world paused; `Simulation::run_passes` once,
  whether or not a tick ran and whether or not the scale is zero; then `GatherState::take`
  into the `FrameOutput`. The caller sleeps until
  `next_deadline` with `Executor::sleep_until` and calls again. The frame callback runs on
  the task that drives the frame, a frame-critical worker, between steps, so what it
  drains (a jitter buffer, a session queue) is read in tick order and never concurrently
  with a step; it must not block, and `None` is for ending a bounded run at its last tick
  and nothing else, since the ticks it forgets are simulation time lost.
- `Server::step(executor, frame)` is one `Simulation::step` with `frame`, nothing paced
  and no pass run, and it is the whole of what `Headless::step` and a SOUR test do per
  tick. Both paths apply the same edits to the frame before stepping, in this order: a
  pending time scale replaces `frame.time_scale`; every bundle queued for this step goes
  into `frame.loaded`; a `frame.loaded` entry for a region the store holds is replaced by
  the stored state, the entry lending its `mode` and its `assets` ("Saves and loads"
  below). After the step, both paths reconcile the store against the world the step left,
  never against the frame, in this order: diff every export the lifecycle made into the
  store against the baseline its region had when it was exported ("Saves and loads"
  below); record the baseline of every bundle in `frame.loaded` whose region is not among
  `take_load_failures()`; remove from the store every region `World::regions` holds after
  the step, and from the baselines every region it does not; answer every `Load` request
  for a kept region by queuing its bundle for the next step; and take the requests and
  the load, session and asset failures into the `StepOutput`, logging each failure at
  `warn` with the region or player and the kind. A frame's output holds every step's
  requests and failures in step order. Nothing is consumed silently, an event the game's
  session applier fails included (`lactose-sim` reports it as a `SessionFailure`): a SOUR
  test asserts `StepOutput::failures.is_empty()` after every tick.
- `behind` is reported in `FrameOutput` and `RunReport`, never acted on here: the overload
  ladder is not in this crate yet. A frame that steps zero ticks still runs the passes and
  still reports a `next_deadline` one period ahead while paused.
- The timer root's task: `Headless::run` spawns the loop on a frame-critical worker
  (`Executor::spawn` with `WorkerClass::FrameCritical`), the task owning the server for
  the run, and drives it from the calling thread with `Executor::block_on`; it runs on
  whichever frame-critical worker picks it up, as architecture section 5.1 says, and its
  sleeps are the timers' (D113). `Headless::step` is driven by `block_on` from the calling
  thread with no task of its own: the thread that owns a `Headless` is never a worker. The
  spike and the milestone 4 binaries write their own loop around `Server::frame` and
  `Client::frame` and may spawn it the same way.

**Time scale** (architecture section 11; `crates/sim/README.md`, "Time")

- The scale is the assembly's decision on the server, and it reaches the pacer by
  direction, because a pause must reach the world before the ticks stop and an unpause
  must make a tick due before any tick can carry it. `Server::set_time_scale(scale)`
  records `scale` for the next step, by either path, to carry as `TickInput::time_scale`
  in place of what the frame callback put there; a `scale` equal to `Server::time_scale`
  with none pending is dropped. Nonzero: the pacer paces by `thousandths / 1000` from the
  call and `Server::time_scale` is `scale` at once, so a paused server has ticks due at
  its next `frame` and the first of them carries the unpause. Zero: the pacer and
  `Server::time_scale` change only after the step that carried it, a `frame` whose poll
  owes no tick runs that one tick for it, and under `frame` the step that carries a zero,
  pending or the callback's own, is the frame's last, the rest of its budget forgotten as
  after `None`, so however many ticks the frame owed (a 30 Hz loop, a 2x scale, a
  catch-up), no tick runs after the pause. A zero set while a nonzero scale is pending
  and the world's mirror is already zero (a paused server's unpause taken back before any
  step carried it) cancels the pending scale and puts the pacer and `Server::time_scale`
  back to zero, and no tick runs for it. After any step that carried a scale, by either
  path and from either origin (a pending scale or the callback's own entry), the pacer and
  `Server::time_scale` are that scale. So between runs, with no scale pending,
  `Server::time_scale` equals the world's mirror; with a nonzero one pending it is that
  scale; with a zero pending it is the scale the pacer still paces by. For a pause:
  `set_time_scale(PAUSED)`, the next frame runs exactly one tick, which carries
  `Some(PAUSED)`, the world's mirror reads `PAUSED`, then the pacer is at zero and no tick
  is due; for an unpause: `set_time_scale(REAL_TIME)`, the pacer is at one, the next
  frame's first due tick carries `Some(REAL_TIME)` and the mirror reads it. Under `frame`,
  a scale the callback puts in `time_scale` itself is carried and mirrored the same way but
  cannot unpause, since no tick is due while paused: a loop that pauses and resumes calls
  `set_time_scale`. The passes still run once per frame while paused. A pause whose frame
  owed no tick runs its one tick ahead of the pacer's schedule, so the schedule drifts by
  at most one tick per pause, which nothing catches up.
- On a client the scale is never decided: the fold reads the newest due frame's
  `time_scale` that is `Some` into the first new tick's `TickInput` and into the pacer at
  the fold, before the poll, times the clock estimate's `rate_thousandths / 1000`. So a
  replicated pause stops the ticks at the frame that folds it (the frame carrying the
  pause stays queued, since `StagedInbox::due` does not pop, and the hook applies it at
  the next step's sync point), and a replicated unpause makes ticks due at the frame that
  folds it, the first of which carries `Some(REAL_TIME)` and pops both frames.

**The client's frame** (architecture sections 9.2, 9.4 and 14; `crates/predict/README.md`,
"The loop `app` writes"; `crates/net-api/README.md`, "The staged frame")

- `Client::push(frame)` is `StagedInbox::push` on the prediction world's inbox, between
  runs; `false` is logged and enters nothing. The replicated world's inbox is `net`'s at
  milestone 4, when `push` pushes into both.
- The clock target of a resync with a hint `h` (step 8's, or none) is stated once and used
  by every resync below: the last clock estimate's `server_tick` advanced by the ticks this
  client's pacer granted since the estimate was taken (the sum of every poll's budget), or
  `h` if that is later or there is no estimate, plus the lead; with neither an estimate nor
  a hint, the world's own tick. The pacer paces by the replicated scale times the estimate's
  rate, so the granted ticks follow the server's through a pause or slow motion, and the
  target does not overshoot the server by the pause's length or the ticks slow motion did
  not run, as wall time would; the pacer drops what it owes beyond the cap, so a client that
  fell behind lands short, and the hint lifts it. Step 8's hint is the newest tick the inbox
  ever accepted (`StagedInbox::accepted_tick`), never before the reason's tick, whose frame
  it accepted: a tick the server has reached, and at or beyond every frame queued, where the
  reason's tick (`Behind`'s `server` is the oldest queued frame after the present) can land
  short of frames already queued and resync again at the next frame. A stale estimate or a
  stall therefore never puts the world behind a frame the inbox already holds, and one
  resync covers what queued during it.
- `Client::frame(executor, now, sample)` does the following, in this order, and nothing
  else:
  1. A pending clock resync (a `snap` from `clock`, or the kept failure of the previous
     frame): `pacer.resync(now)`, then `Predictor::resync` to the clock target with no
     hint.
  2. The fold, from `StagedInbox::due(next)` for `next = tick() + 1`: the newest due frame's
     `time_scale` that is `Some` into the input and into the pacer ("Time scale" above), and
     each due frame's `regions` as `crates/net-api/README.md` states: `Irrelevant` removes
     any pending `loaded`, `assets` and `modes` entry and inserts `unloaded`; `Relevant`
     inserts `loaded` through `RegionRelevance::to_bundle(baseline, None, table)` with
     `baseline` from `Cooked::baseline(relevance.baseline_hash)` and `table` the message's
     table pinned through `Cooked::pin`, or `None`, its `mode` then set to `Simulated` with
     a table and `Frozen` without one: `to_bundle` gives the replicated world's mode, the
     prediction world runs its kernels and steps its bodies only in a `Simulated` region,
     and a region enters `Simulated` only once its table is published (architecture section
     14; `build_client` requires `Directed`, under which the lifecycle loads the mode the
     bundle names); `Assets` inserts the table pinned and the region's mode `Simulated`; a
     `full` frame first clears what earlier due frames folded and puts every region of
     `World::regions` in `unloaded`. A region the server holds `Frozen` while it has its
     table (inactive under `Anchored`, or by a `Directed` frame's mode) is simulated by the
     client's prediction world all the same, because `RegionChange` carries no mode: the
     client runs kernels and steps bodies there that the server does not. Each update
     corrects a claimed entity there, since the server sends a claimed entity whole in
     every packet (`lactose-net-host`'s whole-send rule, architecture section 9.4; 58
     corrections in 60 frames in the proof below, whose harness follows that rule); a
     sender of changed rows alone would leave it uncorrected until the next full frame. A
     mode variant is a `lactose-net-api` change, decided before `lactose-net`'s packer sends
     region changes. A region whose baseline or pin `Cooked` cannot give is left out of the
     fold.
     The client remembers the newest frame tick whose fold failed and resyncs once per
     failing frame tick: when the newest failing due frame is newer than the remembered
     tick, the failure is logged (`Source::Cooked`, `Resync`), its tick remembered, and
     unless step 1 resynced this frame the world is resynced at its own tick with the pacer
     untouched: a content resync, reported in the output so the full frame that follows
     retries the region (architecture section 14). A failing frame folded again, as one that
     stays queued through a pause is every frame, is left out again and resyncs nothing, so
     a pause does not send a `ResyncRequest` per frame.
  3. `pacer.poll(now)`, preceded by `pacer.resync(now)` while the pacer is paced at zero,
     as the server's frame does, its budget's ticks added to the estimate's granted count.
  4. One `Predictor::step` per due tick, with the fold's input in the first and an empty
     one after. The local player's sample for `next` is `history.get(next)` when the
     history holds one, else `sample(next)`, inserted: a tick predicted before a backward
     resync is predicted again with the sample the server was sent, since
     `SampleHistory::insert` keeps the first and the packet carries the history. `sample`
     runs on the task that drives the frame and must not block.
  5. `Predictor::reconcile`, its wall time measured around the await; then the prediction
     world's lifecycle failures, which `app` alone takes on a client:
     `take_load_failures`, `take_session_failures` (empty while the fold carries no
     session event) and `take_asset_failures`, each logged at `warn` as the server's are
     ("Errors" below). A rollback re-applies a step's recorded input, and with it a refused
     bundle, which the lifecycle reports again; it can also refuse a bundle its first
     application loaded, when a correction made live an entity the bundle's overlay holds.
     The client keeps the tick and region of every refusal it reported since the last
     `Predictor::resync` (steps 1, 2 and 8 clear them); a refusal with a kept tick and
     region is a repeat and is dropped unlogged, every other is reported, so a refusal is
     reported once and none is lost.
  6. `Predictor::passes`.
  7. `take_present_events`.
  8. If `Reconciled::resync` is `Some`: `pacer.resync(now)`, then `Predictor::resync` to
     the clock target, with the hint the clock target names above. Otherwise, if step 5
     reported a refused bundle or took an asset failure and neither step 1 nor step 2
     resynced this frame, the world is resynced at its own tick with the pacer untouched: a
     content resync, as step 2's, reported so the full frame that follows retries the region
     (architecture section 14: a client-side asset failure requests a resync). It also
     restarts the ring after the refusal, so no later rollback re-applies it. No asset
     failure reaches a client today: the fold sets no `TickInput::failures`, and `predict`
     strips them from the ticks it re-simulates.
  9. If the budget was `behind`: `pacer.resync(now)`.

  A kept failure of any run resolves with the client kept (`ClientError::client` is
  `Some`) and a clock resync marked: the policy's response is `Resync`, the caller calls
  `frame` again, and that frame performs it in step 1 and reports it.
- `ClientOutput::resync` is `Some(tick)` only for a frame that resynced the world, by step
  1, 2 or 8 (a clock resync, a fold that failed, a reconciliation behind or too old, or a
  refused bundle or asset failure), and `tick` is where the world now stands: the caller
  sends a `ResyncRequest` for it. `events` is what `take_present_events` returned;
  `reconciled` is what `reconcile` returned, moved; `rollback` is the wall time of the
  reconcile await, zero when nothing was re-simulated.
- `Client::clock(estimate)` sets the lead to `lead_ticks` rounded up, the pacer's scale
  to the world's `TimeScale` times `rate_thousandths / 1000`, keeps the estimate with its
  granted count at zero, and on `snap` marks a clock resync for the next frame's step 1.
  `build_client` takes the lead the caller chose, since no estimate exists at the join.
- `Client::history` holds the last `REDUNDANT_SAMPLES` samples by tick, filled by `frame`
  and never by the caller; the caller encodes it into every input packet.

**The shown frame** (architecture sections 3, 5.1, 5.3 and 11; the milestone 2 plan,
decisions 5.1, 5.5, 5.6 and 5.11; D162; `crates/present-api/README.md`, "The registration
and the frame `app` writes")

**Status: facade.** This block and the ones after it, down to "Errors", are what
`frame-impl` makes true; the rows of "Errors" and "Shutdown" that name a shown world are
theirs too. Each sentence is one a dependent crate may rely on.

- A shown world is a `Server` world that is drawn: milestone 2's local world, the kind the
  headless binary runs (decision 5.1). `build_shown` builds it by `build_server`'s path,
  from the same arguments, and adds three things and nothing else: the `lactose-sim`
  builder option that leaves a `Server` world's presentation events for the frame ("What
  D162 asks of other crates"); the writing half of `lactose_present_api::snapshot_channel`,
  installed with `ExtractState::set_snapshot`; and `RenderSide::sink()`, installed with
  `ExtractState::set_sink`. None of the three is hashed state ("Determinism of a shown
  world"). `frame-impl` adds `lactose_present_api::{register_types, setup,
  register_systems}` to this crate's engine lists, for every world kind, after
  `lactose_anim`'s entry in each: "Registration" above states the lists as they are before
  that change and changes with it, as does `tests/schema_record/engine.txt`. A world with
  no output installed runs extract's pass and writes nothing
  (`crates/present-api/README.md`), so the headless world and the shown one are one
  registration.
- `Shown` owns that `Server` and the render side: the `Box<dyn RenderSide>` and the
  `PresentLane` the binary built. Every run takes `Shown` by value and hands it back, as a
  `Server`'s runs do. Between runs `Shown::server` and `Shown::server_mut` give what a
  `Server` offers by reference: the time scale, `resync`, the hash, the store, `relevance`,
  `encode_save`. The world's runs take a `Server` by value and are not among them: `step`
  and `frame` are `Shown`'s own, and a save point of a world while it is shown is not here
  yet ("Not in this crate yet"). The world's extract outputs are the frame's: a caller that
  installs a sink or a writer itself breaks the frame's count of full passes.
- `Shown::frame(executor, now, frames, window)` is one paced frame. It does the following,
  in this order, and nothing else:
  1. **The ticks.** `Server::frame`'s first half, unchanged: the poll with its resync
     while paused, the budget, and `frames(next)` and `Server::step` for each tick ("The
     timer root" above). The render task in flight, if any, runs beside these ticks: it is
     that of the last frame that started one. That is the overlap of architecture section
     5.3, and the only one. (`Server::frame` is this half and step 4's passes and gather,
     back to back.)
  2. **The wait for the frame before.** It awaits the render task in flight, up to the
     frame's *limit*: the next tick's deadline, which the pacer gave in step 1
     (`TickBudget::next_deadline`), less `JOIN_MARGIN`, 2 ms. The await is raced with
     `Executor::sleep_until(limit)`, and the task is polled first at each wake. If the
     task resolves first, the frame takes back what the task owned, and what that task's
     frame did is `ShownOutput::joined`. If the limit comes first, this frame is *busy*
     ("The frame's three states"): it runs nothing more and resolves with `deferred` as
     `Some(Rendered::Busy)`, a margin before the next tick is due. So the frame itself
     waits no later than its limit. That the wake at the limit comes on time is the
     executor's to give, and rests on a worker the frame's job never takes (*One worker
     stays out of the frame's job*, below). A frame whose limit has passed when its ticks
     are done looks once and waits for nothing: its sleep resolves at its first poll,
     since `Sleep::poll` answers a passed deadline before it registers a timer. That is
     every frame that is behind, whose next deadline is its own `now`, and a frame whose
     ticks overran. On an executor of one frame-critical worker every frame looks so,
     whatever its limit (the same bullet). Past the bound on deferring, and only there,
     the frame awaits the task with no limit. Once the task is taken back, and until step
     5, no tick's stage and no method of the render side is in flight but the ones the
     steps name.
  3. **The state.** The frame *draws*, *waits* or is *hidden* ("The frame's three
     states"). Deciding may take one job (`Executor::run_job`; "The render side"), which
     calls `RenderSide::open`, when no region is known to be open. Where the present role
     shares the graphics queue that job is started only with no present call in flight.
     The task awaits the call up to the same limit, under step 2's timer, and takes its
     report when it returns; if the limit comes first, the frame is busy as in step 2, and
     the call is awaited with no limit only past the same bound ("The targets and the
     present thread").
  4. **The passes**, unless the frame waits. The sink is changed where the state or the
     last frame's counts ask for it ("Counts, torn frames and resets"); then
     `Simulation::run_passes` once, whether or not a tick ran; `GatherState::take` into
     `ShownOutput::world.gathered`; `World::take_present_events`; and
     `ExtractState::publish` with that batch.
  5. **The render task**, unless the frame waits. It is spawned on a frame-critical worker
     (`Executor::spawn`, `WorkerClass::FrameCritical`) and owns the render side, the
     snapshot's reading half, the lane and the protocol's state until it resolves. It
     runs:
     1. the frame's job, one call of `Executor::run_job` ("The render side"), in which:
        `SnapshotReader::acquire`, once; `RenderSide::present` with the acquired frame and
        the frame's time; and, where the frame draws, `RenderSide::draw` with the target,
        the extent and the target's last read;
     2. where the frame drew and the present role shares the graphics queue, the hold: the
        await of the present call in flight, if any;
     3. where the frame drew, `RenderSide::submit`, from the task's own poll: the latch
        and the submit, with nothing of this crate's between them;
     4. where it submitted, the present protocol's one decision ("The targets and the
        present thread"): at most one call queued on the lane, and none awaited after it.
        A frame whose submit was held queues none, and nor does a frame that is shed.
  6. `frame` resolves with the world's half as `Server::frame` reports it, `joined`, and
     `deferred` where the frame waited. It does not wait for the render task it started.
- So extract is exclusive (architecture section 5.3). The passes of a frame run after its
  ticks have resolved and before a tick of the next frame starts, since one task owns the
  `Simulation` and awaits each of its runs; and they run with no method of the render side
  in flight, since the frame took the last render task back, resolved, before them. A
  render task's job reads the snapshot and seals the region only after its own frame's
  `publish`, and the next pass begins only after that task was taken back. So no frame
  reads the snapshot buffer or the ring region extract is writing (the milestone 2 plan,
  criterion 3(a)), by order, with no lock held across either.
- So the next frame's extract begins only after this frame is submitted.
  `crates/renderer/README.md` allows it from `prepare` on ("What `frame-design` must still
  decide"); the frame does not take that, because the render side is one object with one
  owner at a time, and a pass that ran beside `record` would write through the sink while
  the rest of the object is lent to recording jobs. What the order costs is pictures. What
  a record may cost a tick's start is not the order's but the workers': whether one is
  left to wake the root while the record runs (*One worker stays out of the frame's job*,
  below). While a record outlasts the tick period, or on a shared queue a submit is held
  for a present call, the frame that begins waits for the task until its limit. It draws
  as soon as the task resolves, and it is busy if the limit comes first: it has run its
  ticks and draws nothing. So a record a little over the period costs a picture now and
  then, and not every second one: each frame draws a little later in its period than the
  one before, until one meets its limit. `reviewer-gpu` ran this order on MoltenVK with a
  queue of its own and a margin of 2 ms, ten seconds a run (the review of D162's third
  draft). A record of 17 ms showed 56.8 pictures a second, one of 18 ms 53.7 and one of 20
  ms 48.4; run again as step 2 is written, in the review of the fourth draft, 56.7, 53.7
  and 48.2. A join that waited with no limit showed 56.9, 53.8 and 48.6 and started its
  ticks 17.5 to 20.4 ms late at P99; a look that waited for nothing showed 30.0 at each.
  `reviewer`'s model, at the margin of 2 ms, gives the wait 56.7, 53.4 and 48.0. In the
  probe's runs with the wait every tick started within half a millisecond of its deadline
  at P99. That lateness is the probe's own timed wake and not the executor's keeper, so
  those runs show what the order does to the pictures, and say nothing of a record whose
  jobs fill the frame-critical workers, which is the bullet named above. The ticks are
  what a `Server` world may not give up: beyond the cap on catch-up a server "climbs the
  overload ladder" and does not slow time (architecture section 5.1), and "Server time is
  never slowed" (section 9.7).
- **The limit, and what wakes the frame.** The limit is an instant the pacer already has,
  the next tick's deadline, less a constant. No deadline between ticks is asked for and no
  display's refresh: a frame that draws as soon as its render task resolves needs the next
  tick's deadline and a timed wake, and nothing else. The frame reads no clock for it. The
  executor's timer keeps the instant, and `Sleep` reads the executor's clock where it is
  polled (`crates/task/README.md`, "Workers"). So the `now` a caller passes is that
  clock's: `Instant::now()`, read just before the call, as the root reads it. The limit is
  in the time base of that `now`, and `Sleep` compares it with the clock. A caller whose
  `now` runs behind the clock has every limit already passed, so every frame looks; one
  whose `now` runs ahead waits until that instant on the clock, which bounds nothing. What
  wakes the driving task from the wait is the render task's join slot, when the task
  resolves, or that timer. The timer is fired by a frame-critical worker when it looks for
  work, or by the keeper, the one sleeping worker that parks only until the earliest
  deadline. It is the wake the root's own sleep to the tick's deadline has, and it is
  exposed as that sleep is: a worker inside a job looks for no work, so while every
  frame-critical worker is inside one, no timer fires until one of them is done. The
  exposure is the same in kind and greater in size, and the wait adds to it twice. A limit
  falls inside the record of the frame before, where a look needed no wake at all. And a
  frame that draws in the middle of a period has its record run across the next tick's
  deadline, which a look's record reaches only by what it overruns the period. The next
  bullet is what bounds both. On a shared queue the present call's reply slot wakes step
  3's wait in the same way. One `Sleep` serves both waits of a frame. It is among the
  timers from its first pending poll until it fires, resolves or is dropped, and it takes
  the timer's lock at each poll before its deadline, where it fires, at the poll that
  resolves it and where it is dropped unfired (`Sleep::poll`); the wait polls it at each
  of its wakes. So a period whose render task is still in flight when the ticks are done
  has two timers, the frame's and then the root's, and every other period has the root's
  alone. A frame suspended in either wait when the executor shuts down is dropped there
  and then, unpolled, with its `Server`, on the thread that called the shutdown, and
  resolves nothing. Only a frame that is polled after the shutdown began hears of it, from
  the join or from the `Sleep`, and resolves `Cancelled` with `ShownKept::Nothing`
  ("Shutdown").
- **One worker stays out of the frame's job.** A timer is fired by a worker that looks for
  work, and a worker inside a job looks for none. A tick's job ends within the tick. A
  frame's job is the record: it may fork onto every frame-critical worker and outlast the
  period, and with the wait above it runs across the limit of the frame after it and,
  where that frame drew in the middle of its period, across the next deadline as well. So
  the limit bounds how long a frame waits and not when a tick starts, and the executor
  must keep a worker for that:
  - *What a record that fills the workers does.* `reviewer-concurrency` measured it on
    `lactose-task` as it is, with four frame-critical workers, in the review of D162's
    fourth draft (a debug build, load averages of 10 to 16). A limit set 3 ms into a
    fan-out that filled the four workers woke 7.07 ms late at the median where the fan-out
    was four children of 10 ms (7.36 to 7.96 ms at P99), 2.1 to 2.8 ms late where it was a
    range of 2,000 units of 20 µs (8.45 to 8.50 ms at P99), and 0.06 to 0.08 ms late where
    it was forty children of 1 ms (0.66 to 0.77 ms at P99). With three workers idle it
    woke 0.005 ms late (0.018 to 0.043 ms at P99). Paced, over 600 frames with a stand-in
    record of 17.5 ms (3 ms on one worker, a fan-out of 14 ms on four, 0.5 ms of submit),
    the wait drew 49.5 to 50.6 pictures a second and started its ticks 3.5 to 4.0 ms late
    at the median, 13.3 to 13.7 ms at P99; the look drew 30.0 and started them 1.2 ms
    late, 4.5 to 5.2 ms at P99. With the fan-out as a range the wait's ticks were as late
    at P99 and the look's 6.0 to 10.1 ms. So the look already cost ticks under such a
    record, and the wait costs more: it puts the record across deadlines the look kept
    clear. `JOIN_MARGIN` does not mend it, since a larger margin lowers that lateness only
    by what it adds, and removes it only as the look.
  - *The width.* The jobs `Executor::run_job` starts, and the children their scopes fork,
    occupy `Executor::run_job_width()` frame-critical workers at most at any moment: the
    count of them less one, and one where there is one. `lactose-task` is asked for it
    with `run_job` ("What D162 asks of other crates", *The width*), as a count beside the
    run permit: `RECORD_RESERVATION` leaves the record a worker while the physics step
    blocks, and the width leaves the ticks and the timers a worker while the record runs.
    With two frame-critical workers or more, one of them is at every moment inside no such
    job. It takes graph jobs and tasks as today, fires the due timers where it looks for
    work and is the keeper while it sleeps; the root a timer wakes is polled there, and
    the tick's jobs start there. A timed wake is then late by the keeper's wake (D144,
    D156), or by what is left of a job or a poll that is not the frame's job's: a tick's
    or a pass's own job, which the root awaits in any case, or a render task's poll that
    calls `submit`.
  - *What it costs.* The record's throughput. A fan-out that would fill every
    frame-critical worker takes W/(W − 1) as long on W of them: four thirds at four, eight
    sevenths at eight, and at two it runs on one. The stand-in above, whose fan-out is 56
    ms of work, goes from 17.5 ms to 22.2 ms where the fan-out is cut for three workers (3
    ms, 18.7 ms and 0.5 ms), and to 31.5 ms where it stays four children of 14 ms, three
    at once and the fourth after them. That is with the job's own worker running its
    scope's children itself, under the place it holds, which the ask requires ("What D162
    asks of other crates", *The width*). An opener that parked beside them would leave the
    fan-out W − 2 workers: the stand-in's 56 ms would take 28 ms on four and the record
    31.5 ms where it is 22.2 ms, and on two the record would never end. A frame that waits
    draws one picture a record at most: 45 a second at 22.2 ms and 32 at 31.5 ms, where
    the stand-in with no width drew 49.5 to 50.6 with its ticks late as above.
    `reviewer-concurrency` ran such a record in the review of D162's fifth draft, on
    `lactose-task` as it is, cut by hand so that the opener ran one share itself and one
    worker stayed out, with 14 ms of fan-out for each worker. On four workers the record
    took 21.73 ms and the frame drew 43.6 pictures a second; uncut, 31.02 ms and 30.0; on
    three workers 24.06 ms and 39.9; on two 31.01 ms and 30.0. Its ticks started 0.001 to
    0.004 ms late at the median, and 0.013 to 1.3 ms at P99, a tail that a control with
    three workers idle and three cores spinning showed as well. With every worker in the
    record the same stand-in drew 54.7 to 55.0 and started its ticks 5.4 to 5.8 ms late at
    the median, 13.6 to 13.9 ms at P99. And a limit set 3 ms into the fan-out woke 0.001
    to 0.006 ms late at the median and at P90 with a worker kept out, on two, three and
    four workers, against 7.03 ms with none. A record that leaves a worker idle of itself
    pays nothing: in the same review a stand-in whose record ran on one worker drew 54.7
    to 54.8 pictures a second with its ticks 0.013 ms late at the median, and 2.6 to 4.9
    ms at P99 where the look read 1.0 to 2.9 ms in the same sitting. And a record that
    still fits the period on one worker fewer loses no picture. So the binary makes its
    graph with `GraphDesc::jobs` no greater than the width ("The render side", *The
    frame's job*).
  - *One frame-critical worker.* The width is one and no worker is free, so nothing keeps
    the limit. In `reviewer-concurrency`'s runs on one worker the limit won none of 100
    waits, and the wait was a join: 301 of 600 frames drew past their limit and their
    passes overran the next deadline 278 times, by up to 15.9 ms, where a look's overran
    it 26 times, by 0.79 ms at most. Both drew 54.3 pictures a second. So where
    `Executor::frame_worker_count()` is one, step 2 is the look: the frame polls the
    render task once when its ticks are done, and is busy if the task has not resolved.
    Step 3's wait stands there, since no job of the frame's runs during it and the one
    worker keeps the timer. Under either form a tick whose deadline falls inside the
    record's job waits for the job, 9.3 to 10.4 ms at the median in those runs and 17.4 ms
    at P99: one worker cannot run a record and start a tick at once. The pacing this file
    states is for two frame-critical workers or more. The frame's invariants hold at one
    as at any count, since none of them depends on a time.
  - *What is not measured.* The width's mechanism: nothing implements it, so what a queue
    of the pool's and a count at every take cost is unmeasured, and the lane that makes it
    records the lateness of a timed wake inside a fan-out that fills the width ("What D162
    asks of other crates"). Its effect is measured on the stand-in alone, by a record cut
    by hand (*What it costs*), in debug builds, on a machine that ran a GPU suite beside
    it. And the record: the stand-in is not the renderer's. Milestone 2's scene is seven
    rows and no run has timed its recording jobs, so whether a real record has the
    stand-in's shape, a fan-out that fills the workers for most of a period, is not known.
    `frame-pace` reads both ("Pacing").
- **The margin.** `JOIN_MARGIN` is the time the frame may still need between its limit and
  the next tick's deadline, and it has two parts. After a wake at the limit the frame does
  nothing more: it resolves, and the root sleeps for what is left. But the task is polled
  first at each wake, so a wake that comes late and finds the task resolved is followed by
  the frame's passes. The wake may be late by what D144 allows a timed wake on Windows, 1
  ms, where the laptop measured 503 to 863 µs at P99; on macOS D156 allows 50 µs. That is
  the keeper's wake, with a worker idle to keep it, which the width gives; the margin does
  not cover a wake kept behind jobs. The passes are held to 1 ms for 50,000 dirty rows by
  criterion 4 of the milestone 2 plan. The two sum to 2 ms, which is also the margin
  `reviewer-gpu`'s probe ran. What the margin costs is the pictures of the records that
  end inside it: such a frame is busy, and the picture is drawn a period later. A job that
  asks `open` after a wait is not counted in the margin. It follows a wait only where a
  torn frame was taken back, or a present call returned, near the limit, and the next tick
  may then start late by the job's time, as after any overrun ("Pacing"). The margin is a
  constant, as `RECORD_RESERVATION` is. `frame-pace` reads the lateness of each frame's
  first tick by what the frame before it did and beside the record's greatest width, and a
  follow-up commit changes the constant and nothing else if the file shows the margin
  wrong; a lateness beside a record that was on every frame-critical worker at once does
  not show that ("Pacing"). It does not scale with the world's time scale: at a scale
  where the tick period is shorter than the margin, every frame's limit has passed, and
  every frame looks.
- **The plain look is the variant weighed.** D162's third draft had step 2 poll the render
  task once, after the ticks, and wait for nothing. It needs no timed wake and no margin.
  Its record starts at a deadline and reaches the next only by what it overruns the
  period, so a tick is late for it by less than for the wait, and not by nothing, where
  the record's jobs fill the workers (the bullet above). But it draws only at a tick's
  deadline, so a record that runs a little over the period costs every second picture on
  either queue layout, 30.0 a second in the runs above where the wait shows 48.4 to 56.8.
  It is not taken. It remains as what a frame does once its limit has passed, and as what
  every frame does on an executor of one frame-critical worker. A larger margin moves
  every frame toward it, which is the answer if `frame-pace` shows that the keeper's wake
  costs ticks on some machine. A join that waited with no limit was the second draft's and
  is withdrawn (D162).
- `Shown::step(executor, frame, time, window)` is the step-driven form, for the
  fixed-frame mode of decision 5.1, a recorded test and a replayed log: the join of a
  frame in flight, if any; `Server::step` with `frame`; steps 3 to 5 at `time` and
  `window`; then the join of the render task it started, before it resolves. Nothing of a
  step's frame overlaps the next step. It awaits with no limit wherever the paced form
  awaits up to its own: the task in flight at its start, the present call before a job
  that asks `open` on a shared queue, and the task it started. So a step is never busy,
  and `Rendered::Busy` is the paced form's alone. It asks the executor for no timer. On a
  shared queue a step's render task holds its submit as a paced frame's does, and a held
  frame is dropped: where a present call outlasts a step, the next step waits for the call
  and its frame is not shown, and the one after it is. A step's frame is shed as a paced
  frame's is, by the same decision. The gathered frame is taken and dropped, as the
  headless binary drops it. The graph runs and the jobs a step starts are in a fixed order
  (the step's runs; the job that asks `open`, where no region is known open; the passes'
  runs; the frame's job), so a program that gets the same answers from its render side
  starts the same runs in the same order, which is what a replayed log follows,
  `Executor::run_job` being recorded as a run of one node (`crates/task/README.md`,
  "Chaos"; `Workers::replay`; "What D162 asks of other crates"). An await of a present
  call starts no run and moves none. The paced form interleaves a render task with the
  next ticks by wall time and is not replayable, as `Headless::run` is not.
- The frame's time. Neither form reads a clock. `FrameTime::now` is `frame`'s `now`.
  `FrameTime::fraction` is `1 − (next_deadline − now) / period`, clamped to `[0, 1]`, with
  `next_deadline` the pacer's after the frame's poll and `period` the tick period at the
  world's time scale; it is one while the world is paused. So it is zero at the newest
  tick's deadline and one a period later, and a frame that is behind draws at one. The
  step form takes the caller's `FrameTime` as it is and reads no pacer. The fraction is of
  one tick period, and it is the weight of the pair's newer state over the whole pair,
  whatever the pair's two ticks are: `lactose-renderer` draws `p + (c - p) * fraction`,
  `lactose-presentation` interpolates by the same weight, and nothing rescales it. So
  after a frame that waited or caught up, the pair spans several ticks and the picture
  covers them in one period. A frame that waited in step 2 draws at the same `FrameTime`
  as one that did not. The wait is not added, since the frame has no clock to read it
  from, so such a picture's world is older than its fraction says by the wait, which is a
  period less the margin at most. Its camera is not: the latch is taken at the submit. A
  frame given a `now` of its own between two ticks would carry the wait in its fraction
  ("Not in this crate yet").
- `Shown::flush` joins the frame in flight, awaiting its render task, and resolves with
  what it did; a present call that frame queued may still be in flight. `Shown::stats` is
  the counters since the build.

**The frame's three states** (architecture section 5.3; `crates/renderer/README.md`,
"Regions"; `docs/verification.md`, the row on a reader that stops acquiring)

- A frame *draws* when the window has an area and a region is open to extract's writers.
  It is *hidden* when the window has none (`window` is `None` or has a zero dimension), or
  when it is starved (below). Otherwise it *waits*, for a region. And a frame of any
  window waits where the frame before it is still on the render side at the frame's
  limit: it is *busy* (below).
- Whether a region is open is what the render side last said: `Submitted::region` of the
  last frame that drew, else the answer of `RenderSide::open`. A frame whose window has an
  area calls `open`, in a job of its own (`Executor::run_job`), when the last answer was
  not `Ready`: in the first frame, after a frame that was torn, and after each `Wait`. `open`
  never waits, and nothing of this crate waits for a region's point: the frame asks again
  at its next call. The point is in `Rendered::Waited` for a caller that may wait off the
  workers, as the fixed-frame mode's driver thread does for each frame's own point.
- A frame that draws runs steps 4 and 5 in full.
- A frame that waits runs its ticks and nothing else: no pass, no gather, no publication,
  no acquire, no render task. `ShownOutput::deferred` is `Some(Rendered::Waited(point))`,
  or `Some(Rendered::Busy)` for a busy frame, and `world.gathered` is empty. The world's
  presentation events stay in its queue, and the structural log keeps what the passes have
  not read. Waiting, and not hiding, is the answer to a device that is behind: a frame
  that hid would detach the sink, and the frame that drew again would need a full pass,
  every instance written again and a `cut` in the snapshot
  (`crates/present-api/README.md`, "The triple buffer").
- A frame is *busy* when the render task in flight has not resolved by the frame's limit
  (step 2), or when, on a shared queue, step 3 must ask `open` and a present call is still
  in flight at the limit ("The targets and the present thread"). It waits as a frame with
  no region does and reports `Rendered::Busy`, and `ShownStats::busy` counts it; `joined`
  is `None` where the task was unresolved, and a later frame reports that task's frame.
  This is the frame's half of what keeps the render side from slowing a world's time
  (architecture sections 5.1 and 9.7): the frame waits for a record that outlasts the
  period, or for a submit held for a present call, no later than its limit, and what they
  cost is the pictures of the frames that meet their limit meanwhile. The other half is
  the executor's, a worker left to start the tick while the record runs ("The shown
  frame", *One worker stays out of the frame's job*). No invariant of the frame moves. A
  busy frame does only what a waiting frame already did, and the wait before it changes
  when a frame decides, never what it may decide:
  - No pass runs while a method of the render side is in flight. The wait ends in one of
    two ways and there is no third. Where the task resolved, the frame took it back before
    its passes, as a look did. Where the limit came first, the task stays in flight,
    untouched, and the frame runs no pass.
  - Publications and acquires alternate. A busy frame publishes nothing and acquires
    nothing, and the task in flight acquires once, for its own frame's publication. A
    frame that draws after a wait publishes once, after the task it waited for has
    acquired.
  - The structural log and the events are read within `log_retention` ticks. One count, of
    the ticks since the last passes, bounds a wait for either reason (below). The wait of
    step 2 falls inside one frame, after that frame's ticks, and no tick runs during it.
    So the count a frame decides on is what it was when its ticks ended, the bound is
    still 116 ticks, and 116 and the cap of 4 still sum to `log_retention`.
  - Nothing hashed depends on a deferral, or on the wait. A pass run later in a frame, or
    some frames later, moves no hash, and the ticks a frame runs depend on `now` and the
    frame callback alone, both fixed before the wait begins ("Determinism of a shown
    world").
- A frame defers its passes, for a region or for the frame before it, only while the world
  has run at most `WorldConfig::log_retention − CATCH_UP_CAP` ticks since its last passes:
  116 at the defaults, which is 1.9 s at real time. The frame after such a one has then
  run at most `log_retention`, which is what a step keeps of the structural log
  (`crates/sim/README.md`, "Runs", step 3), so no consumer that reads the log once a pass
  finds its cursor stale because a frame waited. Past that the frame defers for nothing.
  It awaits the render task in flight with no limit, as `Shown::step` does, and on a
  shared queue the present call before it asks `open`: the one place where the frame
  itself holds a tick for the render side, and the frame after it is late as after any
  overrun ("Pacing"). And where no region is open it is *starved*: it is hidden, and so is
  every frame after it until `open` answers `Ready`, each of them asking first.
- A starved frame is hidden whatever `open` answers, an error the policy logs included: it
  runs its passes with the sink detached and its job with no `draw`, as a hidden frame
  does, and never defers. It reports `Rendered::Hidden`, with `ShownOutput::deferred`
  `None`; where its `open` failed it is counted in `ShownStats::torn` as well as in
  `hidden`. So an `open` that keeps failing cannot hold the passes: the world's events and
  its structural log are read within `log_retention` ticks whatever the render side says.
- A frame that is hidden runs its ticks; then step 4 with the sink detached
  (`ExtractState::set_sink(None)`, at the first such frame); then step 5 with no `draw`,
  no `submit` and no present call. The snapshot is written and published, the events are
  delivered, and the presentation schedule runs on real time (architecture section 11),
  every frame. No region is written, so none fills while the window is away: of the two
  answers `crates/renderer/README.md` leaves a frame with nothing to draw into, this is
  neither. The frame does not prepare and its writers do not run. The first frame that
  draws again installs a new sink first, so its pass is full.
- The reader acquires in every frame that published and in no other, so publications and
  acquires alternate strictly, in all three states. An acquire therefore never falls inside
  a pass's span and no pair is a window pair (`crates/present-api/README.md`, "The triple
  buffer"): the triple buffer stands, and no fourth buffer is asked for. And no backlog of
  event batches forms behind a reader that stopped. A hidden window acquires and delivers.
  A waiting frame publishes nothing, and its events wait in the world, those of
  `log_retention` ticks at most, for the publication that follows. A process stopped in a
  debugger publishes nothing either. Every event is delivered, in the frame that publishes
  it, and none is dropped or capped.

**The render side** (the milestone 2 plan, decision 5.11; D64;
`crates/render-graph/README.md`, "What each consumer needs";
`crates/renderer/README.md`, "What `frame-design` must still decide")

- `RenderSide` and `PresentSide` are declared here, in `lactose-app`, and name types of
  `lactose-rhi-api`, `lactose-present-api`, `lactose-ecs` and `lactose-task` alone. The
  binary implements both over crates this crate may not name and hands them over as `dyn`
  objects. That is how decision 5.11's "through `-api` traits" is read: traits over `-api`
  types. They are in neither `-api` crate because each trait names both crates' types (a
  `PresentFrame` and an `InstanceSink` beside a `TimelinePoint`, an `Extent` and an
  `ImageHandle`). An edge from `lactose-present-api` to `lactose-rhi-api` would put the
  device's interface under every boundary crate that names `lactose-present-api`; the
  other edge would put the ECS and the simulation under every device backend. One caller
  and one implementor do not earn a third crate.
- A method of `RenderSide` is never called while another is, nor while a pass of the world
  runs: the sink it gave and the object never run at once. The object has one owner at a
  time: the task that drives the frame between frames, the render task from its spawn to
  its join, and a job for the span of the closure the object was moved into. `Send` is all
  the trait asks, since the object is moved and never shared: an implementor may hold a
  `Cell` and needs no lock (`tests/shown.rs`, section 6, implements it for such a type).
- **The frame's job.** `open`, `present` and `draw` take a `JobContext`, since the graph's
  `record` and the presentation schedule fork from one, and the task that calls them is in
  no job. The frame gets one from `Executor::run_job(job)`, which `lactose-task` is asked
  for ("What D162 asks of other crates"): the closure runs once, as one job on a
  frame-critical worker, with that job's `JobContext`, and the future resolves to what it
  returned. The task moves the render side, the snapshot's reading half and what it asks
  into the closure, and takes them back, with the answers, in the closure's value. So the
  frame holds the render side and a `JobContext` together with no lock, no `Sync` bound,
  no world and no system: nothing a world registers is declared for the frame, in this
  crate or in any other (D152; the milestone 2 plan, decision 5.18). The job is taken by a
  worker that holds a run permit, as a graph's job is, so while the physics step blocks it
  runs on the workers `RECORD_RESERVATION` leaves awake, and what it forks is its own
  scope's. The job and what it forks occupy `Executor::run_job_width()` frame-critical
  workers at most, one fewer than there are, so that a worker is left for the timers and
  the ticks ("The shown frame", *One worker stays out of the frame's job*). An object over
  `lactose-render-graph` is therefore made with `GraphDesc::jobs` no greater than that
  width: recording jobs beyond it run after the first ones and not beside them. A paced
  frame starts two such jobs at most: the one that asks `open`, started by the task that
  drives the frame, and the frame's job, started by the render task. `submit` takes no
  context and is called from the render task's own poll. `lactose-presentation` is given
  no system that runs on a world, so it joins no carrier list: its schedule is called by
  the binary's object inside the frame's job. Until the primitive exists,
  `tests/shown.rs`, section 6, runs this shape over a stand-in built on a graph of one
  node, which `frame-impl` deletes.
- What each method owes, beside its doc comment, written for an object over
  `lactose-renderer` and `lactose-render-graph`:
  - `open` answers `Ready` or `Wait` and nothing else. It meets `Opened::Unsubmitted`
    itself, with the empty submit the renderer's "Regions" names (a declaration with no
    pass, compiled, recorded and submitted, then `Renderer::submitted` with its point), and
    answers what `Renderer::open` says after it.
  - `sink` gives a sink for one world's passes. A pass names its world's kind.
  - `present` is the presentation schedule on the acquired pair (decision 5.5):
    interpolation, the camera rig, the remote sample, the delivery of the frame's events.
    It writes no region and reaches no device queue, since a hidden frame calls it too:
    the remote sample's rows wait in the object and are uploaded in `draw`, where the
    object lends the renderer to `RemoteWriter`.
  - `draw` is the remote batch, `Renderer::prepare`, the declaration into the target's
    image at the frame's extent, with the import's `ready` the point `Draw::ready` gives,
    then `RenderGraph::compile` and `record`. It returns the frame's counts by kind. It
    makes the target's image where the index has none or has one of another extent. It
    makes no submit.
  - `submit` is `Renderer::latch`, `RenderGraph::submit` and `Renderer::submitted`, then
    `Renderer::open` for the next frame, which never waits. It and `open`'s empty submit
    are the only calls of the object that reach a device queue, which is what the frame
    orders against a present where the roles share one ("The targets and the present
    thread").
  - `present_queue` is `Capabilities::queues.present` of the device the object submits
    to, asked once, at the build. The frame names no device and cannot check the answer:
    an object that answers `Own` over a device whose roles share a queue puts its own
    submit behind a present, on a worker.
  - After an error from `draw` or `submit` the object has ended the frame as
    `Renderer::torn` and the graph's torn frame do, before it returns.
- The frame's values (`crates/renderer/README.md`, "What `frame-design` must still
  decide"). This crate decides the target's index, the frame's one extent, which is the
  window's when the frame's call began, the time and the fraction. The binary's object
  decides the rest and keeps it: the target's format, the projection, the boom, the mesh,
  and in the fixed-frame mode the sample its `LatestSample` holds, which the binary
  publishes before each step.
- A `dyn` call is made a constant number of times a frame: `open`, `present`, `draw`,
  `submit` and one present call at most, beside the sink's one call a chunk
  (`crates/present-api/README.md`). Nothing criterion 4 reads crosses the boundary for
  each row (decision 5.11).

**The targets and the present thread** (the milestone 2 plan, decision 5.6;
`crates/rhi-api/README.md`, "The present thread", "The swapchain" and "States";
`crates/platform/README.md`, "The present thread and its pump")

- A frame draws into one of exactly `TARGETS` offscreen targets, two
  (`OFFSCREEN_TARGET_IMAGES_MIN`). A target is an index. Its image is the render side's,
  made at the frame's extent in the first `draw` that names the index, and made again when
  `Draw::extent` is not its image's. The image replaced is kept until `Draw::ready` and
  the point of the frame that last drew it are both reached (`Device::timeline_value`, read
  in a later `draw`) and the device takes its destroy. The device refuses one while a heap
  slot holds the image (`pending`; `crates/rhi-api/README.md`, "Handles and lifetimes"),
  and the present side's slot lets go of an image at the first call that names another for
  the same index, and at `close`. Until then the slot keeps the last target's image from
  being destroyed and does nothing else: no command names the image.
- The present side writes a slot only behind a wait for its own last `read`. A slot is
  written only once no pending pass declares it (`crates/rhi-api/README.md`, "The heap and
  the sampler table"). The pass that last declared a target's slot is in the last submit
  that named the target, and the last `read` this side reported covers it, since its
  submits signal one timeline in order. So before it puts another image in a slot it
  waits for that point, within its bound, and reports `read: None` if the point is not
  reached, as for any call that could not present. The resize's wait is for the same
  point, but a new image can arrive with no resize before it: the extent went from E1 to
  E2 and back, and the frame at E2 was dropped or torn, so the call carries the extent the
  present side last asked for and an image it has not seen. `frame-impl` owes that case on
  `lactose-rhi-null`, where a slot written with no wait is refused (`uuwp-used`).
- The present thread reads a finished target in one call, and between calls no command of
  its own names one. That is the frame's answer to `crates/rhi-api/README.md`'s "which
  image the present thread holds between its presents": none in a command, and in its
  heap slot the last target's, as above. `PresentSide::present` names
  `finished.image` in one submit, which waits on the device for `finished.point` and
  signals a point of the present side's own timeline, `PresentReport::read`. The frame
  takes the target back by that point and by nothing else: a call's return frees nothing.
- The protocol's state is `Side`'s: `call`, the one present call in flight and the target
  it names; and `read[target]`, the `read` of the last call that named the target and
  reported one. One task owns it at a time. Nothing is shared with the present thread but
  the call and its reply.
- A frame draws into the target the last queued call did not name, and into target zero
  before any call, with `Draw::ready` that target's `read`. No call in flight names that
  target, so its last read is known when the frame begins and no present reads what the
  frame writes. Every submit of the frame that uses the image waits for `ready`: the render
  side imports it from `Undefined` with that point as the import's `ready`
  (`crates/render-graph/README.md`, "Imports").
- **Where the present role has a queue of its own** (`RenderSide::present_queue` is
  `QueuePath::Own`), the render task looks once at the call in flight, after the frame's
  submit. If it has returned, or none was made, its report is kept (its `read` where it
  gave one, `shown` counted, and `waiting`) and, unless the frame is shed (below), one
  call is queued for the target just finished, `Finished { target, image, extent, point }`
  with the image and the point `submit` returned; `Rendered::Drew::queued` is `true`. If
  it has not returned, nothing is queued and `queued` is `false`: that frame is never
  shown and the next frame draws into the same target. The frame's submit waits for no
  present, since the two queues are apart.
- **Where the present role shares the graphics queue** (`QueuePath::Shared`), a present's
  driver call holds the queue's lock (D158), and no frame-critical worker may wait behind
  one (architecture section 5.1; D153). So no method of the render side that reaches a
  queue is called while a present call is in flight. Before `RenderSide::submit` the
  render task awaits the call in flight, if any, and takes its report: that is the *hold*.
  The await is of the call's future: no worker waits and no run permit is held. Before a
  job that asks `open`, which may make the empty submit, the task that drives the frame
  awaits the call as well, up to the frame's limit ("The shown frame", step 3): it takes
  the report of a call that has returned, and where one is still in flight at the limit,
  the frame is busy and starts no job ("The frame's three states"); it awaits the call
  with no limit only past the bound on deferring, and in `Shown::step`. The present thread
  reaches the queue only inside a call, and a call is queued only after a frame's submit,
  or by `stop`, so when the render side takes the queue's lock no present holds it. After
  the submit comes the decision a queue of its own makes, made here after the hold. A
  frame whose submit was not held queues one call for the target just finished, unless it
  is shed (below), and `Rendered::Drew::queued` is `true`. A frame whose submit was held
  (`Rendered::Drew::held`: the call was still in flight when the frame's job returned)
  queues nothing, whatever the awaited call reported: `queued` is `false`, the frame is
  never shown, and the next frame draws into the same target ("Pacing" says why). The hold
  is for the whole of that call, its wait for a swapchain image included, and not for the
  driver's present alone. An error the awaited call reports is decided before the submit,
  and the submit is made whatever the response: on a lost device `submit` returns at once,
  and the frame's `Stop` reaches the caller when the frame is taken back.
- **Step 3 waits to the same limit as step 2, and is not left a look.** Where the roles
  share a queue, the frame before is on the render side until its present call has
  returned, and one rule for both waits is one thing to hold: a frame waits for the frame
  before until its limit, and is busy past it. A look there would be busy more often than
  it was in D162's third draft, since step 2's wait takes a task back later in its period
  than a look at the deadline did, and the call that task queued is younger when step 3
  meets it. And the frames it bites are already short of pictures: `open` is asked in the
  first frame, after a torn frame and after a `Wait`, and a `Wait` comes when the device
  is behind. In `reviewer-gpu`'s runs with the GPU saturated, 23% to 36% of the frames
  whose task had resolved had a present call in flight; under a drift of 0.1%, 8%; at the
  display's own rate, 0 or 1 of 600. The wait asks for no second timer, the limit's
  `Sleep` serving both. What follows it is a job, whose time the margin does not count
  ("The shown frame", *The margin*), so the tick after such a frame may start late by the
  job. No probe has run step 3 with a wait: the reasons above are reasoning,
  `frame-impl`'s null test of the shared queue holds the order ("Proof for the shown
  frame"), and `frame-pace` reads the lateness after such frames.
- **A frame is shed where the present side's count rises to `SHED_DEPTH`** ("Pacing", *The
  shedding rule*, says why, and what it costs). Each report carries
  `PresentReport::waiting`: how many of the presents made before that call were, as the
  call began, finished on the device and not yet displayed, or `None` where the swapchain
  reports no display time. `Side` keeps three things of the reports: whether the last one
  read below `SHED_DEPTH`; whether the one taken since the last decision *rose* to it,
  reading `SHED_DEPTH` or more where the one before it read below; and how many of the
  reports after the last frame that queued no present are still to be passed over. At the
  decision, a frame that would queue a call, with no earlier call in flight on a queue of
  its own and not held on a shared one, queues none where the report rose. `SHED_DEPTH` is
  three. The frame is submitted and dropped as a held frame is: the next frame draws into
  the same target, `Rendered::Drew::shed` is `true`, `queued` is `false` and
  `ShownStats::shed` counts it. A report is read by the first decision after its call
  returned and by no other, so a rise sheds one frame. A shed frame queued no present, so
  the calls after it count one present fewer until its gap has left what they count: for
  as many calls as the count's level, which is `SHED_DEPTH` calls where the level sits at
  the depth. Those reports are passed over: the next `SHED_DEPTH` reports that carry a
  count change neither what `Side` keeps of the last one nor whether one rose, so a shed
  frame's own gap is never read as the count falling, and the rule is not armed again by
  its own shed. Where the level is above the depth the gap outlasts them, and the calls
  left read at the depth or above and change nothing. After them the rule reads as before:
  a count that stays at the depth sheds no more until one has read below it and a later
  one has risen. Every other frame that queues no present leaves the same gap, and one
  rule serves them all: a frame dropped for a call still in flight, held or not queued at
  its submit, and a frame that drew nothing, having waited for a region or been busy,
  hidden or torn. The count does not depend on why a present is missing. What the rule
  needs of each is which calls were queued before the gap. A call whose return is still to
  be taken when a frame queues none is one: its count was taken before the gap. So that
  call's report sheds nothing, being read as the report before the next and never as a
  rise, and it is passed over only where an earlier pass-over is still counting. Once that
  call's return is taken, whatever it brought, the pass-over is set as a shed sets it: the
  next `SHED_DEPTH` reports that carry a count, which are those of the calls queued after
  the gap, are passed over. Where no call's return is still to be taken, the pass-over is
  set at that frame. So a dropped frame sets it at the return of the call it was dropped
  for, and a frame busy at step 3 at the return of the call it waited behind. A frame that
  waited for a region, a hidden frame and a torn one set it at once, or with a call still
  out at that call's return, which the next decision takes on a queue of its own, and the
  next hold or step 3's wait on a shared one. A frame busy at step 2 cannot reach `Side`,
  which the render task in flight owns. That task reads the report it takes as any other,
  and it makes its decision at its end, which had not come at the busy frame's limit, so a
  call it queues is queued a frame late and after the gap. `Shown` keeps that such a frame
  passed, and where the task is taken back the pass-over is set there, the report of the
  call that task queued being the first passed over; unless that task's own frame queued
  none behind a call still out, where the rule above has the gap already. A pass-over is
  set and never added to, so a run of such frames, as a hidden window's, passes over the
  `SHED_DEPTH` reports after its last. Where such frames keep coming within `SHED_DEPTH`
  calls of one another, as where every other frame is busy behind a record longer than its
  period, every report is passed over and the rule is off. Each of those frames left a
  present out as a shed would have, so the rule owes nothing there; that is reasoning, and
  no run's. And a rise is answered by the first frame after it that queues no present,
  whatever the reason: a frame that drew nothing clears it as a shed does, so a rise that
  step 3's wait took in a frame which then drew nothing sheds no later frame. A report
  whose `waiting` is `None` changes nothing and is not among the reports passed over, and
  a call that failed brings none; so where the swapchain reports no display time no frame
  is shed, and the drop above is the whole rule.
- So at most one present call is in flight and at most one is queued a frame (architecture
  section 3; D159), and the lane never grows. On either layout that is back-pressure and
  not pacing: a present call that outlasts a frame costs pictures. On a shared queue it
  also holds one render task, and with it the passes of the frames that begin meanwhile,
  and no tick within the bound on deferring.
- **The present thread never acquires a release it skipped.** A frame's submit releases
  its target toward the present role whether or not a call will be queued for it. The
  release is skipped when no call was queued, or when the call returned having made no
  submit (`read: None`). No later call names that frame: a call names the `Finished` it was
  given; a later call for the same index is queued only after a later frame's submit, which
  released the image afresh; and a call that made no submit leaves nothing recorded
  (`PresentSide::present`). The frame's next re-take of the target from `Undefined` ends
  the skipped release (`crates/render-graph/README.md`, "What each consumer needs"; D160,
  item 7 (c)).
- A call that could not present is no error. No swapchain image within `acquire`'s bound,
  a stale swapchain, a resize or a present refused as pending: the call reports `read:
  None` and the next call tries again. `Presented::Stale` is a submit that was taken:
  `read: Some`, `shown: false`. The present side resizes at the start of a call, when the
  swapchain said it was stale or `finished.extent` is not the extent it last asked for,
  after a wait, within a bound of its own, for the points its presents signalled
  (`crates/rhi-api/README.md`, "The swapchain"). It does not compare the swapchain's own
  extent, which is only about the one asked for. At a surface with no area the swapchain
  stays stale and the call reports `read: None`; the frame is hidden by then, or by its
  next call.
- An error of a call, other than `DeviceLost`, tells the frame that no submit named the
  image, and the frame keeps the target's older `read` (`PresentSide::present`). A backend
  owes the mapping that keeps this true: on Vulkan a failure of the driver's present after
  its submit was taken, a lost surface or an out of memory, is `DeviceLost` or a taken
  `Presented::Stale`, never another error. It is `rhi-vulkan`'s and `swapchain`'s to hold.
- Presents in flight on the device are bounded by the swapchain's images: a call presents
  only an image `acquire` gave it, `acquire` gives none while every image is in flight, and
  one call runs at a time. Frames in flight on the device are bounded by the render side's
  regions, through `Region::Wait` (`crates/renderer/README.md`, "Regions").
- Every wait the protocol puts on the device is for a point that a submit already taken
  will signal. A frame's submits wait for a `read`, which a call reports only once its
  submit was taken; a present's submit waits for a frame's point, which `submit` returns
  only for a frame it submitted. So no submit waits on a signal the host owes, nothing
  here meets the watchdog on an unmet host wait (architecture section 5.3), and at shutdown
  this crate owes no host signal (`crates/platform/README.md`, "Shutdown", step 2). A torn
  frame's point is the graph's to signal, with its next submit or at its drop
  (`crates/render-graph/README.md`, "Errors and drops").
- When the present thread is slower than the frame, a finished target is dropped, not
  shown late, on either layout. The decision is made once a frame: at the frame's submit
  on a queue of its own, and after the hold on a shared one. Where the display takes
  images at least as fast as frames finish, a call has returned by the next submit, and
  every frame is queued with no hold. Where it does not, frames are dropped, every second
  one or more where a call outlasts a frame; on a shared queue each dropped frame was held
  first, and the frames that met their limit during its hold were busy. That is a present
  call the frame was too fast for. An image that waits in the swapchain behind another
  blocks no call and is shown late all the same, by a refresh for each image ahead of it;
  the drop does not reach those, and the shedding rule bounds them where the swapchain
  reports display times ("Pacing", *The drop* and *The shedding rule*).
  `ShownStats::drawn`, `queued`, `held`, `shed`, `busy` and `shown` say what happened, and
  `frame-pace` records them ("Pacing").

**Threads, and what each owns** (architecture sections 3 and 5.1;
`crates/platform/README.md`, "A client binary, by thread")

| Thread | What it runs of the shown frame | What it owns |
|---|---|---|
| The task that drives the frame: the timer root, on whichever frame-critical worker polls it, or a step-driven run's driver thread through `Executor::block_on` | Steps 1 to 4 and 6: it awaits the world's runs, awaits the render task up to the frame's limit and takes it back once it has resolved, starts the job that asks `open` (only with no present call in flight, where the roles share a queue, a call it awaits up to the same limit), changes the sink, publishes and spawns | `Shown`: the `Server`, and between frames the render side and the protocol's state |
| Frame-critical workers | The jobs of the ticks and of the passes; the job that asks `open` and the frame's job, and what `open`, `present` and `draw` fork from their contexts, the graph's recording jobs among them, on all of the workers but one at any moment (`Executor::run_job_width`); the render task's own polls, which call `submit` and touch the lane. No worker waits for a present: a poll that finds a call in flight on a shared queue returns `Pending` | Nothing between jobs. The render task owns the render side and the protocol's state from its spawn to its join, and a job owns what was moved into its closure until it returns |
| The present thread | `PresentSide::present` and `close`, one call at a time, through the lane's pump | The `Box<dyn PresentSide>`: the swapchain, its encoder, its timeline, its heap slots |
| The solver pool's threads | The physics step, while all but `RECORD_RESERVATION` of the other frame-critical workers are parked | The backend's own |
| A thread of the binary's, made through `Executor::platform` | The drop of `Stopped::render` | That object, from `stop` on |
| The main thread | Nothing of the frame: the window's pump. The binary reads the window's extent and its close through `lactose_platform::WindowReports` | The window |

State that more than one of them touches, and the rule for each:

- The world's `Simulation`: one owner, the driving task; every run takes it by value.
- The snapshot channel: a pass's job writes the writer's buffer, the driving task
  publishes, the frame's job acquires. Its lock is D157's, taken once by the pass, once by
  `publish` and once by `acquire`, in that order in every frame, so here it is never
  contended.
- The renderer's open region: written by the passes' jobs through the sink, sealed and
  read by the render side's methods. Never at once: a pass runs only after the last render
  task was taken back and before the next is spawned.
- The render side and the protocol's state: moved between the driving task, the render
  task and the one job either is running, never shared. No lock and no atomic of this
  crate's: a move rides the task's join slot or the job's completion slot, which are
  `lactose-task`'s (below).
- The graphics queue, where the present role shares it: reached by the render side's
  `open` and `submit` and by the present side's `present` and `close`. Never at once, by
  order: a call is queued only after a frame's submit, and `open` and `submit` are called
  only with no call in flight. The queue's lock is the backend's (D158), and between
  those four it is then never contended.
- A target's image: a `Copy` handle on both sides. Whose turn it is to use it is the
  protocol's, by the two points of "The targets and the present thread".
- The device, an `Arc<dyn Device>` of the binary's: used by the render side's methods, by
  the present side on the present thread and by a step-driven run's driver thread
  (`wait_timeline`). Its synchronisation is the backend's (D158).
- The present call: the lane's queue and the call's reply slot (D159).

The locks a frame takes are these, and none is this crate's own.

- Permitted on the per-frame path by architecture section 3, by name: the snapshot
  channel's, three times as above; a device backend's, inside the render side's and the
  present side's methods and the sink's writes (`crates/rhi-api/README.md`, "Threads");
  and the lane's two (D159). The queue's is taken once, when a call is queued. The reply
  slot's is taken when the call's result is stored and at each poll of the call by the
  task that holds it: at each wake where a task awaits it, which the render task's hold
  and `stop` do with no limit and a frame's step 3 up to its own. A frame that is busy
  at step 3 leaves the driving task's waker in the slot, and the present thread then
  wakes that task once, when the call returns, as the join slot's does.
- `lactose-task`'s own, which section 3 neither permits nor lists. The task registry, one
  map for the whole executor that every background spawn shares, is taken at the render
  task's spawn and at its finish. The task's join slot is taken when the task completes
  and at each poll of the join: at each wake of a frame's wait, and once where a frame
  only looks. A busy frame leaves the driving task's waker in the slot, and that task is
  then woken once when the render task resolves: a root asleep on its timer is polled,
  finds its sleep pending and sleeps on. And for each job (`Executor::run_job`), as for
  each graph run of the ticks and of the passes, the run registry is taken at its start
  and its end, and the run's completion slot at its start, at its end and at each poll of
  its future. Every tick of milestone 1 already takes the last two, and the timer's lock;
  a spawn and a join each frame are new with this frame, and so is the timer's lock for a
  frame's own `Sleep`, taken at each poll of the sleep before its deadline and where it
  fires, resolves or is dropped, in a period whose render task is still in flight when its
  ticks are done.

Whether section 3 should name the executor's internal locks is a question for the
planner, which D162's merge raises: today they are neither permitted nor listed. This
crate adds no lock of its own, no atomic and no channel to the per-frame path.

**Pacing** (architecture sections 5.1, 5.3 and 11; D144, D156; `docs/verification.md`,
the row on the keeper's timed wait on Windows)

- The timer root paces the frame and nothing else does. The binary's root sleeps until
  `ShownOutput::world.next_deadline` with `Executor::sleep_until` and calls `Shown::frame`
  again, as a loop around `Server::frame` does. Neither the render side nor the present
  thread paces a tick. A frame waits for the render task in flight only until its limit, a
  margin before the next tick's deadline ("The shown frame", step 2), and the frame's job
  leaves a frame-critical worker free to wake the root and start the tick (*One worker
  stays out of the frame's job*, there). So a record that outlasts the period holds no
  tick, on an executor of two frame-critical workers or more. A present that blocks holds
  none either, since its caller holds a future and queues nothing behind it, and no frame
  waits for a display. Where the present role shares the graphics queue a present call
  still in flight holds one frame's submit, and no worker and no tick (below). The one
  exception is a frame past the bound on deferring ("The frame's three states").
- At milestone 2 a frame is therefore a tick's: one frame at each tick deadline, 60 a
  second at real time, drawn at a fraction near zero, which is the state of the tick before
  the newest. A display faster than that repeats frames.
- The surface carries a faster frame as it is. A call of `Shown::frame` between two
  deadlines owes no tick, runs an idle pass, which writes nothing where nothing was edited,
  acquires the same pair and draws it at a later fraction. What is not here is the second
  deadline, a display's refresh: `lactose-platform` reports none
  (`crates/platform/README.md`, "Not in this crate yet") and no milestone 2 row reads one.
  The 144 Hz of the plan's decision 5.3 is a cost test's timer and not the client's frame.
  "Not in this crate yet" names it.
- A frame is late when the task that drives it is late: by the keeper's wake, or by ticks,
  passes or a job that asks `open` that overrun the period. No frame waits for the render
  task past its limit while frames may defer: a record that outlasts the period, and a
  submit held for a present call, cost pictures ("The shown frame"). A record makes a
  frame late another way where it leaves no worker to wake the root: on an executor of one
  frame-critical worker, where a tick whose deadline falls inside the record's job waits
  for the job, and wherever the width asked of `lactose-task` is not held (*One worker
  stays out of the frame's job*, there). `frame-pace` reads it (below). The wait for the
  render task ends a margin before the deadline; where that wait's own wake is late by
  more than the margin leaves, or a job follows it, the next tick starts late by the
  rest, and the pacer treats it as any late frame. Past the bound on deferring a frame
  awaits the render side with no limit and is late by that wait. The pacer counts what is
  then owed and the frame runs it at the cap, as a server's does.
- **Where the present role shares the graphics queue** (`Capabilities::queues.present` is
  `QueuePath::Shared`; the row of `docs/verification.md` on which queue families present),
  a submit made while a present's driver call holds the queue's lock waits behind it
  (D158). No frame-critical worker does: the frame's submit, and `open`, are made only
  with no present call in flight. The render task awaits the call before its submit, which
  parks no worker and holds no run permit, and the driving task starts no job that asks
  `open` while one is in flight ("The targets and the present thread"). So the physics
  step stays the one sanctioned blocking wait on a worker (architecture section 5.1), and
  no frame-critical worker waits behind a present on any layout. D153 rules out "a frame
  submit waiting behind a present", and that rule applies where the device offers the
  present role a queue of its own, as D153's first sentence sets. Where the device offers
  none, D153 has the roles share the graphics queue, "a present that blocks holds its
  queue on any driver", and something of the frame gives way whichever thread submits.
  D162 chooses what: the picture of the frame whose submit was held, and the passes of the
  frames that begin meanwhile; never a frame-critical worker and, within the bound on
  deferring, never a tick. What the hold changes:
  - *The latch.* `RenderSide::submit` is called after the hold, so the sample it latches
    is the newest at the submit, as on a queue of its own. No held frame is shown, so
    every picture shown was submitted with no hold, and its world is older than its camera
    by its record alone, as on a queue of its own.
  - *The pace.* The hold delays one render task and, through the frame's limit, no tick:
    a frame that begins while a submit is held waits until its limit and is then busy
    ("The frame's three states"). A hold lasts for what is left of one present call, which
    lasts as long as the present side's bounded waits, `acquire`'s bound of about a second
    and the driver's present allow (`crates/rhi-api/README.md`, "The swapchain"). Only a
    call that outlasts the bound on deferring, 1.9 s at the defaults and real time,
    reaches a tick.
  - *The drop.* A held frame is submitted and never shown: the decision a queue of its own
    makes at its submit, made here after the hold. What the drop sheds is a present call
    that blocks for a whole refresh. Shown, a held frame would be a picture as old as its
    hold, queued behind the call that held it, and every later call would block as long.
    `reviewer-gpu` ran the frame's order on MoltenVK with both roles on one queue, frames
    at the display's 60 Hz (the review of D162's second draft); every age in these runs is
    the latch's at the return of its present call. With held frames shown and a look in
    place, one present call stalled for 40 ms left the calls after it blocked for the rest
    of the run in 7 runs of 8: the latch was 11.7 to 16.6 ms old at the median, against
    0.13 to 1.67 ms on a queue of its own after the same stall. With no stall at all, a
    frame period 0.1% and 0.4% shorter than the display's ended with 45% to 55% of all
    frames held and the latch 15.7 to 16.4 ms old. With held frames dropped, the same
    stall cost 1 or 2 frames in 5 runs of 6 and 20 in the sixth, the latch 0.28 to 0.34 ms
    old in 4 of the 6 and 4.0 and 8.0 ms in the others; and the drift cost 2 or 3 frames a
    run, the latch 3.7 to 5.8 ms old, where a queue of its own dropped 3 at 5.5 ms. No
    tick started late in any of these runs. D162 records them. They show that with the
    drop no call stays blocked. They do not show that no image waits.
  - *What the drop leaves.* An image that waits in the swapchain and blocks nothing, on
    either queue layout. A FIFO swapchain of three images has one on the display and one
    waiting while the third is still free, so one waiting image costs every later picture
    a refresh and no call blocks. Two waiting images block the next call; the drop acts
    only once that block has grown to a whole period, sheds one image, and two wait again
    at once. `reviewer-gpu` measured it in the review of the third draft, by the time each
    image reached the display (`VK_GOOGLE_display_timing` on MoltenVK). With frames 0.1%
    faster than the display, the latch was 49 to 65 ms old at the display from the first
    dropped frame on, 54.4 ms at the median over a minute on a shared queue and 54.9 ms on
    a queue of its own, against a floor of 22 to 31 ms in the same sitting; its age at the
    return read 5.9 and 6.5 ms at the median, and 2 frames were dropped in the minute on
    each. At the display's own rate one image stayed waiting from start to end in 3 runs
    of 4: 37.3 to 40.6 ms at the display against 24.0 ms in the fourth, with 0.12 to 0.22
    ms at the return in all four. What bounds the images that wait is the shedding rule
    (below), where the swapchain reports display times, and nothing where it does not.
  - *A present that stalls.* A window whose presents stall, as an occluded one's may,
    costs a shared queue its pictures: each call holds one frame for the call's bound, the
    frames that begin meanwhile are busy, and the held frame is dropped. It costs no tick
    while each call returns within the bound on deferring, as one bounded by `acquire`'s
    second does. Under MoltenVK a present with no drawable may take about three seconds
    for each image, or more (`crates/rhi-api/README.md`, "The swapchain"). That is past
    the bound, and the frame then awaits each call with no limit, its ticks waiting with
    it. What follows is the two reviewers' models' and no measurement, since nothing of
    the frame runs yet. Through calls of three seconds, `reviewer`'s model had the frame
    wait 2.6 s on average at each call; the pacer then ran what was owed at the cap, in
    bursts of about 120 ticks, which spend the bound again; and the world ran 42.2 ticks a
    second, about two thirds of real time. `reviewer-concurrency`'s model gave 39.8 to
    40.9. Through calls of two seconds `reviewer`'s ran 60, and through calls of one
    second `reviewer-concurrency`'s ran 1,200 ticks of 1,201. The pacer drops no debt:
    what is owed grows while the calls stall, by about 18 ticks a second in that model,
    and is run back at the cap once they recover, 381 owed ticks after twenty seconds of
    such calls all run by the end of the row. A world that must ride out longer calls may
    raise `WorldConfig::log_retention`: the bound is `log_retention − CATCH_UP_CAP` ticks,
    so 240 covers calls of 3.9 s at real time. That costs memory and no tick. A step keeps
    that many ticks of the structural log, and a frame that deferred that long leaves the
    world's events of as many ticks for one publication and as many ticks of the log for
    one pass, whose consumers read them at once. The default stays 120, the timer root's.
    The answer to a window that shows nothing is a hidden frame (`window: None`), which
    makes no submit and queues no call; `lactose-platform` reports no occlusion for the
    binary to act on ("Not in this crate yet").
  - *The counts.* `Rendered::Drew::held` and `ShownStats::held` count the frames whose
    submit was held, each of them dropped, and `shed` counts the frames shed. On either
    layout `drawn` less `queued` is the frames dropped; on a shared queue that is `held`
    and `shed` together. A queued call may still show nothing (no image acquired, a stale
    swapchain, an error, a lane that closed), so the frames never shown are `drawn` less
    `shown`. `ShownStats::busy` counts the frames that deferred their passes behind a
    render task or a present call. None of these shows a picture that was shown late. The
    latch's age at the return of its present call shows a call that blocked, and only its
    age at the display shows an image that waited (`frame-pace`, below).
- **The shedding rule, on either layout.** The frame bounds what a swapchain keeps waiting
  only where the swapchain reports when each image reached the display, which `rhi-api`'s
  `Swapchain` is asked for ("What D162 asks of other crates").
  - *The signal.* `PresentReport::waiting` is the present side's count, taken as a call
    begins, before its waits and its acquire: of the presents it made before that call,
    those that are finished on the device, their `read` reached by one read of
    `Device::timeline_value`, and that the swapchain's report has not named as displayed,
    neither them nor a later one. Presents are a frame period apart, so a count of *k*
    says that a picture presented *k* frames ago is finished and has still not reached the
    display. A present still behind the GPU is not counted. A saturated GPU delays every
    present's display with no image waiting behind another, and a frame dropped there
    relieves nothing, since its GPU work runs all the same. That is what a present call's
    block cannot tell apart: `reviewer-gpu` ran a drop after any call that blocked over 1
    ms, and with the GPU saturated it showed 25.1 and 34.0 pictures a second where the
    drop alone showed 50.3 to 50.5.
  - *What the count reads on the M1.* `reviewer-gpu` took the count in the review of
    D162's fourth draft, on MoltenVK 1.4.2 and on native Metal, at 60 Hz with a swapchain
    of three images, beside the age at the display of the present before each call. It
    reads one at 16.5 to 33.0 ms, which is the floor, no image waiting behind another. It
    reads two at 32.9 to 35.4 ms, the floor's top band, still with no image waiting: 8.8
    to 9.8% of the calls of a run that sweeps the frames' phase against the refresh. It
    reads two at 33 to 50 ms with one image waiting, and three at 49.5 to 65.0 ms with
    two, where the present call blocks. It never reads none while the GPU keeps up, since
    the path to the display is 16.5 to 17.9 ms at its least, about a frame period or more.
    So the count reads one more than the images that wait behind another, the picture in
    its own last refresh before the display being counted with them, and one more again in
    the floor's top band; and a swapchain of three images blocks where the count reads
    three. With the GPU saturated it read none to two.
  - *The rule.* At step 5's decision, a frame that would queue a call queues none where
    the count rose to `SHED_DEPTH`: the last report reads `SHED_DEPTH` or more and the one
    before it read less ("The targets and the present thread"). `SHED_DEPTH` is three. A
    finished picture three frames old with the display still ahead of it has two more
    behind it, and every later picture waits as long until a frame is left out. The rule
    acts on the rise and not on the level, so that a shed which relieved nothing is not
    repeated; and the `SHED_DEPTH` reports after a shed are passed over, since a shed
    frame presented nothing and its gap reads as one picture fewer in each of them ("The
    targets and the present thread"). Without that the rule feeds on its own sheds
    wherever the level sits at the depth. `reviewer-gpu` ran the rise alone at a depth of
    two on the M1, whose floor reads two in its top band. With no image waiting at any
    point it shed 63 frames in a minute, about 58 of them for the gap of the shed before;
    and with the frames' phase held in the band, 302 in 34.4 s, at 15 a second in 22 of
    those seconds. With the reports passed over it shed 9 in the minute, one or two at
    each crossing of the band, and one and then none in 25.3 s at the held phase. At a
    depth of three under drift it shed 3 a minute with or without. The cost: where a shed
    relieved nothing, because a device's path to the display is that long, the count does
    not read below the depth again and the rule stays off, so a backlog that then builds
    on that floor meets the drop alone. The run at a depth of two under drift shows it:
    one shed, at 10 s, and then the latch 32.4 to 65.2 ms old at the display, 52.6 ms at
    the median, which is the drop's own figure. A count that stays at the depth because
    the reports stopped sheds once as well, and then grows with each call; a window
    resized at every frame may be that case under MoltenVK, by its source and not by a run
    ("What D162 asks of other crates", *A present the display never took*). A frame
    dropped for a call in flight leaves the same gap as a shed frame and sets the same
    pass-over, once that call's report is in, and so does every other frame that queues no
    present ("The targets and the present thread"). `reviewer-gpu` ran both at a depth of
    three in the review of D162's fifth draft: on the M1, with each report counted two
    refreshes late so that the floor read three, the ages staying real, and a shed or a
    drop forced every ten seconds for 40 s. The rise alone fed on its own sheds, 480 of
    them, 12 a second, the counts around each reading two, three, two, two, two, three.
    With the reports passed over it shed 5, at start-up and the 4 forced, each followed by
    three counts of two, then three, and by no further shed. A forced drop that set no
    pass-over was followed by one shed, three calls later, and by nothing more; one that
    set it was followed by none, in a run that shed once, at start-up, and held one frame
    of its own beside the 4 forced; and at the M1's own level, where the count reads two,
    no shed followed a drop that set none. On that floor at three a backlog that built
    after the first shed was not shed, the latch 38.7 to 64.9 ms old and 53.4 ms at the
    median, which is the cost stated above. So the passing over is measured at a depth of
    two and of three, the three on a floor made by delaying the reports, since MoltenVK
    did not honour a later `desiredPresentTime` to lengthen the path itself. The probe's
    forced drops had no call in flight, so what it passed over were the three reports
    after each drop. The frame passes over the same three by starting after the report of
    the call the frame was dropped for, whose count was taken before the gap. One frame of
    the run that set the pass-over was held for a call in flight, on a shared queue, and
    went through that order at a level of four, above the depth, where the count shows no
    gap: it shows that the order does no harm there, and not that it covers a gap.
    `reviewer-gpu` replayed three orders over that run's counts by call, in the review of
    the sixth draft, at its three drops at the depth. With no pass-over at a drop a shed
    follows each, three calls later, which is what the run that set none measured; with
    the pass-over set before the report of the call the frame was dropped for is read, the
    same three sheds, that report being counted as the first passed over and the gap's
    last report read as the count falling; and with it set after that report, none. No GPU
    run has made the order on a queue of its own, and `frame-impl` holds it over scripted
    reports ("Proof for the shown frame"). A frame that drew nothing leaves the same gap,
    the count not depending on why a present is missing, and sets the same pass-over ("The
    targets and the present thread", which gives the order for each). That is the replay's
    reasoning: no run has made such frames on a floor at the depth, and for a frame busy
    at step 2, whose gap lies before the call the late task queues, the order is reasoning
    from the count's definition alone.
  - *What it gives.* A shed takes a refresh off the age of every picture after it. So the
    rule turns the latch's age at the display back where it passes three frame periods, 50
    ms at 60 Hz, whatever the swapchain's image count; how far that is above a device's
    floor depends on the device's path to the display. `reviewer-gpu` ran it on the M1 in
    the same review, with frames 0.1% faster than the display, a minute a run. On a shared
    queue 3 frames were shed and the latch was 33.0 to 49.7 ms old at the display, 40.9 ms
    at the median; on a queue of its own the median was 41.2 ms, on native Metal 39.7 ms,
    and with the reports after a shed passed over 41.0 ms. The drop alone dropped 2 and
    left 40.7 to 65.0 ms, 53.7 ms at the median. The drift itself owes a tenth of a
    percent of 3,600 frames a minute. A run that swept the floor shed none. Keyed on the
    count, a block that is the GPU's sheds nothing: with the GPU saturated the count read
    two at most, on both layouts. What the rule holds is not the floor. Its steady state
    is one image waiting behind another: under drift, once the drift has left an image
    waiting, the count read two at 99.7% of the calls or more and the latch was 32.8 to
    49.7 ms old, against the floor's 16.6 to 35.4 ms, in every run at this depth, on both
    layouts and on native Metal. The native Metal run read one for its first 14 s, until
    its drift left an image waiting, and two at 76% of its calls in all. And at the
    display's own rate one stall of 40 ms left an image waiting for the rest of the run,
    the latch 31.1 ms old before it and 47.7 ms after. So on the M1 the rule costs a
    constant refresh, 15 ms at the median, after any disturbance, where the drop alone
    leaves 27 to 29 ms. That is the trade against a depth of two.
  - *A depth of two is weighed and not taken.* It is the last refresh, and it would hold
    the M1 at its floor if the count could tell the floor from an image waiting. It cannot
    there: the floor reads two over its top band with no image waiting, about 9% of a
    sweep, and the floor's P99, 34.2 to 34.9 ms, is over two frame periods, 33.3 ms.
    `reviewer-gpu` ran it both ways. With the rise alone it held the latch at 16.7 to 34.9
    ms under drift, 26.1 ms at the median, by shedding 101 frames in the minute where the
    drift owes 3.6, and at the floor it shed 63 a minute that relieved nothing. With the
    reports after a shed passed over it turns itself off under drift: one shed, and then
    the drop alone. A depth of three has a period of slack above the floor's top. The
    depth that fits is the device's own: two where the path to the display is well under a
    period, four where the floor reaches three. `frame-pace`'s floor run gives it for each
    device, and a rule that reads the floor in place of a constant is not built ("Not in
    this crate yet").
  - *Where no display time is reported,* `waiting` is `None`, no frame is shed, and the
    drop is the whole rule. What remains is what `reviewer-gpu` measured: under drift the
    latch 49 to 65 ms old at the display against a floor of 22 to 31 ms, two refreshes,
    about 33 ms; and at the display's own rate one waiting image in 3 runs of 4, 37.3 to
    40.6 ms against 24.0 ms. The drop bounds the queue at the swapchain's image count less
    one, so a swapchain of more images keeps more. No count of the frame's shows any of
    it, and `frame-pace` cannot see it there: the file says that the device reports no
    display time.
  - *The swapchain's image count is the backend's choice,* as D158 has it: `SwapchainDesc`
    carries the extent alone, and `Swapchain::image_count` reports what was made. MoltenVK
    gave 3, and 2 when asked for its least. The frame asks for no count. The rule reads
    the pictures on their way, which bounds a picture's age at the display whatever the
    image count is from three up. A swapchain of two never reaches a count of three. It
    bounds itself by blocking: its present call then blocks every frame, 2.7 ms at the
    display's rate in `reviewer-gpu`'s run, with the latch 32 to 48 ms old under drift.
    Whether a backend that reports no display time should ask for its surface's least
    count is the swapchain lane's to settle from `frame-pace`, which records the count.
  - *What is measured and what is not.* The count's readings, the depth and the rule are
    `reviewer-gpu`'s on the M1, under MoltenVK and native Metal, at 60 Hz with three
    images. No other device has run them, and `frame-pace` reads each one after it. The
    report's delay there is under two milliseconds: MoltenVK's report came 0.61 to 0.65 ms
    after the refresh at P50 and 0.81 to 0.91 ms at P99, Metal's 0.50 to 0.53 ms and 0.56
    to 0.73 ms, 1.72 ms at most, and none out of order. So the count is one too many only
    for a call that begins within that time of a refresh. A device whose report arrives a
    refresh late reads one too many at every call, and the rule then acts as a depth of
    two would, with its cost; the swapchain's README states its report's delay, and which
    instant its time is. The exclusion of the report of a call that a frame was dropped
    for never acted at this depth, in any run; it acted 3 and 26 times at a depth of two
    with the GPU saturated. A count that rises by two at once, as after a stall on a
    swapchain of more than three images, is shed once and stays a level above the depth
    until it falls by itself. A rule that sheds for as long as each shed lowers the count
    is the variant, if a device shows the case; and so is one that arms again on a rise
    above the level a shed left, which would answer a backlog on a floor at the depth, and
    which no run has tried. `SHED_DEPTH` is a constant, and a follow-up commit changes it
    and nothing else.
- Two other places for a shared queue's submit were weighed. *Through the present lane*:
  the lane runs one call at a time, so the submit would wait for the same present call;
  and it would be a second call a frame where architecture section 3 permits one (D159),
  with the render side moved to the present thread for it, the latch made there, and a
  lane closed under it dropping the render side on the caller's worker. *On a thread of
  its own*: the submit would wait for the present's driver call alone; but it asks for a
  thread architecture section 5.1 does not list, a lane's two locks a frame that section 3
  does not permit, and the render side moved across for every submit. And it wins nothing
  the hold lacks, once held frames are dropped and busy frames defer; and it does nothing
  for an image that waits in the swapchain, which the thread would queue as the hold does.
  In `reviewer-gpu`'s runs on MoltenVK, whose present waits inside the queue's lock, with
  the GPU near saturation, the thread showed 54.7 to 55.7 frames a second, the latch 30.6
  ms old at the median when its present call returned, and its first tick started 9.0 ms
  late at the median; the hold showed 52.1 to 52.6, the latch 10.0 to 10.2 ms old, and no
  tick late. On a stand-in for a driver whose present waits outside the lock, the hold
  showed 53.6 to 55.4 frames a second and the thread 55.2 to 55.7, the latch 8.9 to 10.1
  ms and 9.9 to 10.2 ms old, with no tick late under either. Awaiting the call adds
  nothing to the surface but `present_queue`, `held` and `Rendered::Busy`. The figures of
  these runs are ages at the return of the present call. What would reopen the thread is
  `frame-pace` from a shared-queue device with a display ("Not in this crate yet").
- `frame-pace` is the result file the frame asks of `measure-m2`, on the M1 and on
  Windows, from a run of `lactose-app-client` of ten seconds or more, shown, and on
  Windows also minimised (`MainThread::set_minimised`). It records, for each paced frame:
  the lateness of its first tick's start against the deadline the root slept to, taken in
  the frame callback; how long the call of `Shown::frame` took, taken by the root, which
  shows a frame that waited in step 2; whether the frame drew, waited for a region, was
  busy or hid; whether its present was queued, whether its submit was held and whether it
  was shed; and, taken by the binary's render side, the record, which is the time from the
  start of the frame's job to `draw`'s return, the record's greatest width, which is the
  most frame-critical workers inside the frame's job at one moment, the job's own worker
  counted from the job's start to `draw`'s return and every other for as long as it is
  inside a recorder, a closure the object forks or the writer's closure that
  `lactose-presentation`'s `upload` calls from the jobs it forks in `present`
  (`RemoteWriter::write`, which the binary stamps as it stamps a recorder), taken from the
  entry and the return of each by the slot that ran it, the record's longest job, which is
  the longest of those closures by the same stamps, the time `RenderSide::submit` took
  and, on a shared queue, the hold, which is the time from `draw`'s return to `submit`'s
  entry. It records each present call in three parts, taken by the binary's present side:
  its own waits (for its last read, and the resize), the acquire, and the driver's submit
  and present, which is the part under the queue's lock; and the `waiting` each call
  reported. And it records the latch's age twice. The render side stamps each latch and
  `Finished::point` joins it to its present. The first row is the age at the display: the
  swapchain's report gives the time each present reached the display, by present id, and
  the present side keeps each id beside its call ("What D162 asks of other crates"). Its
  source is `VK_GOOGLE_display_timing`'s `actualPresentTime` on MoltenVK, `presentedTime`
  on Metal, `VK_EXT_present_timing` where a device offers it, and `VK_KHR_present_id` with
  `VK_KHR_present_wait` only where the swapchain's README shows from a run that its
  completion follows the display ("What D162 asks of other crates"); a device with none of
  these reports no display time. An age at the display below half the floor run's median
  is dropped and counted: MoltenVK reports a present the display never took with its own
  clock's time, which read 1 to 8 ms on the M1 where the path's least is 16.5 ms. The
  second row is the age at the return of the present call, as the present side stamps it.
  Where the swapchain reports no display time the file says so and has the second row
  alone. The first row is read against the run's own floor: the same binary in the same
  sitting with its frames a thousandth slower than half the display's rate (at 60 Hz, a
  time scale of 0.4995), where no image waits behind another and the frames' phase against
  the refresh sweeps about every 17 s. A floor at exactly half the rate reads one phase
  and not the floor: on the M1, whose panel and host clock did not drift apart within a
  run, three such runs read 29.9, 21.4 and 22.3 ms, each within 0.3 ms, where floors swept
  at 60 Hz read 17.6 to 35.4 ms, 25.0 to 26.5 ms at the median and 34.2 to 34.9 ms at P99,
  and the floor run as it is specified here read 17.0 to 35.1 ms, 27.3 ms at the median
  and 34.7 ms at P99. The display's rate is the display mode's exact figure, not one
  rounded to whole hertz and not a swapchain's fallback, and the file records it and where
  it came from, beside the refresh a swapchain reports where it reports one. MoltenVK
  1.4.2 reports its refresh from the mode's rate and, where the mode gives zero, as a
  built-in panel's does, from `NSScreen.maximumFramesPerSecond`, an integer and the
  panel's greatest (`MVKSwapchain.mm`, as `reviewer-gpu` read it in the review of D162's
  sixth draft). A rate taken a thousandth fast, as 59.94 Hz taken as 60 is, puts the floor
  run's frames at exactly half the true rate, which is the one phase above; taken half as
  far out, the sweep takes 33 s and a minute covers the phase under twice. So the file
  records how many times the floor run swept: how often the latches' phase went round the
  refresh fitted to the display times, a fit left free here, since times that follow the
  frames show no sweep at all. A run that swept fewer than twice is no floor and is
  recorded as that, with no reading made against it, and it is made again with its frames
  set from the fitted refresh: a display whose rate was misreported then sweeps, and a
  source that follows the frames still does not. Both of its runs last a minute or more,
  so that the sweep covers the phase several times and a drift between the frames and the
  display, where a display has one, has time to show. The file reports the P50 and P99 of
  the lateness, of the record, of the record's longest job, of the submit's time, of the
  hold, of each part of the present call, with its P90, and of the latch's two ages; the
  greatest `waiting`, the share of the calls that reported two or more, and the count of
  the ages dropped; the greatest width any record reached, and how many records reached
  `Executor::frame_worker_count()`; the counters of `Shown::stats`, `busy` and `shed`
  among them; and the frames that reported `behind`, beside the device's `QueueLayout`,
  `Executor::frame_worker_count()` and `Executor::run_job_width()`. It records what moves
  those figures as well: the load averages before and after the run, the swapchain's image
  count, whether a validation layer was on, and the display's state (its refresh rate,
  whether the window was occluded, whether another client was drawing). Each row is run
  once on a quiet machine. A device that reports display times is flagged where its
  greatest `waiting` stayed at none or one through a run whose present calls blocked: the
  acquire, or the driver's submit and present, took over a millisecond at P90. The median
  is read beside it and is not the key, since it misses a run whose queue filled late: in
  one of `reviewer-gpu`'s for D162's fifth draft, 20 s under drift with no frame shed, the
  present call read 0.29 ms at the median and 1.94 ms at P90, where behind a full queue it
  read 3.2 ms or more at the median and with no image waiting 0.37 ms at P90 at most. Its
  report may then not follow the display, as `VK_KHR_present_wait`'s does not on MoltenVK.
  Which call blocks is the driver's. MoltenVK 1.4.2's acquire never waits, 0.30 ms at most
  in `reviewer-gpu`'s runs for D162's fifth draft: it takes the drawable inside
  `vkQueueSubmit2`, at the first use of the image, where behind a full queue the submit
  took 3.1 ms at the median and 16.7 ms at most in one run, and the whole present call 9.6
  ms at the median in another; a present call that waited for no image left the latch 0.09
  to 0.24 ms old at its return, by the median of each run. Other drivers wait in the
  acquire or in the present. A saturated GPU holds the count down as well, so the flag is
  a question and no verdict. The floor run answers it on a display of fixed rate: the file
  gives how far the reported times lie from one phase of the refresh, the grid test of the
  ask ("What D162 asks of other crates"), which read 0.07 ms at most on the M1, and beside
  that bounded result the period a free fit lands on. Where the bounded test refuses a
  source and the floor run did not sweep, the refusal says nothing of the source yet: the
  frames may have sat at half the display's true rate, as on a display at 59.94 Hz taken
  as 60, where the bounded test refuses a source that follows the display with one that
  does not. The run made again from the fitted refresh says which. The lateness is read
  against D144's 1 ms on Windows and D156's 50 µs on macOS. Those are criteria of a tick's
  start with idle workers, so a figure over one in a shown run is a finding for the
  planner and no failed criterion. Four readings are fixed, and everything else is
  recorded with no threshold:
  - On every layout, the submit's P99 is read beside the P99 of the present call's third
    part. A submit that took as long as a present's driver call waited behind one on a
    worker. Where the roles share a queue the frame's order rules that out; where they do
    not, only a driver that serialises its queues inside itself can cause it, which no
    capability reports. It is the same defect on either, and it is reported.
  - On a shared queue, the hold is read against the present call's third part. A hold much
    longer than the part under the queue's lock is what a thread for the submit would
    remove, and is the figure that reopens that question.
  - The latch's age at the display is read against the swept floor: the run's median
    against the floor's P99. A median above the floor's P99 is an image that waits behind
    another. On the M1 that reading told `reviewer-gpu`'s runs apart: those with an image
    waiting read 35.6 ms or more against a floor's P99 of 34.2 to 34.9 ms, and those with
    none 33.6 ms or less, where the reading of D162's fourth draft, a refresh above the
    floor's median, missed three backlogs of four. Under the shedding rule one waiting
    image is the steady state ("The shedding rule", *What it gives*), so there it is
    recorded and is no finding. A median more than a refresh above the floor's P99, or a
    greatest `waiting` above `SHED_DEPTH`, is a finding, and questions the rule or its
    report's delay. The share of the calls that reported a `waiting` of two or more is
    recorded beside it, with no threshold: 99.7 to 99.9% with one image waiting on the M1,
    against 8.8 to 9.8% in a sweep at 60 Hz, its frames 0.1% slower than the display, and
    none in the floor run at 0.4995, which presents at every second refresh and read none
    at 1,588 calls and one at 210, its top band. It is not the reading, for two reasons. A
    run at the display's exact rate whose phase sits in the floor's top band reads two at
    every call with no image waiting, 32.7 to 34.3 ms in two runs. And the ages check the
    count from outside it: where a source does not follow the display the count is wrong,
    and its share with it. The floor's own P99 against two frame periods says whether the
    device could take a depth of two: 34.2 to 34.9 ms against 33.3 ms on the M1, which
    could not. The age at the return is read as the sign of a call that blocks: near a
    whole refresh at the median, a present waits for an image at every frame. A low age at
    the return says nothing of a backlog.
  - `busy` is read beside the record's P99, and the lateness of each frame's first tick
    beside the record's greatest width and by what the frame before did. Busy frames
    beside a record whose P99 is under a period less the margin are not the record's: on a
    shared queue they are the hold's or step 3's, and on a queue of its own they point at
    the wake. The greatest width covers the whole of the frame's job, `present` with
    `draw`: the jobs `lactose-presentation`'s `upload` forks in `present` are stamped at
    the writer's closure, so a width that was not held there shows as one not held in the
    record does. A lateness beside a greatest width equal to
    `Executor::frame_worker_count()` says that no worker was left to wake the root while
    the record ran: the width was not held, or the binary has one frame-critical worker.
    That is a finding for `lactose-task` or for the binary's worker count, and
    `JOIN_MARGIN` does not mend it, since a larger margin lowers it only by what it adds.
    The record's longest job does not show it, and is a figure of the record's cost alone:
    a worker fires timers only where it looks for work, and one that drains a range or
    runs its scope's children goes from one closure to the next without looking, so that
    in `reviewer-concurrency`'s range of 20 µs units the limit woke 8.5 ms late at P99
    beside closures of 20 µs each. With no such record, the lateness of a frame's first
    tick after a busy frame is the keeper's wake less the margin, and after a frame that
    drew following a wait it is what the passes ran past the margin. A figure above zero
    in either of those two is what changes `JOIN_MARGIN`. None of the three is a failed
    criterion.
- The frame names no use that needs a tick or a frame to start within 35 µs of its deadline
  on macOS, which is what `docs/proposals/tick-start-macos-27.md` asks a milestone 2 design
  to say if it has one. A frame that starts late takes its fraction from `now`, so the
  lateness moves what is drawn by that share of a period, 50 µs being three thousandths of
  one, and breaks nothing. D156's 50 µs stands.

The keeper's unmeasured list on Windows (`docs/verification.md`; D144, D153), read against
this frame:

| Item | Does the frame use it | What follows |
|---|---|---|
| The keeper at a frame worker's priority | Yes. The root's sleep, and the frame's wait to its limit, end on the keeper: with two frame-critical workers or more, a worker the width keeps out of the frame's job. The record of the frame before on the other workers, the present thread and, on a shared queue, a present's driver call compete with it for a processor, which no milestone 1 run had | `frame-pace`: the lateness of each frame's first tick, beside the record's greatest width, in a shown run on Windows |
| A wait asked once for the time left less a fixed margin | No. That item is a shape of the keeper's own wait (`docs/plans/windows-timer-proposal.md`, section 5, part B: one wait for the time left less a margin, then a spin), and the frame's wait to its limit is not it. The frame adds a second deadline to some periods, kept by the keeper's wait as D144 built it, and asks no finer wake than D144 measured: `JOIN_MARGIN` allows that wake D144's whole millisecond, with a worker idle to keep it | `frame-pace` on Windows: the lateness of the ticks that follow a frame's wait. The keeper's shape stays re-gated to the change that gives the frame a display's refresh as a deadline, which is the first to ask for a shorter wait |
| The spin's power cost | More than milestone 1 did, in some periods. A period whose render task is still in flight when its ticks are done has two deadlines, the frame's limit and the tick's, and the keeper spins the last 500 µs before each; every other period has one, shown or not | Recorded and not gated: `frame-pace` gives the frames that waited, each a second deadline. No milestone 2 criterion reads power, and its measurement stays re-gated with the wait above |
| The `SetEvent` for each frame worker that `Pool::open_gate` makes at the end of each `block_in_place` | Yes. In the overlap the record's jobs run on the reserved workers while the physics step blocks, and the gate's reopening is what gives the record its other workers back | `record-stall` on Windows, at a reservation of one and of two: beside the record phase's time, the time from each `block_in_place`'s end to the record's next job on a worker that was parked |
| A 1 ms period under a minimised window | Yes. A hidden frame still runs its ticks at their deadlines | `frame-pace`, the minimised run on Windows, recorded with no threshold; a frame that reports `behind` there is a finding for the planner |

**Counts, torn frames and resets** (`crates/renderer/README.md`, "What `frame-design` must
still decide" and "Writers"; `crates/present-api/README.md`, "The triple buffer")

- A new sink, `ExtractState::set_sink(Some(render.sink()))`, is how the frame asks a world
  for a full pass. It is installed in step 4, before the passes, of the next frame that
  draws, in four cases and no other, and `ShownStats::resets` counts each:
  1. the last frame that ran its passes was hidden;
  2. a frame since the world's last full pass was torn;
  3. a drawn frame's counts say the world's kind was lost (`KindCounts::lost`);
  4. a drawn frame's counts say rows of the world's kind were dropped or unplaced, and
     that frame's own pass was not a full one.
- Case 4's exception is a world the renderer was not sized for. A full pass that leaves
  rows out is not mended by another, and a new sink after every such frame would cut the
  snapshot every frame. The frame logs it at `warn` and draws what fits. A later frame
  whose pass was not full and whose counts are stale asks again, which is how the frame
  finds that the world fits once more. A lost kind is always answered, since it shows
  nothing until a full pass. `RendererDesc::rows` and `instances` are the binary's to size
  so that a full pass of every world shown fits one region (`crates/renderer/README.md`,
  "Regions").
- A full pass of a `Server` world publishes a `cut`: the frame after a reset blends
  nothing, once.
- The counts read are those of the world's kind (`FrameCounts::of`): at milestone 2,
  `server`. The remote batch's dropped and unplaced rows are in no kind's counts and need
  nothing: the next batch writes every remote instance again.
- A frame is torn when a method of the render side returns an error the policy logs
  ("Errors"). From `open`: nothing ran, the frame defers its passes as a frame that waits
  does, and `deferred` is `Some(Rendered::Torn)`; unless the frame is starved, when it is
  hidden and runs its passes, is counted in `torn` and in `hidden`, and reports
  `Rendered::Hidden` ("The frame's three states"). From `present`: `draw` is not called, and
  the open region keeps the frame's pass for the next `prepare`. From `draw` or `submit`:
  the object has ended the frame as torn. In each case the frame asks for a region again at
  its next call, a new sink is owed, no present is queued and `ShownStats::torn` counts
  it.
- An error a present call returns is a present skipped: the target's `read` is unchanged
  and a later frame queues again. A lane call that resolves `Cancelled`, because the
  present thread is gone, is the same. On a shared queue the error is met where the call's
  report is taken: at the hold before a submit, or at step 3's wait before a job that
  asks `open`. The submit is made, and its frame, having been held, is dropped as any
  held frame is. A call that returned an error, or was cancelled, brings no count:
  the report after it has none before it to rise from, and sheds nothing.

**Determinism of a shown world** (architecture sections 8.1 and 8.3; the milestone 2 plan,
criterion 3; D152)

- A shown world's hash after every tick is the hash of the world `build_server` builds from
  the same arguments and steps with the same frames: with the render side attached or not,
  on any backend, in every SOUR mode. What keeps it so:
  - One assembly path. `build_shown` is `build_server` and three installs; no registration,
    setup, applier or hook differs.
  - Nothing the render side says reaches a world, and nor does how long it takes. Its
    answers, and whether its task has resolved or a present call has returned by a frame's
    limit, decide whether a pass runs now, later in the frame or some frames later and
    whether a sink is installed, and nothing else: no value of its is written to a column,
    a resource or a `TickInput`. The ticks a paced frame runs depend on `now` and the
    frame callback alone, and are done before the frame waits for anything of the render
    side; a step-driven run's, on its steps.
  - A pass leaves the hash as it found it (D152; `lactose-present-api` shows it for
    extract). So passes run some frames later, or with another sink, or with none, move no
    hash. `ExtractState`, the snapshot's writer and the sink are a service's, in no column.
  - The presentation events' queue and the keys a world has delivered are outside every
    region (`crates/sim/README.md`, "The world hash"). So leaving them for the frame, and
    taking them some frames later, moves no hash; the `lactose-sim` option must keep that.
  - The frame's jobs are no system and touch no world: what they are handed is the render
    side and the snapshot's reading half. `Executor::run_job` is in an execution log as a
    run of one node, which a replay follows as it follows a graph's, and in no hash, save
    or recorded stream.
  - The frame orders no entity. A key it passes on is a `StableKey` in the sink's rows and
    the snapshot's, as `lactose-present-api` makes them; a target is an index below two.
- Criterion 3's tests, which `frame-impl` writes:
  - (a) and (b) are in this package's `integration` target, module `shown`, over a render
    side with no device (`tests/shown.rs`, section 5). (a): a sink and a render side that
    log their calls, a system of the tick that logs its start, and a frame callback; over
    paced frames and stepped ones, through each of the three states and through a busy
    frame, no tick's system starts between a pass's `begin` and its `end`, and no
    `present` or `draw` is called between a pass's `begin` and the `publish` after it.
    (b): frame N's `draw` waits at a barrier until a system of tick N+1 has started, and
    that system waits until `draw` has started, through `Shown::frame`, since `step` joins
    before it resolves. The frame that begins while `draw` is at the barrier may be busy,
    and the test asserts nothing of what it deferred. Each of the two waits is bounded, at
    ten seconds, and past its bound fails the test with a message that names the side that
    never arrived, so a regression fails and does not hang to the runner's timeout. The
    bound is kept by a clock that is no worker's, a thread of the test's own or a
    barrier's own timeout, and never by the executor's timers, which only a
    frame-critical worker that looks for work fires: the failure the bound is for leaves
    none. The
    test builds an executor of its own, of two frame workers or more, since one worker
    cannot hold both, and records no hash. Each asserts on order and never on time: the
    bound is a failure's and no assertion reads a duration.
  - (c) is in `lactose-app-client`'s `integration` target, module `recorded`. That package
    alone may name the real render side in normal code, so the test steps the wiring that
    ships, and the mover in `game-sim`, with no copy of either (D152). It steps one world
    with `Shown::step`, the render side on `lactose-rhi-null`, and the same world with
    `Server::step`, and SOUR finds the hashes equal in every mode. On null a breach of (a)
    on the real ring is a refusal as well (`host-overlap`, `unflushed`, `pending`). The
    package joins `[harness]` with the test and `[feature_roots]` as a binary that ships,
    so `cargo xtask features` compares its boundary tree in full with its
    dev-dependencies included: they may add nothing to it (`docs/verification.md`).

**What `lactose-app-client` wires** (the milestone 2 plan, decisions 5.1, 5.6, 5.11 and
5.18; `crates/platform/README.md`, "A client binary, by thread")

The package is `frame-impl`'s. D162 adds no manifest for it: no device backend and no
`lactose-presentation` exists at D162's merge, and `lactose-platform`,
`lactose-render-graph` and `lactose-renderer` are facades, so a `main` could construct
nothing; and a `lactose-app-*` package joins `[feature_roots]` in the change that adds it
(`tools/xtask/ci-policy.toml`), where an empty one would pass a comparison that means
something only for the binary that ships. The package is a library target that holds the
two objects and the wiring, and a `main` that calls it, so that criterion 3(c)'s test links
what ships. It declares nothing a world registers (decision 5.18). `tests/shown.rs`,
sections 1 to 4, is its code against this surface. In `main`'s order:

1. `lactose_platform::event_pump`, and the guard that queues `exit`; `start_executor` with
   `Workers::platform` the platform's `ClientPlatform`.
2. In a main-lane call, the window (`MainThread::window`), the device made from its
   handles, and the device's `Swapchain`.
3. The cooked pack set (`lactose_asset::pack_set`, D136), the cooked shaders and the
   pipeline cache file.
4. `RenderGraph::new`, `Renderer::new`, the presentation schedule and the `LatestSample`
   the latch reads: the fields of its `RenderSide`.
5. Its `PresentSide` over the swapchain, moved to the present thread with
   `lactose_platform::present_thread`, whose lane is the `PresentLane`.
6. `build_shown(&executor, &GAME, WorldConfig::shown(), start, cooked, render, lane)`.
7. The root, spawned on a frame-critical worker as `Headless::run` spawns its loop:
   `sleep_until` the deadline; the window's extent and its close, read through
   `WindowReports`; `Shown::frame`; the output to its log and its counters. A frame that
   fails is matched on `ShownError::kept` and ended by what it kept, never passed up with
   `?`, which the type does not allow ("Errors").
8. The shutdown ("Shutdown" below), on every path.

The fixed-frame mode of decision 5.1 replaces step 7 with a driver thread of the binary's
own, never a worker: `Executor::block_on(shown.step(..))` with a `TickInput` and a
`FrameTime` of the mode's own, a fixed extent and a fixed sample; then
`Device::wait_timeline` for the frame's point, within a bound, so that a region is free at
every step and every tick's frame is drawn. A step that did not draw fails the mode. Its
render side copies the target to a buffer in a pass of the frame's own declaration, after
the triangles and before the hand-off, so the image read is the one the graph rendered
before the present thread's draw, and no target is taken back from the present role to
read it: an image taken back so has no defined contents (`crates/rhi-api/README.md`,
"States"; D160, item 7 (c)). After the last step, `Shown::stop`, and the binary takes its
own object back as its own type (`RenderSide: Any`) for the buffer and for the dump of the
instance transforms, the fraction and the camera.

**The mover of the local world** (the milestone 2 plan, decisions 5.1 and 5.18)

Specified here, written by `frame-impl` in `game-sim`, and linked there by the binary and
by criterion 3(c)'s test.

- A component, beside `Counter`:

  ```rust
  #[component(key = "sketch::orbit", persistent, replicated)]
  pub struct Orbit {
      #[field(id = 1)]
      pub centre: Vec3,
      #[field(id = 2)]
      pub radius: Scalar,
      /// Ticks to go round once. Zero leaves the row where it is.
      #[field(id = 3)]
      pub period: u32,
      /// Ticks added to the world's tick: where on the circle the row is.
      #[field(id = 4)]
      pub phase: u32,
  }
  ```

- A system for each entity, in `Stage::Simulation`:
  `#[system(name = "game_sim::orbit", worlds = [server, prediction])]`
  `fn orbit(ctx: Ctx, orbit: &Orbit, transform: Mut<Transform>)`. With `t` the tick being
  run (`Ctx::tick`): `step = (t + phase) mod period`, the addition wrapping;
  `angle = TAU × step / period`, with `step` and `period` made `Scalar` before the
  division; `(sin, cos) = Scalar::sin_cos(angle)`;
  `translation = centre + radius × (cos, 0, sin)`;
  `rotation = Quat::from_rotation_y(angle)`. It writes `lactose_physics_api::Transform`
  every tick and nothing else, never `Teleported`, so the row is interpolated. A row with
  `Orbit` has no body.
- It is a function of the row and the tick, with no state that accumulates: a rollback, a
  save and a load, and a full pass place the row where an unbroken run does. (At the
  tick's wrap the row jumps unless `period` divides 2^32.) Its arithmetic is
  `lactose-math`'s alone, under the boundary's lints.
- `game-sim` gains dependencies for it, beside `lactose-core`, `lactose-ecs`,
  `lactose-reflect` and `lactose-sim`: `lactose-math`, and `lactose-physics-api` for
  `Transform`. It gains `lactose-present-api` too, for `MeshInstance`, if the scene enters
  by the sketch's session applier and not by a cooked baseline (below). Each edge is
  allowed: the first two are `[boundary]` crates and the third is in `[boundary_internal]`'s
  `any` (`crate-tiers.toml`).
- The physics sync reads every `Transform` row changed since its mark
  (`crates/physics-api/README.md`), so it visits each `Orbit` row every tick and finds no
  body. At the scene's seven rows that costs nothing to speak of; a scene of many such
  rows would have the sync skip a row with no body first.
- It is registered by `game_sim::register_types` and `game_sim::register_systems`, beside
  what is there. That changes `game-sim`'s saved types and may move the recorded streams of
  the tests that link `game-sim`, which `frame-impl` records again in the commit that adds
  it.
- The fixed-frame scene is seven rows, each with `Orbit`, `Transform` and `MeshInstance`,
  of seven phases and one period. How they enter the world, a cooked baseline of the pack
  set or the sketch's session applier, is `frame-impl`'s; the binary registers nothing.

**How each call of the frame is honoured**

`lactose-app` itself calls `lactose-present-api` and `lactose-task`, and holds
`lactose-rhi-api`'s values. Every call that reaches a device is made by the binary's two
objects, as `tests/shown.rs` writes them, through `lactose-renderer`,
`lactose-render-graph` and `lactose-rhi-api`. The backend columns are drawn from the tables
cited, which hold the evidence for each.

| Call of the frame | What it calls | Null | Native Metal | Vulkan (MoltenVK on the M1) | Cited |
|---|---|---|---|---|---|
| Step 4's pass: the sink's `write` | `write_buffer` on the open region, disjoint ranges at once | Fills the range; refuses an overlap (`host-overlap`) | A write into the buffer's contents | A write into the mapping | renderer, "How each call is honoured" (a writer's `write`), "Writers" |
| Step 4's pass and `publish`; the job's `acquire` | No device: the channel's exchange under its lock | The same on every backend | | | present-api, "The triple buffer", "Events and publication" |
| The job that asks `open`, and the frame's job | `Executor::run_job`: no device. One closure queued to the frame-critical workers, which with what it forks occupies `Executor::run_job_width()` of them at most, and its value handed back | The same on every backend | | | task, "Graphs" and "Scopes", as they hold for a run of one node; "What D162 asks of other crates" |
| `RenderSide::open` | `Renderer::open`: `timeline_value` | The counter | `MTLSharedEvent`'s value | The semaphore's counter | renderer, "How each call is honoured" (`open`), "Regions" |
| `RenderSide::open` after torn frames with none prepared | A declaration with no pass through `RenderGraph::{compile, record, submit}`, then `Renderer::submitted`; made with no present call in flight where the roles share a queue | Checks every command in order, applies the states, completes when its waits are reached | Command buffers enqueued in slice order; `encodeSignalEvent` | `vkQueueSubmit2` with timeline semaphores | graph, "Declaring", "How each call is honoured" (`submit`); renderer, "Regions" |
| `RenderSide::present` | No device: the presentation schedule on the pair | The same on every backend | | | present-api, "A frame's transforms"; decision 5.5 |
| `RenderSide::draw`: the target's image | `create_image`, exclusive, a colour attachment that is sampled; `destroy_image` of the one replaced, after `timeline_value` | Usage and a state for each subresource; refuses a destroy while pending or while a slot holds the image (`pending`) | `makeTexture`; the object released | An image; the object destroyed | rhi-api, "How each call is honoured" (`create_image`, `destroy_image`), "Handles and lifetimes" |
| `RenderSide::draw`: the target's import | `FrameDecl::import_image`: from `Undefined` on the graphics role, `ready` the present's `read`, left a colour attachment, handed off to the present role for a sampled read | States tracked from the declaration; the transition from `Undefined` ends a release no acquire met; where the roles have two queues, a submit that uses the image with no wait for `ready` is refused (`no-qfot`), and on one queue nothing is refused and nothing needs to be | No layouts and no ownership: the transfer's two halves are nothing; the wait is `encodeWaitForEvent` | A barrier from undefined, its source scope every stage and every write access ("Two rules of the layers below"); a release toward the present family where the roles have two queues | graph, "Imports", "What each consumer needs", "How each call is honoured" (`record`: around each pass); rhi-api, "States", "The present thread"; D160, item 7 (a) and (c) |
| `RenderSide::draw`: the remote batch, `Renderer::prepare`, `Renderer::declare` | As the renderer's rows for a writer's `write`, `prepare` and `declare` | As those rows | As those rows | As those rows | renderer, "How each call is honoured" |
| `RenderSide::draw`: `RenderGraph::compile`, `record` | As the graph's rows for `compile` and `record` | As those rows | As those rows | As those rows | graph, "How each call is honoured" |
| `RenderSide::submit`: `Renderer::latch` | `write_buffer`, `flush_buffer` on the frame's camera buffer | Judged at the submit, after the write | A write into shared memory | A write into the mapping | renderer, "How each call is honoured" (`latch`); graph, "A frame, by step" |
| `RenderSide::submit`: `RenderGraph::submit`, `Renderer::submitted`, `Renderer::open` | `Device::submit` for each of the frame's submits, with its waits, the import's `ready` among them, and its signals, the frame's point among them; `timeline_value`. Made with no present call in flight where the roles share a queue | Checks every command in order, applies the states, completes when its waits are reached; pending until the caller sees its point | Command buffers enqueued in slice order; `encodeWaitForEvent`, `encodeSignalEvent` | `vkQueueSubmit2` with timeline semaphores | graph, "How each call is honoured" (`submit`), "Submission"; rhi-api, "Timelines" |
| The present call: `Lane::run`, and a task's look at its `LaneCall` after a submit on a queue of its own, or, where the roles share a queue, the render task's await of it before a submit and the driving task's before a job that asks `open` | No device: the lane's queue and the call's reply slot; the call runs on `lactose-platform`'s present thread | The same on every backend | | | task, "Lanes"; platform, "The present thread and its pump" |
| `PresentSide::present`: the resize | `wait_timeline` for its own points, within a bound; `Swapchain::resize` | Images with no window | A `CAMetalLayer`, its bounds and `contentsScale` the view's | A swapchain; the present fence of each present waited for by `resize` within `acquire`'s bound | rhi-api, "The swapchain", "How each call is honoured" (`create_swapchain`, `acquire`, `present`, `resize`) |
| `PresentSide::present`: `Swapchain::acquire` | The acquire, bounded by the backend | An acquire that can be made to return `None` | `nextDrawable`, which has its own one-second limit | An acquire with a finite timeout | The same |
| `PresentSide::present`: the encoder | `create_encoder` on the present role; `transition_images`, the acquire half of the hand-off; `write_heap_slot` for a target's image it has not seen, behind `wait_timeline` for its last `read`, within a bound; the pass that samples the target into the acquired image | A command list; the state and owner of each subresource, applied at the submit in order; an acquire with no equal release is refused (`no-qfot`); the image for each slot, and a write refused while a pending pass declares the slot (`uuwp-used`) | A command buffer made on first use; nothing for the transition; a render command encoder, `useResource` for the declared slot's texture; a write of the texture's resource id into the argument buffer | A command pool and one command buffer; the acquire across families; `vkCmdBeginRendering`; an update-after-bind descriptor write | rhi-api, "The present thread", "States", "The heap and the sampler table", "How each call is honoured" |
| `PresentSide::present`: `Swapchain::present` | A submit and a present in one call, the submit waiting for the frame's point and signalling `read` | Completes when its waits are reached; refused (`pending`) past the bound with nothing submitted | `encodeWaitForEvent`, `encodeSignalEvent`; `present(drawable)` before the commit | `vkQueueSubmit2`, then `vkQueuePresentKHR` after the submit | rhi-api, "The swapchain", "Timelines" |
| `PresentSide::close` | `wait_timeline` for its last `read`, within a bound; `clear_heap_slot`; the swapchain's drop, which waits for the submits that use its images | The slot emptied; refuses while a pending pass declares it (`uuwp-used`), and the slot then keeps its image, whose destroy is refused (`pending`) until the device goes | The placeholder's id written; the layer let go | The placeholder written; the swapchain destroyed | rhi-api, "The heap and the sampler table", "Handles and lifetimes"; platform, "Shutdown", step 2 |
| A step-driven run's wait for a frame's point | `Device::wait_timeline` with a bound, on the driver thread | A counter and a condition variable | `MTLSharedEvent`: a listener, a wait with a timeout | A timeline semaphore | rhi-api, "Timelines", "How each call is honoured" |
| The drop of `Stopped::render` | `RenderGraph`'s drop, then `Renderer`'s, then `destroy_image` for each target | Refuses a destroy while pending | The objects released | The objects destroyed | graph and renderer, "Errors and drops"; rhi-api, "Handles and lifetimes" |
| Any of them on a lost device | `ErrorKind::DeviceLost`, and after it every wait returns at once and every drop succeeds | A flag its own call sets | A command buffer's error status | `VK_ERROR_DEVICE_LOST` | rhi-api, "Device loss" |

Two rules of the layers below that the frame leans on:

- **The re-take from `Undefined` ends a release no acquire met** (D160, item 7 (c)).
  `crates/rhi-api/README.md`, "States", says so: a transition from `Undefined` "ends a
  release no acquire met and takes the resource with no transfer, its contents undefined",
  and "an acquire of that release is then refused". Null does it, as
  `crates/render-graph/README.md`, "What each consumer needs", states. The frame makes
  such a re-take whenever a finished target was not queued, or its call made no submit. On
  native Metal there is no ownership and nothing to end; on Vulkan it is a barrier from
  undefined on the releasing queue.
- **Such a re-take follows a write on the frame's own queue.** Before D162 every re-take
  followed a present's read. A re-take after a frame no present read (one dropped, at its
  submit on a queue of its own or after its hold on a shared one; one whose call made no
  submit; one whose present was skipped), or after a torn frame whose submit wrote the
  target, follows that frame's own attachment store: a write after a write. Where the
  roles have two queues a frame's release makes its write available. On one queue nothing
  is recorded for the hand-off, and a torn frame that wrote the target and released
  nothing leaves the write on either layout. So the barrier from undefined carries the
  earlier write in its source scope on every layout, which "States" says as well: "Vulkan
  gives such a transition a source scope of every stage and every write access
  (`ALL_COMMANDS`, `MEMORY_WRITE`): the earlier use on its own queue may be a write, an
  attachment a dropped or torn frame stored, and not only a read." `rhi-vulkan` owes the
  barrier, and the suite's output case `retake_after_a_store_on_one_queue_is_clean` holds
  it there under synchronisation validation (`crates/rhi-api/README.md`, "The conformance
  suite"). Null tracks states and not availability, so it cannot show the defect. A shared
  queue's drop makes the case an everyday one: every held frame is a store and then a
  re-take on one queue. `reviewer-gpu` ran the frame's order with the drop on MoltenVK,
  both roles on one queue, under the Khronos layer's synchronisation and thread-safety
  validation (the review of D162's second draft). With a source scope of every stage and
  no access, the layer reported the write after a write: 10 errors over 93 dropped frames
  in one run and 1 after 1 in another. With `MEMORY_WRITE` in the scope it reported
  nothing in three runs, and every readback was exact (540, 598 and 1201 frames).
  `frame-impl` owes the same run through the shipped frame, once a Vulkan backend can run
  it, with held frames, a call that makes no submit and a torn frame in it. The layer does
  not report a re-take after a present's read whose barrier names too narrow a source
  stage, which `reviewer-gpu`'s probe ran unreported: that order rests on Vulkan's rules
  for the barrier's every-stage source, and a clean one-queue run is no evidence of it.
- **A submit that uses an image another queue's pending submit changed waits for it**
  (D160, item 7 (a)). The graph makes every submit that names an import wait for the
  import's `ready`, and the frame gives the present's `read` as that point, so no submit of
  a frame is refused for it. A wait for a point already reached costs no GPU wait.

**What D162 asks of other crates**

Named in their smallest form. None is made by the facade's change; the signatures above
stand without any of them, and `frame-impl` cannot write the bodies without the first two.
Without the third no frame is shed and `frame-pace` has no age at the display.

- `lactose-sim`: a builder option, `SimulationBuilder::keep_present_events(self) -> Self`,
  after which a `Server` world's step leaves its presentation events in the world's queue
  for a caller that takes them after each frame's passes; and `crates/sim/README.md`,
  "Runs", step 4, saying so. The default is today's. It changes no hashed state. Without
  it a shown world delivers no event and nothing else here changes.
- `lactose-task`: one job for a task that is in none. The job and what it forks leave one
  frame-critical worker out (*The width*, below). A lane on `crates/task` makes it before
  `frame-impl` starts; D162's change edits nothing in that crate.

  ```rust
  impl Executor {
      /// Runs `job` once, as one job on a frame-critical worker, and resolves to what it
      /// returned.
      pub fn run_job<F, R>(&self, job: F) -> JobRun<R>
      where
          F: FnOnce(&mut JobContext<'_>) -> R + Send + 'static,
          R: Send + 'static;

      /// How many frame-critical workers the jobs `run_job` starts, and the children
      /// their scopes fork, occupy at most at any moment: `frame_worker_count()` less
      /// one, and one where that is one. A constant of the executor.
      pub fn run_job_width(&self) -> u32;
  }

  /// A job in flight. `Send` where `R` is.
  #[must_use = "futures do nothing unless awaited"]
  pub struct JobRun<R> { /* private */ }

  impl<R: Send + 'static> Future for JobRun<R> {
      type Output = lactose_core::Result<R>;
  }
  ```

  What the frame relies on, as the sentences `crates/task/README.md` would carry:
  - *A job like any other.* The closure is queued when `run_job` is called, as a graph run
    starts at the call, and a frame-critical worker runs it once: never inside the call,
    and never a background worker. Where a task on a frame-critical worker calls it, the
    job may run on that worker, once the task's poll has returned, or on another that
    takes it sooner. A caller that is no worker, as the thread that drives a stepped run
    is, never runs it. It is one job as one call of `GraphNode::run` with one unit is. It
    has that job's `JobContext`, whose `slot` is the worker's; the job scratch is reset
    when it returns; one abort-on-unwind guard wraps it; it may open scopes one after
    another, and their children are its own. It is one entry and never split. A worker
    that takes it while the gate is closed puts it back unstarted, as the rest of a range
    is put back. It has no `finalize`.
  - *The gate and the reservation.* A worker takes it only while it holds a run permit, as
    it takes a graph's job. So while `block_in_place` holds the gate it starts on a worker
    the reservation leaves awake, or waits for the gate; a job that is running when a
    block begins finishes; and one parked on its own scope's latch holds no permit.
  - *The width.* The jobs `run_job` starts, and the children their scopes fork, occupy
    `run_job_width()` frame-critical workers at most at any moment, counted over all such
    jobs together: `frame_worker_count()` less one, and one where that is one. A worker is
    occupied from the moment it takes such a job until the closure has returned, whether
    it is running the closure, running a child of one of its scopes, parked on a scope's
    latch or waiting for the gate; and for as long as it runs a child of such a job that
    it took from elsewhere. A worker with a scope of such work open runs that scope's
    children itself, under the place it holds: while one of them is still to be started it
    takes that one and does not park, the gate letting it run. One that parked and was
    woken looks for its scope's children again before it parks again, whatever woke it. So
    no child of its scope waits queued while its opener could start it, and where the
    width is one they all run on the job's own worker. So with two frame-critical workers
    or more, at every moment one of them is occupied by no such job: it takes graph jobs
    and tasks as today, fires the due timers where it looks for work, and is the keeper
    while it sleeps. A timed wake is then late by the keeper's wake, or by what is left of
    a job or a poll that is no such job's. The width parks nobody and spins nobody: a
    worker that may not take such work looks past it, takes whatever else there is, and
    sleeps as any idle worker does. An entry queued while fewer than the width are
    occupied is taken, and one queued while the width is full is taken once a worker
    leaves it, with no wake-up lost in either case. Under `block_in_place` the gate
    applies first, as now: a worker needs a run permit to take anything. A worker the
    width counts may hold none, as when it is parked on its latch or waits for the gate.
    It is scheduling and nothing else: an execution log records what ran where, and a
    replay follows the log. How two such jobs in flight at once share the width, side by
    side or one after the other, is the lane's, and the frame does not rely on it, since
    it never has two: the job that asks `open` is awaited before a render task is spawned,
    and a render task is taken back before the next `open`. Their safety is not the lane's
    to choose: a worker inside one such job runs the children of the scope it has open and
    nothing else of such work, never a child of another scope and never another such job,
    which would nest a scope (architecture section 5.1, "Fork-join inside a job only").
    There is no knob. Every `run_job` job is so counted, this crate being the one caller.
  - *Why a count beside the run permit, and one way to keep it.* The gate cannot be the
    width. It counts every kind of work: a worker without a run permit takes no job and
    polls no task. A gate closed at the width while the frame's job runs would leave the
    keeper to fire the timer on time and the root it woke to wait for a permit behind the
    record's children, which is the lateness the width is there to remove. So the pool
    counts the workers such work occupies, beside the permits. One way, which the lane may
    replace: such a job, the children of its scopes, the upper half a thief splits off and
    the rest a worker puts back go on a queue of the pool's that holds such work alone,
    each entry tagged with the scope that forked it as a deque's entries are, and never on
    a worker's own deque, so that no thief takes one blind. A worker that holds no place
    takes from that queue only after raising the count while it is below the width, and
    lowers it when the entry is done. The job's own worker keeps its place from the job's
    start to the closure's return. A worker with a scope of such work open takes that
    scope's children from the queue under the place it holds, with no raise of the count,
    before it parks: never another scope's entry and never a job, and it parks on the
    latch only when none of its scope's children is queued. An entry of its scope that is
    queued after it parked, the half of a range or what a worker put back, wakes it, and
    it reads the queue for its scope's entries again after every wake, before it parks
    again. The opener's loop as it is does not: once `wait_for_children` has found no
    child of its own to run, it parks again at every wake without looking, which would
    swallow that wake and leave the half queued beside a parked opener. No wake-up would
    be lost by it, since the worker that lowers the count wakes a sleeper for the half,
    but the fan-out would be back on the other places alone. Without the take an opener
    would look for its children on its own deque, as it does today, find none and park: on
    two frame-critical workers, where its place is the whole width, no worker could take
    them and the job would never return, and on four the fan-out would have the two other
    places alone. The pool's look for visible work counts that queue only while the count
    is below the width; and a worker that lowers the count with the queue not empty wakes
    a sleeper, the one side's push before its read of the count meeting the other's
    lowering before its read of the queue. That pair is a store-buffering pattern: each
    side writes one word and then reads the other, so each needs a `SeqCst` fence between
    its write and its read, or `SeqCst` operations on both words, and release with acquire
    loses the wake-up. And a range needs a split rule of its own: `drain_range` splits
    what is left of a range when its worker's own deque is empty, which is always so once
    the halves go to a queue of the pool's. The lane holds four things in loom models
    through the shipped functions, in three configurations: a width of one; a width of two
    with two such jobs in flight, on three workers, so that a worker which holds no place
    tries to raise the count and the first of the four means something; and a width of two
    with one such job on three workers, whose scope forks a range of two units or more.
    The four: the count never exceeds the width; an entry queued while the count is below
    the width is taken, and one queued while the count is at the width is taken once it is
    lowered, with no wake-up lost in either; no child stays queued while its opener is
    parked on the latch, the gate being open; and no worker inside one such job runs
    another scope's entry or another such job. In the third configuration the worker that
    takes the range and puts a half back is held until that half has run, so that a half
    left queued beside a parked opener is a deadlock loom reports: left free, that worker
    would take the half itself where it lowers the count, and the model would end clean.
    Under loom 0.7.2 a model whose reader is the main thread, its writer spawned, may
    explore one execution, so a model counts only beside weakened variants of the shipped
    orderings that fail it. Architecture section 5.1 gains the width with that lane's
    merge, as an amendment of its own. It is owed in two places, whatever way the lane
    takes: the split bullet of "Scheduling mechanics", for the width and for where the
    halves of such a job's range go, and "Fork-join inside a job only", for how the opener
    of such a job finds its children. The way sketched here departs from both, for such
    work alone: from "the thief pushes one half to its own deque", and from an opener's
    deque that "holds only its own children", which then holds none of them.
  - *What was weighed for it, and what the lane shows.* Timers fired inside a long job: a
    root woken between a range's units would go onto the firing worker's own deque behind
    its share (`keep_woken_task`), and running it inline would nest a scope. A width the
    render side keeps for itself by forking one child fewer: an opener parked on its latch
    fires no timer either, so it costs two workers and not one, it rules out a forked
    range, and nothing checks it; `reviewer-concurrency` reached that park in the executor
    as it is, in 1 record of 300 that forked every share, the limit then 7.03 ms late.
    Bounding the record's jobs near a millisecond, which `reviewer-concurrency`'s runs
    showed to be as good as a free worker: a recording job is a run of consecutive passes
    that the plan cuts, and a recorder is one call of the binary's, so neither this crate
    nor `lactose-task` can bound one. What the lane shows, beside the loom models: by
    order and never by a time, each wait bounded by a thread of the test's own, that with
    a `run_job` job whose scope forks `frame_worker_count()` children and one more, each
    held where it begins, a task's `sleep_until` resolves and a graph run started
    meanwhile completes while they are still held, which an executor that keeps no width
    fails, every worker being held, where a job that forks only as many as the width
    passes on any executor, as `reviewer-concurrency`'s cut records did on the executor as
    it is; the same on an executor of two frame-critical workers, where every child of the
    job's scope runs on the job's own worker while the other fires the timer and completes
    the graph run; the same with the opener parked on its latch, on three frame-critical
    workers or more, where the scope forks one child fewer than the width, each of them
    has begun on another worker before the scope's body returns and is held there, and a
    second `run_job` job started once the opener has parked does not begin before a child
    is released, the timer resolving and the graph run completing meanwhile: the count is
    full with the parked opener in it, whatever way the lane shares two jobs, and the test
    sees the park by a count of parked openers kept under `cfg(test)`, as
    `Pool::sleepers()` shows the idle protocol's; that on an executor of one
    frame-critical worker the width is one and a job's children run on its own worker;
    and, as figures with no threshold, the lateness of a timed wake set inside a fan-out
    that fills the width, for the three shapes `reviewer-concurrency` ran ("The shown
    frame", *One worker stays out of the frame's job*), which is also the first measure of
    what the queue and the count cost.
  - *What the future says.* `JobRun` resolves `Ok(value)` once the closure has returned,
    which is after every scope it opened has closed: no child of it is executing or ever
    will, and what they and the closure wrote happens before the resolution. A job that
    started finishes and hands its value back, shutdown or not. It resolves `Cancelled`
    when shutdown began before the job started; the closure is then dropped unrun, under
    the abort guard, on the thread that discards the queued work. Dropping `JobRun` does
    not cancel the job, and its value is then dropped by the worker that ran it or by the
    thread that dropped the future, whichever comes last. This is architecture section
    5.1's "Resolution means quiescent" for a job, with a rule of its own for a job that
    started, which a graph run of one node does not have. So the lane that makes `run_job`
    adds `JobRun` to that sentence's futures, as an amendment of its own, and holds the
    race the rule turns on in a loom model through the shipped functions: a worker taking
    the job against shutdown beginning. The future resolves `Ok` or `Cancelled` and never
    both, the closure is run or dropped exactly once, and `Ok` comes only after every
    scope it opened has closed.
  - *What it costs.* One allocation a call is the lane's target, which an entry of the
    lane's own can meet and a graph run of one node does not. Its locks are at most the
    ones a graph run takes: the run registry at its start and its end, and the completion
    slot at its start, at its end and at each poll.
  - *Tags and spans.* It runs untagged unless the closure enters a tag, as a graph job
    does, and has one span, target `whisk`.
  - *Chaos.* In an execution log it is the one job of a run of one node: it takes a run
    ordinal as `run_graph` does, its scopes' children are recorded as that job's, and a
    replay needs a program that calls `run_graph` and `run_job` in the same order.
  - *A lane call polled after it resolved.* Asked of the same lane, and not of `run_job`:
    a `LaneCall` polled again after it returned `Ready` panics, as a `GraphRun` does,
    where today it keeps the waker and stays `Pending` for ever. On a shared queue the
    frame holds one call that three places may poll (step 3's wait, the render task's hold
    and `stop`), and a slip that polls one twice then fails with a message and does not
    hang.

  What it may not be used for:
  - A wait in the closure for anything but its own scopes. A job that blocks keeps its
    run permit, which under the gate is a reserved one; `block_in_place` stays the
    physics step's alone (architecture section 5.1).
  - A world's work outside its schedule. A system reaches a job through its schedule and
    its `Job` parameter; nothing inside the determinism boundary is a task, and nothing
    there calls this. The boundary's lint file holds it: `lactose_task::Executor::run_job`
    is disallowed in every boundary crate, beside `run_graph` and `spawn`
    (`crates/core/clippy.toml`, with the list `cargo xtask boundary` pins). D162's own
    change adds that entry, before the method exists. Until it does the entry stands
    under `allow-invalid`, which would also hide an entry whose path never comes to exist.
    So the lane that makes the method shows once, in its proof, that clippy refuses a
    boundary crate's call of `Executor::run_job` by the path the entry names.
  - What a graph is for: work with units, dependencies or a `finalize` is a `GraphNode`.

  Why this and nothing smaller. It is `run_graph` over one node of one unit, less
  `GraphNode`'s `&self` and `Sync`, which make an owner that moves its state in and out
  take a lock or write `unsafe`; it adds no capability a graph run lacks, and one limit a
  graph run does not have, the width. A job opened in place inside a task's poll, with no
  queue and no future, was weighed and is not asked for: a task's poll is in no execution
  log, so its scopes' children could not be replayed, and a step-driven run's driver
  thread is no worker and has no slot. This crate is its one caller, twice a frame at
  most.
- `lactose-rhi-api`, by lane `swapchain`: an optional display-time report on `Swapchain`,
  added with that lane's merge as an amendment of D158. The facade is unstable
  (architecture section 20), and D162's change edits nothing in that crate. What the
  report is, in its smallest form:
  - *A present has an id.* Each present the swapchain takes, `Presented::Shown` or
    `Stale`, has an id the caller learns at that call's return, increasing by one from the
    swapchain's first; a `resize` does not restart it. The backend gives the driver that
    id with every present, none left out. On MoltenVK that is `VkPresentTimesInfoGOOGLE`
    chained to every `vkQueuePresentKHR`, with `desiredPresentTime` zero, since any other
    value schedules the present. Without the chain every report carries id zero: a run of
    `reviewer-gpu`'s so made read 601 reports, all of id zero, and a count of nine or more
    at 592 of 600 calls.
  - *A report by id.* One method that never waits and reaches no queue gives the reports
    that arrived since its last call, oldest first: for each, the present's id and the
    time its image reached the display. Each id is reported once at most and in order. It
    is called on the thread that presents, between that thread's other calls of the
    swapchain: every Vulkan source's read is externally synchronised on the swapchain with
    the acquire and the present (`vkGetPastPresentationTimingGOOGLE`,
    `vkWaitForPresentKHR`, `vkAcquireNextImageKHR`, `VkPresentInfoKHR::pSwapchains`), so a
    read from another thread is invalid whichever of them the driver blocks in, and
    thread-safety validation reports it. A driver that keeps a bounded history is drained
    within it: MoltenVK keeps 60 reports and overwrites the oldest, and a read as each
    call begins is far inside that. A report lost all the same is covered by a later one,
    since the count takes a present as displayed once its own id or a later one is
    reported.
  - *A present the display never took* (one discarded at a `resize`, or replaced) is
    either not reported, a report of a later id telling the caller so, or reported so that
    the caller can tell it from one displayed. The count needs neither in particular: it
    takes a present as displayed once its id or a later one is reported, and a present
    counted so that was never displayed does no harm. `frame-pace` needs them told apart,
    since such a report's time is no display time and puts a false age at the bottom of
    its first row. Metal tells them apart: `presentedTime` is zero for such a present (one
    and two presents in two of `reviewer-gpu`'s runs), and the backend leaves it out.
    MoltenVK 1.4.2 cannot: it replaces a zero `presentedTime` with a reading of its own
    clock (`MVKImage.mm`), and in every run the window's first one or two presents read 1
    to 8 ms at the display, under the path's least of 16.5 ms. That is allowed there, the
    swapchain's README states it, and `frame-pace` drops such ages by their size
    ("Pacing"). MoltenVK 1.4.2 also loses the reports of a swapchain destroyed before its
    presents reach the display: it keeps the reports by `VkSwapchainKHR`, and records none
    for an image whose swapchain is gone (`MVKImage.mm`, `endPresentation`). The count
    covers such a present once a later id is reported. Through a live resize that replaces
    the swapchain at every frame none may be until the resize ends, so the count climbs,
    one frame is shed, the count grows with each call after it, and `frame-pace` has no
    ages for those frames. That is read from the source and is suspected: no run has made
    the resize. Two answers were named and neither is asked for: a backend that reads a
    replaced swapchain's reports before it destroys it, once its presents complete, which
    is this lane's; or a count that takes every present made before the last `resize` as
    displayed, which is the binary's present side's.
  - *The time is on the host's monotonic clock,* in a form the caller can compare with an
    `Instant` it read. An `Instant` cannot be made from a raw clock value, so the form is
    the lane's: an offset from an `Instant` the backend read beside the driver's clock,
    for one, with the error of that pairing stated. MoltenVK's `actualPresentTime` is
    Metal's `presentedTime` in nanoseconds, on the mach host clock, which `Instant` reads
    on macOS; a pairing's error is the gap between its two reads, 0.008 to 0.031 ms in
    `reviewer-gpu`'s probe.
  - *`None` where the device gives none.* Whether a swapchain reports at all is known from
    its creation, as an `Option`, a capability or a method that answers `None`, so that a
    caller tells "nothing has reached the display yet" from "this device never says".
    `lactose-rhi-null` reports none. The sources are `VK_GOOGLE_display_timing`'s
    `actualPresentTime` on MoltenVK, which 1.4.2 offers; `CAMetalDrawable.presentedTime`
    on Metal; and `VK_EXT_present_timing` where a device offers it, at the stage
    `IMAGE_FIRST_PIXEL_OUT` or `IMAGE_FIRST_PIXEL_VISIBLE` (the SDK has it at 1.4.363, and
    MoltenVK 1.4.2 does not offer it). `VK_KHR_present_id` with `VK_KHR_present_wait` is a
    source only where the swapchain's README shows, from a run, that the wait's completion
    follows the display. The run is a floor run as `frame-pace` makes it, on a display of
    fixed rate: the frames a thousandth slower than half the display's rate, so that their
    phase against the refresh sweeps, for a minute or more. The test is the grid. Its
    period is the display's refresh as the system reports it, refined by a fit to the
    reported times that moves it by five parts in ten thousand at most; and 99 in 100 of
    the reported times lie within a tenth of that period of one phase. The bound on the
    fit is what makes it a test: half the frames' own period is a thousandth longer than
    the refresh, and a period fitted freely to times that follow the frames lands there
    and finds them on one phase. Times that follow the display do, an image reaching the
    glass at a refresh and at no other time: MoltenVK's display times kept one phase
    within 0.07 ms over the 1,789 presents of such a run of `reviewer-gpu`'s, in the
    review of D162's fifth draft, and within 0.012 ms in a run under drift. Completions
    that follow the frames do not: `VK_KHR_present_wait`'s lay up to 8.3 ms either side in
    the same runs, the whole period. The review took its period from the display times,
    16.6668 ms in the floor run, which a device whose one source is the pair does not
    have; so the period here is the reported refresh. `reviewer-gpu` made the test in that
    form in the review of the sixth draft, offline, over the times of three of those runs
    and the reported 16.666666 ms: the display times passed, 99.9 to 100% of them within
    the tenth, and the wait's completions were refused at 25.6 to 30.0%, where a fit left
    free within three parts in a thousand passed the same completions at 99.9%, on the
    frames' period or on half of it. Where the bounded fit finds no phase, as on a display
    whose rate varies or is not the rate reported, the pair is no source. A display at
    59.94 Hz reported as 60 is that case, emulated in the same review from the floor run's
    times. The frames sit at exactly half the true rate and do not sweep; the times of
    both sources lie on one phase of the true refresh, which the bound excludes; and the
    bounded test refuses both, the display times at 24.4% and the completions at 25.6%,
    where a free fit passes both. That is safe for the pair, which is refused, and wrong
    for a source that does follow the display, which `frame-pace` meets in its own grid
    reading: its count of the floor run's sweeps shows the run for what it was ("Pacing").
    The extension promises no such thing ("There is no requirement for any precise timing
    relationship between the presentation of the image to the user and the update of the
    presentId value"). MoltenVK completes the wait when the present's command buffer
    completes, 31.7 to 48.5 ms before the display in `reviewer-gpu`'s run and 38.75 ms at
    the median: a count made from it read none at all 1,800 calls where the count from
    display times read two at 1,751, so no frame would be shed and `frame-pace`'s first
    row would leave out the swapchain's queue. Where it is a source, "never waits" is a
    poll with a timeout of zero for each outstanding id, on the thread that presents, and
    the time is the poll's: late by up to the gap between polls, which is a frame period
    where it is polled only as a call begins. The grid run therefore polls at an interval
    under its tolerance, 0.25 ms in a run of `reviewer-gpu`'s for the fourth draft: polled
    only as each call begins, the poll's time is that call's start, which follows the
    frames whatever the source does, and the test would refuse a source that follows the
    display. A device with none of these answers `None`.
  - *What its README states.* Which instant the time is: the refresh at which the image is
    on the glass, as the first two sources give it, or which stage of the third, since
    what the count reads at a device's floor differs by a refresh between an early stage
    and a late one. How late a report may arrive after that instant, since the shedding
    rule counts a picture as on its way until its report is in: under a millisecond on
    MoltenVK and under two on Metal, as `reviewer-gpu` measured ("Pacing", *The shedding
    rule*). How a present the display never took is reported. How the time compares with
    an `Instant`, and within what error. Where the source is `VK_KHR_present_wait`, the
    grid run that makes it one, with what it read against the tenth of a period and the
    interval it polled at, and that its time is a poll's. A report costs the caller no
    wait and no lock held across a driver call that blocks; the README states the bound
    within which the report's call returns, and the lane holds it at every call of a run
    in which each present call waits for an image, the report being called on the thread
    that presents, between its presents.
  - *What uses it.* The binary's present side alone, on the present thread: it counts
    `PresentReport::waiting` from the reports and its own timeline's value ("Pacing", *The
    shedding rule*), and gives `frame-pace` each present's display time beside the
    `Finished::point` it presented. The count is that side's code, and `lactose-rhi-null`
    reports nothing for it to count, so `frame-impl` holds it by a test over scripted
    reports ("Proof for the shown frame"). `lactose-app` names none of it: the trait
    `PresentSide` and `PresentReport::waiting` are all it sees. The signatures are the
    swapchain lane's to choose; the items above are what the frame needs of them.
- `lactose-app`, by `frame-impl`: `lactose-present-api`'s three registration functions in
  the engine lists ("The shown frame").
- Sentences of other READMEs that leave a choice to `frame-design` become references to
  D162 in the change that next touches each, with five readings to carry over.
  `crates/render-graph/README.md` names a system with a `Job` parameter beside a `ResMut`
  of a `dyn` trait as the one lock-free way to hold the graph and a `JobContext` together,
  and it and `crates/renderer/README.md` say that trait belongs to a foundation crate: the
  frame's way is `Executor::run_job`, a second lock-free one with no system and no world,
  and the trait is `RenderSide`, this crate's. The renderer's "A frame, by step", step 1,
  has the frame wait for a region's point off the frame-critical workers: the frame asks
  again at its next call, and no thread waits. `crates/rhi-api/README.md`, "The present
  thread", has the present thread hold one target at a time, the newest the frame has
  finished: no command of the present side's names a target between its calls, and its
  heap slot may hold the last target's image until a call names another for the index or
  `close`. `crates/render-graph/README.md` owes a test that recording runs in parallel on
  "at least two frame-critical workers", and `crates/presentation/README.md` has `upload`
  write its forked rows "at once": inside a job that `Executor::run_job` started the forks
  have the width, so they run side by side on three frame-critical workers or more and one
  after another on two, on the job's own worker, and a test that needs two at once there
  needs three workers, or a context that is no such job's. `crates/presentation/README.md`
  has the frame acquire the pair "in its own task": the acquire is made inside the frame's
  job, which that task starts (step 5). The facade stands with those sentences as they
  are.

**Errors** (architecture sections 3, 14 and 19)

- `ErrorPolicy::respond(source, error)` is the whole policy and it is a table. Milestone 1:

  | `Source` | `Response` |
  |---|---|
  | `Executor` | `Stop` |
  | `Load` | `Stop` |
  | `Server { kept: true }`, `Server { kept: false }` | `Stop` |
  | `Prediction { kept: true }` | `Resync` |
  | `Prediction { kept: false }` | `Stop` |
  | `Save` | `Log` |
  | `Cooked` | `Resync` |

  A server failure stops whether or not the world was kept, because a server never rolls
  back and a prepare failure means an unjournaled edit removed what a system names: there
  is no state to go back to. A cooked-content failure in a client's fold resyncs because
  architecture section 14 makes a client-only asset failure a request for a resync rather
  than a divergence: the world keeps its tick (a content resync) and the full frame that
  follows retries the region. `Stop` means the run that failed ends with the error and the
  error reaches the binary's `main`, which returns it; the runtime's abort on panic is
  separate and unconditional. The milestone 1 table reads no `ErrorKind`. Milestone 4 adds
  `Source::Net` and `Source::Input`, `Response::Disconnect` and `Response::Refuse`, and
  reads kinds: `StorageFull` on a save (the disk-full path), `InvalidData` and
  `VersionMismatch` on a load (the corrupt-save path), `Network` from the transport.
- `ErrorPolicy::MILESTONE_2` (facade) is what `build_shown` builds with: milestone 1's
  rows, and one source more, the first whose response reads the error's kind.

  | `Source` | `ErrorKind` | `Response` |
  |---|---|---|
  | `Render` | `DeviceLost` | `Stop` |
  | `Render` | any other | `Log` |

  `Source::Render` is an error from a method of the `RenderSide`, or the result of a
  present call. `Log` tears the frame or skips the present ("Counts, torn frames and
  resets") and the next frame runs. `Stop` on a lost device ends the run with `ShownError
  { kept: ShownKept::Shown(..) }`: the world is whole, and the binary calls `Shown::stop`,
  which ends in order on a lost device because every wait there returns at once and every
  drop succeeds (`crates/rhi-api/README.md`, "Device loss"). Recovery is not here ("Not in
  this crate yet"). Under `MILESTONE_1`, `Source::Render` is `Stop` whatever the kind. A
  render task decides its own errors with a copy of the policy. A `Stop` it met reaches
  the caller where its frame is taken back: as the error of the `step` that started the
  task, of the first `frame` that takes the task back, or of `flush`. `stop`
  logs it and goes on.
- Every response is logged at `LOG_TARGET` with the source and the error before it is
  acted on. The lifecycle's load, session and asset failures are not errors of a run. On a
  server they are returned in the `StepOutput` of the step that reported them and in the
  frame's output, logged per event at `warn`, and the tick carries on; what to do with a
  refusal is the caller's. On a client they are the prediction world's, taken in step 5 of
  the frame and logged at `warn`, never returned (`ClientOutput` has no failures field): a
  refusal or an asset failure becomes a resync, reported in `ClientOutput::resync`.
- `ServerError` and `ClientError` convert into `lactose_core::Error`, so `?` into a plain
  `Result` drops the world as a plain error would; a caller that wants the world matches.
  After `Cancelled` (the executor shut down mid-run) the world is gone and `server` or
  `client` is `None`; so is the server after a step whose unload export could not be
  diffed into the store ("Saves and loads" below).
- `ShownError` (facade) is a shown run's failure: the error, and what survived it. An error
  of the world's own run is decided by the rows above as a `Server`'s is, and the render
  side changes none of them. `ShownKept::Shown` is the world and the render side, after an
  error the world's run survived or a render side's `Stop`. `ShownKept::Render` is the
  render side alone, ended as `Shown::stop` ends it, when the world did not survive and the
  executor did. `ShownKept::Nothing` is after `Cancelled`: the world and the render side
  were dropped with the tasks that held them. `ShownError` does not convert into
  `lactose_core::Error`, unlike `ServerError`: `?` into a plain `Result` would drop `kept`
  where it stands, which in a root task is a frame-critical worker, where a render side's
  drop waits for the device ("Shutdown"). A caller matches `kept` and ends what it holds
  with `Shown::stop`, as `tests/shown.rs`, section 3, does. `lib.rs` holds the missing
  conversion at compile time.

**Saves and loads** (architecture sections 15.2, 15.3 and 15.5; D129;
`crates/persist/README.md`; `crates/io/README.md`)

- The store. A server holds the saved state of every region that is not loaded, in
  `Server::kept`, as a `SavedRegion` (the records at the registry's schemas and the
  baseline they were made against), so no region a save or an unload produced is lost
  between saves. A region enters the store from a `Start::Loaded` saved world, or from the
  export of its unload, which `Server::step` diffs with `RegionRecords::from_export` after
  the step against the baseline the region had when it was exported: its
  `Server::baseline` from before the step if it was loaded at the step's start (a
  `Directed` unload runs before the frame's bundles), else the baseline of the bundle this
  step loaded it with. Then every region `World::regions` holds after the step leaves the
  store, and every region it does not leaves the baselines: the store is reconciled
  against the world, never against the frame, because under `Anchored` the sync point
  loads a frame's bundles before the region policy runs, so a bundle can load and its
  region unload again in one step (a player joining, the server queuing the saved
  neighbourhood, and the player's `Leave` arriving in the step that loads it), and under
  `Directed` a region unloaded and reloaded in one frame whose reload bundle is refused is
  not loaded after the step; either way the export is the region's saved state, and it
  stays kept. The export of a region unloaded and reloaded in one frame and loaded after
  it is not kept: the frame replaced it, as `crates/sim/README.md` says. So after every
  step no region is both loaded and kept, and a region leaves the store only by being
  loaded. A diff that fails (the registry and the schema table disagree, or the region
  has no recorded baseline, `NotFound`) would lose that region's saved state: the step
  finishes every other region's bookkeeping (the other exports, the baselines, the store,
  the `Load` answers), then fails with the first such error in ascending region order and
  `ServerError::server` `None`, so a server missing a region is never handed back to be
  saved; the policy's response is `Stop`.
- Reloads from the store. Under `Anchored`, every `RegionRequest::Load` the lifecycle
  issues for a kept region is answered by the server itself: the bundle
  `Loader::bundle(&records, baseline, RegionMode::Frozen, table)`, with `table` the
  region's `Cooked::table` pinned through `Cooked::pin`, or `None` when it has none or the
  pin fails (logged at `warn`; the region loads `Frozen` until a table arrives through
  `TickInput::assets`), is queued for the next step, which puts it in `frame.loaded`. The
  request is still reported in `StepOutput::requests`, so the store's driver can resolve
  the table and release the region later; its own bundle for the region, if it sends one
  in the step the server's is queued for, lends `mode` and `assets` as below. The caller
  answers only the `Load`s whose region is not in `Server::kept` read after the step that
  issued them: a bundle it sends for a kept region once the server's has loaded is
  refused as already loaded, a `LoadFailure` that fails a SOUR test's
  `failures.is_empty()`. A `Cancel` drops a queued bundle; the records stay kept. Under
  `Anchored`, a `Start::Loaded` also queues every saved pinned region for the first step, as
  `Loader::bundle(.., RegionMode::Frozen, table)` built at `build_server`: a pinned region
  is simulated whenever the server runs, so it loads at the first step, as it would have
  stepped had the world never been saved, and is never requested. Its table is required:
  being never requested, a pinned region loaded without one would wait frozen with nothing
  in the output to say so, so `build_server` fails with `NotFound` when `Cooked::table`
  has none for it and with the pin's error when `Cooked::pin` fails; a driver resuming an
  `Anchored` world therefore publishes every saved pinned region's table before
  `build_server`, or retries the build once it is published. Under `Directed`, a
  `Start::Loaded` queues every saved region for the first step, as
  `Loader::bundle(.., RegionMode::Simulated, table)` built at `build_server`, with `table`
  `None` when the region has none (so a region that cannot be bundled, or whose table
  cannot be pinned, fails the build), and a region unloaded through
  `TickInput::unloaded` comes back when a frame names it in `loaded` (next bullet). A
  bundle the server cannot build for a kept region at a request (`Loader::bundle` fails)
  is a `LoadFailure` in the output with that error, and the records stay kept; the
  lifecycle does not request the region again, so the caller recovers it with its own
  `loaded` entry for the region (next bullet), the one case in which it answers a kept
  region.
- A frame's own `loaded` entry for a region the store holds, or has queued, loads the
  stored state: the entry lends its `mode` and, when `Some`, its `assets`; its `baseline`,
  `overlay`, `resources` and `transient_resources` are dropped and logged at `info`, and
  the baseline recorded for the region is the stored one. A stored state that cannot be
  bundled (as at a request) is a `LoadFailure` at that step, the entry dropped and the
  records kept. The rule is in that direction
  because a saved region must not be lost by a caller that loads it fresh, which under
  `Directed` is the only way to ask for it back; a reset to the cooked state is "Not in
  this crate yet". `TickInput::loaded` is keyed by region, so there is no order between
  the server's bundle and the frame's and no second bundle for the sync point to refuse:
  one bundle goes in, and this rule says which.
- Binds (D142). A server built `Start::Loaded` starts with the saved world's
  binds. `build_server` reads them with `Loader::binds` over the records of every saved
  region, the ones it queues for the first step and the ones that stay in the store
  alike, and hands them to `SimulationBuilder::binds` beside the saved tick and counter,
  so the world holds the bind of every saved entity with a predicted origin before any
  region loads. A save writes an owner as its `NetId`, and a region loads to the bytes
  its export read only in a world that holds its owners' binds: without them a row whose
  owner resides in a region left in the store loads under the owner's `Net` key, where
  the server that saved holds its predicted origin, and the resumed world hashes apart
  from the one never saved until a player comes near enough for that region to load,
  which is the normal resume under `Anchored` (`crates/persist/README.md`, "Binds";
  `crates/sim/README.md`, "Saves and loads"). `load` makes the same call over the
  sections it decoded, once migrated, and drops the result: a save whose sections bind one
  prediction key to two `NetId`s fails there with `InvalidData`, before any world exists,
  and a `SavedWorld` that a save point produced takes the same path at `build_server`,
  which fails as `Loader::binds` or the builder does. Nothing else changes. A save
  point's `SavedWorld` already holds every region the world has a bind for, loaded or
  kept, so its binds need no field; the store answers a `Load` as before, and the world,
  which held the owner's bind all along, loads the row under its origin. The journal
  tooling records the same value at a checkpoint, `Loader::binds` over
  `Server::save_point`'s regions, as `lactose_sim::journal::Journal::binds`. All of this
  reads the records of every saved region, which `load` and the store hold in memory at
  milestone 1. A store that keeps unloaded regions' sections on disk cannot read them
  this way: the binds then travel beside its index, and `build_server` takes them from
  there (milestone 3; `crates/persist/README.md`, "Not in this crate yet").
- `Server::save_point` is `Simulation::save_point`, by value, then one
  `WorkerClass::Background` task per exported region running
  `RegionRecords::from_export(&export, &baseline, &schemas)` with the region's
  `Server::baseline`, joined in ascending region order, and beside them every kept
  region's `SavedRegion`, shared: a `SavedWorld` at the save's tick holding every region,
  loaded or kept. A loaded region with no recorded baseline (one created through
  `Simulation::world_mut`) is `NotFound`; a failed export or diff hands the server back
  unchanged. A `SavedWorld` is also what `load` produces and what `Start::Loaded` takes, so
  a save point can start a server with no file between.
- `Server::encode_save(executor, saved)` first runs `Container::check_sections` over the
  header and every region's records on the calling thread, since `assemble` never reads the
  records: a save the next start's `load` would refuse (a runtime `NetId` at or above the
  counter, which the world's own export never holds since the world keeps its counter above
  every runtime `NetId` it is given, D142; one entity held by two regions) fails there with
  no task spawned. Then it spawns one `WorkerClass::Background` task per region of
  `saved.regions`, each `records.encode_section()`, and once every task resolved,
  `Container::assemble(&ContainerHeader::new(saved.tick, saved.next_net_id, schemas.hash()),
  sections)` with the sections in ascending region order and this world's `Server::schemas`.
  The future fails as `persist` fails and writes nothing, so under `Headless::save` the slot
  keeps the previous save and the policy logs (`Source::Save`, `Log`). The world is not
  touched: the encode runs beside the next frame.
- `Server::schemas` is `SchemaTable::from_registry(types.registry())` built once at
  `build_server`, so every save and relevance message of the world is written with the
  table the world was built with. `Server::baseline(region)` is the baseline of the bundle
  that loaded the region, while it is loaded; it moves into the region's `SavedRegion` at
  the unload and a reload records the reloading bundle's. Milestone 3 replaces the table
  with the store's answer by hash and the signature is unchanged.
- The slot is written with `lactose_io::Io::replace(slot, bytes)` and nothing here wraps
  it: its contract (`crates/io/README.md`, "The atomic replace") is the slot's contract,
  the order of replaces per path, the `ReplaceFailed` that hands the bytes back and the
  error table included. `Headless::start` starts the one `Io` of the process with the
  binary's platform and `IoConfig::for_platform`, and creates `HeadlessConfig::saves` with
  `Io::create_dir_all` when it is `Some`, so a save never fails `NotFound` for a missing
  directory the binary meant to have; `Headless::shutdown` shuts the pool down before the
  executor, as `io` requires.
- `load(game, bytes, cooked)` runs on the calling thread before any world exists:
  `game.types()`, `SchemaTable::from_registry`, `game.migrations()`, `Container::decode`
  over the whole file (which refuses, with `InvalidData`, a counter not above a runtime
  `NetId` a section spawns and one entity held by two sections: the checks across
  sections, `crates/persist/README.md`, "The container"), then every section's baseline
  from `cooked.baseline(records.baseline_hash)`, then the check that needs them: a
  `SpawnFromBaseline` of a cooked `NetId` that another section's baseline holds as a row,
  where that section has no `Destroy` of it, is `InvalidData` naming both regions, since
  the second region's load would find the entity live. Then for each section in ascending
  region order `Loader::bundle(&records, baseline, RegionMode::Frozen, None)` to validate
  the section (the bundle is dropped), and `Loader::migrate(&records)` as the entry's
  records, so a save that cannot load fails at load and a kept region is re-saved at the
  registry's schemas without a reload between. Then `Loader::binds` over the migrated
  records of every section, its result dropped ("Binds" above; D142).
  The first error ends the load with nothing built and nothing to undo. The result's
  `tick` and `next_net_id` are the builder's; its `regions` are the store's. A save
  carries no session state, so every player joins again through `TickInput::sessions`,
  and under `Anchored` the regions they stand in come back from the store as the policy
  requests them.
- `Headless::save(slot)` is the three calls in turn from the calling thread, each blocked
  on. A failed save copy is `Source::Server`, `Stop`: the error is returned and the server
  is as it was. A failed encode or replace is `Source::Save`, `Log`: the error is returned
  and the world is unchanged; the slot holds the previous save unless `Io::replace` failed
  after its rename, as its contract states, and the bytes a `ReplaceFailed` hands back are
  dropped, since a tool retries by saving again.

**Relevance** (architecture section 9.3; `crates/net-api/README.md`)

- `Server::relevance(executor, region)` is `World::export_region(region)`, then
  `RegionRecords::from_export(&export, &baseline, &schemas)?.to_relevance(assets)` with
  the region's `Server::baseline` and `assets` the table the world holds for the region
  (`World::region_assets(region).map(|pinned| pinned.table().clone())`), or `None` if none
  has entered: the client builds the region over the assets the server simulates it with,
  never over a table the caller resolved separately. On the calling thread, between runs,
  for the one client the spike has, the executor unused. `NotFound` for a region that is
  not loaded or whose baseline was never recorded. Milestone 4 moves the diff to a
  background task and shares one message between the clients a region becomes relevant to
  in the same frame, with this signature.

**Shutdown**

- `Server::shutdown` and `Client::shutdown` drop the world, which drops `Physics` and its
  backend, which terminates the solver pool; then they wait until every solver thread has
  exited, on the count the pool's `exit_handler` raises. Dropping a world without them
  terminates the pool and does not wait: rayon's `ThreadPool::drop` returns before the
  threads are gone. Neither is called from a worker or a solver thread.
- `Headless::shutdown` is `Server::shutdown`, then `Executor::execution_log` read (D112:
  read, not taken), then `Io::shutdown`, then `Executor::shutdown`, which joins the
  workers. After a run tore the world there is no server and `Server::shutdown` is
  skipped: the torn world's solver pool is terminated when that world drops and is not
  waited for. The binaries do the same for every world they hold, then the pool, then the
  executor. A world dropped while its run is in flight is dropped when the run's jobs
  finish (`crates/sim/README.md`, "Runs"); nothing here holds a world across a shutdown.
- A shown world (facade) ends in the order `crates/platform/README.md`, "Shutdown", gives
  a client. `Shown::stop` is that order's step 2. It joins the frame in flight; awaits the
  present call in flight; queues `PresentSide::close` as the last call on the present
  thread and awaits it, so the swapchain is dropped there; drops the lane; and hands back
  the `Server` and the render side. Once it is called no present is queued and no frame is
  submitted, and this crate owes no host signal ("The targets and the present thread").
  The binary then awaits `PresentThread::stop`; waits within a bound for the last frame's
  point and drops `Stopped::render` on a thread of its own made through
  `Executor::platform`, the graph before the renderer, since those drops wait for the
  device and no worker may; calls `Server::shutdown`; drops what else holds the device,
  the last of it in a main-lane call; and shuts the executor down. `stop` is awaited by
  the task that drives the frame, or through `Executor::block_on`. With a frame in flight,
  a `stop` called after the executor was shut down resolves `Cancelled` with nothing
  handed back, since the join of that frame's render task is what the executor cancelled.
  What a shutdown does to a frame it meets under way follows from its order
  (`crates/task/src/pool.rs`, `shut_down`): it sets its flag, wakes the workers, and at
  once cancels every registered task, on the thread that called it, before it cancels the
  timers and joins the workers. A task suspended at that moment is dropped there,
  unpolled. So a timer root suspended in step 2's wait or in step 3's, in `Shown::step` or
  in `stop`, is dropped on the thread that called the shutdown, with its `Server` and
  whatever else it holds: it is never polled again, it resolves nothing, and no
  `ShownError` is made. A render task suspended at its hold is dropped there too, with the
  render side. A task that is inside a poll when the shutdown begins finishes that poll: a
  render task then resolves with a sound render side, which is dropped on a worker, by
  that task's completion or by the worker that finishes the frame's job. The present
  thread drops a task only where its reply lands between the flag and that sweep: a task
  woken after the shutdown began is cancelled, and its future dropped, by the thread that
  woke it, and such a drop waits for the device inside a present call, with
  `PresentThread::stop` waiting behind it. That window is narrow, and it is the exception.
  Only a frame that is polled after the shutdown began resolves at all: one driven through
  `Executor::block_on`, whose driver is no task of the executor's, or a root caught inside
  a poll. It learns of the shutdown from the render task's join, where that task was
  suspended and the sweep cancelled it, or from its own `Sleep`, where the task was inside
  a poll. From either it resolves `Cancelled` with `ShownKept::Nothing`, the `Server`
  dropped where the frame was polled, and the render side where its task or its job was
  dropped, as above. The order above never meets any of it, since `Shown::stop` comes
  before the executor's shutdown. With no frame in flight `stop` touches no executor: it
  awaits the present thread alone and hands both back. The binary shuts the executor down
  last.
- `Shown::stop` is the one end the facade gives a shown world, and the one path on which
  it says where the render side is dropped: nowhere, since `stop` hands it back. A `Shown`
  that is dropped, or a run future of one, drops the render side where the drop lands.
  That is the dropping thread when no frame is in flight, or when the frame's render task
  is suspended at an await, since the drop cancels the task and drops its future there. It
  is the frame-critical worker that finishes the frame's job, or the render task's poll,
  when one is running. Wherever it lands, the drop waits for the device for as long as
  submits are pending (`crates/rhi-api/README.md`, "Handles and lifetimes"), which on a
  worker is a wait with its run permit held; and `PresentSide::close` is never called, so
  the swapchain is dropped wherever the present thread's state is. No type stops a binary
  from dropping a `Shown`. What the surface does is keep `?` from doing it unasked:
  `ShownError` does not convert ("Errors").
- `build_shown`'s failure drops the two objects it was handed on the calling thread, which
  may be the root's worker, since a binary may build inside its root task. A render side
  through which nothing was submitted waits for nothing when it drops; a caller whose
  render side has submits pending builds where a drop may wait.

**Logging** (architecture sections 19 and 23)

- Everything this crate logs is at target `LOG_TARGET`, `"app"`: the crate's role name, an
  exception to section 19's codename rule like `io` and `physics` (D119, D130), since the
  assembly has no codename and `EnvFilter` matches a directive's target by prefix, so the
  engine's codename `lactose` would also select every `lactose_*` crate's default-target
  events. The policy's responses at `error` for `Stop`, `warn` for `Resync` and `Log`; the
  lifecycle's failures at `warn`; a frame's entry replaced by the store at `info`; a
  refused `push` at `debug`; a frame's `behind` at `debug`.

**The headless app** (architecture section 2)

- `Headless::start(game, config, cooked)` is `start_executor`, `Io::start`,
  `Io::create_dir_all` of `config.saves` when `Some`, then `build_server`; a failure of a
  later step shuts the earlier ones down before returning. `Headless::run(ticks, frames)`
  is `Server::resync(now)` and then the timer root above with `frames` wrapped to return
  `None` once `ticks` ticks have run, so the run ends at exactly `ticks`; the `RunReport`
  counts the ticks, the frames (the last included), the frames reported `behind`, and
  holds `Server::hash` after the last tick. What a frame produced is dropped (the gathered
  frame) or logged at `debug` (requests) and `warn` (refusals). `Headless::step(frame)` is
  one `Server::step` from the calling thread, returning its `StepOutput`. After a run tore
  the world, `server()` is `None`, `tick()` is the tick it was torn at, and `hash`, `step`,
  `run` and `save` fail with `Cancelled`.
- A `Headless` is built on the thread that owns it and every method is called from that
  thread; none is called from a worker (`block_on` on a worker is a debug assertion).

## The crash hook

`install_crash_hook(binary, dir)` installs the process's panic hook (architecture section
5.1, D155). Every engine binary calls it first in `main`, before its subscriber and before
anything that can panic; RICOTTA keeps a hook of its own, since a tool does not link the
assembly. It is safe code over `std` alone: no signal handler, no minidump, no second
process. On a panic, on any thread, the hook:

1. takes a lock it never releases, so that a panic on another thread waits for the abort
   and the report is the process's first panic's;
2. formats the report once, as text: `<binary> crashed`, `thread: <name>` (`<unnamed>` for
   a thread without one), the panic's location and message as `std` prints them,
   `backtrace:`, and a backtrace captured with `Backtrace::force_capture`, whatever
   `RUST_BACKTRACE` says;
3. writes it to stderr and flushes, before any file system call, so that a file system
   that hangs or fails cannot cost it;
4. when `dir` is `Some`, creates the directory with its parents and writes the same text to
   a new file in it, `<binary>-<YYYYMMDD>T<HHMMSS>.<mmm>Z-<process id>.txt`, in UTC, so
   that two crashes in one directory never share a name and the files sort by time; the
   file is opened with `create_new`, so no report replaces another;
5. writes one line to stderr: `<binary>: the crash report is in <path>`, or `<binary>: no
   crash report file at <path>: <error>` naming the directory or the file that failed;
6. calls `std::process::abort`, in every profile.

A directory that cannot be created or written, or a file that cannot be written, costs the
file alone: the failure is that one line, never a second panic, and the abort follows
(`docs/verification.md`, the writable disk). `None` writes stderr alone. The directory is
the binary's to choose (`crates/app-headless/README.md` gives the headless binary's), and
the hook does not resolve it: a relative path is taken against the working directory at the
panic. `binary` names the file, so it holds no path separator. A hardware fault, a signal
and a stack overflow never reach a panic hook; they are read from the operating system's
own report (D155). Tests never install the hook (D16), so `#[should_panic]` and a property
test's shrinking keep unwinding; a test that expects an abort runs a binary in a subprocess,
as `crates/app-headless/tests/crash.rs` does.

## The saved-type record

`tests/schema_record/engine.txt` records every type the engine's registration set saves
(`Game::types()` of a game with no crate, which every game's set contains): its key, schema
version and layout digest as `SchemaTable::schemas()` gives them, its role and persistence
policy, its field IDs with their names and type tags (a field holding a `#[value]` struct
written `struct Name`, an array of them `[struct Name; N]`, so the record says which fields
hold IDs of their own), nested `#[value]` structs and arrays of them included, and its
retired IDs. `derived` types are not saved and are not in it. The headless binary's set
adds one milestone 1 stand-in type and has no record; a game binary's set gets its own file
beside this one when it ships (milestone 5). `tests/schema_record.rs` derives the record
from the build and fails, naming the type, on a type whose digest moved while its version
did not (raise the version and register the step; a transient type needs the step only once
it is made persistent) and on a version that fell, transient types included: no load reads
a transient type's version, but one made persistent later is read from the version and
digest it was saved under, so each of its layouts needs a version of its own
(`crates/persist/README.md`, "Migrations"). It fails too on a reused field ID: a recorded
ID that now names a field of another name, a recorded ID that left the fields without being
retired, and a retired ID no longer retired, at every depth (a nested struct's IDs are
compared while the field holding it holds a struct of the same name or the type's digest is
unchanged). A field whose type changes under its ID and name is a layout change, which the
digest and version govern, except a `#[value]` struct or enum replaced by one of the same
layout, which no digest sees (D129). Any other difference (a new type, a removed one, a
version that rose with any digest, a new field) fails until the record is rewritten in the
commit that makes the change, with
`LACTOSE_SCHEMA_RECORD=update cargo nextest run -p lactose-app --test integration schema_record::`.
The command refuses to write while any of the failures above stands, and when what it would
write would not read back as the build's record (a key or type tag a line cannot hold); a
missing record is written whole, which is how a set's first record is made, so the record's
diff is what review reads. A rename keeps its ID and costs no bump, but the record cannot
tell it from an ID given to a new field of the same type; `LACTOSE_SCHEMA_RECORD=rename` in
place of `update` takes a new name under a recorded ID whose type is unchanged as a rename.
The record is compared with LF line endings, so a checkout that gives it CRLF checks the
same. No engine crate has a `register_migrations` yet and `Game::migrations` folds only the
game's crates, so the first step an engine type needs adds that registration point beside
`Game::types`' engine list.

## Not in this crate yet

- A reset of a kept region to its cooked state (`Server::forget(region)`), and a reload of
  a kept region under `Directed` by name rather than by a frame's `loaded` entry: neither
  is needed by the headless app or the spike. A reset must also drop the binds of the
  entities it forgets, which the world keeps as it keeps an unloaded entity's, and the
  ECS has no call for that yet; the same is owed to a region a frame replaces under
  `Directed` with a state that lacks one of its entities (D142,
  `docs/verification.md`).
- The journal's writer, reader and replay driver (`crates/sim/README.md`, "Journal
  replay"). What the writer takes from a server at a checkpoint is already here:
  `Server::save_point` for the regions and, through `Loader::binds` over them, the binds
  (D142).
- The reason of a resync in `ClientOutput` (clock or content), so the caller can send
  `ResyncReason::AssetFailure` for a content resync: the spike sends `BehindCatchUp` for
  every resync at milestone 1.
- The replicated world (`WorldKind::Client`) beside the prediction world: built by
  `build_client` once `lactose-net` provides its `StagedState` hook (milestone 4);
  `Client::push` then pushes into both inboxes and `Client::frame` steps both, the
  replicated world first. `build_client` loses its `entity` parameter when
  `JoinAccepted::entity` lands (proposed with D122).
- `Cooked` is replaced at milestone 3 by `lactose-asset`'s traits: `Cooked::baseline`
  becomes `lactose_asset::RegionBaseline::lookup(catalog, content_hash)` after a
  `request_newest` of `RegionBaseline::id(content_hash)`, `Cooked::table`
  becomes `AssetCatalog::resolve` of the region's set, `Cooked::pin` becomes
  `PinnedTable::pin` over the published store. `load`, `build_server`, `build_client`,
  `Headless::start` and the fold change signature with it, listed then.
- The overload ladder (architecture section 9.7): `FrameOutput::behind` is its input;
  turning `RegionPolicy::activation_radius`, decimation and join refusal are milestone 4's.
- The input task of architecture section 9.2 (the dedicated network thread, the jitter
  buffer, substitution with `PlayerInput::substitute`): `net`'s at milestone 4; the spike
  writes its own and feeds the result through the frame callback.
- `Server::relevance` on a background task and shared between clients (the signature is
  final); `ServerError` and `ClientError` carrying a `Source` of their own.
- A load against a re-cooked baseline: `load` asks `Cooked::baseline` only by the hash a
  section saved (`records.baseline_hash`), so a save made against a baseline since
  re-cooked fails `NotFound` there. Carrying it over is `Loader::migrate`, then
  `RegionRecords::rebase` onto the region's current baseline, then `Loader::bundle`, the
  sequence `lactose-persist`'s `tests/migrate.rs` runs for a type the baseline carries;
  `load` runs it once the store can name a region's current baseline (milestone 3,
  `docs/verification.md`, "A region's current baseline (D136)"), with the re-homing pass
  for what the rebase drops. The same store lets `load` check a cooked entity a section
  spawns from the baseline against its home region's baseline when the file has no section
  of that region; today that region's first fresh load finds the entity live.
- The slot listing and the load's reads through `Io::read_dir` and `Io::open`: `load`
  takes the bytes, which the binary reads with `Io::read_file` or `std::fs` at milestone 1.
- The bodies of the shown frame, `lactose-app-client`, criterion 3's tests and the mover:
  lane `frame-impl` ("Proof for the shown frame").
- A frame between ticks with a display's refresh as its deadline. `Shown::frame` carries
  such a frame as it is ("Pacing"); the deadline is what is missing, and
  `lactose-platform` reports no display's refresh. Drawing as soon as a render task
  resolves does not wait for it: step 2 does that within the tick's own period ("The shown
  frame"). What the refresh deadline would add is a frame whose `now` is its own, so that
  a picture drawn late in a period carries that in its fraction, and pictures faster than
  ticks. The keeper's wait asked once less a margin, and the measurement of the spin's
  power, are re-gated to the change that adds it (`docs/verification.md`).
- A shedding rule at the swapchain's floor, and any rule where the device reports no
  display time. The rule sheds at a constant count, three, which on the M1 leaves one
  image waiting behind another, a refresh above the floor, where display times are
  reported ("Pacing", *The shedding rule*). The last refresh needs a depth read against
  the device's own floor, which `frame-pace` measures and the frame does not, or the time
  each image became ready, which nothing reports. A device that reports nothing leaves the
  drop alone, with the residue "Pacing" states. A present mode other than the surface's
  default, and an image count the caller chooses, are `lactose-rhi-api`'s to add if
  `frame-pace` asks (`crates/rhi-api/README.md`, "Not in this crate yet").
- A record on every frame-critical worker. The frame's job leaves one out ("The shown
  frame", *One worker stays out of the frame's job*). A width the binary chooses, or
  timers fired from inside a long job, would give it back; neither is asked of
  `lactose-task` until `frame-pace` shows a record that needs the worker.
- A pass run alone. A frame that waits defers every pass, the replication gather among
  them. A local world's gathered frame goes nowhere; a listen server that is shown needs
  its gather every frame whatever the device does, and `lactose-sim` runs the passes as one
  schedule. The change that shows a server with clients asks for the split.
- A save point of a world while it is shown: `Server::save_point` takes the server by
  value and `Shown` lends it by reference. No milestone 2 row saves a shown world; the
  change that needs it adds the run to `Shown`, which joins the frame in flight first.
- A shown client's two worlds (milestone 4): the prediction world and the replicated world
  through one render side, each with a sink and a snapshot channel of its own, in the frame
  `crates/present-api/README.md` writes for them. `Shown` is one `Server` world;
  `FrameCounts` is by kind already.
- Recovery from a lost device (architecture section 19): a new device, both objects made
  again by the binary, and a full pass. `Stop` with the world kept is where it starts.
- A second look at the present call, where the present role has a queue of its own: what
  `frame-pace` asks for if it shows frames dropped on a quiet machine, on a display at
  least as fast as the frame ("The targets and the present thread"). The smaller form is
  `reviewer-gpu`'s, from the review of D162's first draft: a look at the frame's start, by
  the task that owns the protocol's state. If the call has returned and the last frame
  was dropped, that frame's target is queued then, the frame draws into the other target
  and queues nothing at its own submit, so one call is still queued a frame and nothing
  is shared. A present queued the moment the last call returns is the larger form, and
  needs a target's state shared between two tasks, which nothing here has.
- A submit thread, where the present role shares the graphics queue: a thread outside the
  frame-critical workers that makes the frame's submit and waits for the queue's lock
  itself, so that a frame is held for a present's driver call alone. It asks for a thread
  architecture section 5.1 does not list and a lane's locks that section 3 does not permit
  a frame, so it is a plan change. No measurement in reach asks for it: with held frames
  dropped and busy frames deferred, the hold did as well as the thread or better in
  `reviewer-gpu`'s runs, by the latch's age at the return of its present call
  ("Pacing"). What would reopen it is `frame-pace` from a
  shared-queue device with a display: a hold much longer than the part of its present call
  under the queue's lock.
- An occluded window's frames hidden by the binary: `lactose-platform` reports no
  occlusion. Until it does, a window whose presents stall costs a shared queue its
  pictures, for a call's bound each, and its world's pace only where a call outlasts the
  bound on deferring: the world then ran at about two thirds of real time through calls of
  three seconds in the reviewers' models, the debt kept and run back afterwards
  ("Pacing").
- The overload ladder's render half: `ShownStats` and `FrameOutput::behind` are its
  inputs, and nothing acts on either.
- Everything else a client binary adds: milestone 4.
- An edge against the replication gather: `net::gather` is in no `SystemSet`, so a game's
  exclusive `Pass::Gather` system cannot be ordered against it with `Registration::order`
  until `net-api` names a set; no game crate has one at milestone 1.

## Proof for the shown frame

The shown frame is a facade: of it, only the shape of the frame's job runs, over a
stand-in for `Executor::run_job`. What is held now, and by what:

| What | How |
|---|---|
| The surface names no backend | `lactose-app`'s normal dependencies gain `lactose-present-api` and `lactose-rhi-api`, both foundation crates; `cargo xtask tiers` and `cargo xtask boundary` hold `[server_denylist]` over its normal and build closure. `lactose-render-graph`, `lactose-renderer` and `lactose-input-host` are dev-dependencies, for `tests/shown.rs` alone |
| The types the contract promises | `lib.rs`, at compile time: `Shown`, every run future, `ShownOutput`, `ShownError` and `Stopped` are `Send`; `Box<dyn RenderSide>` is `Send + 'static`, which is what a task and a job's closure ask of what is moved into them, and `Sync` is not asked (section 6 implements the trait for a type that holds a `Cell`); `Box<dyn PresentSide>` is `Send + 'static`, the present thread's state; `ShownError` does not convert into `lactose_core::Error`, held by an assertion that fails to build if it does; every value that crosses a trait is `Copy`; `TARGETS` is two or more and `TargetIndex` admits exactly the indices below it; `RECORD_RESERVATION` is one or more; `JOIN_MARGIN` is above zero and below `TICK_PERIOD`; `SHED_DEPTH` is two or more |
| `RenderSide` can be written over the graph and the renderer as merged | `tests/shown.rs`, section 1: `present_queue` from the device's capabilities; `open` with the empty submit after torn frames; `sink`; `present`; `draw` (the target made or replaced, the remote batch, `prepare`, the import from `Undefined` with `ready`, compile, record, and `torn` on each failing path); `submit` (the latch, the submit, `submitted`, the next `open`); and the drop in order. Compile-only |
| `PresentSide` can be written over `lactose-rhi-api`'s swapchain | Section 2: the resize behind its wait, a slot written behind its wait whenever the image is not the one it holds, the acquire, an encoder that a call which submits nothing drops, the present with its wait and its signal, `Stale` as a submit that was taken, `close` behind its wait with a slot left where the clear is refused. Compile-only |
| The binary's wiring and its shutdown type-check | Section 3: `build_shown`; the root's loop around `Shown::frame` with a window's extent; every arm of `ShownKept`, in one function that a failed frame and a failed step both end through, with no `?` on a `ShownError`; `Shown::stop` through to the render side's drop on a thread of the platform's. Section 4: the fixed-frame mode's driver around `Shown::step`, and the render side taken back as its own type. Compile-only |
| `frame-impl`'s tests can be written | Section 5: a render side and a present side with no device, which count their calls, the render side answering either queue path; a shown world and the same world unshown, stepped alike with the hashes compared; the protocol's choice of target. Compile-only |
| The frame's job needs no lock, no world and no `Sync` | Section 6, `the_frame_job_is_one_job_with_the_render_side_moved_in_and_out`, which runs on an executor of two frame workers under chaos, every range split, steals shuffled and yields injected. A render side that is `Send` and not `Sync` is moved into a closure; the closure runs as one job, through a stand-in of `Executor::run_job`'s signature; `RenderSide::open` is called with the job's context and forks through it, disjoint `&mut` chunks in one scope and a range in a second, each unit exactly once; the object and the answer come back in the closure's value. It does so from a task spawned on a frame-critical worker, which then calls `submit` from its own poll, and again through `block_on`; neither job runs on the thread that drives the test, which is all the test says of where a job runs, since a job a task starts may run on that task's worker once its poll has returned; each of the test's two waits is bounded, at ten seconds, with a message, by a thread of the test's own and not by the executor's timers, which no worker fires while every frame worker is stuck in a job; and the render side leaves by value, as its own type. The stand-in is a graph of one node whose closure sits in a mutex, which is the lock the primitive removes: `frame-impl` deletes it and calls `Executor::run_job` |

What `frame-impl` proves, beside criterion 3's three tests ("Determinism of a shown
world"): each state's order of calls, with a render side that logs them; the wait of step
2, on an executor of two frame workers or more, with a render side whose `draw` is held
and a `now` chosen so that the frame's limit is ahead or already past: a task that
resolves before the frame's limit is taken back in that frame, which runs its passes and
reports it in `joined`; a task still in flight at the limit leaves the frame busy, its
ticks run, no pass, no publication and no method of the render side called, `deferred`
`Rendered::Busy`, and a later frame reports the task in `joined` and runs its passes; a
frame that is behind, or whose limit has passed, looks once and resolves busy at once with
the render task still held (that it registers no timer is `lactose-task`'s, by
`Sleep::poll`, which answers a passed deadline before it registers one, and the executor
exposes no count of timers to assert on); on an executor of one frame worker, every frame
looking so, whatever its limit; the bound on deferring, at `log_retention − CATCH_UP_CAP`
ticks, for a region and for the frame before, with the starved frames after the one and
the await with no limit after the other; the four cases of a new sink, and no second reset
after a full pass that left rows out; a starved frame whose `open` fails, hidden with its
passes run and counted torn and hidden; the protocol on a queue of its own, with a present
side that holds its call (no target drawn while a call names it, `ready` the last `read`,
`queued: false` while a call is in flight, nothing kept of a call that reported no read);
the protocol on a shared queue, with the same present side (no `open` and no `submit`
while a call is in flight; a frame that must ask `open` waiting for the call until its
limit, asking when it returns before and busy when it does not; `held` where the call had
not returned when the frame's job did, that frame submitted once it returns, not queued,
and its target drawn again by the next frame; `queued` in every frame that drew and was
neither held nor shed; `stop` with a render task held; a render task held at its await
while `block_in_place` holds the gate at `RECORD_RESERVATION`, beside a job whose units
still run inside the block); the shedding rule on each layout, with a present side that
reports what it is told: no frame shed while `waiting` is `None` or below `SHED_DEPTH`;
the frame after a report that rose to `SHED_DEPTH` submitted and not queued, counted in
`shed` and not in `held`, its target drawn again by the next frame; no second frame shed
while the count stays at the depth; the `SHED_DEPTH` reports after a shed passed over, so
that counts among them that read below the depth and then at it shed nothing, a report
with no count and a call that failed not being among them; and one shed again for a report
after them that reads below and a later one that rises; no frame shed on the report of a
call that a frame was dropped for, or on the first report after a call that failed; the
`SHED_DEPTH` reports after a dropped frame passed over as after a shed, the report of the
call it was dropped for not among them, where that report is taken after the drop on a
queue of its own and by the hold on a shared one, so that counts that read below the depth
for those reports and then at it shed nothing; the same after a frame that drew nothing,
on each layout: a frame that waited for a region, a hidden frame and a torn one, each once
with no call's return still to be taken, where the `SHED_DEPTH` reports passed over are
the next ones, and once with a call still out, where they are the ones after that call's
report, which sheds nothing whatever it reads; a frame busy at step 2, where they begin
with the report of the call the late task queued, or follow the report of the call that
task's own frame was dropped for; on a shared queue, a frame busy at step 3, where they
follow the report of the call it waited behind; in each of these, and after a dropped
frame, the report before the gap reading the depth, the `SHED_DEPTH` reports after it
below the depth and the next at it, with no frame shed, which an order one report early or
one late does not pass; a run of hidden frames passing over the `SHED_DEPTH` reports after
its last and no more; a rise that step 3's wait took in a frame which then waited for a
region, with no later frame shed for it; a held frame that is not counted shed; every wait
of these tests bounded, at ten seconds, with a message that names what never arrived, the
bound kept by the driving thread's own clock or a thread of the test's and never by the
executor's timers, since the failures the bounds are for leave no worker to fire one; in
`lactose-app-client`, the present side's count over scripted display-time reports and
timeline values, with no device: a present counted from when its `read` is reached until
its id or a later one is reported, one that is never reported covered by a later id, and
`None` where the swapchain reports none; the policy's two rows; `stop` with a frame and a
call in flight, with none after a shutdown, and after a lost device; a paced frame driven
through `Executor::block_on` and suspended in its wait while a clone of the executor is
shut down, `Cancelled` with nothing kept, once with the render task suspended, which the
join reports, and once with it inside a poll, which the `Sleep` reports; it is driven
through `block_on` because a root task met in its wait is cancelled unpolled and resolves
nothing, which the same test shows of a root whose future is dropped on the thread that
shut the executor down; the fraction at a deadline, a period later and while paused, and
unchanged by a wait in step 2; on `lactose-rhi-null`, that a run through each state is
refused nothing on each queue layout, the extent going E1, E2, E1 with the frame at E2
dropped or torn among them, and held frames dropped among them on a shared queue; and
section 6 against `Executor::run_job`, with the stand-in deleted, on an executor of two
frame workers, where the width is one and the job's own worker runs every child it forks,
and of three, where one other worker shares them. Once a Vulkan backend can run it, a run
on one queue under the Khronos layer's synchronisation validation, with held frames in it
("Two rules of the layers below").

## Proof for D142

Each row is a test of `tests/proof/`, by its name, and the sentence it holds.

| Test | What it holds | File |
|---|---|---|
| `a_resume_that_keeps_a_predicted_owners_region_matches_the_world_never_saved` | Under `Anchored`, with a pinned region A and a region B that no player is near: a bound entity with a predicted origin in B owns a row of A, and B has unloaded into the store. `save_point`, `encode_save`, `load`, `Start::Loaded`. The resumed server holds the owner's bind before its first step, loads A at the first step with the row's `OwnedBy` the predicted origin, and equals the server that was never saved in hash and counter at each of K ticks, B staying in the store of both; a `Join` near B then loads it from the store in both and they stay equal. The same through a `SavedWorld` with no file. With the builder's binds taken out the resumed server differs at its first tick, which is the failure the test names | `store.rs` |
| `the_binds_of_a_save_point_are_the_stores_and_the_worlds` | `Loader::binds` over a save point's regions holds the bind of an entity in a loaded region and of one in a kept region, and nothing else; after the step that despawns one, neither the world nor the next save point holds its bind | `store.rs` |
| `a_save_whose_sections_bind_one_origin_twice_fails_at_load` | A splice of two sections that each spawn an entity with one predicted origin under two `NetId`s: `load` is `InvalidData` naming the key, both `NetId`s and both regions, with nothing built; `build_server` given the same regions as a `SavedWorld` fails the same way | `cross_section.rs` |
| `a_direct_runtime_spawn_above_the_counter_saves_and_loads` | Replaces `a_direct_runtime_spawn_above_the_counter_fails_the_encode_and_the_last_save_stays`, which pins the behaviour D142 ends: the save encodes, its header's counter is above the spawned `NetId`, and it loads to a server whose next allocation is not that `NetId` | `cross_section.rs` |

## Proof for milestone 1

Every row below runs. `tests/proof/` is one test binary with a module per area (the
registration, the server's frame and time scale with the refusals and relevance, the store
and its baselines, the client's frame); `tests/headless.rs` holds the headless app's rows,
the policy table and `a_recorded_headless_run`, the SOUR test of the assembly, which the
runner records under every mode since `lactose-app` is a `[harness]` package; the
`Server::shutdown` row is a unit test in `src/server.rs`. Every pacer runs over a stand-in
clock: instants computed from one base and never read between frames, so a test's tick
counts do not depend on the machine's load. `tests/spike.rs` compiles the spike's loop
against the surface. `lib.rs` pins at compile time that
`CrateRegistration` and `Game` are `Copy + Send + Sync + 'static`, that every
configuration, saved world, world, run future, output and save future is `Send`, and that
`dyn Cooked` is `Send + Sync`.

What the tests prove, each asserting on counts, orders, bytes and hashes and never on
timing:

| What | How |
|---|---|
| The registration order | A game of three sketch crates whose functions record their calls: types, setups and systems in the stated order, each kind alike; a failing `types` names its crate; a frame's sample reaches the player's entity (`lactose_input::player_entity`), so the input applier is installed |
| A headless run is a function of its frames | `Headless::step` for N ticks and `Headless::run` for N ticks with the same frames hash equal; two executors with different worker counts hash equal; a SOUR run under workers, layout and chaos modes, of the sketch with player 1 as a dynamic body its input steers, which fails unless the last sample was applied and the body moved; `run` on a replay executor is `Unsupported` before any tick |
| The frame | A frame callback that records the ticks it was asked for sees consecutive ticks; `None` stops the frame at its tick and the passes still run; `behind` is reported when more than the cap is owed; a poll at the build instant owes one tick and a run after a gap starts with none owed |
| Time scale, server | With the real `TickPacer` over a stand-in clock, at 60 Hz frames, at 30 Hz frames and at a 2x scale (frames owing two ticks): after `set_time_scale(PAUSED)` the next frame runs exactly one tick whose `TickInput` carries `Some(PAUSED)`, the mirror reads `PAUSED`, `time_scale()` reads `PAUSED` after that step and not before, and the following frames run no tick; a callback's own `Some(PAUSED)` in the first of two due ticks ends the frame after that tick; after `set_time_scale(REAL_TIME)` `time_scale()` reads `REAL_TIME` at once, the next frame runs ticks and the first carries `Some(REAL_TIME)`; on a paused server, `set_time_scale(REAL_TIME)` then `set_time_scale(PAUSED)` in one gap runs no tick and leaves `time_scale()` and the mirror at `PAUSED`, and `PAUSED` then `REAL_TIME` in one gap on a running server runs ticks with the mirror at `REAL_TIME`; between runs with no scale pending `time_scale()` equals the mirror; the same through `step`; a pause carried by `step` on a new server, or in a frame that is behind, leaves the next frame no tick, and a pause pending on a frame owing nothing is carried by one tick; a repeated scale is dropped; over 50 pause cycles the schedule drifts by at most one tick per pause |
| Time scale, client | A due frame carrying `Some(PAUSED)` pauses the pacer at the fold and the frame runs no tick and pops nothing; a later frame carrying `Some(REAL_TIME)` makes ticks due at its fold, the first step carries `Some(REAL_TIME)` and the hook pops both frames; a new client paced at rate zero, or whose first due frame carries a pause, runs no tick |
| The store | A region unloaded under `Directed` is in `kept` after the step and not in `baselines`; a frame that loads it fresh loads the stored state with the frame's mode and table, `info` logged; a `save_point` after the unload holds it; under `Anchored`, a saved world of N regions with one pinned, started `Loaded`, loads the pinned region at the first step, reports no refusal, and after a `Join` into a saved region the lifecycle's `Load` is answered from the store at the next step and the region leaves `kept`; a world resumed that way matches the one that was never saved, counter and hash, at every tick; such a resume fails at `build_server` with `NotFound` when the pinned region has no table and with the pin's error when its pin fails, and under `Directed` a pin failure at the build fails it while a missing table loads the region with none; a save after K ticks holds every one of the N regions; join then leave: a saved world of 25 regions started `Loaded` under `Anchored`, a `Join` whose `Load`s the store answers, and the player's `Leave` in the step that loads the 25 bundles, so every bundle is accepted and its region unloaded again in one step: all 25 are in `kept` after that step, none in the baselines, and a save then holds all 25; under `Directed`, a region unloaded and reloaded in one frame whose reload bundle is refused is in `kept` with its export's records and not in the baselines, and one whose reload is accepted is loaded, not kept, with the reloading bundle's baseline; an unload whose diff fails (a region created through `Simulation::world_mut`, no baseline) fails the step with `NotFound` and `server: None`; a kept region whose table cannot be pinned at a store answer is no refusal: it loads `Frozen` with no table and takes one through `TickInput::assets`; a region is never both loaded and kept |
| Baselines | A region loaded, unloaded and saved is diffed against the bundle it was loaded with, also when its reload in the same frame is refused; a refused bundle records nothing; a reload records the reloading bundle's baseline |
| Save and load | `save_point`, `encode_save`, `Io::replace` into a temporary directory, `load`, a `Start::Loaded` headless app stepped K ticks against the original stepped K more: equal hashes; the same through a `SavedWorld` with no file; a `Start::Loaded` with a corrupt section fails at `load`; a failed replace leaves the previous file; `tests/proof/cross_section.rs`: a direct runtime spawn above the counter fails `encode_save` with nothing written and the last save still loads, and a splice of a section spawning a cooked entity from the baseline with a home section that leaves the row unchanged (frozen) or changes it (simulated) is refused by `load` naming both regions, while each play's own save loads |
| Refusals | A bundle for a loaded region is a `LoadFailure` in `StepOutput::failures` and in `FrameOutput::failures`; a session event the applier refuses, or a `Join` for a region the atlas lacks under `Anchored`, and an asset failure the frame carries appear the same way; `is_empty` otherwise |
| The client's frame | Over the proof's game: a full frame folded loads exactly its regions; a kept failure keeps the client and the next frame resyncs on entry and reports it; `clock` with `snap` resyncs at the next frame; a fold whose `Cooked` fails resyncs at the world's own tick with the pacer's deadline unchanged, and a failing frame that stays queued through a 120-frame pause resyncs once, not per frame, while a newer failing frame resyncs again; `build_client` under `Anchored` is `InvalidData`; after a backward resync the re-predicted ticks step with the history's samples and the sampler is not called; a stale estimate with `Behind` resyncs to at least `server + lead`, and `TooOld` to at least its `frame + lead`; an estimate taken, then a replicated pause of 120 frames and an unpause, then a resync with no hint lands at the estimate plus the ticks the pacer granted plus the lead, not the pause's length further; the history holds the last eight samples |
| The fold's mode | A server and a client in one process, the server sent each tick the claimed entity's exact rows and the samples the client predicted with: after the join's full frame is folded the region is `Simulated`, the claimed counter stands ahead of the server's by exactly the ticks the client leads by, and the only correction in 60 frames is the first frame of the fold's tick restoring that tick (the full frame carries the join's rows, which the fold places `lead` ticks later); the same over 240 frames with the player a dynamic body falling onto a fixed slab, which rests at the server's height; a relevance message with no table folds the region `Frozen`, and the frame that brings the table as `RegionChange::Assets` makes it `Simulated`; a full frame after the table, or an `Irrelevant` between the table and a `Relevant` with none, in one fold, leaves the region `Frozen` with no table; today's cost, pinned by a test named for it: a region the server holds `Frozen` with its table published folds `Simulated`, the client's counter runs ahead of the server's and at least 55 of 60 frames correct it, each at its own tick, with no resync |
| The resync hint | A client that misses 12, and 25, frame periods while their frames queue, then takes them all: exactly one resync, to at or beyond the newest queued frame plus the lead, and the world at or beyond each later frame's tick plus the lead for twenty frames after |
| A refused bundle on the client | A frame naming the loaded region `Relevant` again, older than the world and with a row the prediction did not make: the step refuses the bundle and the rollback re-applies it, one `warn` is logged and the frame reports a content resync at the world's own tick; the frame runs half a period late and the next, on the period, still owes its tick (the content resync left the pacer); five later frames each roll back and log nothing; a refusal in a frame whose fold failed for another region is not resynced twice, and a later frame's rollback from the restart re-applies it without a second `warn`; two loaded regions refused at one tick log two `warn`s; a refusal kept from a frame whose fold failed, a clock resync back before its tick and the same region refused at the same tick again log a second `warn` and resync, and so do a refusal reported in a frame older than a ring of depth two, the reason's resync back before its tick and the same region refused at the same tick again; a refusal only a rollback makes (a correction makes live, before the tick a region's bundle loaded at, an entity that bundle's overlay holds) logs one `warn` and resyncs the content; an asset failure stepped into the prediction world by hand resyncs the content once (a unit test in `src/client.rs`: the fold sets no `TickInput::failures`, so no frame reaches this path today) |
| Relevance | The message's `assets` is the world's table for the region, and `None` before one entered |
| The policy | Each source's response, and that a `Stop` reaches the `Result` of the run that failed |
| Shutdown | A chaos run's log is read after the world is dropped and before the executor is shut down; after `Server::shutdown` the solver pool's exit count equals its thread count, with the probe's slow `exit_handler` |
| The solver pool's threads | Every thread of a three-thread pool ran the `start_handler` before its broadcast job (a unit test in `src/runtime.rs`); the floating-point half cannot be observed from a safe test, since no safe code leaves the deterministic state and a new thread starts in it here, so removing `enter` from the handler passes. `lactose-physics-api`'s step asserts the deterministic state, in debug builds, before and after the backend's step: the executor worker the step ran on, not a solver thread; `lactose-physics-rapier` asserts it inside its pool's `install`, on the solver thread that steps |
