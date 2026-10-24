# The assembly (`lactose-app`)

What turns every crate's registration into an executor, a solver pool and the worlds, and
then runs them: the timer root, the owner of the error policy, the place the `dyn`
resources are wired, the client's loop around the prediction world, and the save path from
the sync point to the slot. The thin binaries link it: `app-headless` at milestone 1,
`app-client` and `app-server` at milestone 4.

| | |
|---|---|
| Tier | assembly |
| Determinism boundary | outside |
| Unsafe | none, `#![forbid(unsafe_code)]` |

**Status: facade.** Every public signature is final and every deciding body is `todo!()`;
`WorldConfig::headless`, `ErrorPolicy::respond` and the accessors are real, since they are
values. Every sentence under "Behaviour the signatures cannot express" is one a dependent
crate may rely on and the implementation must meet. Architecture sections 2, 3, 4, 5.1,
5.2, 8.3, 9.2, 9.3, 9.7, 11, 15.3, 16, 18, 19, 20 and 22 are the design; where this file
and the architecture disagree, the architecture wins and this file is wrong. Decision D131
records the choices those sections left open. `crates/sim/README.md`,
`crates/task/README.md`, `crates/predict/README.md`, `crates/net-api/README.md`,
`crates/net-host/README.md`, `crates/persist/README.md`, `crates/physics-api/README.md`,
`crates/physics-rapier/README.md`, `crates/input/README.md` and `crates/anim/README.md` are
the contracts this crate builds on and never restates: what they say about runs, the sync
point, the pacer, the predictor's loop, staged frames and the fold, the container, the
solver pool and the registration functions holds here unchanged.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `app-headless`, the milestone 1 netcode spike,
the SOUR test of the assembly and, from milestone 4, `app-client`, `app-server` and `net`
are written against this surface. `tests/headless.rs` and `tests/spike.rs` are
compile-only sketches of what each of them writes: a change that breaks one is a signature
change to list.

## Surface

| Item | Purpose |
|---|---|
| `CrateRegistration`, `Game` | What a game crate contributes, as function pointers (its generated `register_types`, `setup`, `register_systems`, `register_migrations`), and the game as the binary hands it over: every crate in call order, the game's own ordering edges, its session applier |
| `Workers`, `start_executor` | What the assembly decides for the executor beyond the builder's defaults (SOUR's workers and chaos modes, a replay, the platform), and the one place an executor is built |
| `CATCH_UP_CAP`, `LOG_RETENTION`, `TICK_PERIOD` | The timer root's constants: four ticks per frame, the structural-log retention every world is built with, the tick period every pacer is built with |
| `WorldConfig`, `Start` | What the assembly decides for one world (layout, region control, policy, physics, retention), and where a server world starts: fresh, or from a decoded save |
| `build_server`, `Server` | A server world between runs: the simulation, its pacer, the schema table, the time scale, the baseline of every region it stepped; `step` for an unpaced tick, `frame` for one frame of the timer root, `save_point` and `encode_save` for a save, `relevance` for a region's relevance message |
| `ServerStep`, `ServerFrame`, `ServerSave`, `FrameOutput`, `ServerError` | The server's runs in flight, what a frame produced (ticks, `behind`, the next deadline, the gathered frame, the lifecycle's requests and exports), and how a run fails with the server kept or gone |
| `build_client`, `Client` | A client's prediction world between runs: the predictor, its pacer, the lead, the sample history; `push` for a staged frame, `clock` for an estimate, `frame` for one frame of the timer root's client half |
| `ClientFrame`, `ClientOutput`, `ClientError` | The client's frame in flight, what it produced (ticks, `behind`, the next deadline, the reconciliation, the rollback's wall time, a resync, the presentation events), and how it fails |
| `ErrorPolicy`, `Source`, `Response`, `LOG_TARGET` | The policy as a table from where an error came from to what the assembly does with it; the `tracing` target everything here logs at |
| `Cooked`, `Loaded`, `load` | Where cooked content comes from until the asset store exists; a decoded save; the load path from the slot's bytes to a `Start::Loaded` |
| `SaveEncode`, `write_slot`, `SlotWrite` | A save being encoded on background tasks, and the atomic slot write |
| `HeadlessConfig`, `Headless`, `RunReport` | The headless app: the executor and one server world, stepped from a test or run at tick rate by a tool, saved to a slot, shut down in order |

