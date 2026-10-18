# CURD (`lactose-sim`)

The deterministic simulation: what turns a `World`, a tick `Schedule` and an `Executor` into
a simulation that ticks, hashes, loads and unloads regions, and applies at the input sync
point what enters the boundary.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | inside |
| Unsafe | none, `#![forbid(unsafe_code)]` |

**Status: implemented.** Every behaviour below runs and is tested (the table at the end
names the tests), and the simulation tests record their hashes under SOUR. Nothing is
optimised: the world hash is sequential outside the crate's own pass, and the ECS run is
the naive one. Every sentence under "Behaviour the signatures cannot express" is one a
dependent crate may rely on. Architecture sections 4, 5.1, 6, 8.2, 8.3, 9.1, 9.2, 9.3, 9.7, 11, 14, 15.2, 15.3
and 15.5 are the design; where this file and the architecture disagree, the architecture
wins and this file is wrong. Decision D118 records the choices those sections left open
and what the two design reviews changed. `crates/ecs/README.md` is the storage contract
this crate builds on and never restates: what it says about the input sync point, regions,
snapshots, exports and change tracking holds here unchanged.

Amending a signature is allowed in the change that needs it. Say so in the change description
and list what changed, because `app`, `predict`, `net`, `persist`, `physics-api`, `anim`,
`input` and the game crates are written against this surface. `tests/consumers.rs` and
`tests/sour_glue.rs` are compile-only sketches of what each of them writes: a change that
breaks one is a signature change to list.

## Surface

| Item | Purpose |
|---|---|
| `TypeTable` | Registers every component and resource type once, for the registry and for the world hash; a crate's generated registration function fills it |
| `SimulationBuilder`, `Simulation` | A world of one `WorldKind` with what setup functions insert, its tick schedule, its pass schedules, its hashers and its region lifecycle; built once, run from the timer root |
| `SimulationBuilder::check` | A check over the world's registry and the systems registered for its kind, run at `build` before any schedule is built; its error fails the build |
| `Simulation::step`, `Step`, `TickInput` | One tick from one frame of input, the unit of everything that enters the boundary |
| `Simulation::run_passes`, `PassRun` | A frame's passes between ticks |
| `Simulation::save_point`, `SavePointRun`, `SavePoint` | The save copy of every loaded region at a sync point |
| `RunError` | How a run fails: the error, and the simulation when nothing ran |
| `Stage`, `Pass`, `Registration`, `StageBuilder`, `sets::INPUT` | The four stages of the tick schedule, the two passes, how a registration function adds to them and how `app` orders other crates' sets, and the one set a consumer orders itself after at the sync point |
| `PlayerSample`, `InputApplier` | A player's quantised input for a tick, and the hook that writes the typed component |
| `SessionEvent`, `SessionApplier` | A player joining or leaving, and the hook that spawns, re-claims or despawns the player's entity |
| `StagedState` | The hook `net` and `predict` fill at the sync point |
| `RegionBundle` | What a frame carries for a region that loads |
| `RegionControl`, `RegionAtlas`, `RegionCell`, `Within`, `RegionPolicy`, `StreamingAnchor` | Activation, streaming, release and pinned regions: the server's policy and its inputs |
| `RegionLifecycle`, `RegionRequest`, `RegionExport`, `LoadFailure`, `SessionFailure`, `AssetFailureEvent` | The lifecycle's state and outputs |
| `Hashers`, `Simulation::world_hash`, `Simulation::hash` | The world hash of architecture section 8.3 |
| `RegionDigest`, `DigestRows`, `Differences`, `FieldDifference` | The field-level diff produced on a mismatch |
| `TickDelta`, `CurrentTick`, `TimeScale`, `TICK_RATE_HZ` | Time as the simulation sees it |
| `harness::{ChunkCapacity, layout_mode, run_recorded}` | What a simulation test under SOUR uses |
| `journal::{Journal, JournalFrame, JournalLoad}` | Journal replay's types; the facade names them only |

