# lactose-predict

The client's prediction world (architecture sections 4, 9.4 and 10): the owner that steps
it ahead of the server with the local player's inputs, the ring of per-tick snapshots it
rolls back through, the hook that applies what the server staged at the input sync point,
and the reconciliation that compares the server's exact rows with what was predicted and
re-simulates from the first difference. Copy-on-write snapshots (COW) are not built unless
the milestone 1 measurement says so; nothing here would change if they were.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | inside |
| Unsafe | none, `#![forbid(unsafe_code)]` |

**Status: facade.** Every public signature is final and every body that decides anything is
`todo!()`; nothing here runs. This file is the contract the implementation lane meets, and
every sentence under "Behaviour the signatures cannot express" is one a dependent crate may
rely on. Architecture sections 5.1, 5.3, 6, 6.1, 7, 9.2, 9.4, 10, 11, 16 and 22 are the
design; where this file and the architecture disagree, the architecture wins and this file
is wrong. Decision D122 records the choices those sections left open and what the design
review changed. `crates/ecs/README.md` (snapshots, binds, the alias table, change stamps,
`ColumnMut::write`, presentation-event keys), `crates/sim/README.md` (the step, the hook,
the frame, `world_mut`), `crates/physics-api/README.md` ("Snapshot and restore"),
`crates/input/README.md` (the applier, substitution) and `crates/net-api/README.md` (the
staged frame, the inbox, who reads what) are the contracts this crate builds on and never
restates.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because the client's `app`, `lactose-net`,
presentation and the milestone 1 netcode spike are written against this surface.
`tests/consumers.rs` is a compile-only sketch of what each of them writes: a change that
breaks one is a signature change to list.

## Surface

| Item | Purpose |
|---|---|
| `Predictor`, `LocalPlayer`, `DEFAULT_DEPTH` | The one owner of the prediction world: built from a `Prediction` simulation with the local player and its entity, stepped ahead, reconciled and run by value from the timer root |
| `Predictor::{step, reconcile, passes}`, `StepRun`, `ReconcileRun`, `PassesRun`, `PredictError` | The three runs, each a `Send` future that hands the predictor back; a failure before anything ran hands it back inside the error |
| `Predictor::{simulation, simulation_mut, tick, local, ring, stats, set_depth, resync}` | What `app` reads and does between runs |
| `Ring`, `SlotBytes` | The rollback ring: its depth, its oldest tick (the horizon), its newest (the present), what it weighs by part |
| `PredictionHook` | The `StagedState` hook `Predictor::new` installs: the frames applied inside the tick, the proxies moved along their history, the claimed entities' facts recorded |
| `Reconciled`, `Correction`, `CorrectionCause`, `Divergence`, `ResyncReason` | What one reconciliation did: the ticks confirmed, every correction with its depth, cause, entities and divergence, the ticks re-simulated, and whether the world must resync |
| `PredictionStats` | The monotonic counters the milestone 1 spike records: ticks predicted and re-simulated, frames, corrections, mismatched fields and entities, substitutions, unechoed spawns, resyncs, the deepest correction, the largest scalar error |
| `check_prediction_reads`, `prediction_may_name` | The build check of D121: a prediction-world system may name only replicated, transient or derived components |

