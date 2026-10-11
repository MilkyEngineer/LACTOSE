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
presentation-event keys), `crates/sim/README.md` (the step, the hook, the frame,
`world_mut`), `crates/physics-api/README.md` ("Snapshot and restore"),
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
| `Predictor::{step, reconcile, passes}`, `StepRun`, `ReconcileRun`, `PassesRun`, `PredictError` | The three runs, each a `Send` future that hands the predictor back; a failure before anything ran, or that left the world whole, hands it back inside the error |
| `Predictor::{simulation, simulation_mut, tick, local, set_local, ring, stats, set_depth, resync}` | What `app` reads and does between runs |
| `Ring`, `SlotBytes` | The rollback ring: its depth, its oldest tick (the horizon), its newest (the present), where it restarted, what it weighs by part |
| `PredictionHook` | The `StagedState` hook `Predictor::new` installs: the frames applied inside the tick, the proxies moved along their history, the claimed entities' facts recorded |
| `Reconciled`, `Correction`, `CorrectionCause`, `Divergence`, `ResyncReason` | What one reconciliation did: the ticks confirmed, every correction with its depth, cause, entities and divergence, the ticks re-simulated, and whether the world must resync |
| `PredictionStats` | The monotonic counters the milestone 1 spike records: ticks predicted and re-simulated, frames, corrections, mismatched fields and entities, substitutions, unechoed spawns, resyncs, slots without physics, the deepest correction |
| `check_prediction_reads`, `prediction_may_name`, `EXCLUSIVE_SYSTEMS` | The build check of D121: a prediction-world system may name only replicated, transient or derived components, and an exclusive one must be one of the engine's, by system key |

Hidden plumbing: none. The record the hook fills and the predictor reads is a
crate-private service resource of the prediction world; the ring's slot is a private type,
so a snapshot's representation (a full copy today, copy-on-write if measured necessary) is
the implementation's. Everything public is keys, ticks and counts; nothing reads a clock or
computes a float, and the crate carries the boundary `clippy.toml`.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| `app` (client, milestone 1 spike, then milestone 4) | One owner of the prediction world beside the replicated world, from the same registration functions, running ahead of the server from the start | `SimulationBuilder::new(WorldKind::Prediction, types)` with every crate's `register_types`, `setup` and `register_systems` (`lactose_net_api::setup` for the inbox, `lactose_physics_api::setup` for physics), `.tick(Tick(accepted.tick.0 + lead))`, then `Predictor::new(simulation, LocalPlayer { player, entity }, depth)`; "The loop `app` writes" below |
| | Step ahead each frame with the local inputs; the frames the network thread delivered applied | `Predictor::step` with a `TickInput` carrying `inputs[local.player]` for the tick; the frames through `StagedInbox::push` on `simulation_mut().world_mut()` before the step, folded as for the replicated world; the step's hook pops them |
| | Compare, roll back, correct, re-simulate; know what it cost | `Predictor::reconcile` after the frame's steps: `Reconciled::{confirmed, corrections, resimulated, resync}`; the time measured around the call, outside the boundary, divided by `resimulated` |
| | The ring's depth from the budget and the measured per-tick cost; the horizon | `Predictor::set_depth`; `Ring::{depth, oldest, newest, rebuilt_at, len, bytes, slot_bytes}`; a frame older than the ring is `ResyncReason::TooOld` |
| | Resync when the world must be rebuilt or has fallen behind; a changed player entity | `Reconciled::resync`, `Predictor::resync(tick)`, then the server's full frame through the inbox; `Predictor::set_local` |
| | Extract on the prediction world every frame; the presentation events | `Predictor::passes` after `reconcile`; `simulation_mut().world_mut().take_present_events()` after it |
| | Every cost the spike records as a value | `PredictionStats` and `Correction::divergence`: correction rate, rollback cost per tick, divergence; `Ring::bytes` for the snapshot size ("What the spike records" below) |
| `lactose-net` (milestone 4) | Which fields of a `StagedFrame` the prediction world reads, in what order, and what it does with each | "The hook" below; the prediction world applies every structural field as the replicated world does, without `RemoteSlot`, so it holds every relevant entity as a proxy |
| `lactose-physics-api` | The call order a re-simulation keeps, and which keys `corrected` receives | "Physics" below: `World::snapshot` then `snapshot`; `World::restore` then `restore` at the same tick; corrections written stamped; `corrected` with the keys whose rows were written, before the next step |
| `lactose-input` | The local player's recorded samples re-applied through `PlayerInputApplier`; a substitution replacing one | "Inputs and re-simulation" below: the ring's `TickInput`s are re-passed to `step`; `PlayerInput::substitute` and `after_substitute` replace the recorded samples; `StreamingAnchor` beside the claim on the local player's entity so `player_entity` resolves |
| Presentation (milestone 2) | Events from mispredicted ticks cancelled and re-emitted ones deduplicated; which entities a correction moved | `take_present_events` after `reconcile`, which the ECS deduplicates and cancels against the delivered-events log that survives a restore; `Correction::entities` and `Divergence` for smoothing; `World::claimed(local)` on the prediction world for the copies that shadow the replicated world's (section 5.3) |
| The build check (D121) | A prediction-world system that reads a component the server never corrects refused at build; an exclusive one that could read anything refused unless it is an engine's | `check_prediction_reads`, handed to the prediction world's builder as its check function ("The build check" below) |
| Game crates | Nothing: no game system names this crate | The prediction world's filter is `ClaimedBy`, the world's own; predicted keys are the ECS's rule |