Hidden plumbing, which the implementation may reshape: the input sync point's systems, in a
private module (`sim::begin_tick`, `sim::apply_frame_regions`, `sim::apply_sessions`,
`sim::apply_inputs`, `sim::apply_staged_state`, `sim::apply_region_policy`, and the save
pass's `sim::save_copy`), and the three private service resources they take (the frame
slot, the lifecycle slot, the save slot). No consumer can name a system or the lifecycle:
`sets::INPUT` is the only hook, and `Simulation::lifecycle` the only way to the lifecycle.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| `app` (headless, milestone 1) | Build from a registry, a kind, SOUR's layout and chaos, an executor, setup functions and registration functions; order other crates' sets | `TypeTable`; `SimulationBuilder::{new, layout, setup, systems, check, inputs, sessions, staged_state, regions, policy, log_retention, build}`; `Registration::order` from a registration function of `app`'s own; chaos is the executor builder's |
| | A `'static` task owning the world and schedules, paced by `TickPacer`, driving `Schedule::run_tick` | `Simulation` is `Send` and owns both; `step` drives `run_tick`, `run_passes` the passes; the task, every clock and the time scale it paces by are `app`'s ("The timer root" below) |
| | The overload ladder's inputs surfaced, not decided | `TickBudget::behind` from the pacer; `RegionPolicy::activation_radius` is the rung the ladder turns; decimation and join refusal are not here |
| | Pause and time scale as a replicated resource | `TickInput::time_scale`, mirrored into `TimeScale` (replicated) by the sync point |
| | Players joining and leaving as journaled inputs | `TickInput::sessions`, `SessionApplier` |
| Game crates | `TickDelta`, the tick | `Res<TickDelta>`, `Res<CurrentTick>` (or `Ctx::tick`); the time scale is not theirs to read |
| | Configuration resources; world-global persistent state | A setup function inserts configuration, `derived`; world-global persistent state is a region resource of a pinned region |
| | Add sets to the simulation stage without owning the stage list | `Registration::stage(Stage::Simulation).sets(..).add(..)`; the stages are `Input`, `Simulation`, `Physics`, `Post` |
| `physics-api`, `anim` | A place for the sync, the step and the write-back, and for what reads them; the solver pool | `Stage::Physics` for the exclusive sync at its sync point and the once-per-tick step with `Job`; `Stage::Post`, whose sync point runs the exclusive write-back first, for readers of stepped rows; `Stage::Simulation` for the sim skeleton's state machines; a setup function hands in the backend's service with the pool `app` built outside the boundary |
| `input` | How a per-tick batch of inputs reaches the sync point; where the typed write happens | `TickInput::inputs` as `PlayerSample` bytes by player; `InputApplier` beside the component's definition |
| `net-api`, `net` | Apply staged state at the sync point; the gather as a pass; spawn with the server's key and bind; sessions | `StagedState` on a client; `net-api`'s `net::gather` system, placed by `lactose_net_api::register_systems`, copies the rows changed since its `ChangeMark` and the structural log since its `LogCursor` into `GatherState`'s frame; `&mut World` inside the hook; `TickInput::sessions` on the server |
| `predict` | The deterministic step to re-run; snapshot, restore, horizon; the proxy update at the sync point | `step` with the ring's `TickInput`s; `world_mut()` between runs; `StagedState` in the prediction world; `StreamingAnchor` on the local player's entity; `app` runs `run_passes` on the prediction world too |
| `persist` | Save at a sync point into records; load through the one path an unload uses | `save_point` → `SavePoint { tick, next_net_id, regions }`; `SimulationBuilder::{tick, next_net_id}` plus `TickInput::loaded`; `RegionLifecycle::take_exports` on unload |
| Region lifecycle | Policy inputs and outputs as data, testable without `net` or `asset` | Inputs: `StreamingAnchor` rows, joins, `RegionPolicy`, `RegionAtlas`, pinned regions, `TickInput::{loaded, assets}`; outputs: `RegionLifecycle::{take_requests, take_exports, take_load_failures, take_session_failures, take_asset_failures}` and the modes on the world |
| The world hash | `world_hash(region) -> u64`, per-type hashers from the erased registry, a diff on mismatch | `Simulation::world_hash`, `Hashers` filled by `TypeTable::register`, `RegionDigest::diff` |
| SOUR | Build from the session, run ticks, record a hash per tick, write the chaos log | `harness::layout_mode`, `harness::run_recorded`, `Simulation::hash`; the glue is `tests/sour_glue.rs` |
| Journal replay | A place for the types | `journal` |

## The registration a game crate writes

Types, resources and systems are registered by three functions, so that the registry and
the hash table cannot disagree, every resource a system names exists before the schedules
prepare, and no system is added that its types do not support:

```rust
use lactose_core::Result;
use lactose_ecs::{SystemSet, World};
use lactose_sim::{Registration, Stage, TypeTable};

pub static MOVEMENT: SystemSet = SystemSet::new("game_sim::movement");

/// Every component and resource of this crate, through `TypeTable::register::<T>()`.
pub fn register_types(types: &mut TypeTable) -> Result<()> {
    // types.register::<Health>()?; types.register::<Gravity>()?; ...
    Ok(())
}

/// Every resource and service a system of this crate names; world resources are `derived`.
pub fn setup(world: &mut World) -> Result<()> {
    // world.insert_resource(Gravity(..)); (the physics resource is `app`'s:
    // `lactose_physics_api::setup` in `app`'s setup closure)
    Ok(())
}

/// Every system of this crate, placed in its stage.
pub fn register_systems(registration: &mut Registration) {
    registration.stage(Stage::Simulation).sets(&[&MOVEMENT]);
    // registration.stage(Stage::Simulation).add(&APPLY_GRAVITY).add(&SWORD_HIT); ...
}
```

`app` calls every crate's `register_types` on one `TypeTable`, hands it to
`SimulationBuilder::new`, and adds every crate's `setup` with `SimulationBuilder::setup`
and `register_systems` with `SimulationBuilder::systems`, then one registration function of
its own for the `Registration::order` edges between crates' sets. A system whose `worlds`
exclude the builder's kind is skipped by `StageBuilder::add`, so one set of functions
serves the server, the client and the prediction world; a function that must differ by
kind reads `Registration::kind`.

## Behaviour the signatures cannot express

**Time**

- `TICK_RATE_HZ` is 60 and `TickDelta` is `Scalar::ONE / Scalar::from_u32(60)`, inserted by
  the builder in every world; the same bits on every target. `TickDelta` is never changed:
  time scale changes how often ticks run, never the delta.
- The time scale is `app`'s decision on the server and enters the world as an input:
  `TickInput::time_scale`, which `sim::begin_tick` writes into the `TimeScale` resource, so
  it is journaled and `lactose-predict`'s ring carries it. The world's `TimeScale` is a
  mirror: the pacer's scale is `app`'s own copy (on the server its decision, on a client
  the latest value `net` received), never read back from the world, and no game system
  reads it, since a paused world runs no system that could unpause it and the scale is not
  simulation state. The resource is `replicated` so that `net` carries it to clients, and
  `derived`. Because clients learn the scale from the world's mirror, a pause is entered
  by running one tick whose frame carries `time_scale: Some(PAUSED)` and then setting the
  server's pacer to zero, and left by setting the pacer first and putting `Some(scale)` in
  the next frame. `app` gives the pacer `f64::from(thousandths) / 1000.0`, outside the
  boundary; `sim` computes no float from it.
- `CurrentTick` is rewritten by `sim::begin_tick` from `World::tick` at every input sync
  point and read through `CurrentTick::get`; a write (a decoded value, or `world_mut`) is a
  bug the type cannot prevent, bounded to one tick. Between runs it equals
  `Simulation::tick`; `World::tick` is the source of truth.