Hidden plumbing: none. The record the hook fills and the predictor reads is a
crate-private service resource of the prediction world; the ring's slot is a private type,
so a snapshot's representation (a full copy today, copy-on-write if measured necessary) is
the implementation's. Everything public is keys, ticks, counts and one `Scalar`; nothing
reads a clock, and the crate carries the boundary `clippy.toml`.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| `app` (client, milestone 1 spike, then milestone 4) | One owner of the prediction world beside the replicated world, from the same registration functions | `SimulationBuilder::new(WorldKind::Prediction, types)` with every crate's `register_types`, `setup` and `register_systems`, `.tick(accepted.tick)`, then `Predictor::new(simulation, LocalPlayer { player, entity }, depth)`; "The loop `app` writes" below |
| | Step ahead each frame with the local inputs; the frames the network thread delivered applied | `Predictor::step` with a `TickInput` carrying `inputs[local.player]` for the tick; the frames through `StagedInbox::push` on `simulation_mut().world_mut()` before the step, folded as for the replicated world; the step's hook pops them |
| | Compare, roll back, correct, re-simulate; know what it cost | `Predictor::reconcile` after the frame's steps: `Reconciled::{confirmed, corrections, resimulated, resync}`; `resimulated` into the next `TickPacer::poll`; the time measured around the call, outside the boundary, divided by `resimulated` |
| | The ring's depth from the budget and the measured per-tick cost; the horizon | `Predictor::set_depth`; `Ring::{depth, oldest, newest, len, bytes, slot_bytes}`; a frame older than the ring is `ResyncReason::TooOld` |
| | Resync when the world must be rebuilt or has fallen behind | `Reconciled::resync`, `Predictor::resync(tick)`, then the server's full frame through the inbox |
| | Extract on the prediction world every frame; the presentation events | `Predictor::passes` after `reconcile`; `simulation_mut().world_mut().take_present_events()` after it |
| | Every cost the spike records as a value | `PredictionStats` and `Correction::divergence`: correction rate, rollback cost per tick, divergence; `Ring::bytes` for the snapshot size ("What the spike records" below) |
| `lactose-net` (milestone 4) | Which fields of a `StagedFrame` the prediction world reads, in what order, and what it does with each | "The hook" below; the prediction world applies every structural field as the replicated world does, without `RemoteSlot`, so it holds every relevant entity as a proxy |
| `lactose-physics-api` | The call order a re-simulation keeps, and which keys `corrected` receives | "Physics" below: `World::snapshot` then `snapshot`; `World::restore` then `restore` at the same tick; corrections written without recording; `corrected` with the keys whose rows were written, before the next step |
| `lactose-input` | The local player's recorded samples re-applied through `PlayerInputApplier`; a substitution replacing one | "Inputs and re-simulation" below: the ring's `TickInput`s are re-passed to `step`; `PlayerInput::substitute` and `after_substitute` replace the recorded samples; `StreamingAnchor` beside the claim on the local player's entity so `player_entity` resolves |
| Presentation (milestone 2) | Events from mispredicted ticks cancelled and re-emitted ones deduplicated; which entities a correction moved | `take_present_events` after `reconcile`, which the ECS deduplicates and cancels against the delivered-events log that survives a restore; `Correction::entities` and `Divergence` for smoothing; `World::claimed(local)` on the prediction world for the copies that shadow the replicated world's (section 5.3) |
| The build check (D121) | A prediction-world system that reads a component the server never corrects refused at build | `check_prediction_reads`, handed to the prediction world's builder as its check function ("The build check" below) |
| Game crates | Nothing: no game system names this crate | The prediction world's filter is `ClaimedBy`, the world's own; predicted keys are the ECS's rule |

## The loop `app` writes

The timer root is `app`'s (`crates/sim/README.md`, "The timer root"); this is its client
half for the prediction world, with `tests/consumers.rs` compiling it against `TickPacer`:

```rust
loop {
    // 1. Every frame the network thread delivered, into both inboxes; a refused frame
    //    enters nothing. Then the due frames' time scale and region changes folded into
    //    the first new tick's `TickInput`, exactly as for the replicated world.
    // 2. The pacer, told how many ticks the previous frame's reconcile re-simulated.
    let budget = pacer.poll(Instant::now(), resimulated);
    // 3. The new ticks, each with the local player's sample for that tick.
    for i in 0..budget.ticks {
        let next = Tick(predictor.tick().0 + 1);
        let mut input = if i == 0 { folded } else { TickInput::default() };
        input.inputs.insert(local.player, sample(next).to_player_sample(next));
        predictor = predictor.step(&executor, input).await?;
    }
    // 4. Reconcile what the steps' hook popped: compare, roll back, re-simulate.
    let started = Instant::now();
    let (p, reconciled) = predictor.reconcile(&executor).await?;
    predictor = p;
    resimulated = reconciled.resimulated;
    rollback_cost_per_tick = started.elapsed() / reconciled.resimulated.max(1);
    // 5. Extract, then the presentation events and the corrections to presentation.
    predictor = predictor.passes(&executor).await?;
    let events = predictor.simulation_mut().world_mut().take_present_events();
    // 6. A resync if one is due; a clock resync if the pacer is behind.
    if let Some(reason) = reconciled.resync { pacer.resync(now); predictor.resync(target)?; /* ResyncRequest */ }
    if budget.behind { pacer.resync(now); }
    executor.sleep_until(budget.next_deadline).await?;
}
```