Hidden plumbing, which the implementation may reshape: the solver pool's construction
(`runtime::solver_pool`), the pending time scale and the baseline table inside `Server`,
and the clock estimate and lead inside `Client`.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| `app-headless` (milestone 1) | Start an executor and a server world from the game's registrations, run N ticks at tick rate, report the hash, save, shut down in order; `main` returns the first fatal error | `Headless::{start, run, hash, save, shutdown}`, `RunReport`; the policy's `Stop` is `main`'s `Err` |
| | The same from a save | `load` before any world exists, `Start::Loaded` |
| A SOUR test of the assembly | Build from the session's workers, layout and chaos; one unpaced tick at a time with the hash after each; the chaos log at the end | `Workers::{frame_workers, chaos}`, `WorldConfig::layout`, `Headless::{step, hash, shutdown}`; the glue is `tests/headless.rs`, section 4 |
| The milestone 1 netcode spike, server side | Step the server once per frame with the sessions and inputs its jitter buffer released; take the gathered frame for its packer; the player's entity after a join; a full frame for the join | `Server::frame` with a frame callback, `FrameOutput::gathered`, `Server::simulation` with `lactose_input::player_entity`, `Server::relevance` into a `StagedFrame::resync` |
| The spike, client side | Build the prediction world on `JoinAccepted`; queue staged frames; one frame with a scripted sample; the last eight samples to send; a resync to request; the rollback cost and correction count to record | `build_client`, `Client::{push, frame, history}`, `ClientOutput::{reconciled, rollback, resync}` |
| | Pace by its own round-trip estimate until `net` computes one | `Client::clock` with a `ClockEstimate` it fills by hand |
| Game crates (milestone 5; the sketches now) | Register once and be built into every world kind, with no code of theirs running before `main` and nothing a dev-profile restart cannot redo | `CrateRegistration` as a `static` of function pointers; `Game::crates` in call order; `Game::sessions` for the player's entity |
| | Order their own sets against another crate's, the engine's included, without owning a stage list | `Game::order`; a game crate names `lactose_input::sets::APPLY`, `lactose_physics_api::sets::WRITE_BACK` and `lactose_anim::sets::ADVANCE` directly, since it depends on those crates |
| `persist` | Its pure calls placed on threads: one encode per region on background workers, the assembly once they resolve; the loader run before any world exists with the registry the world will have; the records' baseline by hash | `Server::encode_save`, `load`, `Cooked::baseline`, `Server::baseline` |
| `io` (facade on its branch) | The slot write as its `Io::replace`, named in prose until the crate lands | `write_slot`, "Saves and loads" below |
| `asset` (milestone 3) | A stand-in with the store's shape, replaced by its traits with a listed signature change | `Cooked`; "Not in this crate yet" |
| `app-client`, `app-server`, `net` (milestone 4) | The places named now and the signatures that will change | `Client` gains the replicated world; `build_client` loses `entity`; `Workers::platform` takes the platform crate's instance; "Not in this crate yet" |
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
section 3, is the headless binary's `main` in full):

