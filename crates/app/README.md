# The assembly (`lactose-app`)

What turns every crate's registration into an executor, a solver pool and the worlds, and
then runs them: the timer root, the owner of the error policy, the place the `dyn`
resources are wired, the client's loop around the prediction world, the server's store of
saved regions, and the save path from the sync point to the slot. The thin binaries link
it: `app-headless` at milestone 1, `app-client` and `app-server` at milestone 4.

| | |
|---|---|
| Tier | assembly |
| Determinism boundary | outside |
| Unsafe | none, `#![forbid(unsafe_code)]` |

**Status: implemented.** Every public signature is the facade's (D131), unchanged, and every
body is implemented; the proof below runs. Every sentence under
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

D142 changed no signature of this crate: `build_server` and `load` do what the bullet
"Binds" under "Saves and loads" states, and "Proof for D142" names the tests that hold it.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `app-headless`, the milestone 1 netcode spike,
the SOUR test of the assembly and, from milestone 4, `app-client`, `app-server` and `net`
are written against this surface. `tests/headless.rs` holds the binary's `main` (which
`crates/app-headless` is) and the SOUR test of the assembly, and `tests/spike.rs` is a
compile-only sketch of the spike's loop: a change that breaks one is a signature change to
list.

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

Hidden plumbing, which the implementation may reshape: the solver pool's construction and
the exit count a world waits on (`runtime::solver_pool`, `runtime::SolverExit`); the
pending time scale, the scale the last step carried, the bundles queued for the next step
and the migrations inside `Server`; the clock estimate with the ticks granted since it was
taken, the lead, the pending resync, the newest frame tick whose fold failed and the tick
the last frame's steps started from inside `Client`.

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
| `app-client`, `app-server`, `net` (milestone 4) | The places named now and the signatures that will change | `Client` gains the replicated world; `build_client` loses `entity`; `Workers::platform` takes the platform crate's instance; `Server::relevance` already takes the executor; "Not in this crate yet" |
| The error policy (architecture section 19) | One place that decides, as a value a test can read | `ErrorPolicy::MILESTONE_1`, `Source`, `Response`; "Errors" below |

## The registration a game crate writes

A game crate exposes one value naming its generated functions, and nothing runs before
`main` (architecture section 18):

```rust
use lactose_app::CrateRegistration;
use lactose_core::Result;
use lactose_ecs::World;
use lactose_persist::Migrations;
use lactose_sim::{Registration, TypeTable};

pub fn register_types(types: &mut TypeTable) -> Result<()> { Ok(()) }
pub fn setup(world: &mut World) -> Result<()> { Ok(()) }
pub fn register_systems(registration: &mut Registration) {}
pub fn register_migrations(migrations: &mut Migrations) -> Result<()> { Ok(()) }

pub static GAME_SIM: CrateRegistration = CrateRegistration {
    name: "game-sim",
    types: register_types,
    setup: Some(setup),
    systems: register_systems,
    migrations: Some(register_migrations),
};
```

The binary lists the crates and hands the game to the assembly (`tests/headless.rs`,
section 3, is the headless binary's `main` in full, with the cooked content it builds in
memory):

```rust
use std::sync::Arc;

use lactose_app::{Cooked, Game, Headless, HeadlessConfig, Start, Workers, WorldConfig};

pub static GAME: Game = Game {
    name: "game",
    crates: &[GAME_SIM],   // in call order; game-ai and game-content follow at milestone 5
    order: None,           // the game's own `Registration::order` edges
    sessions: None,        // the game's session applier; `None` drops and logs joins
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
  platform crate's instance at milestone 4.

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
- `Workers::platform` from the platform crate, the present thread, and everything a
  client binary adds: milestone 2 and 4.
- An edge against the replication gather: `net::gather` is in no `SystemSet`, so a game's
  exclusive `Pass::Gather` system cannot be ordered against it with `Registration::order`
  until `net-api` names a set; no game crate has one at milestone 1.

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