The same registration functions serve the prediction world as the server: a system that
names `prediction` runs there over the rows that have `ClaimedBy`; `app` adds no system
of this crate's, since it has none.

## Behaviour the signatures cannot express

**Ownership and runs** (architecture sections 5.1 and 9.4; `crates/sim/README.md`,
"Runs")

- `Predictor::new` takes a `WorldKind::Prediction` simulation between runs (any other
  kind is `InvalidData` and the simulation is dropped with the error, as a wrong build
  would be), replaces its `dyn StagedState` service with a [`PredictionHook`] for `local`,
  inserts the record the hook fills, takes the ring's first slot at the simulation's
  current tick (the tick `JoinAccepted` names, before any step), and calls
  `World::set_rollback_horizon` with it. If the world holds a `Physics` resource then,
  every slot carries a physics snapshot and every restore restores it; a `Physics` inserted
  later is not snapshotted, and a world without one rolls back its rows alone.
- Every run takes the predictor by value and hands it back when it resolves, because the
  simulation's runs do and for the same reason: a run shares the world with jobs on other
  workers. A failure before anything ran (`step` without the local player's sample, a
  `RunError` with its simulation, a restore the ECS or the physics refused) hands the
  predictor back inside `PredictError`; after `Cancelled` the world is torn and `predictor`
  is `None`. `PredictError` converts into `lactose_core::Error`, so `?` into a plain
  `Result` drops the predictor as a plain error would. A future dropped mid-run drops the
  world when the run's jobs finish, as the simulation's does.
- Between runs `app` does, through `simulation_mut()`, exactly what it does to the
  replicated world: pushes frames into the `StagedInbox`, reads `World::regions` for the
  fold, takes the presentation events and the lifecycle's asset failures. Any other write
  to simulation state there is an unjournaled edit the ring does not hold: the next
  rollback undoes it.
- The order within a frame is push, fold, step the new ticks, reconcile, passes. Frames
  pushed after the last step of a frame are popped by the next frame's first step and
  reconciled after it. `reconcile` reconciles every frame the hook popped since the last
  call and nothing else; it never pops the inbox itself. `passes` runs after `reconcile`,
  so extract never sees a tick a rollback discarded (re-simulation calls `step` alone).

**The ring** (architecture sections 9.4 and 10)

- A slot is taken after every step, every re-simulated step and every correction, at the
  world's current tick, so between runs `Ring::newest()` is `Predictor::tick()`, the
  present. It holds: the world snapshot (`World::snapshot`: every column with its stamps,
  the entity table, the resources, the regions with their modes, seeds and tables, the
  input-tick table, the relationship index); the physics snapshot taken right after it
  (`physics_api::snapshot`: the backend's whole world and the sync's pending work), if the
  world has physics; the `TickInput` that produced the tick, as `app` handed it to `step`;
  the staged frames the hook popped in that step, oldest first, which are the proxies'
  per-tick history of section 10; and the record: every entity that carries `ClaimedBy`
  for the local player after the tick, with its region and the bytes of each of its
  replicated components as stored (reference fields hold origin keys), which a frame's
  exact rows for that tick are compared against. Derived and transient components are not
  in the record: the server sends neither.
- The ring keeps the newest `depth` slots and drops the oldest when a newer one is taken;
  `depth` is at least two (the present and one tick to return to; `InvalidData`
  otherwise), `DEFAULT_DEPTH` is sixty, and `set_depth` applies at once. `app` derives
  the depth from the frame re-simulation budget divided by the per-tick cost it measured,
  under its own cap (section 9.4): a rollback to the oldest slot then fits the budget.
- The oldest slot's tick is the horizon: after every step, correction, `set_depth` and
  `resync` the predictor calls `World::set_rollback_horizon` with it, so the ECS prunes the
  delivered presentation-event keys and the aliases of despawned predicted entities below
  it. A frame for a tick before the horizon cannot be reconciled and is a resync
  (`ResyncReason::TooOld`), as section 9.4 says of corrections deeper than the ring.
