# CURD (`lactose-sim`)

The deterministic simulation: what turns a `World`, a tick `Schedule` and an `Executor` into
a simulation that ticks, hashes, loads and unloads regions, and applies at the input sync
point what enters the boundary.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | inside |
| Unsafe | none, `#![forbid(unsafe_code)]` |

**Status: facade.** Every public signature is final and every body that decides anything is
`todo!()`; nothing here runs. This file is the contract the implementation lane meets, and
every sentence under "Behaviour the signatures cannot express" is one a dependent crate may
rely on. Architecture sections 4, 5.1, 6, 8.2, 8.3, 9.2, 9.3, 9.7, 11, 14, 15.2, 15.3 and
15.5 are the design; where this file and the architecture disagree, the architecture wins
and this file is wrong. Decision D118 records the choices those sections left open.
`crates/ecs/README.md` is the storage contract this crate builds on and never restates:
what it says about the input sync point, regions, snapshots, exports and change tracking
holds here unchanged.

Amending a signature is allowed in the change that needs it. Say so in the change description
and list what changed, because `app`, `predict`, `net`, `persist`, `physics-api`, `anim`,
`input` and the game crates are written against this surface.

## Surface

| Item | Purpose |
|---|---|
| `TypeTable` | Registers every component and resource type once, for the registry and for the world hash; a crate's generated registration function fills it |
| `SimulationBuilder`, `Simulation` | A world of one `WorldKind` with its tick schedule, its pass schedules, its hashers and its region lifecycle; built once, stepped from the timer root |
| `Simulation::step`, `Step`, `TickInput` | One tick from one frame of input; the future the timer root drives |
| `Simulation::save_point`, `SavePointRun`, `SavePoint` | The save copy of every loaded region at a sync point |
| `Stage`, `Pass`, `Registration`, `StageBuilder`, `sets::INPUT` | The four stages of the tick schedule, the two passes run between ticks, and how a registration function adds to them |
| `systems::{begin_tick, apply_region_lifecycle, apply_inputs, apply_staged_state}` | The input sync point as exclusive systems, in that order; `systems::save_copy` is the save pass |
| `PlayerInput`, `InputApplier` | A player's quantised input for a tick, and the hook that writes the typed component |
| `StagedState` | The hook `net` and `predict` fill at the sync point |
| `RegionAssets`, `RegionBundle`, `RegionModeChange` | What a frame carries for regions |
| `RegionControl`, `RegionAtlas`, `RegionCell`, `Within`, `RegionPolicy`, `StreamingAnchor` | Activation and streaming: the server's policy and its inputs |
| `RegionLifecycle`, `RegionRequest`, `RegionExport`, `LoadFailure`, `AssetFailureEvent` | The lifecycle's state and outputs |
| `Hashers`, `Simulation::world_hash`, `Simulation::hash` | The world hash of architecture section 8.3 |
| `RegionDigest`, `DigestFields`, `Differences`, `FieldDifference` | The field-level diff produced on a mismatch |
| `TickDelta`, `CurrentTick`, `TimeScale`, `TICK_RATE_HZ` | Time as the simulation sees it |
| `harness::{layout_mode, run_recorded}` | What a simulation test under SOUR uses |
| `journal::{Journal, JournalFrame, JournalAssets, JournalLoad}` | Journal replay's types; the facade names them only |