- Nothing in this crate reads a clock. A step takes ticks, not time: the timer root owns
  `Instant`, `TickPacer` and the catch-up cap; `step` and `run_passes` are the deterministic
  bodies it calls.

**The timer root** (architecture sections 5.1 and 11) is `app`'s task, in outline
(`tests/consumers.rs` compiles it against `TickPacer`):

```rust
let period = Duration::from_secs(1) / TICK_RATE_HZ;
let mut pacer = TickPacer::new(Instant::now(), period, 4);
loop {
    let budget = pacer.poll(Instant::now()); // re-simulation does not share the cap (D122)
    for _ in 0..budget.ticks {
        simulation = simulation.step(&executor, next_frame()).await?;
    }
    simulation = simulation.run_passes(&executor).await?;   // once per frame, paused or not
    if budget.behind { /* server: climb the ladder; client: resync (section 11) */ }
    pacer.set_time_scale(f64::from(time_scale.thousandths) / 1000.0); // app's own value
    executor.sleep_until(budget.next_deadline).await?;
}
```

**Runs**

Every run takes the simulation by value and hands it back when it resolves, as
`Schedule::run_tick` does with the world, because the run shares the world with jobs on
other workers. A run that fails before anything ran hands the simulation back inside the
`RunError` (`simulation: Some`): the kind check of `save_point`, and `Schedule::prepare`,
which every run performs first (and `run_passes` for both passes before either runs) and
which a resource or service removed through `world_mut()` or by a hook can fail. Two
failures after something ran hand it back too, since the world survives them: `Gather`
prepared again after `Extract` ran (an extract system can remove what a gather system
names), the simulation then holding `Extract`'s work and not `Gather`'s; and a
`save_point` whose export failed, which changed nothing. After `Cancelled`, the executor
having shut down mid-run, the world is torn and `simulation` is `None`. A `RunError` never hands back the `TickInput`: a caller that retries keeps a clone
of the frame. `RunError` converts into `lactose_core::Error`, so `?` into a plain `Result`
drops the simulation as a plain error would; a caller that wants to keep it matches. Every
schedule is built and prepared at `build`, so a `ScheduleError` is a build error of kind
`InvalidData` carrying it, never a run error, unless the world was changed since.

`SimulationBuilder::check(f)` (D122) adds a check that `build` runs after every setup and
registration function and before any schedule is built or prepared. Each check, in the
order added, is called once with the world's registry and every system descriptor added
for the builder's kind, in the order added: this crate's six input systems first, then
those of the registration functions in the order their calls added them, across stages
and passes, then `sim::save_copy` on a server. The first error fails the build as
returned and the later checks do not run. `app` hands
`lactose_predict::check_prediction_reads` to the prediction world's builder. The six
exclusive systems of the input sync point are listed by name in
`lactose_predict::EXCLUSIVE_SYSTEMS`; renaming one changes that list in the same commit.

`Simulation::step(executor, input)` does the following, in this order, and nothing else:

1. On a `Server` or `Client` world, `World::set_rollback_horizon(tick)` with the current
   tick, since neither rolls back; the prediction world's horizon is `lactose-predict`'s.
