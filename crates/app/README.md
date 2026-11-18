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
`Source::Render` and `ErrorPolicy::MILESTONE_2` are D162's, which adds to D131's surface
and changes no signature D131 fixed. Every signature of theirs is final and every body
that decides anything is `todo!()`, which lane `frame-impl` writes. The blocks from "The
shown frame" to "What D162 asks of other crates" are the contract `frame-impl` meets, and
"Proof for the shown frame" says what is held now. Architecture sections 3, 5.1, 5.3, 8.1,
8.3, 11, 16 and 19 and the milestone 2 plan's decisions 5.1, 5.5, 5.6, 5.11, 5.16 and 5.18
are their design. They build on `crates/present-api/README.md` and
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
| `HeadlessConfig`, `Headless`, `RunReport` | The headless app: the executor, the IO pool and one server world, stepped from a test or run at tick rate by a tool, saved to a slot, shut down in order |
| `WorldConfig::shown`, `RECORD_RESERVATION` (facade) | The configuration of a world that is shown, and the physics reservation a frame's record is left with while the step blocks: one, until `record-stall` reports |
| `build_shown`, `Shown` (facade) | A shown world between runs: a `Server` built by `build_server`'s path with extract's outputs installed, and the render side driven beside it; `step` for one tick and its frame, joined; `frame` for one paced frame; `flush` to join the frame in flight; `stop` to end the render side and hand both back; `server` and `server_mut` for what a `Server` offers by reference; `stats` |
| `RenderSide`, `PresentSide`, `PresentLane` (facade) | The two traits the assembly drives the render side through, which name no backend (decision 5.11): the object the binary builds over the device, the graph, the renderer and the presentation schedule; the state of the present thread, over the swapchain; and the lane to it |
| `TARGETS`, `TargetIndex`, `Region`, `FrameTime`, `Draw`, `KindCounts`, `FrameCounts`, `Submitted`, `Finished`, `PresentReport` (facade) | What crosses those traits, all plain data: the two offscreen targets by index; whether a region is open, or the point that frees one; the frame's time and fraction; what a frame draws into; a drawn frame's counts by world kind; what a submit left; what one present call reads, and what it did |
| `ShownStep`, `ShownFrame`, `ShownFlush`, `ShownStop`, `Rendered`, `ShownOutput`, `ShownStats`, `Stopped`, `ShownError`, `ShownKept` (facade) | The shown world's runs in flight; what a frame did on the render side (drew, hid, waited, was torn); what a paced frame produced; the counters since the build; what `stop` hands back; and how a run fails, with what survived it |
| `Source::Render`, `ErrorPolicy::MILESTONE_2` (facade) | The render side as a source of errors, and the table a shown world is built with: a lost device stops the run, and any other error tears the frame or skips the present |