- `Ring::bytes()` and `slot_bytes(tick)` report the world snapshot's, the physics
  snapshot's and the record's bytes. The milestone 1 measurement of the snapshot cost per
  tick, which decides whether copy-on-write snapshots are built, is the implementation
  lane's: `crates/predict/tests/cost.rs` times `World::snapshot` and
  `physics_api::snapshot` as `step` takes them, for fifty claimed dynamic bodies plus 0 to
  10,000 statics with the physics test backend, and prints the bytes; the backend lane's
  conformance scene gives Rapier's bytes and time for the same scene; the netcode spike
  records both on the aarch64 client. The budget is 100 µs per tick (section 22) and the
  record goes in `docs/verification.md`'s spike row. No signature holds a snapshot and the
  slot is private, so copy-on-write inside `lactose-ecs`'s `WorldSnapshot`, or a slot that
  shares unchanged chunks with its predecessor, changes nothing here.

**The hook** (architecture sections 6, 9.2 and 9.4; `crates/sim/README.md`, "The input
sync point"; `crates/net-api/README.md`, "The staged frame")

- `sim::apply_staged_state` calls `PredictionHook::apply(world, tick)` once per tick after
  the inputs have been applied. In a predicted tick it pops every frame due at `tick` from
  the world's `StagedInbox` (`pop_through(tick)` until `None`) and records each with the
  tick it popped it at; in a re-simulated tick it applies the frames the ring's slot holds
  for the tick, in the same order, and pops nothing. It does, for each frame, oldest
  first, in this order and nothing else:
  1. A full frame is a rebuild, applied whole as the replicated world's hook applies a
     frame (`crates/net-api/README.md`, "What the replicated world's hook does"): each
     spawn an upsert or a `World::spawn` under `StagedSpawn::key` with every component,
     then `World::bind(origin, net)` if `origin` is `Some`; `ClaimedBy { local }` inserted
     where `slot` is `None` and removed where it is `Some`; never `RemoteSlot`; then
     moves, removals, every row (exact or not, claimed or not) and tombstones. `app`'s
     fold unloaded and reloaded every region in the same step. The record notes the
     rebuild at `tick`; nothing of the frame is compared.
  2. In a delta frame, each spawn in ascending `NetId` order: `World::bind(origin, net)`
     if `origin` is `Some` and has no alias, whoever's prediction it was, since a proxy
     keyed by another player's predicted origin needs its alias too. Then, if `slot` is
     `Some` and the key is not live: `World::spawn` under the key with every component, a
     proxy, without `RemoteSlot` or `ClaimedBy`. If `slot` is `Some` and the key is live
     and not claimed: an upsert, each component written whole so an equal value does not
     dirty the row, except `Transform`, `Velocity` and `Teleported`, which are written as
     this tick's proxy position (item 5). If `slot` is `Some` and the key is live and
     claimed (the server released it), or `slot` is `None` (the server has it claimed):
     a fact for the reconciliation, recorded and not written.
  3. Each move in ascending key order: `World::move_to_region` for a live unclaimed entity
     not already there; a fact for a claimed one.
  4. Each removal in ascending (key, component) order: `World::remove` of the component
     from a live unclaimed entity (per registered type until the erased remove D121
     proposes); a fact for a claimed one.
  5. Each row in ascending (key, component) order. A dequantised row (`exact: false`) of
     a live unclaimed entity: `Transform`, `Velocity` and `Teleported` are written as the
     proxy's position for this tick, so the kinematic body sweeps there during this
     tick's step and the physics sync pushes the write like any row write
     (`crates/physics-api/README.md`, "Bodies"); any other component is written whole as
     an upsert writes it. A proxy no row names this tick keeps its row: it rests where it
     was last put. An exact row (`exact: true`) of a claimed entity: a fact. A row of an
     entity that is not live is skipped (its spawn was lost and the server resends it); a
     dequantised row of a claimed entity, or an exact row of an unclaimed one, is a defect
     of the sender, logged and skipped.
  6. Each tombstone in ascending key order: `World::despawn` of a live unclaimed entity,
     with what it owns; a fact for a claimed one.
  7. `acked_input`, if `Some`: every live entity whose origin key is
     `Predicted { player: local, .. }` with an input tick before the acknowledgement and no
     alias is despawned now, with what it owns (architecture section 7: a predicted spawn
     the server never echoes is destroyed when the acknowledgement passes its input tick;
     "passes" is strictly after, so the frame of the spawn's own tick, which carries its
     echo, is never too late). The spawn arriving later anyway, as a spawn of a key that
     is not live, is a fact and re-spawns the entity at its tick as a correction.
  8. `substituted` and `acked_input` are recorded for the reconciliation.
- At every tick, before the frames, the hook ensures the local player's own entity
  (`LocalPlayer::entity`), if live, carries `ClaimedBy { local }` and `StreamingAnchor`,
  inserting what is missing: whatever path made the row live (a rebuild, a bundle, a
  correction), `lactose_input::player_entity` resolves from then on, and the input
  applier writes the sample at the same sync point, before this hook runs.
- An entry that fails (a spawn into a region the world has not loaded, a move the world
  refuses, bytes a component's decode rejects) is logged at target `predict` and skipped,
  never returned as the hook's error: the rest of the frame applies. The hook's own error
  is logged by `sim` at target `curd` and the tick runs.
- `app` reads `time_scale` and `regions` of the same frames, for the `TickInput` of the
  step whose hook pops them, as the replicated world does; a frame's region bundles are
  therefore in the ring's `TickInput` for that tick, and a re-simulation reloads them.

**Reconciliation** (architecture section 9.4)

- `reconcile` takes the frames the hook popped since the last call, oldest first. For a
  frame of server tick *T* popped at prediction tick *a* (never before *T*: the hook pops
  frames due at or before its tick):
  - A full frame drops every slot before *a* and sets `Ring::rebuilt_at` to *a*; *T* is
    confirmed; nothing is compared.
  - A frame with *T* before `rebuilt_at`, while the ring still holds that slot, is from
    before the rebuild: its facts are skipped (the hook applied its structural content),
    *T* is confirmed.
  - A frame with *T* before `Ring::oldest()` is `ResyncReason::TooOld { frame: T, oldest }`:
    this and every later frame of the call are discarded unreconciled, and `app` resyncs.
  - Otherwise the frame's facts are compared, below.
- The facts of a frame, each about one entity the server has or had claimed for the local
  player: a spawn with `slot: None` (the entity exists, with these components, exact,
  claimed, in this region); a spawn of a live claimed entity with `slot: Some` (released,
  with these dequantised components); an exact row (this component has these bytes); a
  move (this region); a removal (this component is absent); a tombstone (the entity is
  gone). The slot a fact is compared at is the first slot at or after *T* whose regions
  include the entity's region (the spawn's or move's region, else the record's); a fact
  whose region no slot holds is skipped and logged, since the region loads at the tick
  the frame was popped at and the entity arrives with it.
- A fact matches when the record of its slot agrees: the entity present with `ClaimedBy`
  (absent, for a tombstone; present and unclaimed, for a release), in that region, with
  that component present (absent, for a removal) and equal field by field. Fields are
  compared over every field with an ID in the component's schema, by canonical bytes: an
  integer, `bool`, enum, identity newtype, entity reference or asset reference as its
  memory bytes (storage holds origin keys on both machines, so a reference compares as
  bytes), and a scalar leaf (`FieldKind::Scalar` with `lactose_math`'s tag, at any depth
  of a `#[value]` struct or array) by its canonical bits, so one NaN is one value and
  negative zero is its own. Full precision, no tolerance: the same build on the same
  bytes produces the same bits, so any difference is a different input, which is what a
  misprediction is. A row whose length is not the registry's element size for its
  component, or whose component the registry lacks, is a defect of the sender: logged,
  skipped.
- A substituted tick *t* in a frame (architecture section 9.2; `crates/input/README.md`,
  "Redundancy and substitution") replaces the ring's sample for *t*: the sample of slot
  *t*'s `TickInput` becomes the sample of slot *t* − 1 (as replaced, if that tick was
  substituted too) through `PlayerInput::substitute`, and tick *t* − 1 is a correction
  point with cause `Substituted`. The first frame *T* after a substituted *T* − 1 whose
  own tick is not substituted repairs the ring's sample for *T* through
  `PlayerInput::after_substitute` with slot *T* − 1's sample, and *T* − 1 is a correction
  point with cause `Repaired`. A substituted tick before the horizon is `TooOld`. The
  client's own redundancy window is `net`'s and is not touched.