2. Puts `input` where the sync point systems take it, and runs the tick schedule with
   `Schedule::run_tick`. The first stage's sync point is the input sync point: the ECS
   zeroes transient columns and clears events and input ticks (every channel is already
   empty, cleared at the previous run's closing sync point, D126), then the exclusive
   systems of `Stage::Input` run, this crate's six first (below), then every other
   exclusive system placed there, then the stages' systems. The tick advances by one.
3. Trims the structural change log of the entries older than the builder's
   `log_retention` (120 ticks by default): the step keeps a ring of one
   `StructuralLog::cursor` per step, seeded with the log's end at `build` so the first
   steps' entries are kept like every later step's, and trims to the oldest with
   `World::trim_structural_log`, which ignores a cursor made stale by a restore, a load or
   the stamp rebase, so those heal on their own. A consumer whose `LogCursor` has fallen
   further behind gets `None` from `StructuralLog::since` and does a full pass; no consumer
   trims the log itself. The retention must be at least the timer root's catch-up cap: a frame
   that runs several ticks trims before its passes run, so a consumer that reads the log once
   per frame (`net-api`'s replication gather) finds its cursor stale at every gather after a
   catch-up frame if the retention is below the cap. `build` cannot check it, since the cap is
   `app`'s.
4. On a `Server` world, takes and drops the presentation events, so the queue does not
   grow (architecture section 9.4) whatever drives the steps; every other kind leaves them
   for `predict` or presentation.
5. Resolves with the simulation at the new tick.

`Simulation::run_passes(executor)` prepares the `Pass::Extract` and `Pass::Gather`
schedules, then runs each with `Schedule::run_pass`, in that order and only if a system
was registered for it (the tick does not advance, nothing is zeroed and no event is cleared
at its first sync point, and its closing sync point clears every channel, as every run's
does). The timer root
calls it once per frame after the frame's ticks, paused or not, on the server and on both
client worlds: what was edited between ticks is still extracted and replicated, and the
prediction world's local player is extracted every frame (section 9.4). A gather therefore
covers every tick since the last frame, up to the catch-up cap, and `net`'s staging buffer
is per frame rather than per tick. `lactose-predict`'s re-simulation calls `step` alone, so
extract never runs against a discarded tick.

- The futures are `Send`; they are driven by the task that owns the simulation. A future
  dropped mid-run drops the simulation when the run's jobs finish.
- A `Client` world runs this crate's systems and nothing of the game (architecture section
  5.3): its step is the sync point, the empty stages and nothing else. A `Prediction` world
  runs the systems that name `prediction`.
- A world has no region until a bundle loads. A region created directly through
  `world_mut()` is an unjournaled edit: tests only, never in `app`. Everything written
  through `world_mut()` is unjournaled; `app` changes the `RegionPolicy` there and nothing
  else of the world's state, and moves `net`'s frames through
  `lactose_net_api::{StagedInbox, GatherState}`, which are service resources and not world
  state.

**The input sync point** (architecture sections 6, 9.1, 9.2 and 14)

This crate's six exclusive systems are in `sets::INPUT`, placed first in `Stage::Input`,
and ordered among themselves by `after` edges. Their order is fixed in every world:

1. `sim::begin_tick`: `CurrentTick` takes the world's tick; `TimeScale` takes the frame's
   `time_scale` when it is `Some`.
2. `sim::apply_frame_regions`: the frame's `failures`, then under `Directed` its `unloaded`,
   then `loaded`, `assets` and, under `Directed`, `modes`. "Regions" below gives the rules.
3. `sim::apply_sessions`: on a `Server` world, each held `Join` whose region is now loaded
   and each entry of `TickInput::sessions`, in ascending `PlayerId` order, through
   `SessionApplier::apply`; on any other kind the frame's sessions are dropped and logged,
   since a client learns of players through staged state.
4. `sim::apply_inputs`: for each entry of `TickInput::inputs` in ascending `PlayerId`
   order, `World::set_input_tick(player, input_tick)`, then `InputApplier::apply`. Without
   an applier (the builder's default) every input is dropped and logged, after its input
   tick is set.
5. `sim::apply_staged_state`: `StagedState::apply(world, tick)`. The default does nothing.
   A `Prediction` world's `dyn StagedState` is replaced after build by
   `lactose_predict::Predictor::new`, which owns the simulation from then on and reinstalls
   the hook at `resync` (D122).
6. `sim::apply_region_policy`: under `Anchored`, the policy ("Regions" below); under
   `Directed`, nothing. Last, so it sees the sessions' spawns.

A crate that must act at the sync point after all of that places an exclusive system in
`Stage::Input` in a set declared `.after(&[&lactose_sim::sets::INPUT])`. Nothing can be
ordered before `sets::INPUT`: two unordered exclusive systems in one stage are a schedule
error, this crate's edges already order its own, and a `Registration::order` edge whose
`after` is `sets::INPUT` is a `ScheduleError::Cycle` at `build`. Two crates that both add exclusive
systems to one pass, or after `sets::INPUT`, do not name each other: `app` orders their
sets through `Registration::order(before, after)`, which adds the edge to the schedule
that declares the sets (the tick schedule across any of its stages, or the one pass that
holds both); a set no schedule declares, or two sets declared in different schedules, is
`ScheduleError::UnknownOrderedSet`, an edge against the stage order a `Cycle`, both at
`build`. A pass none of whose systems runs in the world's kind does not exist in that
world, and an edge between two of its sets orders nothing there, so one ordering function
serves every kind. An error from any hook is logged at target `curd` and that entry is
skipped; the tick runs.

A `TickInput` is applied field by field in the order its fields are declared, each map in
ascending key order. Every keyed field is a map, so a frame cannot hold two entries for one
key; `failures` is a list applied in frame order and is not state. The frame for tick *T*
is the one passed to the step that runs *T*; `lactose-predict` re-simulates by passing the
ring's frames again, in order, and a replay passes the journal's.

What `SessionApplier::apply` must do, on the server: for `Join { region }`, spawn the
player's entity in `region` through `World::spawn` with a key from
`World::allocate_net_id`, or re-claim the entity the game keeps for the player across saves
(found by a persistent component of the game's, such as a saved player identity), in
either case adding `ClaimedBy { player }` and `StreamingAnchor` among the game's
components; for `Leave`, despawn that entity, or remove the claim and the anchor and leave
the game's row for the next join, resolving it from world state: the row with `ClaimedBy`
for the player that also carries `StreamingAnchor`, never a side table. A save carries no
session state, so on a load every player joins again. A `Join` is applied at the first
sync point at which its region is loaded, which may be later than its frame: until then
the lifecycle holds it and its region counts as the player's position for activation. A
`Join` for a region the atlas lacks, under `Anchored`, is refused and reported as a
`SessionFailure` of kind `NotFound` (`take_session_failures`), neither held nor applied. A
`Leave` is applied at once, and a `Leave` for a held `Join` cancels it and is all of the
leave: the join never reached the world, so the applier is not called. A newer `Join`
replaces a held one. Held joins are
journaled, since they came through a frame, and not snapshotted, which is fine on the
server, the only kind that honours them.

What `InputApplier::apply` must do, beside the component `lactose-input` defines: decode
`sample.sample` with the component's `CheckedBitPattern`, resolve the player's entity from
world state as the session applier does, and write the value with `World::insert` or
`World::get_mut`. In the prediction world that lookup resolves because `lactose-predict`
writes `StreamingAnchor` on the local player's entity beside its claim, where under
`Directed` it has no policy effect. The component is `transient`: the input sync point
zeroes it, then the applier writes it, and the applier's crate adds an exclusive system
of its own in a set after `sets::INPUT` that declares `writes = [Sample]`, so that its
readers have a declared writer. It is never `derived`: a derived value is neither hashed
nor saved, so a stale sample (a player who left while the game kept the row) would drive
the simulation from state no save, journal or hash holds. The same applier serves the
server, the prediction world and a replay.

What `StagedState::apply` may do: on a client, everything received, through `World::spawn`
with the server's key, `World::bind`, `World::insert`, `World::columns_mut` or
`Mut::set_if_neq` so that an equal value does not dirty a row, `World::move_to_region` for an
entity the server moved between regions, `World::remove` for a component the server removed,
and `World::despawn` for a tombstone; in the prediction world, the proxies' positions for the
tick from their per-tick history. On the server it is the default and does nothing.

**Regions** (architecture sections 6, 9.7 and 14)

`sim::apply_frame_regions` applies, in this order, logging every refusal at target `curd`:

1. Each `TickInput::failures` entry becomes an `AssetFailureEvent { tick, failure }` in the
   lifecycle, in frame order, for `app` and `net` to take: the tick-stamped event of
   section 14. The lifecycle does nothing else with it; policy is outside.
2. Under `RegionControl::Directed`, each `unloaded` region, ascending, is unloaded as
   described below if loaded, else logged; before this frame's bundles, so a resync that
   replaces a region takes one frame. Under `Anchored` the list is ignored and logged.
3. Each `TickInput::loaded` bundle, ascending by region. It is refused with a `LoadFailure`,
   and nothing changes, if the region is loaded already (`InvalidData`); under `Anchored`,
   if the region is absent from the atlas (`NotFound`) or neither pinned, nor requested,
   nor within the release radius of an anchor, this frame's `Join` regions counted as
   anchors (`InvalidData`, "not wanted"); if the overlay was made against another baseline
   (`VersionMismatch`, `Overlay::check_against`); or if a resource is not a registered
   persistent resource whose bytes validate, or a transient key is not a registered
   transient resource (`VersionMismatch` for a schema the registry disagrees with,
   `InvalidData` otherwise). Everything is checked before `World::create_region`, and
   `World::load_region` validates before it mutates, so a refused bundle leaves no row, no
   log entry and no bind. Accepted, it is `create_region` with the bundle's seed and, under
   `Directed`, its mode (under `Anchored`, `Frozen`); `load_region(baseline, overlay)`;
   `insert_region_resource_erased` for each resource, `insert_region_resource_zeroed` for
   each transient key, and `set_region_assets` for the bundle's table if it has one. Under
   `Anchored` the region is then no longer requested.
4. Each `TickInput::assets` table, ascending by region: `World::set_region_assets` if the
   region is loaded after step 3, replacing the one it has (a hot reload, tick-stamped by
   the frame; the mode does not change for it); otherwise dropped and logged. Nothing
   waits for a region: a bundle brings its first table in `RegionBundle::assets`.
5. Under `Directed`, each `modes` entry, ascending, is applied with `World::set_region_mode`
   if the region is loaded, else logged. Under `Anchored` the list is ignored and logged,
   and so is a bundle's `mode`.

`sim::apply_region_policy` does nothing under `Directed`. Under
`RegionControl::Anchored { atlas, pinned }`:

- The radii are read from the `RegionPolicy` resource and clamped: the streaming radius to
  at least one, the activation radius to at most `streaming_radius - 1`, the release radius
  to at least `streaming_radius`, each clamp logged. `build` requires `release_radius >= streaming_radius > activation_radius`
  and `streaming_radius >= 1`; the clamp keeps the invariant when the overload ladder turns
  `activation_radius` at run time.
- The anchors are the regions of every row that has `StreamingAnchor`, whatever the
  region's mode, plus the region of every held `Join`. The active set is every region of
  the atlas within the activation radius of an anchor's cell, the wanted set every region
  within the streaming radius, the kept set every region within the release radius, each
  by `RegionCell::distance` (the larger axis distance) through `RegionAtlas::within`; every
  pinned region is in all three sets, whatever the anchors. The policy's cost follows the
  rings, not the atlas: each set costs the cells within its radius of each anchor, and the
  loaded set is read once per sync point.
- Each loaded region not in the kept set, ascending, is unloaded (below). Each requested
  region not in the kept set, ascending, is no longer requested and gets a
  `RegionRequest::Cancel`; a bundle for it that still arrives is refused. Each wanted
  region not loaded and not requested, ascending, gets a `RegionRequest::Load` and is
  requested until its bundle loads or the request is cancelled; so every pinned region is
  requested at the first sync point and never released.
- Each loaded region, ascending, is set `Simulated` if it is in the active set and
  `World::region_assets` is `Some`, else `Frozen`. So a region enters `Simulated` only
  when its bundle is loaded and its asset table has entered, and leaves it at the sync
  point after its last anchor moves out of the activation radius; a pinned region is
  `Simulated` as soon as its table has entered. The gap between the streaming and release
  radii is the lifecycle's hysteresis: an anchor jittering on a border loads and unloads
  nothing. The overload ladder's own hysteresis is `app`'s, on `activation_radius`.
- World-global persistent state, which belongs to no player's surroundings (a world-wide
  quest flag, an economy), is a region resource of a pinned region: hashed and saved with
  it, and simulated whenever the server runs.
- Because the streaming radius exceeds the activation radius, an entity in a `Simulated`
  region can only cross into a loaded region, where the ECS freezes it with the region
  (section 9.7). A command that names an unloaded region is dropped and logged by the ECS
  (D117); a teleport or respawn into an unloaded region is a two-phase operation the game
  performs ("Not in this crate yet").
- With no anchor, no held join and no pinned region, every loaded region is unloaded and
  nothing is requested, so a server loads its first region only once a player joins: the
  headless app and tests use `Directed`, a `Join`, or a pinned region.

An unload, under either control: on a `Server` world, `World::export_region` into a
`RegionExport` (every row is bound at apply on the server, so the export cannot fail),
then `World::unload_region`, then a `RegionRequest::Release`; a region the same frame
unloads and loads again (a one-frame resync under `Directed`) is replaced, and its
`Release` is withdrawn once the bundle loads. On a world that is not `Server` an unload
does not export, and `take_exports` is always empty there.

- The lifecycle's own state (what is requested, held joins, its outputs) is server-only
  session state, which the server never rolls back; it is not in a snapshot, and
  architecture section 10's list of what a restore keeps is unchanged. A journal records
  it with its checkpoint, since a save does not. A prediction world under `Directed`
  rebuilds what it loaded from the ring's frames; its lifecycle holds asset failures
  alone, for `lactose-predict` to take. A frame's `failures` are stamped each time the
  frame is applied, so `predict` strips `failures` from the frames it re-simulates: the
  first application stamped them.
- An unload's `RegionExport` is what a save of that region would hold: rows in ascending
  `NetId` order, reference fields as stored, the seed and the persistent resources, with the
  transient components and resources listed by key. `lactose-persist` diffs it against the
  baseline into the overlay of section 15.2 and keeps the result as the region's records
  until the region loads again. The overlay is also what section 9.3 sends a client on
  relevance: the server's `app` runs `persist`'s diff on the export and `net` sends the
  overlay; the client builds a `RegionBundle` from it without diffing.
- `RegionAtlas::insert` refuses a region or a cell already placed; `within` yields the
  regions within the radius in ascending `RegionId` order, the centre's region included,
  at a cost of the smaller of the atlas and the (2r + 1)² cells within radius r.

**Saves and loads** (architecture sections 15.2 and 15.3)

- `save_point` runs the save pass, whose one system `sim::save_copy` forks one child per
  loaded region over `World::export_region` and collects the results in ascending
  `RegionId` order into `SavePoint { tick, next_net_id, regions }`; the simulation comes
  back unchanged with it, the tick does not advance. It is the same export an unload makes.
  World resources are not in it ("The world hash" below says why), and neither is session
  state: a save carries no players, held joins or requests, so on a load every player
  joins again. On a world that is not `Server` the run resolves at once to `Unsupported`
  with the simulation inside: a client never exports, and single-player runs a `Server`
  world in-process with a local client. An export that fails (a row without a bound
  `NetId`, which only a direct `world_mut()` spawn can leave on a server) fails the run with
  that error and the simulation inside, unchanged, since an export reads and writes
  nothing. An unload that cannot export keeps the region loaded and logs the error rather
  than lose its state.
- A load is `SimulationBuilder::tick(saved.tick).next_net_id(saved.next_net_id)` and then,
  for each saved region, a `RegionBundle` in a frame, under `Directed` or when the policy
  wants it. A cooked region's persistent region resources come from the caller in
  `RegionBundle::resources`: empty until the bundle format carries them
  (`docs/verification.md`, the seed-and-resources-beside-the-overlay item). The ECS stamps
  loaded rows at the current point and logs them as spawned; no epoch starts. Change
  consumers see a loaded region as new rows.

**The world hash** (architecture section 8.3)

The hash covers every persistent and transient column of the region and every persistent
and transient region resource, and the region's seed, mode and asset table; derived columns
and resources are excluded, and so is everything outside the region (world resources, the
structural log, change stamps, the entity table). World resources are therefore neither
hashed nor saved: simulation state that is hashed or saved is a component or a region
resource, world-global persistent state a resource of a pinned region, and a world
resource is configuration or session state. `build` enforces it by refusing, with
`InvalidData`, any world resource present after the setup functions whose policy is not
`derived`, probing every resource type the `TypeTable` registered; the check runs at build
and covers the types the table knows, so a resource of an unregistered type, or one
inserted after build by a hook or an exclusive system, is the inserting crate's bug. The
6.1 example's `Gravity` and `TotalMass` are therefore `derived`.

The hash is a pure function of that state: identical under every chunk layout, worker
count, archetype history and registration order, on x86-64 and aarch64. The encoding, bit
for bit:

- *Key bytes*: a row's origin key as the sixteen memory bytes of `EntityRef::new(key)`
  (tag `u32` little-endian, 1 `Net` or 2 `Predicted`, then three `u32` words); a region
  resource uses the sixteen zero bytes of `EntityRef::NONE`. A placeholder never reaches
  storage.
- *Leaf bytes*: `Reflect::visit` under the hashing visitor appends, to the stream of the
  top-level field ID each leaf is reported under: `bytes` verbatim (integers, `bool`, enums
  and the identity newtypes are little-endian memory bytes on every supported target); a
  `scalar` as its `canonical_bits()` as a `u64` little-endian, so every NaN is one value and
  negative zero keeps its sign; an `entity_ref` as its sixteen bytes; an `asset_ref` as its
  `asset_id()` as a `u64` little-endian. A raw `f32` or `f64` never appears: registration
  refuses it. The streams are built in a scratch buffer the hasher is handed, so a row
  allocates nothing.
- *Field hash*: `xxh64` with the field ID as the seed over that field's stream, for every
  top-level field with an ID in declaration order, its stream possibly empty.
- *Row hash*: `xxh64` with the component key as the seed over the key bytes, then every
  field's ID as a `u16` little-endian and its field hash as a `u64` little-endian. A type
  with no field with an ID hashes its key bytes alone, so its presence counts.
- *Column sum*: the wrapping sum of the row hashes of every row of the region that has the
  component, with the row count. Addition is the order-independent combiner: no sort.
- *Resource hash*: the row hash of the resource's value under the `NONE` key bytes.
- *Region hash*: `xxh64` with the region ID as the seed over: the seed as a `u64`; the mode
  as a `u8` (0 `Simulated`, 1 `Replicated`, 2 `Frozen`); the asset table as a `u8` presence
  flag, then if present its entry count as a `u32` and each `(AssetId, AssetVersion)` as
  two `u64`s in ascending ID order; then every hashed component some row of the region has,
  in ascending component key order, as its key, its row count as a `u32` and its column
  sum; then every hashed resource of the region in ascending key order, as its key and its
  resource hash. Everything little-endian. A component no row has is absent, so the hash
  does not depend on which other types are registered.
- *Simulation hash* (`Simulation::hash`): `xxh64` with seed zero over the current tick as
  a `u32` and, for every loaded region in ascending ID order, its ID and its region hash as
  `u64`s. A world with no region hashes the tick alone. This is what a SOUR test records.
- Transient columns hash whatever the tick left in them; they are zeroed at the next input
  sync point. Their presence is part of the archetype and so of the hash, as the ECS
  README says of exports.
- `TypeTable::register::<T>` builds a hasher for a persistent or transient `T` by
  monomorphising `T::visit` under the visitor, and none for a derived `T`. It refuses,
  with `Unsupported`, a hashed `T` whose layout holds `FieldKind::Float` anywhere, or a
  `FieldKind::Scalar` field whose type tag is not `lactose_math::Scalar`'s `Field::NAME`;
  the visitor's `scalar<S>` additionally asserts that `S` is `lactose_math::Scalar` by
  `TypeId`, which a wrong tag cannot pass. The hasher trusts that one scalar type and no
  other (`lactose_reflect::ScalarField`).
- `RegionDigest` holds every hashed row and resource by (origin key, component key) with
  its field hashes in field order, an empty list standing for a row whose type has no
  field with an ID, and the seed, mode and asset table; `RegionDigest::hash` recomputes the
  region hash from them and equals `world_hash` of the same state, so a digest explains the
  hash it stands for. `diff` yields every row present on one side only (`field: None`) and
  every field whose hash differs or exists on one side only, ascending; `encode` is one
  canonical byte encoding and `decode` accepts exactly it, so a digest can cross machines.
- `world_hash` fails with `NotFound` for a region that is not loaded, and with
  `Unsupported` for a hashed column of a type the hashers do not cover, which cannot happen
  in a world built by the `TypeTable` they came from.

**SOUR** (architecture section 8.3; `tools/xtask/src/harness/protocol.rs`)

- `harness::layout_mode(capacity, seed)` is the layout mode: `permute_rows` and
  `split_batches` on, drawn from the seed, and the chunk iteration order chosen by the
  capacity so that the matrix covers both orders whatever seeds the runner hands out
  (today one fixed seed per variant): `ChunkOrder::Reversed` for `ChunkCapacity::Rows(7)`
  and `Bytes`, `ChunkOrder::Shuffled` by the seed for every other `Rows(n)`, so `Rows(1)`
  and `Rows(64)`. `Rows(n)` sets `chunk_rows`, `Bytes(16384)` is the production capacity
  under the same perturbations, `Production` is `LayoutConfig::default()` whatever the
  seed, since the runner's production variant means the builder's defaults. Any other byte
  capacity is `Unsupported`, since the ECS has one slab size: a test under a capacity the
  storage cannot apply fails rather than running something else. Chaos mode is the
  executor builder's `ChaosConfig` with `shuffle_steals`, `inject_yields`, `force_split`
  and `record` on and the seed; that type is `lactose-task`'s, so the test's glue builds
  it.
- `harness::run_recorded` steps once per frame, records `(tick, Simulation::hash())` after
  every step, runs no pass (a `Server` world's presentation events are drained by the step
  itself), and hands the simulation back; it uses `Executor::block_on`, so it is called
  from the test's main thread.
- The glue between the runner's `Session` and this crate is `tests/sour_glue.rs`, compiled
  against the real protocol module and the real executor builder. Until the protocol
  module moves into a foundation crate outside the boundary, a test crate includes it with
  `#[path = "../../../tools/xtask/src/harness/protocol.rs"] mod protocol;` (its unit tests
  come along and run there too) and lists `lactose-task` under `[dev-dependencies]`, which
  the boundary allowlist permits; once it moves, the glue lives in one test-support crate
  outside the boundary, `lactose-sour`, that every simulation test crate dev-depends on,
  so the `#[expect]`s below live in one place. The `applied` line reports the worker count
  from `Executor::frame_worker_count`, the layout from `World::layout` (`chunk_rows:
  Some(n)` as `rows:n:seed`, `None` with permutation as `bytes:16384:seed`, the default as
  `production`), and the chaos seed from `Executor::chaos_config`, so all three are what
  the builders report rather than what the session asked for. `tests/sour.rs` is the glue
  as the recording tests use it.
- Every simulation test of this crate and of the game crates records this way; the package
  with such tests is listed under `[harness]` in `tools/xtask/ci-policy.toml`. Until
  `lactose-app-headless` exists, whose test target is the SOUR test crate of section 8.3,
  that package is `lactose-sim`.
- The boundary lint file bans, on the re-exported `Executor`, what a boundary crate must
  not do with it: `frame_worker_count` and `slot_count` (worker identity), `spawn`,
  `run_graph` and `platform` (arbitrary work and threads), `sleep_until` (a clock),
  `execution_log` (scheduling history) and `chaos_config` (scheduling configuration). What
  a boundary crate can do with it is run a
  schedule, block on a future, and build or shut down an executor. The glue and the
  timer-root sketch opt out item by item with `#[expect]`, since they are `app`'s and the
  test's code, outside the step. The physics backend's solver pool, which section 16 sizes
  by the worker count, is built by `app` outside the boundary and handed to the backend
  through a setup function.

**Journal replay** (architecture section 15.5)

`journal::Journal` is a `SavePoint`, what a save does not carry (the asset tables and modes
in force at the checkpoint, every joined player's claimed and anchored entity, the held
joins and the requested regions, which the replay driver restores before its first step;
and `services`, a map from a service key (the xxh64 of the service's declared name,
collision-checked as a system key is) to the opaque bytes `app` took from that service at
the checkpoint, which the replay driver hands back after the regions load and before the
first step: the physics snapshot of `lactose-physics-api` goes there, the backend's whole
world and the sync's pending work, since the solver holds state the rows do not carry and a
replay without it hashes equal only until the first contact; this crate never reads the bytes and they are not hashed state; until the byte form
exists, D124, a replay is bit-exact from a session's start only),
and one `JournalFrame` per tick after it; `JournalFrame::record(tick, &input)` is the
frame's record, mirroring `TickInput` field for field in its application order, with
tables as `VersionTable`s and bundles as `JournalLoad` (the baseline by content hash, with
the overlay, resources, transient keys and table the bundle carried), so every region and
every player the replay runs can be rebuilt. Before its first step the replay driver
loads the checkpoint's regions with their tables and modes, re-claims the players'
entities, and hands the held joins and requested regions to
`RegionLifecycle::restore_session_state` through `Simulation::lifecycle_mut`, called once,
before the first step, with the state a journal recorded; state that contradicts the world
(a requested region that is loaded, a held join outside the atlas) is not checked. Writing,
reading and replaying a journal is the milestone 1 tooling's; this crate names the shape
and the restore so `app` and the tools agree, and `tests/persist.rs` replays one from
checkpoints across a run. A journal replays on the build that wrote it: the checkpoint records
no schema, and the protocol hash of section 9.1 is the only guard against a sample image from
another build.

## Not in this crate yet

- The overload ladder itself: `app` climbs it on `TickBudget::behind` and turns
  `RegionPolicy::activation_radius`; decimation by tick parity is an ECS attribute flag
  when first needed (`docs/verification.md`); join refusal is `net`'s.
- A teleport or respawn into an unloaded region: a two-phase operation the game performs,
  loading ahead (a spawned anchor, a held `Join` or a pinned region brings the region in)
  and moving once it is loaded; a move commanded before that is dropped and logged by the
  ECS (D117). Proposed `docs/verification.md` item: "Teleport and respawn across the
  streaming ring are two-phase in the game; decide with `game-sim` whether `sim` needs a
  frame-level request for a region a game system wants ahead of any anchor (milestone 3)".
- Section 14's hold of a player at a region boundary until the client acknowledges
  readiness: `net`'s at milestone 4, with no field here.
- Hashing `physics-api::snapshot` bytes (architecture section 8.3), added once Rapier's
  serialisation is shown order-stable.
- The journal writer, reader and replay driver, and the digest tool that compares two
  digests across machines; the types are here.
- Per-anchor streaming priority (`StreamingAnchor::priority` is zero) and streaming
  budgets, milestone 3.

## Proof for milestone 1

The tests in `tests/`, each a nextest test that asserts on counts, orders and hashes and
never on timing. `tests/sour.rs` records under SOUR, which runs the package in workers,
repeat, layout and chaos modes; `tests/common/mod.rs` is a small game written as a game
crate writes it, the hooks `app`, `input` and `net` write, and a driver that answers the
lifecycle as `app` and `persist` would.

| What | How |
|---|---|
| A step is a function of its frames | `layout.rs`: two simulations stepped with equal frames hash equal every tick; `sync_point.rs`: a frame assembled in reversed order is one input |
| The sync point order | `sync_point.rs`: test hooks record the calls: regions, then sessions, then inputs, then staged state, then the policy; players ascend, regions ascend; a frame's fields apply in declaration order; a held `Join` is applied the sync point its region loads; a `Join` and its region's bundle in one frame load and apply in that frame |
| The hash encoding | `hash.rs`: golden hashes for a small registry, pinned and checked against a reimplementation over another xxh64; layout mode and registration order change nothing (`layout.rs`: nor worker count); a derived column and a world resource change nothing; a transient value, a region resource, the seed, the mode, the table and a marker component's presence each change it; the registration refusals |
| The digest | `hash.rs`: `digest(..).hash() == world_hash(..)`; one field changed gives one difference; a marker component on one side gives a row difference; `decode(encode(d)) == d`, and nothing else decodes |
| Activation | `regions.rs`, `sync_point.rs`: a join, then an anchor moving, loads and simulates the regions within the radii, freezes the ring, keeps the release ring, cancels and unloads outside it, with hysteresis on a border; a pinned region is simulated with no anchor; a late table; the clamps, a zero streaming radius included; the exports equal a save of those regions; a region replaced in one frame is not released; on a 65,536-region atlas with 64 anchors the requests are exactly the union of the rings; `within` agrees with its definition, walking or scanning |
| Save and load | `persist.rs`: `save_point`, a new simulation from it through frames with every player joining again, an in-place reload and an untouched twin, `K` ticks each: equal hashes; the same with the save taken while a player who left has its row kept; an unload and reload keeps every persistent column; a journal replayed by a small driver reproduces the hashes, from tick zero and from checkpoints across the run with `restore_session_state` |
| Orders | `runs.rs`: two crates' exclusive systems in one pass and after `sets::INPUT` fail to build unordered and run in the order `Registration::order` gives; the edges the schedules refuse, one before `sets::INPUT` included |
| Refusals | `refusals.rs`, `runs.rs`, `sync_point.rs`: a bundle against another baseline, a second bundle for a loaded region, an unwanted bundle, a bad resource, a document schema the registry disagrees with, a table for an unloaded region, a frame's `modes` and `unloaded` under `Anchored`, a `Join` for a region outside the atlas, a non-`derived` world resource at build: each refused as stated, nothing changed, the tick runs, the failure is reported |
| Runs | `runs.rs`: a removed resource fails the next run with the simulation inside the error, and `run_passes` prepares both passes first and `Gather` again after `Extract`, handing the simulation back; `save_point` off the server likewise; `Cancelled` hands nothing back; the log is trimmed to the retention and a lagging consumer sees `None`; the server drains presentation events |
| The build check | `check.rs`: one call per build after every registration function, checks in the order added, with every system of the kind in registration order on a server, a prediction and a client world; a failing check's error fails the build before the schedule prepares and stops the later checks; a passing check leaves the hashes unchanged |
| SOUR | `sour.rs`: the program of `tests/common` for 300 ticks, built from the session and recording `(tick, hash)` after every step |