Hidden plumbing, which the implementation may reshape: the solver pool's construction and
the exit count a world waits on (`runtime::solver_pool`, `runtime::SolverExit`); the
pending time scale, the scale the last step carried, the bundles queued for the next step
and the migrations inside `Server`; the clock estimate with the ticks granted since it was
taken, the lead, the pending resync, the newest frame tick whose fold failed and the tick
the last frame's steps started from inside `Client`; the frame in flight or the idle render
side, the frame's own world and its schedule, the present call in flight, each target's
last read and the flags of the frame's state inside `Shown` (`shown::Flight`,
`shown::Side`); the bit inside `ErrorPolicy` that says which milestone's table it is.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| `app-headless` (milestone 1) | Start an executor, an IO pool and a server world from the game's registrations, run N ticks at tick rate, report the hash, save, shut down in order; `main` returns the first fatal error | `Headless::{start, run, hash, save, shutdown}`, `RunReport`; the policy's `Stop` is `main`'s `Err` |
| | The same from a save, losing no saved region | `load` before any world exists, `Start::Loaded`, the store ("Saves and loads" below) |
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
| | End with frames in flight, and after a lost device, in the order `lactose-platform` gives | `Shown::stop`, `Stopped`, `ShownError::kept`; "Shutdown" |
| The fixed-frame mode of decision 5.1 | One tick and one frame at a time, with a time, an extent and a sample of its own; every frame drawn; the last target's bytes and the frame's values to dump | `Shown::step` with a `FrameTime` and an extent; `Rendered::Drew::point` to wait for; `Stopped::render` taken back by `Any`; `tests/shown.rs`, section 4 |
| `frame-impl`'s tests of criterion 3 | A render side with no device that logs and blocks where the test says; a shown world and the same world unshown, stepped alike; the hash after each tick | `RenderSide` and `PresentSide` implemented by a test; `Shown::step`, `Shown::server`, `Server::step`; "Determinism of a shown world"; `tests/shown.rs`, section 5 |
| `lactose-renderer`, `lactose-render-graph` and `lactose-presentation`, behind the binary's object | A job's context for the record and for the presentation schedule's forks; the sink installed and replaced at the right frames; one extent a frame; the target's last read as a point; no pass of extract between `prepare` and the next open region; the drop off the workers, the graph first | `RenderSide::{open, present, draw}` take `&mut JobContext`; "The frame's three states", "Counts, torn frames and resets"; `Draw`; "The shown frame"; `Stopped::render` |
| `lactose-platform`'s present thread, and the `swapchain` lane | The state its thread owns, declared by the frame; one call in flight; a last call that drops the swapchain; no host signal owed | `PresentSide` as the `S` of `present_thread`, `PresentLane`; "The targets and the present thread"; `Shown::stop` |
| `measure-m2` | What to time and count for `frame-pace` and `record-stall`, and the reservation to vary | `ShownOutput`, `Rendered`, `Shown::stats`; `WorldConfig::physics` beside `RECORD_RESERVATION`; "Pacing" |
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
  1. **The ticks.** `Server::frame`'s first half, unchanged: the poll with its resync while
     paused, the budget, and `frames(next)` and `Server::step` for each tick ("The timer
     root" above). The render task of the frame before, if one is in flight, runs beside
     these ticks. That is the overlap of architecture section 5.3, and the only one.
     (`Server::frame` is this half and step 4's passes and gather, back to back.)
  2. **The join.** It awaits the render task of the frame before and takes back what the
     task owned. What that frame did is `ShownOutput::joined`. From here to step 5 no
     tick's stage and no method of the render side is in flight but the ones the steps
     name.
  3. **The state.** The frame *draws*, *waits* or is *hidden* ("The frame's three
     states"). Deciding may take one job: `RenderSide::open`, when no region is known to
     be open.
  4. **The passes**, unless the frame waits. The sink is changed where the state or the
     last frame's counts ask for it ("Counts, torn frames and resets"); then
     `Simulation::run_passes` once, whether or not a tick ran; `GatherState::take` into
     `ShownOutput::world.gathered`; `World::take_present_events`; and
     `ExtractState::publish` with that batch.
  5. **The render task**, unless the frame waits. It is spawned on a frame-critical worker
     (`Executor::spawn`, `WorkerClass::FrameCritical`) and owns the frame's own world, the
     lane and the protocol's state until it resolves. It runs:
     1. the frame's job, one pass of the frame's own world ("The render side"), in which:
        `SnapshotReader::acquire`, once; `RenderSide::present` with the acquired frame and
        the frame's time; and, where the frame draws, `RenderSide::draw` with the target,
        the extent and the target's last read;
     2. where the frame drew, `RenderSide::submit`: the latch and the submit, with nothing
        of this crate's between them;
     3. where it submitted, the present protocol's one decision ("The targets and the
        present thread"): at most one call queued on the lane, and none awaited.
  6. `frame` resolves with the world's half as `Server::frame` reports it, `joined`, and
     `deferred` where the frame waited. It does not wait for the render task it started.
- So extract is exclusive (architecture section 5.3). The passes of frame N run after the
  ticks of frame N have resolved and before a tick of frame N+1 starts, since one task
  owns the `Simulation` and awaits each of its runs; and they run with no method of the
  render side in flight, since the render task of frame N−1 was joined first. The frame's
  job reads the snapshot and seals the region only after frame N's `publish`, and the next
  pass begins only after the task that ran that job was joined. So no frame reads the
  snapshot buffer or the ring region extract is writing (the milestone 2 plan, criterion
  3(a)), by order, with no lock held across either.
- So the next frame's extract begins only after this frame is submitted.
  `crates/renderer/README.md` allows it from `prepare` on ("What `frame-design` must still
  decide"); the frame does not take that, because the render side is one object with one
  owner at a time, and a pass that ran beside `record` would write through the sink while
  the rest of the object is lent to recording jobs. What it costs: a record that outlasts
  the tick period holds the next frame's passes until it is done, the frame after starts
  late, and its pacer owes the ticks, which it runs at the cap as after any late frame.
- `Shown::step(executor, frame, time, window)` is the step-driven form, for the fixed-frame
  mode of decision 5.1, a recorded test and a replayed log: the join of a frame in flight,
  if any; `Server::step` with `frame`; steps 3 to 5 at `time` and `window`; then the join
  of the render task it started, before it resolves. Nothing of a step's frame overlaps
  the next step. The gathered frame is taken and dropped, as the headless binary drops it.
  The graph runs a step starts are in a fixed order (the step's; the job that asks `open`,
  where no region is known open; the passes'; the frame's job), so a program that gets the
  same answers from its render side starts the same runs in the same order, which is what
  a replayed log follows (`crates/task/README.md`, "Chaos"; `Workers::replay`). The paced
  form interleaves a render task with the next ticks by wall time and is not replayable,
  as `Headless::run` is not.
- The frame's time. Neither form reads a clock. `FrameTime::now` is `frame`'s `now`.
  `FrameTime::fraction` is `1 − (next_deadline − now) / period`, clamped to `[0, 1]`, with
  `next_deadline` the pacer's after the frame's poll and `period` the tick period at the
  world's time scale; it is one while the world is paused. So it is zero at the newest
  tick's deadline and one a period later, and a frame that is behind draws at one. The step
  form takes the caller's `FrameTime` as it is and reads no pacer. The fraction is of one
  tick period whatever the pair's two ticks are: after a frame that waited or caught up,
  the pair spans several ticks and the picture covers them in one period.
- `Shown::flush` joins the frame in flight and resolves with what it did; a present call
  that frame queued may still be in flight. `Shown::stats` is the counters since the build.

**The frame's three states** (architecture section 5.3; `crates/renderer/README.md`,
"Regions"; `docs/verification.md`, the row on a reader that stops acquiring)

- A frame *draws* when the window has an area and a region is open to extract's writers.
  It is *hidden* when the window has none (`window` is `None` or has a zero dimension), or
  when it is starved (below). Otherwise it *waits*.
- Whether a region is open is what the render side last said: `Submitted::region` of the
  last frame that drew, else the answer of `RenderSide::open`. A frame whose window has an
  area calls `open`, in the frame's job, run once for it, when the last answer was not
  `Ready`: in the first frame, after a frame that was torn, and after each `Wait`. `open`
  never waits, and nothing of this crate waits for a region's point: the frame asks again
  at its next call. The point is in `Rendered::Waited` for a caller that may wait off the
  workers, as the fixed-frame mode's driver thread does for each frame's own point.
- A frame that draws runs steps 4 and 5 in full.
- A frame that waits runs its ticks and nothing else: no pass, no gather, no publication,
  no acquire, no render task. `ShownOutput::deferred` is `Some(Rendered::Waited(point))`
  and `world.gathered` is empty. The world's presentation events stay in its queue, and the
  structural log keeps what the passes have not read. Waiting, and not hiding, is the
  answer to a device that is behind: a frame that hid would detach the sink, and the frame
  that drew again would need a full pass, every instance written again and a `cut` in the
  snapshot (`crates/present-api/README.md`, "The triple buffer").
- A frame defers its passes only while the world has run at most
  `WorldConfig::log_retention − CATCH_UP_CAP` ticks since its last passes. The frame after
  such a one has then run at most `log_retention`, which is what a step keeps of the
  structural log (`crates/sim/README.md`, "Runs", step 3), so no consumer that reads the
  log once a pass finds its cursor stale because a frame waited. Past that the frame is
  *starved*: it is hidden, and so is every frame after it until `open` answers `Ready`,
  each of them asking first.
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
  its join. `Send + Sync` is asked because the object rides into the frame's job as a
  service of a world; nothing shares it.
- **The frame's job.** `open`, `present` and `draw` take a `JobContext`, since the graph's
  `record` and the presentation schedule fork from one. The frame gets it with no lock, by
  the one route `crates/render-graph/README.md` names. It keeps a world of its own, of kind
  `Client`, with no row and no region, whose one service holds the render side, the
  snapshot's reading half, what the frame asks and what was answered; and a schedule of
  one stage and one system, which takes `Job` beside `ResMut` of that service's `dyn`
  trait and calls it with the job. `Schedule::run_pass` on that world is the frame's job.
  The render task moves the world in and gets it back, and reads the answers through
  `World::service_mut` between runs. A `#[system]` ships only in a boundary crate or a
  lint-file carrier (the milestone 2 plan, decision 5.18), which this crate is not, so the
  trait, the system and the function that builds that world are `lactose-present-api`'s
  ("What D162 asks of other crates"). `lactose-presentation` is given no system that runs
  on a world, so it joins no carrier list: its schedule is called by the binary's object
  inside this job. The frame's own world is never hashed, saved, stepped or recorded, and
  its tick never advances. `tests/shown.rs`, section 6, runs the route
  against `lactose-ecs` as merged.
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
    makes the target's image where the index has none or has one of another extent.
  - `submit` is `Renderer::latch`, `RenderGraph::submit` and `Renderer::submitted`, then
    `Renderer::open` for the next frame, which never waits.
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
  the same index, and at `close`.
- The present thread reads a finished target in one call, and between calls no command of
  its own names one. That is the frame's answer to `crates/rhi-api/README.md`'s "which
  image the present thread holds between its presents": none. `PresentSide::present` names
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
- After the frame's submit, the render task looks once at the call in flight. If it has
  returned, or none was made, its report is kept (its `read` where it gave one, and
  `shown` counted) and one call is queued for the target just finished, `Finished {
  target, image, extent, point }` with the image and the point `submit` returned;
  `Rendered::Drew::queued` is `true`. If it has not returned, nothing is queued and
  `queued` is `false`: that frame is never shown and the next frame draws into the same
  target.
- So at most one present call is in flight and at most one is queued a frame (architecture
  section 3; D159), and the lane never grows. That is back-pressure, not pacing.
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
  shown late: the decision is made once, at the frame's submit. Where the display takes
  images at least as fast as frames finish, a call has returned by the next submit and
  every frame is queued. Where it does not, frames are dropped; where a call outlasts a
  frame, every second one or more. `ShownStats::drawn`, `queued` and `shown` say what
  happened, and `frame-pace` records them ("Pacing").

**Threads, and what each owns** (architecture sections 3 and 5.1;
`crates/platform/README.md`, "A client binary, by thread")

| Thread | What it runs of the shown frame | What it owns |
|---|---|---|
| The task that drives the frame: the timer root, on whichever frame-critical worker polls it, or a step-driven run's driver thread through `Executor::block_on` | Steps 1 to 4 and 6: it awaits the world's runs, joins the render task, changes the sink, publishes and spawns | `Shown`: the `Server`, and between frames the render side and the protocol's state |
| Frame-critical workers | The jobs of the ticks and of the passes; the frame's job, and what `present` and `draw` fork from its context, the graph's recording jobs among them; the render task's own polls, which call `submit` and touch the lane | Nothing between jobs. The render task owns the render side and the protocol's state from its spawn to its join |
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
  read by the render side's methods. Never at once: a pass runs only between a join and a
  spawn.
- The render side and the protocol's state: moved between the driving task and the render
  task, never shared. No lock and no atomic.
- A target's image: a `Copy` handle on both sides. Whose turn it is to use it is the
  protocol's, by the two points of "The targets and the present thread".
- The device, an `Arc<dyn Device>` of the binary's: used by the render side's methods, by
  the present side on the present thread and by a step-driven run's driver thread
  (`wait_timeline`). Its synchronisation is the backend's (D158).
- The present call: the lane's queue and the call's reply slot (D159).

The locks a frame takes are these, every one permitted by architecture section 3 and none
this crate's own: the snapshot channel's, three times as above; a device backend's, inside
the render side's and the present side's methods and the sink's writes
(`crates/rhi-api/README.md`, "Threads"); and the lane's two, the queue's once when a call
is queued, the reply slot's when the call's result is stored and each time the render task
looks at it, which is once a frame, and when `stop` awaits it. This crate adds no lock, no
atomic and no channel to the per-frame path.

**Pacing** (architecture sections 5.1, 5.3 and 11; D144, D156; `docs/verification.md`,
the row on the keeper's timed wait on Windows)

- The timer root paces the frame and nothing else does. The binary's root sleeps until
  `ShownOutput::world.next_deadline` with `Executor::sleep_until` and calls `Shown::frame`
  again, as a loop around `Server::frame` does. The present thread paces nothing: a present
  that blocks holds no frame, since its caller holds a future and queues nothing behind it.
  No frame waits for a display.
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
- A frame is late when the task that drives it is late: by the keeper's wake, by ticks
  that overrun, or by the join of a record that outlasts the period. The pacer counts what
  is then owed and the frame runs it at the cap, as a server's does.
- Where the present role shares the graphics queue (`Capabilities::queues.present`; the
  row of `docs/verification.md` on which queue families present), `RenderSide::submit`
  takes the queue's lock behind a present's driver call (D158), on a frame-critical worker.
  That is the one place the present thread can hold a frame. `frame-pace` times it.
- `frame-pace` is the result file the frame asks of `measure-m2`, on the M1 and on
  Windows, from a run of `lactose-app-client` of ten seconds or more, shown, and on Windows
  also minimised (`MainThread::set_minimised`). It records, for each paced frame: the
  lateness of its first tick's start against the deadline the root slept to, taken in the
  frame callback; whether the frame drew, waited or hid; whether its present was queued;
  and the time `RenderSide::submit` took. It reports the P50 and P99 of the lateness; the
  counters of `Shown::stats`; the frames that reported `behind`; and the P99 of the
  submit's time beside the device's `QueueLayout`. The lateness is read against D144's 1 ms
  on Windows and D156's 50 µs on macOS. Those are criteria of a tick's start with idle
  workers, so a figure over one in a shown run is a finding for the planner and no failed
  criterion. Everything else is recorded with no threshold.
- The frame names no use that needs a tick or a frame to start within 35 µs of its deadline
  on macOS, which is what `docs/proposals/tick-start-macos-27.md` asks a milestone 2 design
  to say if it has one. A frame that starts late takes its fraction from `now`, so the
  lateness moves what is drawn by that share of a period, 50 µs being three thousandths of
  one, and breaks nothing. D156's 50 µs stands.

The keeper's unmeasured list on Windows (`docs/verification.md`; D144, D153), read against
this frame:

| Item | Does the frame use it | What follows |
|---|---|---|
| The keeper at a frame worker's priority | Yes. The root's sleep ends on the keeper while the record of the frame before, the present thread and, on a shared queue, a present's driver call compete with it, which no milestone 1 run had | `frame-pace`: the lateness of each frame's first tick, in a shown run on Windows |
| A wait asked once for the time left less a fixed margin | No. The frame adds no deadline to the tick's, so nothing asks more of the wait than D144 measured | Re-gated to the change that gives the frame a deadline between ticks, a display's refresh, which is the first to ask for a shorter wait |
| The spin's power cost | No more than milestone 1 did: one deadline a tick, shown or not | Re-gated with the wait above: a deadline between ticks is what multiplies the spins, and no milestone 2 criterion reads power |
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
  does, and `deferred` is `Some(Rendered::Torn)`. From `present`: `draw` is not called, and
  the open region keeps the frame's pass for the next `prepare`. From `draw` or `submit`:
  the object has ended the frame as torn. In each case the frame asks for a region again at
  its next call, a new sink is owed, no present is queued and `ShownStats::torn` counts
  it.
- An error a present call returns is a present skipped: the target's `read` is unchanged
  and the next submit queues again. A lane call that resolves `Cancelled`, because the
  present thread is gone, is the same.

**Determinism of a shown world** (architecture sections 8.1 and 8.3; the milestone 2 plan,
criterion 3; D152)

- A shown world's hash after every tick is the hash of the world `build_server` builds from
  the same arguments and steps with the same frames: with the render side attached or not,
  on any backend, in every SOUR mode. What keeps it so:
  - One assembly path. `build_shown` is `build_server` and three installs; no registration,
    setup, applier or hook differs.
  - Nothing the render side says reaches a world. Its answers decide whether a pass runs
    now or a frame later and whether a sink is installed, and nothing else: no value of
    its is written to a column, a resource or a `TickInput`. The ticks a paced frame runs
    depend on `now` and the frame callback alone; a step-driven run's, on its steps.
  - A pass leaves the hash as it found it (D152; `lactose-present-api` shows it for
    extract). So passes run a frame later, or with another sink, or with none, move no
    hash. `ExtractState`, the snapshot's writer and the sink are a service's, in no column.
  - The presentation events' queue and the keys a world has delivered are outside every
    region (`crates/sim/README.md`, "The world hash"). So leaving them for the frame, and
    taking them a frame later, moves no hash; the `lactose-sim` option must keep that.
  - The frame's own world is in no hash, save or recorded stream.
  - The frame orders no entity. A key it passes on is a `StableKey` in the sink's rows and
    the snapshot's, as `lactose-present-api` makes them; a target is an index below two.
- Criterion 3's tests, which `frame-impl` writes:
  - (a) and (b) are in this package's `integration` target, module `shown`, over a render
    side with no device (`tests/shown.rs`, section 5). (a): a sink and a render side that
    log their calls, a system of the tick that logs its start, and a frame callback; over
    paced frames and stepped ones, through each of the three states, no tick's system
    starts between a pass's `begin` and its `end`, and no `present` or `draw` is called
    between a pass's `begin` and the `publish` after it. (b): frame N's `draw` waits at a
    barrier until a system of tick N+1 has started, and that system waits until `draw` has
    started. The test builds an executor of its own, of two frame workers or more, since
    one worker cannot hold both, and records no hash. Each asserts on order and never on
    time.
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
   `WindowReports`; `Shown::frame`; the output to its log and its counters.
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
  tick's wrap the row jumps unless `period` divides 2^32.) It uses `lactose-math` alone,
  under the boundary's lints.
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
| `RenderSide::open` | `Renderer::open`: `timeline_value` | The counter | `MTLSharedEvent`'s value | The semaphore's counter | renderer, "How each call is honoured" (`open`), "Regions" |
| `RenderSide::open` after torn frames with none prepared | A declaration with no pass through `RenderGraph::{compile, record, submit}`, then `Renderer::submitted` | Checks every command in order, applies the states, completes when its waits are reached | Command buffers enqueued in slice order; `encodeSignalEvent` | `vkQueueSubmit2` with timeline semaphores | graph, "Declaring", "How each call is honoured" (`submit`); renderer, "Regions" |
| `RenderSide::present` | No device: the presentation schedule on the pair | The same on every backend | | | present-api, "A frame's transforms"; decision 5.5 |
| `RenderSide::draw`: the target's image | `create_image`, exclusive, a colour attachment that is sampled; `destroy_image` of the one replaced, after `timeline_value` | Usage and a state for each subresource; refuses a destroy while pending or while a slot holds the image (`pending`) | `makeTexture`; the object released | An image; the object destroyed | rhi-api, "How each call is honoured" (`create_image`, `destroy_image`), "Handles and lifetimes" |
| `RenderSide::draw`: the target's import | `FrameDecl::import_image`: from `Undefined` on the graphics role, `ready` the present's `read`, left a colour attachment, handed off to the present role for a sampled read | States tracked from the declaration; the transition from `Undefined` ends a release no acquire met; a submit that uses the image with no wait for `ready` is refused (`no-qfot`) | No layouts and no ownership: the transfer's two halves are nothing; the wait is `encodeWaitForEvent` | A barrier from undefined; a release toward the present family where the roles have two queues | graph, "Imports", "What each consumer needs", "How each call is honoured" (`record`: around each pass); rhi-api, "States", "The present thread"; D160, item 7 (a) and (c) |
| `RenderSide::draw`: the remote batch, `Renderer::prepare`, `Renderer::declare` | As the renderer's rows for a writer's `write`, `prepare` and `declare` | As those rows | As those rows | As those rows | renderer, "How each call is honoured" |
| `RenderSide::draw`: `RenderGraph::compile`, `record` | As the graph's rows for `compile` and `record` | As those rows | As those rows | As those rows | graph, "How each call is honoured" |
| `RenderSide::submit`: `Renderer::latch` | `write_buffer`, `flush_buffer` on the frame's camera buffer | Judged at the submit, after the write | A write into shared memory | A write into the mapping | renderer, "How each call is honoured" (`latch`); graph, "A frame, by step" |
| `RenderSide::submit`: `RenderGraph::submit`, `Renderer::submitted`, `Renderer::open` | `Device::submit` for each of the frame's submits, with its waits, the import's `ready` among them, and its signals, the frame's point among them; `timeline_value` | Checks every command in order, applies the states, completes when its waits are reached; pending until the caller sees its point | Command buffers enqueued in slice order; `encodeWaitForEvent`, `encodeSignalEvent` | `vkQueueSubmit2` with timeline semaphores | graph, "How each call is honoured" (`submit`), "Submission"; rhi-api, "Timelines" |
| The present call: `Lane::run`, and the task's look at its `LaneCall` | No device: the lane's queue and the call's reply slot; the call runs on `lactose-platform`'s present thread | The same on every backend | | | task, "Lanes"; platform, "The present thread and its pump" |
| `PresentSide::present`: the resize | `wait_timeline` for its own points, within a bound; `Swapchain::resize` | Images with no window | A `CAMetalLayer`, its bounds and `contentsScale` the view's | A swapchain; the present fence of each present waited for by `resize` within `acquire`'s bound | rhi-api, "The swapchain", "How each call is honoured" (`create_swapchain`, `acquire`, `present`, `resize`) |
| `PresentSide::present`: `Swapchain::acquire` | The acquire, bounded by the backend | An acquire that can be made to return `None` | `nextDrawable`, which has its own one-second limit | An acquire with a finite timeout | The same |
| `PresentSide::present`: the encoder | `create_encoder` on the present role; `transition_images`, the acquire half of the hand-off; `write_heap_slot` for a target's image it has not seen; the pass that samples the target into the acquired image | A command list; the state and owner of each subresource, applied at the submit in order; an acquire with no equal release is refused (`no-qfot`); the image for each slot | A command buffer made on first use; nothing for the transition; a render command encoder, `useResource` for the declared slot's texture; a write of the texture's resource id into the argument buffer | A command pool and one command buffer; the acquire across families; `vkCmdBeginRendering`; an update-after-bind descriptor write | rhi-api, "The present thread", "States", "The heap and the sampler table", "How each call is honoured" |
| `PresentSide::present`: `Swapchain::present` | A submit and a present in one call, the submit waiting for the frame's point and signalling `read` | Completes when its waits are reached; refused (`pending`) past the bound with nothing submitted | `encodeWaitForEvent`, `encodeSignalEvent`; `present(drawable)` before the commit | `vkQueueSubmit2`, then `vkQueuePresentKHR` after the submit | rhi-api, "The swapchain", "Timelines" |
| `PresentSide::close` | `clear_heap_slot`; the swapchain's drop, which waits for the submits that use its images | The slot emptied; refuses while a pending pass declares it (`uuwp-used`) | The placeholder's id written; the layer let go | The placeholder written; the swapchain destroyed | rhi-api, "The heap and the sampler table", "Handles and lifetimes"; platform, "Shutdown", step 2 |
| A step-driven run's wait for a frame's point | `Device::wait_timeline` with a bound, on the driver thread | A counter and a condition variable | `MTLSharedEvent`: a listener, a wait with a timeout | A timeline semaphore | rhi-api, "Timelines", "How each call is honoured" |
| The drop of `Stopped::render` | `RenderGraph`'s drop, then `Renderer`'s, then `destroy_image` for each target | Refuses a destroy while pending | The objects released | The objects destroyed | graph and renderer, "Errors and drops"; rhi-api, "Handles and lifetimes" |
| Any of them on a lost device | `ErrorKind::DeviceLost`, and after it every wait returns at once and every drop succeeds | A flag its own call sets | A command buffer's error status | `VK_ERROR_DEVICE_LOST` | rhi-api, "Device loss" |

Two rules of the layers below that the frame leans on:

- **The re-take from `Undefined` ends a release no acquire met** (D160, item 7 (c)). Null
  does it, as `crates/render-graph/README.md`, "What each consumer needs", states;
  `crates/rhi-api/README.md`, "States", says so once item 7's sentence is in, and its
  "Between the two the resource is used by nothing" reads against it until then. The frame
  makes such a re-take whenever a finished target was not queued, or its call made no
  submit. On native Metal there is no ownership and nothing to end; on Vulkan it is a
  barrier from undefined on the releasing queue. The facade stands on null with
  the sentence or without it.
- **A submit that uses an image another queue's pending submit changed waits for it**
  (D160, item 7 (a)). The graph makes every submit that names an import wait for the
  import's `ready`, and the frame gives the present's `read` as that point, so no submit of
  a frame is refused for it. A wait for a point already reached costs no GPU wait.

**What D162 asks of other crates**

Named in their smallest form. None is made by the facade's change; the signatures above
stand without any of them, and `frame-impl` cannot write the bodies without the first two.

- `lactose-sim`: a builder option, `SimulationBuilder::keep_present_events(self) -> Self`,
  after which a `Server` world's step leaves its presentation events in the world's queue
  for a caller that takes them after each frame's passes; and `crates/sim/README.md`,
  "Runs", step 4, saying so. The default is today's. It changes no hashed state. Without
  it a shown world delivers no event and nothing else here changes.
- `lactose-present-api`: the frame's job.

  ```rust
  /// What a frame runs, once, in a job of a world of its own.
  pub trait FrameJob: Any + Send + Sync {
      fn run(&mut self, job: &mut Job<'_, '_>);
  }

  /// A world of kind `Client` with no row, no region and no resource, `work` its one
  /// service, and a schedule of one stage whose one system runs `work` in a job.
  pub fn frame_world(work: Box<dyn FrameJob>) -> Result<(World, Schedule)>;
  ```

  The system, `present::frame_job`, with `worlds = [client]` and a `Job` beside
  `ResMut<dyn FrameJob>`, is private to that crate, which carries the lint file and may
  declare one (D152). It needs no new dependency: `Job`, `World` and `Schedule` are
  `lactose-ecs`'s. `tests/shown.rs`, section 6, is the same code as a test's own. This
  crate implements `FrameJob` for a private type and reads it back by `Any`.
- `lactose-app`, by `frame-impl`: `lactose-present-api`'s three registration functions in
  the engine lists ("The shown frame").
- Sentences of other READMEs that leave a choice to `frame-design` become references to
  D162 in the change that next touches each, with three readings to carry over.
  `crates/renderer/README.md` and `crates/render-graph/README.md` say the `dyn` object's
  trait belongs to a foundation crate: that is `FrameJob`, and `RenderSide` is this
  crate's. The renderer's "A frame, by step", step 1, has the frame wait for a region's
  point off the frame-critical workers: the frame asks again at its next call, and no
  thread waits. `crates/rhi-api/README.md`, "The present thread", has the present thread
  hold one target at a time, the newest the frame has finished: it holds that one for the
  span of a call, and none between calls. The facade stands with those sentences as they
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
  resets") and the next frame runs. `Stop` on a lost device ends the run with `ShownError {
  kept: ShownKept::Shown(..) }`: the world is whole, and the binary calls `Shown::stop`,
  which ends in order on a lost device because every wait there returns at once and every
  drop succeeds (`crates/rhi-api/README.md`, "Device loss"). Recovery is not here ("Not in
  this crate yet"). Under `MILESTONE_1`, `Source::Render` is `Stop` whatever the kind. A
  render task decides its own errors with a copy of the policy; a `Stop` it met reaches the
  caller at the join, as the error of the `step` that started the task or of the next
  `frame` or `flush`, and `stop` logs it and goes on.
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
  were dropped with the tasks that held them. `ShownError` converts into
  `lactose_core::Error` as `ServerError` does.

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
  device and no worker may; calls `Server::shutdown`; drops what else holds the device, the
  last of it in a main-lane call; and shuts the executor down. `stop` is awaited by the
  task that drives the frame, or through `Executor::block_on`. An executor shut down first
  resolves it `Cancelled` with nothing handed back, and the render side is then dropped
  wherever its task was dropped, which may be a worker: the binary shuts the executor down
  last.

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
  such a frame as it is ("Pacing"); the deadline is what is missing, and `lactose-platform`
  reports no display's refresh. The keeper's wait asked once less a margin, and the spin's
  power cost, are re-gated to the change that adds it (`docs/verification.md`).
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
- A present queued when the last call returns, and not only at a frame's submit: what
  `frame-pace` asks for if it shows frames dropped on a display at least as fast as the
  frame ("The targets and the present thread"). It needs a target's state shared between
  two tasks, which nothing here has.
- The overload ladder's render half: `ShownStats` and `FrameOutput::behind` are its
  inputs, and nothing acts on either.
- Everything else a client binary adds: milestone 4.
- An edge against the replication gather: `net::gather` is in no `SystemSet`, so a game's
  exclusive `Pass::Gather` system cannot be ordered against it with `Registration::order`
  until `net-api` names a set; no game crate has one at milestone 1.

## Proof for the shown frame

The shown frame is a facade: of it, only the route of the frame's job runs. What is held
now, and by what:

| What | How |
|---|---|
| The surface names no backend | `lactose-app`'s normal dependencies gain `lactose-present-api` and `lactose-rhi-api`, both foundation crates; `cargo xtask tiers` and `cargo xtask boundary` hold `[server_denylist]` over its normal and build closure. `lactose-render-graph`, `lactose-renderer` and `lactose-input-host` are dev-dependencies, for `tests/shown.rs` alone |
| The types the contract promises | `lib.rs`, at compile time: `Shown`, every run future, `ShownOutput`, `ShownError` and `Stopped` are `Send`; `dyn RenderSide` and its box are `Send + Sync + 'static`, which a world's service must be; `Box<dyn PresentSide>` is `Send + 'static`, the present thread's state; `PresentLane` is `Send + Sync`; every value that crosses a trait is `Copy`; `TARGETS` is two or more and `TargetIndex` admits exactly the indices below it; `RECORD_RESERVATION` is one or more |
| `RenderSide` can be written over the graph and the renderer as merged | `tests/shown.rs`, section 1: `open` with the empty submit after torn frames; `sink`; `present`; `draw` (the target made or replaced, the remote batch, `prepare`, the import from `Undefined` with `ready`, compile, record, and `torn` on each failing path); `submit` (the latch, the submit, `submitted`, the next `open`); and the drop in order. Compile-only |
| `PresentSide` can be written over `lactose-rhi-api`'s swapchain | Section 2: the resize behind its wait, the acquire, an encoder that a call which submits nothing drops, the present with its wait and its signal, `Stale` as a submit that was taken, `close`. Compile-only |
| The binary's wiring and its shutdown type-check | Section 3: `build_shown`; the root's loop around `Shown::frame` with a window's extent; every arm of `ShownKept`; `Shown::stop` through to the render side's drop on a thread of the platform's. Section 4: the fixed-frame mode's driver around `Shown::step`, and the render side taken back as its own type. Compile-only |
| `frame-impl`'s tests can be written | Section 5: a render side and a present side with no device, which count their calls; a shown world and the same world unshown, stepped alike with the hashes compared; the protocol's choice of target. Compile-only |
| The frame's job is reached with no lock | Section 6, `the_frame_job_runs_in_a_world_of_its_own`, which runs: a world of kind `Client` with no row, whose one service is a `dyn` object; a schedule of one system that takes `Job` beside `ResMut` of that trait; `Schedule::run_pass` from a task spawned on a frame-critical worker and again through `block_on`. The object was handed the job and called `RenderSide::open` with its context, the answer was read back through `World::service_mut`, the world's tick did not advance, the service was one object across both runs, and the render side left the world by value |

What `frame-impl` proves, beside criterion 3's three tests ("Determinism of a shown
world"): each state's order of calls, with a render side that logs them; the bound on
waiting, at `log_retention − CATCH_UP_CAP` ticks, and the starved frames after it; the
four cases of a new sink, and no second reset after a full pass that left rows out; the
protocol, with a present side that holds its call (no target drawn while a call names it,
`ready` the last `read`, `queued: false` while a call is in flight, nothing kept of a call
that reported no read); the policy's two rows; `stop` with a frame and a call in flight,
and after a lost device; the fraction at a deadline, a period later and while paused; and,
on `lactose-rhi-null`, that a run through each state is refused nothing.

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