Hidden plumbing, which the implementation may reshape: the two private service resources
the sync point systems take (the frame slot and the save slot).

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| `app` (headless, milestone 1) | Build from a registry, a kind, SOUR's layout and chaos, an executor and registration functions | `TypeTable`, `SimulationBuilder::{new, layout, systems, inputs, staged_state, regions, policy, build}`; chaos is the executor builder's |
| | A `'static` task owning the world and schedules, paced by `TickPacer`, driving `Schedule::run_tick` | `Simulation` is `Send` and owns both; `step` drives `run_tick` and the passes; the task and every clock are `app`'s ("The timer root" below) |
| | The overload ladder's inputs surfaced, not decided | `TickBudget::behind` from the pacer; `RegionPolicy::activation_radius` is the rung the ladder turns; decimation and join refusal are not here |
| | Pause and time scale as a replicated resource | `TimeScale`, in thousandths, read between ticks for `TickPacer::set_time_scale` |
| Game crates | `TickDelta`, the tick, time scale | `Res<TickDelta>`, `Res<CurrentTick>` (or `Ctx::tick`), `Res<TimeScale>` |
| | Add sets to the simulation stage without owning the stage list | `Registration::stage(Stage::Simulation).sets(..).add(..)`; the stages are `Input`, `Simulation`, `Physics`, `Post` |
| `physics-api`, `anim` | A place for the step and for what reads it | `Stage::Physics` for the exclusive step; `Stage::Post` for readers; `Stage::Simulation` for the sim skeleton's state machines |
| `input` | How a per-tick batch of inputs reaches the sync point; where the typed write happens | `TickInput::inputs` as `PlayerInput` bytes; `InputApplier` beside the component's definition |
| `net` | Apply staged state at the sync point; the gather as a pass; spawn with the server's key and bind | `StagedState` on a client; `Registration::pass(Pass::Gather)`; `&mut World` inside the hook |
| `predict` | The deterministic step to re-run; snapshot, restore, horizon; the proxy update at the sync point | `step` with the ring's `TickInput`s; `world_mut()` between steps; `StagedState` in the prediction world |
| `persist` | Save at a sync point into records; load through the one path an unload uses | `save_point` → `SavePoint { tick, next_net_id, regions }`; `SimulationBuilder::{tick, next_net_id}` plus `TickInput::loaded`; `RegionLifecycle::take_exports` on unload |
| Region lifecycle | Policy inputs and outputs as data, testable without `net` or `asset` | Inputs: `StreamingAnchor` rows, `RegionPolicy`, `RegionAtlas`, `TickInput::{loaded, assets}`; outputs: `RegionLifecycle::{take_requests, take_exports, take_load_failures, take_asset_failures}` and the modes on the world |
| The world hash | `world_hash(region) -> u64`, per-type hashers from the erased registry, a diff on mismatch | `Simulation::world_hash`, `Hashers` filled by `TypeTable::register`, `RegionDigest::diff` |
| SOUR | Build from the session, run ticks, record a hash per tick, write the chaos log | `harness::layout_mode`, `harness::run_recorded`, `Simulation::hash`; the glue is shown under "SOUR" |
| Journal replay | A place for the types | `journal` |

## The `#[system]` registration a game crate writes

Types and systems are registered by two generated functions, one for each table, so that
the registry and the hash table cannot disagree and no system is added that its types do
not support:

```rust
use lactose_core::Result;
use lactose_ecs::SystemSet;
use lactose_sim::{Registration, Stage, TypeTable};

pub static MOVEMENT: SystemSet = SystemSet::new("game_sim::movement");

/// Every component and resource of this crate, through `TypeTable::register::<T>()`.
pub fn register_types(types: &mut TypeTable) -> Result<()> {
    // types.register::<Health>()?; ...
    Ok(())
}

/// Every system of this crate, placed in its stage.
pub fn register_systems(registration: &mut Registration) {
    registration.stage(Stage::Simulation).sets(&[&MOVEMENT]);
    // registration.stage(Stage::Simulation).add(&APPLY_GRAVITY).add(&SWORD_HIT); ...
}
```

`app` calls every crate's `register_types` on one `TypeTable`, hands it to
`SimulationBuilder::new`, and adds every crate's `register_systems` with
`SimulationBuilder::systems`. A system whose `worlds` exclude the builder's kind is skipped
by `StageBuilder::add`, so one pair of functions serves the server, the client and the
prediction world; a function that must differ by kind reads `Registration::kind`.

## Behaviour the signatures cannot express

**Time**

- `TICK_RATE_HZ` is 60 and `TickDelta` is `Scalar::ONE / Scalar::from_u32(60)`, inserted by
  the builder in every world; the same bits on every target. `TickDelta` is never changed:
  time scale changes how often ticks run, never the delta.
- `TimeScale` is in thousandths of real time, zero paused, `REAL_TIME` the default. The
  server writes it; `net` replicates it; `app` reads it between ticks and gives
  `TickPacer::set_time_scale(f64::from(thousandths) / 1000.0)`, outside the boundary.
  `sim` computes no float from it.
- `CurrentTick` is written by `systems::begin_tick` from `World::tick` at every input sync
  point; between ticks it equals `Simulation::tick`. `World::tick` is the source of truth.
- Nothing in this crate reads a clock. A step takes ticks, not time: the timer root owns
  `Instant`, `TickPacer` and the catch-up cap; `step` is the deterministic body it calls.