- The rollback. The correction point *k* is the oldest tick over the call's frames at
  which a fact does not match or a sample was replaced. Then, in this order:
  1. `World::restore` of slot *k*, then `physics_api::restore` of its physics snapshot at
     the same tick.
  2. Every fact at *k* is written: a component's bytes without recording (`ColumnMut::
     write` without marking the row, or the erased write D122 proposes), so the row's stamp
     stays as snapshotted and `Changed<T>` in the re-simulated ticks sees what the first run
     saw; a missing component added, an archetype change the structural log records; a
     missing entity spawned through `World::spawn` with every component and `ClaimedBy`
     (`StreamingAnchor` too for the local player's entity); a tombstoned one despawned; a
     moved one moved; a claim inserted or removed. A component the ECS refuses to
     `columns_mut` (`OwnedBy`, or one with an `EntityRef` field such as `Joint`) is
     written through `World::insert` and is stamped: a `Changed` reader then sees it in
     the re-simulated tick, which the first run did not. Stated, not hidden.
  3. `physics_api::corrected(world, keys)` with every entity whose rows were written, so
     the next sync pushes what differs to the solver. Spawns, despawns, moves and claim
     changes reach it through the structural log, as any do.
  4. Slot *k* is taken again, so it holds the corrected state and a later rollback to *k*
     restores it without re-correcting.
  5. Every tick *j* from *k* + 1 to the present is re-simulated: `Simulation::step` with
     slot *j*'s `TickInput`, its `failures` stripped (the first application stamped them)
     and its sample as replaced, the hook replaying slot *j*'s frames; slot *j* taken
     again; and, if a later frame of the call has facts at *j*, those compared there and,
     where one does not match, written as in steps 2 to 4 without a restore, counted as a
     further correction with its own depth. The world ends at the tick it started at.
  6. The horizon is set from the ring's oldest slot.
- `Reconciled::confirmed` is the newest *T* processed; `frames` counts them; `resimulated`
  is the present minus *k*, once, zero without a correction; `corrections` lists every
  correction in tick order, each with the entities whose facts were written (ascending)
  and its `Divergence`: how many entities and fields differed, and the largest absolute
  difference over the scalar leaves that differed, through `lactose_math`, the one float
  this crate computes. `PredictionStats` accumulates all of it.
- After the frames, a frame queued in the inbox whose tick is after the present means the
  prediction world has fallen behind the server: `ResyncReason::Behind { server, present }`
  with the oldest such tick. A queued frame at or before the present is only waiting for
  the next step (none ran this frame) and means nothing. Clock sync keeps the prediction
  world ahead (section 9.2); this is the backstop.
- A failure during a re-simulation (a re-simulated step refused by `Schedule::prepare`, a
  restore refused) hands the predictor back inside `PredictError` with the world at the
  tick the failure left it and the ring holding the slots up to there: not the present.
  `app` treats it as a resync. After `Cancelled` nothing is handed back.
- `Predictor::resync(tick)`: `World::set_tick(tick)` (between runs; it fails during a run
  as `set_tick` does), the ring emptied and a fresh first slot taken at `tick`, the horizon
  set there, the record cleared, the known regions kept, `resyncs` counted. `app` then
  requests a full frame, which rebuilds the world through the inbox and the `TickInput`
  as any full frame does; until it arrives the world predicts from what it has.
- Rapier's internal solver state is not corrected (section 9.4), so a corrected dynamic
  body converges over a few ticks rather than exactly: the frames of those ticks differ
  again by small amounts, each a correction under the rule above. The spike's correction
  rate counts them; whether milestone 4 adds a tolerance for dynamic bodies is decided
  against that baseline, never silently here.

**Physics** (`crates/physics-api/README.md`, "Snapshot and restore")

- Every slot's physics snapshot is taken right after its world snapshot, between runs;
  every restore is `World::restore` then `physics_api::restore` at the same tick, the
  world first; every correction's row writes are followed by `physics_api::corrected`
  with the keys of every entity whose rows were written, before the next step. A
  re-simulated tick's sync then makes the calls the first run's did, apart from what
  `corrected` adds. A world without `Physics` at `Predictor::new` skips all three.
- A proxy's position for a tick is written by the hook at the sync point (item 5 above)
  and pushed by the physics sync as a kinematic sweep; a claimed entity's correction is
  pushed by `corrected` with no sweep.

**Inputs and re-simulation** (architecture sections 5.1, 9.2 and 11; `crates/input/
README.md`, "The applier")

- `step`'s `TickInput` must carry `inputs[local.player]` whose `input_tick` is the tick
  the step runs (`Predictor::tick() + 1`); otherwise `InvalidData` with the predictor
  handed back, since the input tick is what gives a predicted spawn the key the server
  derives (`crates/ecs/README.md`, "Keys"). The sample is written by the world's input
  applier (`PlayerInputApplier`, installed by `app`), inside the tick, before the hook.
- A re-simulated tick re-passes the ring's `TickInput` to `Simulation::step`: the same
  sample bytes through the same applier, the same region bundles (an `Arc` each), the same
  time scale, with `failures` stripped. The prediction world's asset failures are this
  crate's to take (`crates/sim/README.md`): `app` takes them through
  `simulation_mut().lifecycle_mut().take_asset_failures()` once per frame, after
  `reconcile`, and the ring never replays them.
- A rollback's re-simulation runs to the present in the `reconcile` that detects it,
  always: a world left at an older tick would be extracted and presented there. Its length
  is bounded by the ring depth, which `app` sized from the budget. The pacer is told
  (`TickPacer::poll(now, resimulated)`) and runs that many fewer new ticks in the frame
  (section 5.1: catch-up and re-simulation share the cap); a frame still owing ticks is
  `TickBudget::behind`, which is `app`'s clock resync (`TickPacer::resync`), never a world
  resync.

**Presentation** (architecture sections 5.3, 9.4 and 10)

- The ECS keeps the delivered presentation-event keys across a restore and prunes them
  below the horizon this crate sets; a re-emission in a re-simulated tick is not delivered
  again and a delivered key whose tick was re-simulated without re-emitting it is reported
  cancelled (`crates/ecs/README.md`, "Commands and events"). `app` takes
  `World::take_present_events` after `reconcile`, once per frame, and hands the batch to
  presentation: `emitted` to play, `cancelled` to stop.
- A correction's `entities` and `Divergence` go to presentation beside the batch, for the
  visual blend of a corrected entity's jump; a predicted interaction with a proxy is
  presented tentatively until the frame that confirms it (section 9.4), which is the first
  `Reconciled::confirmed` at or after its tick without a correction naming the entity.
- Extract runs on the prediction world through `passes`, after `reconcile`. A restore
  starts a new epoch, so extract's mark is stale after a rollback and its next pass is a
  full one: a correction, which is written without a stamp, reaches presentation that way.
  The prediction world's claimed entities (`World::claimed(local)`) shadow the replicated
  world's copies for extract and the render slot table (section 5.3).

**The build check** (D121; architecture section 9.4)

- `prediction_may_name(info)` is the rule: a component a prediction-world system may name
  is replicated, transient or derived; a persistent component that is not replicated is
  refused, because the server never sends it, no correction ever touches it, and a
  prediction system reading it diverges silently.
- `check_prediction_reads(registry, systems)` applies it to every descriptor whose `worlds`
  include the prediction world, over every component its access table and its finalize's
  access table name (`&T`, `Mut<T>`, `Option<&T>`, `Option<Mut<T>>`, `With<T>`,
  `Without<T>`, `Changed<T>`, `writes = [T]`), and fails with `InvalidData` naming the
  first system and component that break it, or a component the registry lacks. A system
  whose `worlds` exclude the prediction world is not judged.
- It runs at the prediction world's build: `app` hands it to the builder as the check
  function D122 proposes (`SimulationBuilder::check`, run after the registration functions
  and before the schedules prepare, so a refused system is a build error with that
  error); until that exists, the spike and a test call it on their descriptor lists. It
  cannot live in `lactose-ecs`'s schedule (the ECS has no persistence policy to read) or
  in `xtask` (the rule needs the parameter types resolved).
- Not judged: resources. A world resource is `derived` by `sim`'s build rule. A region
  resource is corrected only by the relevance message (replicated resources are not
  gathered), so a prediction system reading a persistent region resource the server
  mutates diverges; that is a `net` item for milestone 4, named in "Not in this crate
  yet".

**What the spike records** (architecture section 22, milestone 1)

- Correction rate: `PredictionStats::corrections` differenced over the time `app`
  measures, or over `frames_reconciled`.
- Rollback cost per tick: the time `app` measures around `reconcile`, divided by
  `Reconciled::resimulated`, for the calls where it is not zero.
- Divergence: `Correction::divergence` per correction, `PredictionStats::{mismatched_fields,
  mismatched_entities, deepest_correction, largest_scalar_error}` over the run.
- Snapshot cost: `Ring::bytes` and `slot_bytes` for the size; the time from
  `tests/cost.rs` and the backend lane's run ("The ring" above).
- Substitutions and unechoed spawns: their counters, which the milestone 4 threshold
  reads beside the correction rate.

## Not in this crate yet

- The erased row access the implementation needs, in `lactose-ecs`: beside D121's
  `World::{insert_erased, remove_erased, spawn_erased}`, which the hook's proxies and the
  corrections' spawns are written against, `World::get_erased(key, ComponentKey) ->
  Option<&[u8]>` for the record and `World::overwrite_erased(key, &ComponentData) ->
  Result<bool>` for a correction: validated through the registry, written without a
  stamp, the component added if the row lacks it, and for a component `get_mut` refuses
  written as `insert` writes it (stamped). Until they exist the implementation monomorphises
  a reader and a writer per registered component type at `Predictor::new`, which changes
  no signature here.
- `SimulationBuilder::check`, in `lactose-sim`, so the build check runs at build; proposed
  in D122. `JoinAccepted::entity`, in `lactose-net-host`, so `app` learns the player's
  entity from the server rather than from the game; proposed there too.
- Forwarding nearby players' inputs into the prediction world (section 9.4, "later"): the
  same `PlayerSample` per player in the `TickInput`; nothing here changes.
- A tolerance for dynamic bodies, decided at milestone 4 against the spike's baseline.
- Region resources corrected per tick: `net`'s, milestone 4; until then a prediction
  system that reads one the server mutates is the game's hazard.
- The visual blend of a correction: presentation's, milestone 2, from `Correction`.
- The lag-compensation tick the client saw: a field of the input sample's schema version 2
  at milestone 4; nothing here.
- Copy-on-write snapshots: built only if the milestone 1 measurement exceeds the budget;
  private to the slot.

## Proof for milestone 1

Nothing runs yet. The implementation lane replaces every `todo!()` and proves:

| What | How |
|---|---|
| The ring | Under SOUR's layout mode: a prediction world stepped `n` ticks, rolled back to each slot and re-simulated with the ring's inputs, hashes equal at every tick to the straight run; the slot's record equals the claimed entities' rows read back; `oldest` and `newest` track the depth; `set_depth` drops and moves the horizon; a frame older than the ring is `TooOld` and nothing changed |
| The comparison | Golden cases over a small registry: equal bytes match; each field kind differing is one field; a scalar leaf's NaN with another payload matches, negative zero does not; a missing component, entity, region or claim is a mismatch with no field; a wrong row length is skipped |
| Corrections | A claimed body whose frame differs is corrected at its tick and the re-simulated ticks hash equal to a run that had the server's rows from that tick on; `corrected` received exactly the written keys (the physics test backend records calls); a `Joint` correction is stamped; a despawned-too-early entity is re-spawned in its known region; a spawn whose region loads later is compared at the first slot with the region; two frames with facts at two ticks correct once at the older and once at the newer with the right depths |
| Substitution | A substituted tick replaces the sample and re-simulates from the tick before; the repair after it replaces the next sample; both counted; the re-simulated world equals a run that had the substitute from the start |
| The hook | Frames applied in the stated order: proxies spawned, moved, removed, tombstoned; a proxy's `Transform` written for the tick and swept by the sync; a full frame rebuilds and resets the ring; a claimed entity's spawn, rows and tombstone are not written; an unechoed predicted spawn is despawned at the frame after its input tick and not at that tick's frame; the local entity carries its claim and anchor after every path; `player_entity` resolves |
| Resync | A frame left in the inbox after the steps is `Behind`; `resync` empties the ring, sets the tick and the horizon, and the next full frame rebuilds |
| Presentation | A rollback over a tick that emitted an event: the re-emission is not delivered again, an event not re-emitted is cancelled, both in the batch `app` takes after `reconcile`; extract's pass after a rollback is full |
| The check | A kernel naming a persistent unreplicated component in a `[server, prediction]` system is refused with its names; the same system `[server]` only passes; transient and derived pass; the finalize's table is judged |
| Cost | `tests/cost.rs`: bytes and microseconds of `World::snapshot` and `physics_api::snapshot` for fifty claimed bodies plus 0 to 10,000 statics with the test backend, printed, against the 100 µs budget; the record's bytes beside them |
| SOUR | A prediction scene with substitutions and corrections at several ticks, 150 ticks, the re-simulated ticks recorded again and equal, ending with the hashes of the straight run given the same facts |