## The loop `app` writes

The timer root is `app`'s (`crates/sim/README.md`, "The timer root"); this is its client
half for the prediction world, with `tests/consumers.rs` compiling it against `TickPacer`,
keeping the predictor on every path that hands it back:

```rust
loop {
    // 1. Every frame the network thread delivered, into both inboxes; a refused frame
    //    enters nothing. Then the due frames' time scale and region changes folded into
    //    the first new tick's `TickInput`, exactly as for the replicated world.
    // 2. The pacer. The cap is for catch-up; re-simulation is bounded by the ring depth
    //    and does not count against it (D122).
    let budget = pacer.poll(Instant::now());
    // 3. The new ticks, each with the local player's sample for that tick.
    for i in 0..budget.ticks {
        let next = Tick(predictor.tick().0 + 1);
        let mut input = if i == 0 { folded } else { TickInput::default() };
        input.inputs.insert(local.player, sample(next).to_player_sample(next));
        predictor = match predictor.step(&executor, input).await {
            Ok(p) => p,
            Err(PredictError { error, predictor: Some(p) }) => { /* keep p, resync */ }
            Err(torn) => return Err(torn.into()),
        };
    }
    // 4. Reconcile what the steps' hook popped: compare, roll back, re-simulate.
    let started = Instant::now();
    let (p, reconciled) = match predictor.reconcile(&executor).await { /* as above */ };
    predictor = p;
    rollback_cost_per_tick = started.elapsed() / reconciled.resimulated.max(1);
    // 5. Extract, then the presentation events and the corrections to presentation.
    predictor = match predictor.passes(&executor).await { /* as above */ };
    let events = predictor.simulation_mut().world_mut().take_present_events();
    // 6. A world resync if one is due; a clock resync if the pacer is behind.
    if let Some(reason) = reconciled.resync { pacer.resync(now); predictor.resync(target)?; /* ResyncRequest */ }
    if budget.behind { pacer.resync(now); }
    executor.sleep_until(budget.next_deadline).await?;
}
```

The same registration functions serve the prediction world as the server: a system that
names `prediction` runs there over the rows that have `ClaimedBy`; `app` adds no system
of this crate's, since it has none. `app` builds the prediction world at the join tick
plus its clock sync's lead (`SimulationBuilder::tick`), so the first predicted ticks'
inputs reach the server in time; the full frame for the join tick is due at the first
step and rebuilds the world there ("Reconciliation" below).

## Behaviour the signatures cannot express

**Ownership and runs** (architecture sections 5.1 and 9.4; `crates/sim/README.md`,
"Runs")