**The timer root** (architecture sections 5.1 and 11) is `app`'s task, in outline:

```rust
let period = Duration::from_secs(1) / TICK_RATE_HZ;
let mut pacer = TickPacer::new(Instant::now(), period, 4);
loop {
    let budget = pacer.poll(Instant::now(), resimulated_this_frame);
    for _ in 0..budget.ticks {
        simulation = simulation.step(&executor, next_frame()).await?;
    }
    if budget.behind { /* server: climb the ladder; client: resync (section 11) */ }
    let scale = simulation.world().resource::<TimeScale>().expect("inserted at build");
    pacer.set_time_scale(f64::from(scale.thousandths) / 1000.0);
    executor.sleep_until(budget.next_deadline).await?;
}
```

**The step**

`Simulation::step(executor, input)` does the following, in this order, and nothing else:

1. On a `Server` or `Client` world, `World::set_rollback_horizon(tick)` with the current
   tick, since neither rolls back; the prediction world's horizon is `lactose-predict`'s.
2. Puts `input` where the sync point systems take it, and runs the tick schedule with
   `Schedule::run_tick`. The first stage's sync point is the input sync point: the ECS
   zeroes transient columns and clears events, channels and input ticks, then the exclusive
   systems of `Stage::Input` run, this crate's four first (below), then every other
   exclusive system placed there, then the stages' systems. The tick advances by one.
3. Runs the passes that have at least one stage, `Pass::Extract` then `Pass::Gather`, with
   `Schedule::run_pass`: the tick does not advance and nothing is cleared.
4. On a `Server` world, takes and drops the presentation events, so the queue does not
   grow (architecture section 9.4); every other kind leaves them for `predict` or
   presentation to take.
5. Resolves with the simulation at the new tick.

- The future is `Send`; it is driven by the task that owns the simulation. The simulation
  is handed back on `Ok` only. `Cancelled` means the executor shut down and the world is
  torn; `InvalidData` means a schedule no longer prepares against the world, which only a
  resource or service removed between steps can cause, since `build` prepared every
  schedule. A future dropped mid-step drops the simulation when the run's jobs finish.
- Every schedule is built and prepared at `build`, so a `ScheduleError` is a build error of
  kind `InvalidData` carrying it, never a step error.
- A `Client` world runs this crate's systems and nothing of the game (architecture section
  5.3): its step is the sync point, the empty stages and the passes. A `Prediction` world
  runs the systems that name `prediction`.
- A world has no region until a bundle loads or the caller creates one directly between
  steps through `world_mut()`; a step over no region runs every sync point and no kernel.

**The input sync point** (architecture sections 6, 9.2 and 14)

This crate's four exclusive systems are in `sets::INPUT`, placed first in `Stage::Input`,
and ordered among themselves by `after` edges. Their order is fixed in every world:

1. `sim::begin_tick`: `CurrentTick` takes the world's tick.
2. `sim::apply_region_lifecycle`: the frame's `failures`, `loaded`, `assets` and, by control
   mode, `unloaded` and `modes`; then the policy. "Regions" below gives the rules.
3. `sim::apply_inputs`: for each `PlayerInput` in ascending `PlayerId` order,
   `World::set_input_tick(player, input_tick)`, then `InputApplier::apply`. A second input
   for one player in a frame is dropped and logged. Without an applier (the builder's
   default) every input is dropped and logged, after its input tick is set.
4. `sim::apply_staged_state`: `StagedState::apply(world, tick)`. The default does nothing.

A crate that must act at the sync point after the frame places an exclusive system in
`Stage::Input` ordered after `lactose_sim::systems::apply_staged_state`, or in a set
declared `.after(&[&lactose_sim::sets::INPUT])`. Nothing may be ordered before
`begin_tick`: two unordered exclusive systems in one stage are a schedule error, and this
crate's edges already order its own. An error from either hook is logged at target `curd`
and that entry is skipped; the tick runs.

A `TickInput` is applied field by field in the order its fields are declared, each list
sorted by its key (`PlayerId`, `RegionId`), so the same entries in any order are one input.
The frame for tick *T* is the one passed to the step that runs *T*; `lactose-predict`
re-simulates by passing the ring's frames again, in order, and a replay passes the
journal's.