```rust
use lactose_app::{Game, Headless, HeadlessConfig, Start, Workers, WorldConfig};

pub static GAME: Game = Game {
    name: "game",
    crates: &[GAME_SIM],   // in call order; game-ai and game-content follow at milestone 5
    order: None,           // the game's own `Registration::order` edges
    sessions: None,        // the game's session applier; `None` drops and logs joins
};

fn main() -> lactose_core::Result<()> {
    let config = HeadlessConfig {
        workers: Workers::default(),
        world: WorldConfig::headless(),
        start: Start::Fresh,
    };
    let mut app = Headless::start(&GAME, config)?;
    let report = app.run(600, Box::new(|_tick| lactose_sim::TickInput::default()))?;
    println!("{:#018x}", report.hash);
    app.shutdown();
    Ok(())
}
```

## Behaviour the signatures cannot express

**Registration** (architecture sections 5.2, 6.1 and 18; `crates/sim/README.md`, "The
registration a game crate writes")

- One `TypeTable` per process, built by `Game::types` in this order and no other:
  `TypeTable::new()`; `lactose_input::register_types`, `lactose_net_api::register_types`,
  `lactose_physics_api::register_types`, `lactose_anim::register_types`,
  `lactose_predict::register_types`; then every `Game::crates` entry's `types` in list
  order. Every world kind is built from the same table, the engine's prediction-only and
  client-only types included, so the protocol hash of architecture section 9.1 is the
  same function of the same list on the client and the server. Registration order changes
  no hash (`crates/sim/README.md`, "The world hash"), so the order is for the log and the
  error message alone: a failing `types` fails `Game::types` with the crate's `name` in the
  message.
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
- `WorldConfig` goes into the builder as `layout`, `regions`, `policy` and
  `log_retention`; `physics` goes to the backend's setup. `log_retention` below
  `CATCH_UP_CAP` is refused at build with `InvalidData`, since the builder cannot check it
  and this crate owns the cap. A `Start::Loaded` sets `tick` and `next_net_id` from the
  save; `Start::Fresh` leaves the builder's defaults.
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
- Each world gets its own solver pool and its own backend, built beside the world by
  `build_server` or `build_client`: `rayon::ThreadPoolBuilder` with
  `executor.frame_worker_count() - config.physics.reserved_workers` threads, at least one,
  named `solver-<n>`, never `use_current_thread`; `lactose_physics_rapier::backend(pool)`.
  A one-worker executor (SOUR's workers mode) gets a one-thread pool. Two worlds in one
  process (the spike) have two pools; milestone 4 decides whether a client's two worlds
  share one.
- A `Platform` is the binary's: `Workers::platform` is `None` for the headless binary and
  the standard platform is used; `app-client` passes the platform crate's instance at
  milestone 4.

**The timer root** (architecture sections 5.1 and 11; `crates/sim/README.md`, "The timer
root")

- Every pacer is `TickPacer::new(now, TICK_PERIOD, CATCH_UP_CAP)` with `now` the instant
  the world is built, so its first tick falls due one period later. `CATCH_UP_CAP` is four
  in every shipped configuration and `LOG_RETENTION` (120) is at least it, pinned at
  compile time.
- `Server::frame(executor, now, frames)` does the following, in this order, and nothing
  else: `pacer.poll(now)`; for each of the budget's ticks, `frames(next)` with `next` the
  tick about to run (`Tick(tick().0 + 1)`), and `Server::step` with what it returns, or an
  end to the frame's stepping on `None`, the due ticks not run being forgotten (the pacer
  counted them) and nothing else changing; `Simulation::run_passes` once, whether or not a
  tick ran and whether or not the scale is zero; then `GatherState::take`,
  `RegionLifecycle::take_requests` and `take_exports` into the `FrameOutput`. The caller
  sleeps until `next_deadline` with `Executor::sleep_until` and calls again. The frame
  callback runs on the task that drives the frame, between steps, so what it drains (a
  jitter buffer, a session queue) is read in tick order and never concurrently with a step.
- `Server::step(executor, frame)` is one `Simulation::step` with `frame`, nothing paced
  and no pass run, and it is the whole of what `Headless::step` and a SOUR test do per
  tick. Both paths apply the same two edits to the frame before stepping: a pending time
  scale replaces `frame.time_scale`, and on the first step of a `Start::Loaded` world the
  save's regions are put in `frame.loaded` ahead of the frame's own entries (a frame that
  also carries a bundle for a saved region has that bundle refused by the sync point as a
  second bundle and reported as a load failure, like any other). After the step, both
  paths record the baseline of every bundle in `frame.loaded` whose region is not among
  `take_load_failures()`, and log every load, session and asset failure at `LOG_TARGET`
  with the region or player and the kind; requests and exports are left in the lifecycle
  for a frame to take (`step` takes nothing).
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

- The scale is the assembly's decision on the server. `Server::set_time_scale(scale)`
  records it; the next step, by either path, carries it as `TickInput::time_scale`,
  replacing what the frame callback put there, and from the next `pacer.poll` the pacer
  runs at `f64::from(scale.thousandths) / 1000.0`. So a pause is entered by one tick
  whose frame carries the zero and then no tick falling due, and left by the next
  `set_time_scale` whose value the next tick carries: the sequence `crates/sim/README.md`
  requires, in one call. `Server::time_scale` is the pacer's current scale; it changes
  when the step that carries the new one runs, not at the call.
- On a client the scale is never decided: the fold reads `StagedFrame::time_scale` into the
  first new tick's `TickInput` and the pacer runs at the replicated scale times the clock
  estimate's `rate_thousandths / 1000`.

**The client's frame** (architecture sections 9.2 and 9.4; `crates/predict/README.md`,
"The loop `app` writes"; `crates/net-api/README.md`, "Staged frames")

- `Client::push(frame)` is `StagedInbox::push` on the prediction world's inbox, between
  runs; `false` is logged and enters nothing. The replicated world's inbox is `net`'s at
  milestone 4, when `push` pushes into both.
- `Client::frame(executor, now, sample)` does the following, in this order, and nothing
  else. The fold: `StagedInbox::due(next)` for the first new tick, each due frame's
  `time_scale` into the input when `Some`, and its `regions` as `crates/net-api/README.md`
  states: `Irrelevant` removes any pending `loaded` and `assets` entry and inserts
  `unloaded`; `Relevant` inserts `loaded` through `RegionRelevance::to_bundle(baseline,
  None, table)` with `baseline` from `Cooked::baseline(relevance.baseline_hash)` and
  `table` the message's table pinned through `Cooked::pin`, or `None`; `Assets` inserts the
  table pinned; a `full` frame first clears what earlier due frames folded and puts every
  region of `World::regions` in `unloaded`. A region whose baseline or pin `Cooked` cannot
  give is left out of the fold and logged (`Source::Cooked`, `Log`); the next full frame
  retries it. Then `pacer.poll(now)`; one `Predictor::step` per due tick with
  `sample(next).to_player_sample(next)` as the local player's input and the sample put in
  the history; `Predictor::reconcile`, its wall time measured around the await;
  `Predictor::passes`; `take_present_events`; then, if `Reconciled::resync` is `Some`,
  `pacer.resync(now)` and `Predictor::resync(target)` with `target` the last clock
  estimate's `server_tick` plus the lead, or for `Behind { server, .. }` without an
  estimate `server` plus the lead, or for `TooOld` without one the world's own tick; and if
  the budget was `behind`, `pacer.resync(now)`. A kept failure of any run resolves with the
  client kept (`ClientError::client` is `Some`) and the policy's response is `Resync`: the
  caller calls `frame` again and that frame's output reports the resync.
- `ClientOutput::resync` is `Some(tick)` only for a frame that resynced the world, and
  `tick` is where the world now stands: the caller sends a `ResyncRequest` for it. `events`
  is what `take_present_events` returned; `reconciled` is what `reconcile` returned, moved;
  `rollback` is the wall time of the reconcile await, zero when nothing was re-simulated.
- `Client::clock(now, estimate)` sets the lead to `lead_ticks` rounded up, the pacer's scale
  to the replicated scale times `rate_thousandths / 1000`, and on `snap` resyncs the pacer
  at `now` and the predictor to `server_tick + lead`, which the next frame reports as a
  resync. `build_client` takes the lead the caller chose, since no estimate exists at the
  join.
- `Client::history` holds the last `REDUNDANT_SAMPLES` samples by tick, filled by `frame`
  and never by the caller; the caller encodes it into every input packet.

**Errors** (architecture sections 3 and 19)

- `ErrorPolicy::respond(source, error)` is the whole policy and it is a table. Milestone 1:

  | `Source` | `Response` |
  |---|---|
  | `Executor` | `Stop` |
  | `Load` | `Stop` |
  | `Server { kept: true }`, `Server { kept: false }` | `Stop` |
  | `Prediction { kept: true }` | `Resync` |
  | `Prediction { kept: false }` | `Stop` |
  | `Save` | `Log` |
  | `Cooked` | `Log` |

  A server failure stops whether or not the world was kept, because a server never rolls
  back and a prepare failure means an unjournaled edit removed what a system names: there
  is no state to go back to. `Stop` means the run that failed ends with the error and the
  error reaches the binary's `main`, which returns it; the runtime's abort on panic is
  separate and unconditional. The milestone 1 table reads no `ErrorKind`. Milestone 4 adds
  `Source::Net` and `Source::Input`, `Response::Disconnect` and `Response::Refuse`, and
  reads kinds: `StorageFull` on a save (the disk-full path), `InvalidData` and
  `VersionMismatch` on a load (the corrupt-save path), `Network` from the transport.
- Every response is logged at `LOG_TARGET` with the source and the error before it is
  acted on. The lifecycle's load, session and asset failures are not errors of a run: they
  are logged per event after each step, at `warn`, and the tick they were reported in
  carries on.
- `ServerError` and `ClientError` convert into `lactose_core::Error`, so `?` into a plain
  `Result` drops the world as a plain error would; a caller that wants the world matches.
  After `Cancelled` (the executor shut down mid-run) the world is gone and `server` or
  `client` is `None`.

**Saves and loads** (architecture sections 15.3 and 15.5; D129; `crates/persist/README.md`)

- `Server::save_point` is `Simulation::save_point`, by value. `Server::encode_save(executor,
  save)` spawns one `WorkerClass::Background` task per region of `save.regions`, each
  `RegionRecords::from_export(&export, &baseline, &schemas)?.encode_section()?` with the
  region's `Server::baseline` and this world's `Server::schemas`, and once every task
  resolved, `Container::assemble(&ContainerHeader::of(&save, &schemas), sections)` with the
  sections in ascending region order. The future fails with `NotFound` for a region whose
  baseline was never recorded (a region no accepted bundle named), and as `persist` fails;
  it writes nothing. The world is not touched: the save copy is the server's between runs
  and the encode runs beside the next frame.
- `Server::schemas` is `SchemaTable::from_registry(types.registry())` built once at
  `build_server`, so every save and relevance message of the world is written with the
  table the world was built with. `Server::baseline(region)` is the baseline of the last
  bundle the sync point accepted for the region; it is kept across an unload, so the export
  of an unloaded region can still be diffed, and replaced by the next accepted bundle's.
  Milestone 3 replaces the table with the store's answer by hash and the signature is
  unchanged.
- `write_slot(executor, slot, bytes)` is architecture section 15.3's write as one
  background task: a temporary file `<slot file name>.<unique>.tmp` created in the slot's
  directory with `create_new`, `bytes` written in full, the file synced, then renamed over
  `slot`; on any failure the previous save is in place and the temporary is removed
  (best effort). The kinds are the io facade's: `StorageFull`, `NotFound` when the slot's
  directory does not exist, `Io` otherwise. When `lactose-io` lands this function is
  `Io::replace(slot, bytes)` with the same contract, its parameter list gains the `Io`,
  and the save directory is created at startup with `Io::create_dir_all`; nothing else
  changes.
- `load(game, bytes, cooked)` runs on the calling thread before any world exists:
  `game.types()`, `SchemaTable::from_registry`, `game.migrations()`, `Container::decode`
  over the whole file, then for each section in ascending region order
  `Loader::bundle(&records, cooked.baseline(hash)?, RegionMode::Simulated, table)` with
  `table` the region's `Cooked::table` pinned through `Cooked::pin`, or `None`. The first
  error ends the load with nothing built and nothing to undo. The result's `tick` and
  `next_net_id` are the builder's; its `regions` are the first frame's `loaded` ("The timer
  root" above). A save carries no session state, so every player joins again through
  `TickInput::sessions`.
- `Headless::save(slot)` is the three calls in turn from the calling thread, each blocked
  on; a failure at any of them leaves the world as it was and returns the error.

**Relevance** (architecture section 9.3; `crates/net-api/README.md`)

- `Server::relevance(region, assets)` is `World::export_region(region)`, then
  `RegionRecords::from_export(&export, &baseline, &schemas)?.to_relevance(assets)`, on the
  calling thread, between runs, for the one client the spike has. `NotFound` for a region
  that is not loaded or whose baseline was never recorded. Milestone 4 moves the diff to a
  background task and shares one message between the clients a region becomes relevant to
  in the same frame; the signature then takes the executor.

**Shutdown**

- `Headless::shutdown` drops the server (which drops `Physics`, whose backend drops its
  solver pool and joins its threads), then reads `Executor::execution_log` (D112: read,
  not taken), then `Executor::shutdown`, which joins the workers. The binaries do the same
  for every world they hold before their executor. A world dropped while its run is in
  flight is dropped when the run's jobs finish (`crates/sim/README.md`, "Runs"); nothing
  here holds a world across a shutdown.

**Logging** (architecture sections 19 and 23)

- Everything this crate logs is at target `LOG_TARGET`, `"lactose"`: the engine's own
  codename, since the assembly is the engine put together and has none of its own. The
  policy's responses at `error` for `Stop`, `warn` for `Resync` and `Log`; the lifecycle's
  failures at `warn`; a refused `push` at `debug`; a frame's `behind` at `debug`.

**The headless app** (architecture section 2)

- `Headless::start` is `start_executor` then `build_server`; a failure of the second shuts
  the executor down before returning. `Headless::run(ticks, frames)` is the timer root
  above with `frames` wrapped to return `None` once `ticks` ticks have run, so the run ends
  at exactly `ticks`; the `RunReport` counts the ticks, the frames (the last included),
  the frames reported `behind`, and holds `Server::hash` after the last tick. What a frame
  produced is dropped (the gathered frame) or logged at `debug` (requests and exports).
  `Headless::step(frame)` is one `Server::step` from the calling thread. After a run tore
  the world, `server()` is `None`, `tick()` is the tick it was torn at, and `hash`, `step`,
  `run` and `save` fail with `Cancelled`.
- A `Headless` is built on the thread that owns it and every method is called from that
  thread; none is called from a worker (`block_on` on a worker is a debug assertion).

## Not in this crate yet

- The `app-headless` binary itself: `tests/headless.rs`, section 3, is its `main`. It is a
  package `lactose-app-headless` in `crates/app-headless`, listed in `[feature_roots]` of
  `tools/xtask/ci-policy.toml` when it lands, with `panic = "abort"` and the crash-dump
  hook of architecture section 5.1.
- The SOUR test of the assembly: `tests/headless.rs`, section 4, is its glue; it runs once
  the bodies exist, and `lactose-app` joins `[harness]` in the same change.
- The replicated world (`WorldKind::Client`) beside the prediction world: built by
  `build_client` once `lactose-net` provides its `StagedState` hook (milestone 4);
  `Client::push` then pushes into both inboxes and `Client::frame` steps both, the
  replicated world first. `build_client` loses its `entity` parameter when
  `JoinAccepted::entity` lands (proposed with D122).
- `Cooked` is replaced at milestone 3 by `lactose-asset`'s traits: `Cooked::baseline`
  becomes a region-baseline asset read through the catalog by content hash, `Cooked::table`
  becomes `AssetCatalog::resolve` of the region's set, `Cooked::pin` becomes
  `PinnedTable::pin` over the published store. `load`, `build_client` and the fold change
  signature with it, listed then.
- The overload ladder (architecture section 9.7): `FrameOutput::behind` is its input;
  turning `RegionPolicy::activation_radius`, decimation and join refusal are milestone 4's.
- The input task of architecture section 9.2 (the dedicated network thread, the jitter
  buffer, substitution with `PlayerInput::substitute`): `net`'s at milestone 4; the spike
  writes its own and feeds the result through the frame callback.
- `Server::relevance` on a background task and shared between clients; `ServerError` and
  `ClientError` carrying a `Source` of their own.
- `Workers::platform` from the platform crate, the present thread, and everything a
  client binary adds: milestone 2 and 4.
- An edge against the replication gather: `net::gather` is in no `SystemSet`, so a game's
  exclusive `Pass::Gather` system cannot be ordered against it with `Registration::order`
  until `net-api` names a set; no game crate has one at milestone 1.

## Proof for milestone 1

Nothing runs: every deciding body is `todo!()`. The crate passes `cargo fmt`, clippy with
`-D warnings` on every target, `cargo xtask tiers`, `cargo xtask boundary`, `cargo xtask
features` and its doctests. `tests/headless.rs` and `tests/spike.rs` compile the
consumers' code against the surface; their three tests run what is real now: the
constants and `WorldConfig::headless`, the milestone 1 policy table, and a registration as
a plain value. `lib.rs` pins at compile time that `CrateRegistration` and `Game` are
`Copy + Send + Sync + 'static`, that every configuration, world, run future and save future
is `Send`, and that `dyn Cooked` is `Send + Sync`.

What the implementation proves, each a nextest test asserting on counts, orders and hashes
and never on timing, and the SOUR test recording under the runner:

| What | How |
|---|---|
| The registration order | A game of three sketch crates whose functions record their calls: types, setups and systems in the stated order, each kind alike; a failing `types` names its crate |
| A headless run is a function of its frames | `Headless::step` for N ticks and `Headless::run` for N ticks with the same frames hash equal; two executors with different worker counts hash equal; a SOUR run under workers, layout and chaos modes |
| The frame | A frame callback that records the ticks it was asked for sees consecutive ticks; `None` stops the frame at its tick and the passes still run; a zero scale runs no tick and the passes; `behind` is reported when more than the cap is owed |
| Time scale | `set_time_scale` reaches the next step's `TickInput` whichever path runs it, replacing the callback's; the pause and unpause sequence leaves the world's mirror equal to the pacer's scale after the next tick |
| Baselines | A region loaded, unloaded and saved is diffed against the bundle it was loaded with; a refused bundle records nothing; a reload replaces the baseline |
| Save and load | `save_point`, `encode_save`, `write_slot` to a temporary directory, `load`, a `Start::Loaded` headless app stepped K ticks against the original stepped K more: equal hashes; a failed write leaves the previous file; a missing directory is `NotFound` |
| The client's frame | Over the spike's game: a full frame folded loads exactly its regions; a kept failure keeps the client and the next frame reports the resync; `clock` with `snap` resyncs; the history holds the last eight samples |
| The policy | Each source's response, and that a `Stop` reaches the `Result` of the run that failed |
| Shutdown | A chaos run's log is read after the world is dropped and before the executor is shut down; the solver pool's threads have exited |