- `Predictor::new` takes a `WorldKind::Prediction` simulation between runs that holds a
  `StagedInbox` (`lactose_net_api::setup` inserted it); any other kind is `InvalidData`
  and a missing inbox `NotFound`, and the simulation is dropped with the error, as a
  wrong build would be. It replaces whatever `dyn StagedState` service the world holds
  (a builder's `staged_state` included) with a [`PredictionHook`], inserts the record
  the hook fills, which holds `local` (the one copy; the hook holds nothing), takes the
  ring's first slot at the simulation's current tick, and calls
  `World::set_rollback_horizon` with it. If the world holds a `Physics`
  resource then, every slot carries a physics snapshot and every restore restores it; a
  world without one rolls back its rows alone. Whether the world holds `Physics` is fixed
  at `new`: a `step` after it changed resolves to `InvalidData` with the predictor handed
  back, since a `Physics` the ring does not snapshot would be silently never restored.
- Every run takes the predictor by value and hands it back when it resolves, because the
  simulation's runs do and for the same reason: a run shares the world with jobs on other
  workers. A failure before anything ran (`step` without the local player's sample, a
  `RunError` with its simulation, a restore the ECS or the physics refused) hands the
  predictor back inside `PredictError`; after `Cancelled` the world is torn and `predictor`
  is `None`. `PredictError` converts into `lactose_core::Error`, so `?` into a plain
  `Result` drops the predictor as a plain error would; `app` matches instead and keeps
  it, as the sketch does. A future dropped mid-run drops the world when the run's jobs
  finish, as the simulation's does.
- A failure during a re-simulation (a re-simulated step refused by `Schedule::prepare`, a
  restore refused) hands the predictor back inside `PredictError` with the world at the
  tick the failure left it; the ring drops every slot after that tick, so
  `Ring::newest()` is `Predictor::tick()` still. `app` treats it as a resync. After
  `Cancelled` nothing is handed back.
- The hook must run in every step: it writes the tick it ran at into the record, and a
  `step` whose tick the hook did not write resolves to `InvalidData` with the predictor
  handed back, the slot taken, so a `dyn StagedState` inserted over the hook through
  `simulation_mut()` is caught at the next step rather than reconciling nothing forever;
  `resync` reinstalls the hook, and is the repair.
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
- `set_local(local)` replaces the player and entity between runs: from the next tick the
  hook keeps the claim and the anchor on the new entity and removes `StreamingAnchor`
  from the old one if it is live, so `player_entity` resolves to the new key. A changed
  player takes effect with the rebuild that follows a re-join (the full frame reloads
  every region, so rows claimed under the old player are gone). The server tells the
  client the player's entity on join (`JoinAccepted::entity`, proposed in D122) and again
  when it changes, a control message of `net`'s at milestone 4.

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
  in the record: the server sends neither. The record is read through
  `World::get_erased` per claimed entity and replicated type, so it costs the claimed
  entities, never a scan of the world's chunks.
- The ring keeps the newest `depth` slots and drops the oldest when a newer one is taken;
  `depth` is at least two (the present and one tick to return to; `InvalidData`
  otherwise), `DEFAULT_DEPTH` is sixty, a placeholder and not a sized value, and
  `set_depth` applies at once. `app` derives the depth from the frame re-simulation
  budget divided by the per-tick cost it measured, under its own cap (section 9.4): a
  rollback to the oldest slot then fits the budget, which is the condition under which
  a correction costs no lead ("Inputs and re-simulation" below).
- The oldest slot's tick is the horizon: after every step, correction, `set_depth` and
  `resync` the predictor calls `World::set_rollback_horizon` with it, so the ECS prunes the
  delivered presentation-event keys and the aliases of despawned predicted entities below
  it. A frame for a tick before the horizon cannot be reconciled and is a resync
  (`ResyncReason::TooOld`), as section 9.4 says of corrections deeper than the ring.
- `Ring::rebuilt_at` is the tick the ring last restarted at (the step that applied a full
  frame, or a `resync`), `Some` while the ring still holds that slot: frames for earlier
  ticks are from before the restart and are not reconciled ("Reconciliation" below).
- If `physics_api::snapshot` fails after a step ran, the slot is taken without its
  physics part, logged and counted (`PredictionStats::slots_without_physics`); a rollback
  to such a slot restores the world alone, and the physics then diffs every row against
  its bookkeeping and converges rather than matches (`crates/physics-api/README.md`,
  "Snapshot and restore"). A slot taken again right after a restore from it (a
  correction's re-take of slot *k*, "Reconciliation" below) keeps `None` when it had
  none: the physics was not restored to that tick, and a snapshot taken then would hold
  the present's solver state under an older tick.
- `Ring::bytes()` and `slot_bytes(tick)` report the world snapshot's, the physics
  snapshot's and the record's bytes. The milestone 1 measurement of the snapshot cost per
  tick, which decides whether copy-on-write snapshots are built, is the implementation
  lane's: `crates/predict/tests/cost.rs` times `World::snapshot` and
  `physics_api::snapshot` as `step` takes them, for fifty claimed dynamic bodies plus 0 to
  10,000 statics with the physics test backend, and prints the bytes; the backend lane's
  conformance scene gives Rapier's bytes and time for the same scene; the netcode spike
  records both on the aarch64 client. The budget is 100 µs per tick (section 22) and the
  record goes in `docs/verification.md`'s spike row; the design review's first figures
  (full-copy `World::snapshot` alone: 1.3 to 2.9 µs and 18.8 KB at no statics, 14.5 to
  19 µs and 262 KB at 1,000, 134 to 149 µs and 2.43 MB at 10,000) already say that
  10,000 statics as rows are over budget before physics, so the count of statics a
  prediction world holds as rows, or copy-on-write, is the decision that measurement
  makes. No signature holds a snapshot and the slot is private, so copy-on-write inside
  `lactose-ecs`'s `WorldSnapshot`, or a slot that shares unchanged chunks with its
  predecessor, changes nothing here.

**The hook** (architecture sections 6, 9.2 and 9.4; `crates/sim/README.md`, "The input
sync point"; `crates/net-api/README.md`, "The staged frame")

- `sim::apply_staged_state` calls `PredictionHook::apply(world, tick)` once per tick after
  the inputs have been applied. In a predicted tick it pops every frame due at `tick` from
  the world's `StagedInbox` (`pop_through(tick)` until `None`) and records each with the
  tick it popped it at; in a re-simulated tick it applies the frames the ring's slot holds
  for the tick, in the same order, and pops nothing. It writes `tick` into the record as
  the tick it ran at. It does, for each frame, oldest first, in this order and nothing
  else:
  1. A full frame is a rebuild, applied whole as the replicated world's hook applies a
     frame (`crates/net-api/README.md`, "What the replicated world's hook does"): each
     spawn an upsert or a `World::spawn` under `StagedSpawn::key` with every component,
     then `World::bind(origin, net)` if `origin` is `Some`; `ClaimedBy { local }` inserted
     where `slot` is `None` and removed where it is `Some`, and `StreamingAnchor` inserted
     beside the claim on `LocalPlayer::entity`; never `RemoteSlot`; then moves, removals,
     every row (exact or not, claimed or not) and tombstones. `app`'s fold unloaded and
     reloaded every region in the same step. The record notes the rebuild at `tick`;
     nothing of the frame is compared. The rebuild tick's own sample was not applied (the
     applier ran before this hook spawned the row) and the local inputs between the full
     frame's tick and the rebuild tick are not in the rebuilt state: the first frame at
     or after the rebuild tick corrects the difference, once.
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
  4. Each removal in ascending (key, component) order: `World::remove_erased` of the
     component from a live unclaimed entity; a fact for a claimed one.
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
     echo, is never too late). An echo that arrives later anyway, as a spawn of a key that
     is not live, is a fact and re-spawns the entity as a correction at that frame's tick.
  8. `substituted` and `acked_input` are recorded for the reconciliation.
- Before and after the frames, the hook ensures the local player's own entity
  (`LocalPlayer::entity`), if live, carries `ClaimedBy { local }` and `StreamingAnchor`,
  inserting what is missing, and that no other entity carries `StreamingAnchor`: whatever
  path made the row live (a rebuild, a bundle, a correction, a `set_local`),
  `lactose_input::player_entity` resolves from the next tick on, and the input applier
  writes the sample at the same sync point, before this hook runs.
- An entry that fails (a spawn into a region the world has not loaded, a move the world
  refuses, bytes a component's validation rejects) is logged at target `predict` and
  skipped, never returned as the hook's error: the rest of the frame applies. The hook's
  own error is logged by `sim` at target `curd` and the tick runs.
- `app` reads `time_scale` and `regions` of the same frames, for the `TickInput` of the
  step whose hook pops them, as the replicated world does; a frame's region bundles are
  therefore in the ring's `TickInput` for that tick, and a re-simulation reloads them.

**Reconciliation** (architecture section 9.4)

- `reconcile` takes the frames the hook popped since the last call, oldest first. A call
  first finds the newest full frame it popped; every frame popped before it is from
  before the restart (rule 2) whatever its tick, and contributes no correction point,
  substitution or `TooOld`. For a frame of server tick *T* popped at prediction tick *a*
  (never before *T*: the hook pops frames due at or before its tick), the rules apply in
  this order:
  1. The call's newest full frame drops every slot before *a* and sets `Ring::rebuilt_at`
     to *a*; *T* is confirmed; nothing is compared; its `substituted` is ignored.
  2. A frame popped before the call's newest full frame, or, while `Ring::rebuilt_at` is
     `Some(r)`, a frame with *T* before *r*, is from before the ring's restart: its facts
     are skipped (the hook applied its structural content), its substitutions are skipped
     likewise, as is any substitution or repair at a tick at or before *r*, and *T* is
     confirmed. Such frames are after the full frame's tick and before the restart; they
     are not older than the ring in any sense that matters.
  3. Otherwise a frame with *T* before `Ring::oldest()` is `ResyncReason::TooOld { frame:
     T, oldest }`: this and every later frame of the call are discarded unreconciled, and
     `app` resyncs.
  4. Otherwise the frame's facts are compared, below.
- The facts of a frame, each about one entity the server has or had claimed for the local
  player: a spawn with `slot: None` (the entity exists, with these components, exact,
  claimed, in this region); a spawn of a live claimed entity with `slot: Some` (released,
  with these dequantised components); an exact row (this component has these bytes); a
  move (this region); a removal (this component is absent); a tombstone (the entity is
  gone). A fact is compared at slot *T* and nowhere else. A fact about an entity absent
  from slot *T*'s record is a mismatch (the entity is spawned there) if its region (the
  spawn's or move's, else the region the predictor last knew it in) is loaded in slot
  *T*, and is skipped and logged otherwise: the region loads at the tick the frame was
  popped at, and the entity arrives whole with the first frame at or after it. The
  predictor knows the region of every entity the local player has claimed, from the
  first fact or record that named it until the server's tombstone or release fact for it
  is reconciled; an entry is never dropped for the ring's age, so an entity the
  prediction world despawned too early is re-spawned by its next fact however long the
  server left it unchanged, and `resync` keeps the entries. The map is bounded by the
  server's live claims for the player.
- A fact matches when the record of slot *T* agrees: the entity present with `ClaimedBy`
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
- Substitution (architecture section 9.2; `crates/input/README.md`, "Redundancy and
  substitution"), in ascending input tick, substitutions and repairs together: a
  substituted tick *t* replaces the ring's sample for *t* with the sample of slot *t* − 1
  (as replaced, if that tick was substituted too) through `PlayerInput::substitute`, and
  *t* − 1 is a correction point with cause `Substituted`. For every input tick *u* that is
  not substituted and whose *u* − 1 is, once a frame with *T* at or after *u* has been
  processed, the ring's sample for *u* becomes its own sample through
  `PlayerInput::after_substitute` with slot *u* − 1's sample as replaced, and *u* − 1 is a
  correction point with cause `Repaired`; a frame spans every tick since the previous
  frame, so *u* is found by the tick, never by the frame. A substitution or repair at a
  tick at or before `Ring::rebuilt_at` is skipped, as the frame's facts are, and a
  repair whose *u* − 1 substitution was skipped (by that rule, or because its frame was
  popped before the call's newest full frame) is skipped too; otherwise one whose *t* − 1
  is before the horizon is `TooOld`. The client's own redundancy window is `net`'s and is
  not touched.
- The rollback. The correction point *k* is the oldest tick over the call's frames at
  which a fact does not match or a sample was replaced. Then, in this order:
  1. `World::restore` of slot *k*, then `physics_api::restore` of its physics snapshot at
     the same tick (skipped for a slot taken without one).
  2. Every fact at *k* is written, stamped: a component's bytes through
     `World::insert_erased`, which adds the component if the row lacks it and stamps the
     row when the bytes differ; a missing entity through `World::spawn_erased` with every
     component and `ClaimedBy` (`StreamingAnchor` too for the local player's entity); a
     tombstoned one despawned; a moved one moved; a claim inserted or removed; a removed
     component removed. Facts are written in the order the hook recorded them: frame
     order, then spawns, moves, removals, rows and tombstones, each ascending by key;
     never in chunk order. A stamp is right: a correction is a change of state, the
     server's, and a `Changed<T>` reader in the re-simulated ticks sees it once, as it
     would a server change, so a derived component a prediction system maintains from a
     replicated one (a cache of a position, a dead flag from health) follows the
     correction instead of holding the mispredicted value until the row next changes.
     This reverses D114's "a correction written without recording" for this crate; the
     ECS keeps `ColumnMut::write` without recording for any consumer that wants it.
  3. `physics_api::corrected(world, keys)` with every entity whose rows were written: the
     physics contract this crate keeps for the backend, which makes the next sync push
     the written rows' five columns; with stamped writes the sync's fresh mark after
     `restore` would see the rows anyway, and the backend ignores an equal value.
  4. Slot *k* is taken again, so it holds the corrected state and a later rollback to *k*
     restores it without re-correcting. Its physics part is taken again too, unless slot
     *k* had none: then it stays `None`, since step 1 did not restore the physics and a
     snapshot now would hold the present's solver state under *k*.
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
  and its `Divergence`: how many entities and fields differed. `PredictionStats`
  accumulates all of it.
- After the frames, a frame queued in the inbox whose tick is after the present means the
  prediction world has fallen behind the server: `ResyncReason::Behind { server, present }`
  with the oldest such tick. A queued frame at or before the present is only waiting for
  the next step (none ran this frame) and means nothing. Clock sync keeps the prediction
  world ahead (section 9.2); this is the backstop. The oldest such tick is
  `StagedInbox::oldest_after(present)`, read after the prediction world's steps, whether
  or not a frame for the present is still queued.
- `Predictor::resync(tick)`: `World::set_tick(tick)` (between runs; it fails during a run
  as `set_tick` does), the ring emptied and a fresh first slot taken at `tick` with
  `Ring::rebuilt_at` set to `tick`, the horizon set there, the record cleared, the hook
  reinstalled over whatever `dyn StagedState` the world holds, the known regions kept
  (only a tombstone or release fact drops one), `resyncs` counted. The frames still
  queued in the inbox (the ones that
  made a `Behind`) are then from before the restart and are skipped, not `TooOld`, so one
  resync is one resync. `app` then requests a full frame, which rebuilds the world through
  the inbox and the `TickInput` as any full frame does; until it arrives the world predicts
  from what it has. A `Behind` or `Divergence` resync keeps the inbox. A re-join (a new
  `JoinAccepted`) also resets it: `app` drops the old session's frames not yet pushed and
  calls `StagedInbox::reset` through `predictor.simulation_mut().world_mut()` beside the
  resync (`lactose-net-api`).
- The ECS asks this crate to despawn a `Net` entity a restore revives, one live at the
  snapshot and despawned before a bind to its key (`crates/ecs/README.md`, "Snapshots and
  bulk copy"). It cannot arise here: a spawn's key is `StagedSpawn::key`, the origin key,
  so an entity with a predicted origin is never live under its `Net` key and no bind can
  name a live `Net` entity. The replay of the frame that tombstoned the entity despawns
  it again regardless.
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
  re-simulated tick's sync then makes the calls the first run's did, apart from what the
  correction adds. A world without `Physics` at `Predictor::new` skips all three.
- A proxy's position for a tick is written by the hook at the sync point (item 5 above)
  and pushed by the physics sync as a kinematic sweep; a claimed entity's correction is
  pushed by the sync with no sweep.

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
  is bounded by the ring depth, and it does not count against the catch-up cap of
  sections 5.1 and 11 (D122 amends them): a correction costs the client no ticks of lead
  while the depth times the per-tick cost `app` measured fits the frame's re-simulation
  budget, which is what `set_depth` is for. `DEFAULT_DEPTH` is a placeholder, not a sized
  value: with a depth `app` has not sized, a rollback to the oldest slot can overrun the
  frame, and the ticks the pacer then finds due are catch-up under the cap. Sharing the
  cap would cost lead on every correction, since a correction's depth is about a round
  trip plus margin, more than the cap at 100 ms, and the convergence storm above corrects
  on consecutive frames; the lead would decay to a world resync faster than clock sync's
  three percent could restore it. `TickPacer::poll` therefore takes no re-simulation
  ticks, and `TickBudget::behind` keeps its meaning: catch-up alone owed more than the
  cap, a clock resync (`TickPacer::resync`), never a world resync.

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
  full one; a correction is stamped besides, so extract sees it either way. The
  prediction world's claimed entities (`World::claimed(local)`) shadow the replicated
  world's copies for extract and the render slot table (section 5.3).

**The build check** (D121; architecture section 9.4)

- `prediction_may_name(info)` is the rule: a component a prediction-world system may name
  is replicated, transient or derived; a persistent component that is not replicated is
  refused, because the server never sends it, no correction ever touches it, and a
  prediction system reading it diverges silently. `OwnedBy` and `PredictedOrigin` are
  persistent and not replicated, so a prediction system naming either is refused; a game
  that needs ownership in the prediction world needs `OwnedBy` replicated, an ECS item.
- `check_prediction_reads(registry, systems)` applies it to every descriptor whose `worlds`
  include the prediction world, over every component its access table and its finalize's
  access table name (`&T`, `Mut<T>`, `Option<&T>`, `Option<Mut<T>>`, `With<T>`,
  `Without<T>`, `Changed<T>`, `writes = [T]`), and fails with `InvalidData` naming the
  first system and component that break it, or a component the registry lacks. An
  exclusive system reads anything through `&mut World`, which no table shows, so one whose
  `worlds` include the prediction world must have its `SystemDescriptor::key` in
  `EXCLUSIVE_SYSTEMS` or it is refused by name. The list is the engine's exclusive
  systems that run there, each reviewed for what it reads: `sim::begin_tick`,
  `sim::apply_frame_regions`, `sim::apply_sessions`, `sim::apply_inputs`,
  `sim::apply_staged_state` and `sim::apply_region_policy` (`crates/sim/README.md`, "The
  input sync point"), `input::player_input_writer`, `physics_api::sync` and
  `physics_api::write_back`. A key is the xxh64 of the declared name, which the schedule
  rejects a collision of, so a look-alike system in a listed set passes nothing; an
  engine crate that adds or renames one changes the list in the same commit, and a game
  crate never runs an exclusive system in the prediction world. Test-support scenes are
  not shipped and are not listed: `physics-api`'s conformance scene has an exclusive
  `input_tick` system, and a test of this crate that builds that scene does not run the
  check. A system whose `worlds` exclude the prediction world is not judged.
- It runs at the prediction world's build: `app` hands it to the builder through
  `SimulationBuilder::check`, which runs it after the registration functions and before
  the schedules prepare, so a refused system is a build error with that error. It
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
  mismatched_entities, deepest_correction}` over the run.
- Snapshot cost: `Ring::bytes` and `slot_bytes` for the size; the time from
  `tests/cost.rs` and the backend lane's run ("The ring" above).
- Substitutions, unechoed spawns and slots without physics: their counters, which the
  milestone 4 threshold reads beside the correction rate.

## Not in this crate yet

- The implementation lane starts only when what it is written against runs: in
  `lactose-net-api`, `StagedInbox::{push, pop_through, due, oldest_after, reset}`;
  in `lactose-input`,
  `PlayerInput::{substitute, after_substitute}`, `player_entity` and `PlayerInputApplier`.
  `lactose-ecs`'s `World::get_erased` (the record), `World::insert_erased` (corrections,
  upserts), `World::remove_erased` (removals) and `World::spawn_erased` (proxies and
  corrected spawns), and `lactose-sim`'s `SimulationBuilder::check`, run. Nothing here is
  written against a typed fallback.
- `JoinAccepted::entity`, in `lactose-net-host`, so `app` learns the player's entity from
  the server rather than from the game; proposed in D122, with the control message that
  carries a changed entity at milestone 4.
- Forwarding nearby players' inputs into the prediction world (section 9.4, "later"): the
  same `PlayerSample` per player in the `TickInput`; nothing here changes.
- A tolerance for dynamic bodies, decided at milestone 4 against the spike's baseline.
- Region resources corrected per tick, and `OwnedBy` replicated: `net`'s and the ECS's,
  milestone 4; until then a prediction system that reads one the server mutates is the
  game's hazard, and one that names `OwnedBy` is refused.
- The visual blend of a correction: presentation's, milestone 2, from `Correction`.
- The lag-compensation tick the client saw: a field of the input sample's schema version 2
  at milestone 4; nothing here.
- Copy-on-write snapshots, or a cap on the statics a prediction world holds as rows:
  decided by the milestone 1 measurement; private to the slot.

## Proof for milestone 1

Nothing runs yet. The implementation lane replaces every `todo!()` and proves:

| What | How |
|---|---|
| The ring | Under SOUR's layout mode: a prediction world stepped `n` ticks, rolled back to each slot and re-simulated with the ring's inputs, hashes equal at every tick to the straight run; the slot's record equals the claimed entities' rows read back; `oldest` and `newest` track the depth; `set_depth` drops and moves the horizon; a frame older than the ring is `TooOld` and nothing changed; a slot taken without physics restores the world alone and is counted |
| The comparison | Golden cases over a small registry: equal bytes match; each field kind differing is one field; a scalar leaf's NaN with another payload matches, negative zero does not; a missing component, entity, region or claim is a mismatch with no field; a wrong row length is skipped; a fact is compared at slot `T` only, and one whose region slot `T` lacks is skipped |
| Corrections | A claimed body whose frame differs is corrected at its tick, stamped, and the re-simulated ticks hash equal to a run that had the server's rows from that tick on; a `Changed`-driven derived cache follows the correction; `corrected` received exactly the written keys (the physics test backend records calls); a despawned-too-early entity is re-spawned in its last known region; two frames with facts at two ticks correct once at the older and once at the newer with the right depths; facts are written in frame order under every layout |
| Substitution | A substituted tick replaces the sample and re-simulates from the tick before; the repair of the first unsubstituted tick after it is found by tick when the frame spans several ticks; both counted; a substitution at or before the restart is skipped, as is one in a frame popped before the call's newest full frame and the repair that depended on either, and one before the horizon is `TooOld`; the re-simulated world equals a run that had the substitute from the start |
| The hook | Frames applied in the stated order: proxies spawned, moved, removed, tombstoned; a proxy's `Transform` written for the tick and swept by the sync; a full frame rebuilds, resets the ring, inserts the anchor, and the first frame at or after the rebuild tick corrects once; a claimed entity's spawn, rows and tombstone are not written; an unechoed predicted spawn is despawned at the frame after its input tick and not at that tick's frame; the local entity carries its claim and anchor after every path and `set_local` moves them; `player_entity` resolves; a hook replaced through `simulation_mut` fails the next step with the predictor kept |
| Resync | A frame left in the inbox after the steps with a tick after the present is `Behind`, one at or before it is not; `resync` empties the ring, sets the tick, the horizon and `rebuilt_at`, reinstalls the hook over one inserted through `simulation_mut`, keeps the known regions, the queued frames are skipped rather than `TooOld`, and the next full frame rebuilds |
| Presentation | A rollback over a tick that emitted an event: the re-emission is not delivered again, an event not re-emitted is cancelled, both in the batch `app` takes after `reconcile`; extract's pass after a rollback is full |
| The check | A kernel naming a persistent unreplicated component in a `[server, prediction]` system is refused with its names; the same system `[server]` only passes; transient and derived pass; the finalize's table is judged; an exclusive prediction-world system outside `EXCLUSIVE_SYSTEMS` is refused, one in a listed set under an unlisted name is refused, and `physics_api::sync` passes by key |
| Cost | `tests/cost.rs`: bytes and microseconds of `World::snapshot` and `physics_api::snapshot` for fifty claimed bodies plus 0 to 10,000 statics with the test backend, printed, against the 100 µs budget; the record's bytes beside them, read through `get_erased` |
| SOUR | A prediction scene with substitutions and corrections at several ticks, 150 ticks, the re-simulated ticks recorded again and equal, ending with the hashes of the straight run given the same facts |