What `InputApplier::apply` must do, beside the component `lactose-input` defines: decode
`sample` with the component's `CheckedBitPattern`, find the player's entity (its own
session fact; this crate keeps no player table), and write the value with `World::insert`
or `World::get_mut`. The component is `derived`, or the applier's crate adds an exclusive
system of its own after `apply_inputs` that declares `writes = [Sample]` so that readers of
a transient sample have a declared writer. The same applier serves the server, the
prediction world and a replay.

What `StagedState::apply` may do: on a client, everything received, through `World::spawn`
with the server's key, `World::bind`, `World::insert`, `World::columns_mut` or
`Mut::set_if_neq` so that an equal value does not dirty a row, and `World::despawn` for a
tombstone; in the prediction world, the proxies' positions for the tick from their
per-tick history. On the server it is the default and does nothing.

**Regions** (architecture sections 6, 9.7 and 14)

`sim::apply_region_lifecycle` applies, in this order, logging every refusal at target
`curd`:

1. Each `TickInput::failures` entry becomes an `AssetFailureEvent { tick, failure }` in the
   lifecycle, in frame order, for `app` and `net` to take: the tick-stamped event of
   section 14. The lifecycle does nothing else with it; policy is outside.
2. Each `TickInput::loaded` bundle, ascending by region: refused with a `LoadFailure` if the
   region is loaded already, or, under `Anchored`, absent from the atlas (`NotFound`).
   Otherwise `World::create_region` with the bundle's seed and, under `Directed`, its mode
   (under `Anchored`, `Frozen`); `World::load_region(baseline, overlay)`;
   `insert_region_resource_erased` for each resource and `insert_region_resource_zeroed`
   for each transient one. On any error the region is unloaded again, so a refused bundle
   changes nothing, a `LoadFailure` carries the error (`VersionMismatch` for an overlay
   against another baseline or a schema the registry disagrees with, which
   `lactose-persist` migrates and rebases before it builds a bundle), and the region stays
   requested. On success the region is no longer requested, and a table that was waiting
   for it is applied.
3. Each `TickInput::assets` table, ascending by region: `World::set_region_assets` if the
   region is loaded; otherwise kept until the region loads, replacing any table waiting
   before it. A table for a loaded region replaces the one it has: that is a hot reload,
   tick-stamped by the frame, and the region's mode does not change for it.
4. Under `RegionControl::Anchored(atlas)`:
   - The anchors are the regions of every row that has `StreamingAnchor`, whatever the
     region's mode. The active set is every region within `RegionPolicy::activation_radius`
     of an anchor's cell, the wanted set every region within `streaming_radius`, both by
     `RegionCell::distance` (the larger axis distance) through `RegionAtlas::within`.
   - Each loaded region not in the wanted set, ascending: `World::export_region` into a
     `RegionExport`, `World::unload_region`, a `RegionRequest::Release`; a table waiting for
     it is dropped. Then each wanted region not loaded and not requested, ascending: a
     `RegionRequest::Load`, and the region is requested until a bundle for it loads.
   - Each loaded region, ascending, is set `Simulated` if it is in the active set and
     `World::region_assets` is `Some`, else `Frozen`. So a region enters `Simulated` only
     when its bundle is loaded and its asset table has entered, and leaves it the sync
     point after its last anchor moves out of the radius. There is no hysteresis here: the
     overload ladder's hysteresis is `app`'s, on `activation_radius`.
   - The frame's `unloaded` and `modes`, and a bundle's `mode`, are ignored and logged.
   - `streaming_radius > activation_radius` is enforced at build, so an entity in a
     `Simulated` region can only cross into a loaded region, where the ECS freezes it with
     the region (section 9.7). A move to an unloaded region cannot arise from movement; a
     command that names one (a teleport past the ring) is dropped and logged by the ECS.
     With no anchor every loaded region is exported and unloaded, which is correct and
     wasteful: the headless app and tests either place an anchor or use `Directed`.
5. Under `RegionControl::Directed`: each `unloaded` region, ascending, is exported,
   unloaded and released as above if loaded, else logged; then each `modes` entry,
   ascending, is applied if the region is loaded, else logged. Nothing is requested. This
   is the client's and the prediction world's mode, since the server decides what is
   relevant to a client (section 9.3), and the test's mode.

- The lifecycle's own state (what is requested, tables waiting, its outputs) is not in a
  snapshot: a rollback restores the loaded set and the modes with the world, and the
  re-simulated frames load again what they loaded.
- An unload's `RegionExport` is what a save of that region would hold: rows in ascending
  `NetId` order, reference fields as stored, the seed and the persistent resources, with the
  transient components and resources listed by key. `lactose-persist` diffs it into the
  overlay and keeps it as the region's records until the region loads again; the server
  sends the same thing to a client on relevance.
- `RegionAtlas::insert` refuses a region or a cell already placed; `within` yields the
  regions within the radius in ascending `RegionId` order, the centre's region included.

**Saves and loads** (architecture sections 15.2 and 15.3)

- `save_point` runs the save pass, whose one system `sim::save_copy` forks one child per
  loaded region over `World::export_region` and collects the results in ascending
  `RegionId` order into `SavePoint { tick, next_net_id, regions }`; the simulation comes
  back unchanged with it, the tick does not advance. It is the same export an unload makes.
  World resources are not in it: `TickDelta`, `CurrentTick`, `TimeScale` and `RegionPolicy`
  are build constants or session state. On a world that is not `Server` the run resolves
  to `Unsupported`.
- A load is `SimulationBuilder::tick(saved.tick).next_net_id(saved.next_net_id)` and then,
  for each saved region, a `RegionBundle` in a frame, under `Directed` or when the policy
  wants it. The ECS stamps loaded rows at the current point and logs them as spawned; no
  epoch starts. Change consumers see a loaded region as new rows.

**The world hash** (architecture section 8.3)

The hash covers every persistent and transient column of the region and every persistent
and transient region resource, and the region's seed, mode and asset table; derived columns
and resources are excluded, and so is everything outside the region (world resources, the
structural log, change stamps, the entity table). It is a pure function of that state:
identical under every chunk layout, worker count, archetype history and registration order,
on x86-64 and aarch64. The encoding, bit for bit:

- *Key bytes*: a row's origin key as the sixteen memory bytes of `EntityRef::new(key)`
  (tag `u32` little-endian, 1 `Net` or 2 `Predicted`, then three `u32` words); a region
  resource uses the sixteen zero bytes of `EntityRef::NONE`. A placeholder never reaches
  storage.
- *Leaf bytes*: `Reflect::visit` under the hashing visitor appends, to the stream of the
  field ID each leaf is reported under: `bytes` verbatim (integers, `bool`, enums and the
  identity newtypes are little-endian memory bytes on every supported target); a `scalar`
  as its `canonical_bits()` as a `u64` little-endian, so every NaN is one value and negative
  zero keeps its sign; an `entity_ref` as its sixteen bytes; an `asset_ref` as its
  `asset_id()` as a `u64` little-endian. A raw `f32` or `f64` never appears: registration
  refuses it.
- *Field hash*: `xxh64` with the field ID as the seed over that field's stream.
- *Row hash*: `xxh64` with the component key as the seed over the key bytes, then for every
  field with an ID in declaration order its ID as a `u16` little-endian and its field hash
  as a `u64` little-endian.
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
- `Hashers::world_hash_forked` forks one child per chunk of each hashed column inside the
  job's scope and sums; the result is bit-identical to `world_hash`. An exclusive pass
  system is where it is called from.
- `RegionDigest` holds every field hash by (origin key, component key, field ID), with the
  seed, mode and asset table; `RegionDigest::hash` recomputes the region hash from them and
  equals `world_hash` of the same state, so a digest explains the hash it stands for.
  `diff` yields every field absent on one side or different, ascending; `encode` is one
  canonical byte encoding and `decode` accepts exactly it, so a digest can cross machines.
- `world_hash` fails with `NotFound` for a region that is not loaded, and with
  `Unsupported` for a hashed column of a type the hashers do not cover, which cannot happen
  in a world built by the `TypeTable` they came from.

**SOUR** (architecture section 8.3; `tools/xtask/src/harness/protocol.rs`)

- `harness::layout_mode(chunk_rows, seed)` is the layout mode: `permute_rows`,
  `ChunkOrder::Shuffled` and `split_batches` on, with the seed; `chunk_rows` as given, or
  `None` for the production 16 KiB capacity under the same perturbations. The production
  layout is `LayoutConfig::default()`. Chaos mode is the executor builder's `ChaosConfig`
  with `shuffle_steals`, `inject_yields`, `force_split` and `record` on and the seed; that
  type is `lactose-task`'s, so the test's glue builds it.
- `harness::run_recorded` steps once per frame, records `(tick, Simulation::hash())` after
  every step, and hands the simulation back; it uses `Executor::block_on`, so it is called
  from the test's main thread.
- The glue between the runner's `Session` and this crate is a few lines in each test
  crate's common module, until the protocol module moves into a foundation crate outside
  the boundary that can hold them:

  ```text
  let session = Session::from_env()?;
  let config = session.config();
  let layout = match config.layout {
      Layout::Production => LayoutConfig::default(),
      Layout::Perturbed { capacity: Capacity::Rows(n), seed } => layout_mode(Some(n), seed),
      Layout::Perturbed { capacity: Capacity::Bytes(_), seed } => layout_mode(None, seed),
  };
  let mut executor = Executor::builder();
  if let Some(workers) = config.workers { executor = executor.frame_workers(workers.get()); }
  if let Some(seed) = config.chaos_seed { executor = executor.chaos(chaos_mode(seed)); }
  let executor = executor.build()?;
  let simulation = SimulationBuilder::new(kind, types).layout(layout).systems(register).build()?;
  // `applied`: workers from `executor.frame_worker_count()`, the layout from
  // `simulation.world().layout()`, the chaos seed the test built with.
  let mut recorder = session.recorder(NAME, &applied)?;
  let simulation = run_recorded(&executor, simulation, frames, &mut |tick, hash| {
      recorder.record(tick.0, hash).map_err(..)
  })?;
  if let Some(log) = executor.execution_log() { session.write_execution_log(NAME, log.as_bytes())?; }
  ```

  The executor does not report the chaos seed it was built with, so that one field of the
  `applied` line is the test's own value; `docs/verification.md` should carry the gap until
  `lactose-task` reports it.
- Every simulation test of this crate and of the game crates records this way; the package
  with such tests is listed under `[harness]` in `tools/xtask/ci-policy.toml`. Until
  `lactose-app-headless` exists, whose test target is the SOUR test crate of section 8.3,
  that package is `lactose-sim`.

**Journal replay** (architecture section 15.5)

`journal::Journal` is a `SavePoint` and one `JournalFrame` per tick after it;
`JournalFrame::record(tick, &input)` is the frame's record, with tables as `VersionTable`s
and bundles by baseline hash and overlay digest. Writing, reading and replaying it is the
milestone 1 tooling's; this crate names the shape so `app` and the tools agree.

## Not in this crate yet

- The overload ladder itself: `app` climbs it on `TickBudget::behind` and turns
  `RegionPolicy::activation_radius`; decimation by tick parity is an ECS attribute flag
  when first needed (`docs/verification.md`); join refusal is `net`'s.
- A player table: who owns which entity is `net`'s and `predict`'s session state; the
  `InputApplier` resolves a player to an entity, and `StreamingAnchor` marks the entity
  whose position activates regions.
- Hashing `physics-api::snapshot` bytes (architecture section 8.3), added once Rapier's
  serialisation is shown order-stable.
- The journal writer, reader and replay driver, and the digest tool that compares two
  digests across machines; the types are here.
- Per-anchor streaming priority (`StreamingAnchor::priority` is zero) and streaming
  budgets, milestone 3.

## Proof for milestone 1

Nothing runs yet. The implementation lane replaces every `todo!()` and proves, under SOUR
in workers, repeat, layout and chaos modes:

| What | How |
|---|---|
| A step is a function of its frames | Two simulations stepped with equal frames hash equal every tick; frames with reordered lists hash equal |
| The sync point order | A test applier and staged-state hook record the calls; inputs ascend by player, regions by ID, the hooks run after the lifecycle |
| The hash encoding | Golden hashes for a small registry, pinned; layout mode and worker count change nothing; a derived column and a world resource change nothing; a transient value, a region resource, the seed, the mode and the table each change it |
| The digest | `digest(..).hash() == world_hash(..)`; one field changed gives one difference; `decode(encode(d)) == d` |
| Activation | An anchor at a cell loads and simulates the regions within the radii, freezes the ring, unloads outside, and the exports equal a save of those regions |
| Save and load | `save_point`, a new simulation from it through frames, `K` ticks each: equal hashes (architecture section 15.5, first two tests) |
| Refusals | A bundle against another baseline, a second bundle for a loaded region, a table for an unloaded region, a frame's `modes` under `Anchored`: each refused or deferred as stated, the tick runs, the failure is reported |
