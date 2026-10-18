# lactose-net-api

Network contracts inside the determinism boundary: what the simulation side (`sim`,
`predict`, `app`) and `lactose-net` agree on about what enters and leaves a world. The
staged frame a client applies at the input sync point and the inbox it waits in, the
frame the server's replication gather fills, the relevance message a region's entry sends,
the remote-slot component a staged spawn leaves behind, and the named place of lag
compensation. The host side of networking (quantisation, the handshake, the transport,
packets and handles, per-client interest, the interpolation buffer, clock sync, control
messages) is `lactose-net-host`, outside the boundary.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | inside |
| Unsafe | none, `#![forbid(unsafe_code)]` |

**Status: implemented for milestone 1.** Every body runs and every sentence under "Behaviour
the signatures cannot express" is one a dependent crate may rely on; "Proof for milestone 1"
names the test that holds each. Architecture sections 4, 5.3, 6, 7, 8.1, 8.2, 9.2, 9.3,
9.4, 9.5, 10, 11, 14 and 15.2 are the design; where this file and the architecture
disagree, the architecture wins and this file is wrong. Decision D121 records the choices
those sections left open, for this crate and for `lactose-net-host` together, and what the
first design review changed.
`crates/sim/README.md` is the contract the hook and the pass here meet ("The input sync
point": `StagedState`; "Runs": `run_passes` and `Pass::Gather`; "Regions": `RegionExport`
and the relevance overlay) and `crates/ecs/README.md` the storage contract they write
against (`ErasedColumn::changed_since`, `World::move_to_region`, `World::spawn`, `bind`,
`insert`, `get_mut`, `remove`, `despawn`); neither is restated here.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `app`, `lactose-net`, `lactose-net-host`,
`lactose-predict`, `lactose-persist` and the milestone 1 spike are written against this
surface. `tests/consumers.rs` sketches what each of them writes and runs the sketches: a
change that breaks it is a signature change to list.

## Surface

| Item | Purpose |
|---|---|
| `StagedFrame`, `StagedSpawn`, `Tombstone`, `TombstoneCause`, `StagedRow`, `StagedRows` | What a client received for one server tick, as the sync point applies it: spawns (or upserts) with the server's key and origin, moves, component removals, rows by (origin key, component) with their server tick and precision, tombstones, the time scale, the processed-input acknowledgement, substituted inputs and region changes |
| `StagedInbox`, `DueFrames` | The service resource `app` fills between runs and a client world's `StagedState` hook drains inside the tick, without a lock; `due` is what `app` folds into the step's `TickInput` |
| `RemoteSlot`, `InterpolationSlot` | The one component this crate adds to a client world: the entity's slot in `lactose-net-host`'s interpolation buffer, written by the hook from the staged spawn |
| `StagingFrame`, `GatheredRow`, `GatheredRows` | What the replication gather produced: every replicated row changed since the last gather, grouped by region, and the structural log since the last gather |
| `GatherState` | The gather's mark, cursor and frame under construction: the service resource the `net::gather` system takes and `app` takes the frame from |
| `RegionRelevance`, `OverlayPayload`, `RegionChange` | The message a region's relevance sends (overlay or digest, seed, resources, table), built from a `RegionExport`, and what the client builds its `RegionBundle` from |
| `register_types`, `setup`, `register_systems` | The registration triple `app` calls for every world kind |
| `lag::{LagCompensationHistory, LAG_COMPENSATION_MAX_TICKS}` | Named: the server's hit-validation history |

Hidden plumbing: none. The private fields of every type are the implementation's to
reshape; the one `#[system]`, `net::gather`, is reached only through `register_systems`.
Everything here is plain records, keys and ticks, read without allocating, computed without
a float or a clock, and keyed by `StableKey`; the crate carries the boundary `clippy.toml`
and its source is part of the boundary build ID.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| `app` (client, milestone 1 spike and 4) | Frames from the network thread into the inbox of each client world, and the due frames' time scale and region changes into the step's `TickInput` | `StagedInbox::push` (first, `#[must_use]`), then `StagedInbox::due(Tick(simulation.tick().0 + 1))`, the tick the next step runs, into `TickInput::{time_scale, loaded, unloaded, assets}` through `RegionRelevance::to_bundle`; the fold rule under "The staged frame" |
| | The client worlds at the server's tick | `SimulationBuilder::tick(accepted.tick)`, or on a re-join `World::set_tick` with `StagedInbox::reset` beside it, from `lactose-net-host`'s `JoinAccepted` |
| `app` (server) | Register the gather and take its frame | `register_systems`, `setup`; `GatherState::take` between runs, the frame to the packer |
| | The relevance message on a region entering a client's set | `RegionRelevance::from_export` over `RegionExport` and `persist`'s overlay; `with_digest` for a client that caches |
| `lactose-net` (milestone 4) | The client's `StagedState` hook's input and output | `StagedInbox::pop_through`, `StagedFrame`'s fields, `RemoteSlot`; rows written and `removed` applied through `lactose-ecs`'s `World::insert_erased` and `World::remove_erased`, spawns through `World::spawn_erased` |
| | Resync and late join as one path | `StagedFrame::resync`, `full` |
| The milestone 1 spike | The same, typed by hand for its few components | `tests/consumers.rs`: the client hook, the prediction hook, `app` between runs on both ends, the gather |
| `lactose-predict` | Binds for its own predicted spawns; which entities the local player has claimed, as claims and releases arrive; the proxies' per-tick history; the reconciliation record; the processed-input acknowledgement; substituted ticks; moves and removals | `StagedSpawn::origin` with the local player; `StagedSpawn::slot` on a spawn of a live key (`None`: claimed, start predicting it; `Some`: released, stop); `StagedRow` with `exact: false` by `(key, tick)`; `StagedRow` with `exact: true` for claimed entities; `StagedFrame::{acked_input, substituted, moves, removed}`; `StagedInbox::oldest_after` for `Behind`, `StagedInbox::reset` on a re-join |
| `lactose-presentation` (milestone 2) | Which slot to sample per remote entity | `RemoteSlot` rows through extract, into `lactose-net-host`'s `InterpolationReader::sample_batch` |
| `lactose-persist` | Nothing new: the overlay it diffs is what `RegionRelevance` carries | `RegionRelevance::from_export(export, baseline_hash, overlay, table)` |
| Game crates | Nothing: no game system names this crate | `RemoteSlot` is never a system parameter |

## The registration `app` writes

```rust
use lactose_core::{Result, Tick};
use lactose_ecs::WorldKind;
use lactose_sim::{Simulation, SimulationBuilder, StagedState, TypeTable};

fn build(kind: WorldKind, start: Tick, hook: Option<Box<dyn StagedState>>) -> Result<Simulation> {
    let mut types = TypeTable::new();
    lactose_net_api::register_types(&mut types)?;
    // game_sim::register_types(&mut types)?; ...
    let mut builder = SimulationBuilder::new(kind, types)
        .tick(start) // the server's tick from `JoinAccepted` on a client
        .setup(lactose_net_api::setup)
        .systems(lactose_net_api::register_systems);
    if let Some(hook) = hook {
        builder = builder.staged_state(hook); // `net`'s on the client, `predict`'s in prediction
    }
    builder.build()
}
```

The same three calls serve the server (where `register_systems` places the gather and the
inbox is unused), the client (where the gather is skipped by its `worlds`) and the
prediction world. `setup` inserts `StagedInbox` and `GatherState` as service resources,
which `build`'s `derived` check does not see.

## Behaviour the signatures cannot express

**The staged frame** (architecture sections 6, 9.2, 9.3 and 9.4)

- One `StagedFrame` per server tick, built on the network thread from every packet that
  arrived for that tick, and shared as one `Arc` between `app` and the two client worlds'
  hooks. Every keyed field is a map, so a frame cannot hold two entries for one key and
  the order packets arrived in decides nothing; `push_row` keeps the newest tick per
  (origin key, component key) and returns `false` for an older or equal one, for a row
  from a tick after the frame's, and for a row of another length than the one held (the
  receiver checks each row's length against the registry before it pushes it, so a held
  row is overwritten in place and the arena never grows with the packets); and the
  network thread never stages a row older than one it already staged for that (key,
  component), in this frame or an earlier one. Rows live in one byte arena: reading a
  frame allocates nothing, and `rows` iterate in ascending (origin key, component key)
  order.
- `tick` is the server tick the frame is for; no row in it is from a later tick, and rows
  may be from earlier ones. The client's worlds are built at, or `set_tick` to (with the
  inbox `reset`), the tick of `JoinAccepted`, which is the tick of the first full frame;
  the replicated world then steps once per server tick in order under `Directed` control,
  so the `tick` its hook receives is a server tick. The prediction world runs ahead of the server, so every
  accepted frame is due at its next step.
- Which tick is which: the step for `tick` advances the world to `tick` and runs the hook
  with it; before that step, `Simulation::tick` is `tick - 1`. Every rule below that names
  a step's tick means the tick the hook receives.
- The inbox: `push` accepts a frame whose tick is after the newest tick the inbox ever
  accepted, queued or already popped, and refuses any other (`false`, and the call is
  `#[must_use]`), so no row can go back in time across frames. Under the receiver rules
  of `crates/net-host/README.md` ("Handles and packets": a packet is staged only if its
  tick is after the newest frame staged; from a resync request or the join until its full
  frame nothing is staged) every frame the network thread hands over is accepted, and a
  refusal is a defect in the receiver, logged. `due(tick)` is the queued frames whose tick
  is not after `tick`, oldest first; `pop_through(tick)` hands the hook the same frames,
  one at a time, removing each. A frame's time scale and region changes go in the
  `TickInput` of the step whose hook pops that frame: `app` pushes every frame it
  received, then folds `due(Tick(simulation.tick().0 + 1))`, the frames due at the tick
  the next step runs, into that step's `TickInput`, and only accepted frames are ever
  folded. Folding `due(simulation.tick())` instead leaves out every frame that arrived in
  time, and its region loads are lost. `oldest_after(tick)` is the tick of the oldest
  queued frame after `tick`, the first one `due(tick)` leaves out; `lactose-predict` asks
  it with the present tick. `reset` empties the inbox and forgets the newest tick it
  accepted: a re-join that moves a client world's tick with `World::set_tick` calls
  `reset` on that world's inbox in the same between-run access, since the new session's
  ticks may lie before the old session's and `push` would refuse every one of them,
  the full frame included. Frames of the old session not yet pushed when `reset` is called
  are dropped, never pushed after it: one accepted after the reset would make `push`
  refuse the new session's.
- The fold, per step: `time_scale` is the last `Some` among the due frames. Region changes
  fold per region in frame order with the later change winning, except that an
  `Irrelevant` followed by a `Relevant` becomes an unload plus a load, which the sim
  applies in that order within the step: `Irrelevant` removes a pending `loaded` and
  `assets` entry and inserts `unloaded`; `Relevant` inserts `loaded` (through
  `RegionRelevance::to_bundle`) and leaves a pending `unloaded` in place; `Assets` inserts
  `assets` unless the region is unloaded in the step. For a `full` frame, `app` first
  clears the `loaded` and `assets` an earlier due frame of the same step folded, then
  puts every region the world has loaded (`World::regions`) in `unloaded`, so that the
  step loads exactly the regions the full frame names, each reloaded from its message.
- Who reads what: `app` reads `time_scale` and `regions`; the replicated world's hook
  (`lactose-net`'s) applies `spawns`, `moves`, `removed`, the rows and `despawns`; the
  prediction world's hook (`lactose-predict`'s) reads `spawns` whose `origin` names the
  local player (binds) and spawns of a live key, whose `slot` says whether the local
  player now has the entity claimed (`None`: start predicting it) or released it (`Some`:
  stop), `moves`, `removed`, the rows (`exact: false` into the proxies' per-tick history,
  `exact: true` into the reconciliation record), `acked_input` and `substituted`. The
  prediction world's hook also applies spawns of unclaimed entities, moves, removals and
  tombstones as the replicated world's does, without `RemoteSlot`, so the prediction world
  holds every relevant entity as a proxy; a claimed entity's spawn, rows, move, removal and
  tombstone are reconciliation facts it never writes directly (D122).
  `TimeScale` is a replicated world resource, and resources are not gathered: the
  server's `app` puts its own value in the frame.
- What the replicated world's hook does, in this order and nothing else, for each frame
  `pop_through` hands it: each spawn in ascending `NetId` order, as an upsert if
  `StagedSpawn::key(net)` is live (each component written whole so that an equal value
  does not dirty the row, through `World::insert_erased`, or `Mut::set_if_neq` in a hook
  typed by hand, added if the row lacks it; `RemoteSlot` set to `slot` or removed when
  `slot` is `None`) and otherwise `World::spawn_erased` in its region with its components,
  then `World::bind(origin, net)` if `origin` is `Some`, then `World::insert(key,
  RemoteSlot { slot })` if `slot` is `Some`; then each move in ascending key order,
  `World::move_to_region(key, region)` unless `World::region_of(key)` is already `region`
  (an owned entity moved with its owner) or the key is not live; then each removal in
  ascending (key, component) order, `World::remove_erased` of that component, nothing if
  the entity or the component is absent; then each row in ascending (key, component)
  order, written as an upsert writes a component (so a row for a pair
  removed in the same frame re-adds the component with the newer value), nothing if the
  entity does not exist (its spawn was lost and the server resends it, since nothing of
  it is acknowledged); then each tombstone in ascending key order, `World::despawn` if
  the entity exists, nothing otherwise, since a region that became irrelevant took its
  rows with it, and nothing for a `TombstoneCause::Left` tombstone of an entity that
  `World::region_of` places in the tombstone's region. The hook despawns nothing else. An entry that fails (a spawn into a region
  the world has not loaded, a move or a decode the world refuses) is logged and skipped,
  never returned as the hook's error: the frame is already popped, and the rest of it
  applies. A frame never names a key both in `spawns` and in `despawns`, never names a
  tombstoned key in `removed`, and never holds a row for a key it tombstones.
- Why an upsert: a region becoming relevant loads its baseline and overlay rows through
  the bundle, and the stream then spawns those same entities; a full frame spawns entities
  the client holds; a change of claim is sent as a spawn of the live entity with `slot`
  set (the player released it) or `None` (the player claimed it). In every case the row is
  live and the spawn carries the whole entity, so the hook writes it whole.
- `moves` come from the gather's `Moved` entries. Positions on the wire are relative to the
  region the entity resides in, and the wire states which region each quantised position
  is relative to, so the network thread adds the right origin after a move; an entity
  moving into a region the client does not have leaves the client's set with a tombstone,
  and one moving into a relevant region from one the client does not have arrives as a
  spawn.
- A frame names an entity in a region only if that region is relevant to the client as of
  that frame, at the latest by its own region changes; the client loads a frame's region
  changes in the step that pops it. The packer names no entity of a region until the
  reliable channel has delivered the region's `RegionRelevance`, and the receiver puts
  each region change into the first frame it stages at or after the change's tick
  (`crates/net-host/README.md`, "Interest and the packer" and "Handles and packets").
- A region leaving a client's set is sent as `Irrelevant` beside a tombstone, at the tick
  it leaves, for every entity of the client's set residing in it (D121, the decision on
  region leave). Of the two ways to stop an entity moved into a region made relevant and
  irrelevant again in one step from staying where it was in both client worlds until a
  full frame (review round 4 of `lactose-predict`, N1), `lactose-net-host`'s packer sends
  the tombstones, and the fold is unchanged: the step that folds a region's `Relevant` and
  then `Irrelevant` into no load still applies the tombstone, which takes the entity out.
  The tombstones also release the entities' handles and interpolation slots on both ends,
  which nothing else does.
- A tombstone's `cause` separates a destruction (`Destroyed`, naming the region the
  entity was destroyed in) from an entity alive on the server that left the client's set
  (`Left`, naming the region it resides in: the one leaving the set, or the one it moved
  into that the client may not name). The hook skips a `Left` tombstone when the entity
  resides in the region it names (D121 addendum, review r1 of `lactose-net-host`, B2).
  The server has the entity in that region, unless the entity is entering again, whose
  whole send and move follow the tombstone's acknowledgement; so that region settles the
  client's copy: its unload is still to come, or the server names the region and the
  entity enters again. A load of the region after the entity left holds it with the
  cooked rows the stream never sends, which a despawn followed by the re-entry spawn
  would lose. In the N1 case the entity resides in the region it came from, so the
  tombstone still takes it out. A later tombstone of the entity replaces a `Left` one the
  packer still owes, and the receiver stages it under the key the `Left` one released
  (review r2 of `lactose-net-host`, N1), so an entity destroyed, or moved to a region the
  client does not hold, before the server saw its `Left` tombstone acknowledged is
  despawned even where the hook kept it.
- `removed` comes from the gather's `Removed` entries of replicated components: a status
  effect ending, a body removed. Without it the client and the prediction world keep the
  component's last value forever and the packer resends it on every whole send; with it
  the packer's mirror drops the row and the hook removes the component.
- `exact` is true for the rows of entities the receiving player has claimed: the server
  sends every simulation-read field of predicted entities at full precision on each update
  (section 9.4), as the component's memory image. Every other row is a dequantised image:
  the network thread dequantises the wire position and adds the region origin before
  staging, so the hook writes one shape of bytes whatever the precision, and the
  prediction world's proxies and the replicated rows hold the same values.
- A delta frame that holds an exact row of an entity holds every replicated component the
  server's entity has at the frame's tick: the packer sends a claimed entity whole in
  every packet that carries it and the receiver stages a packet whole or not at all, and
  `lactose-predict` removes from a claimed entity what such a frame does not name.
- `StagedSpawn::slot` is `None` for an entity the receiving player has claimed, which the
  prediction world presents; `Some` for every other entity, which the network thread
  allocated from its interpolation writer before staging the spawn, and whose states it
  pushes from then on. The hook writes it as `RemoteSlot`, `derived` and `extracted`, so
  extract learns the slot and the renderer samples by it, and removes it when a later
  spawn carries `None`; the slot is released by the network thread when it stages the
  tombstone or the claim.
- `full` marks the full relevance-set snapshot, the one message late join and resync share
  (section 9.3): `regions` names every relevant region as `Relevant`, `spawns` names every
  relevant entity with every replicated component, `time_scale` and `acked_input` are
  `Some`, and `app`'s unload-everything rule above makes every region reload. The hook
  applies it as any frame. The sequencing around it is the receiver's (`crates/net-host/
  README.md`, "Handles and packets"): from the `ResyncRequest` or the `JoinAccepted` until
  it stages the full frame, the network thread stages nothing and drops every packet whose
  tick is not after the full frame's, and it clears its handle mirror and its
  interpolation slots when it stages that frame, since the frame's spawns carry fresh
  ones; so the full frame is the next frame the inbox sees and is accepted.
  `StagedFrame::resync(tick)` is the constructor.
- `substituted` holds the receiving player's input ticks for which the server applied the
  previous input (section 9.2); the server's `app` knows them from `PlayerSample::
  substituted` in the frames it built, and `lactose-predict` re-simulates from the oldest.

**The gather** (architecture sections 5.3, 6 and 9.3; `crates/sim/README.md`, "Runs")

- `net::gather` is an exclusive system in `Pass::Gather`, `worlds = [server]`, taking
  `Job` and `ResMut<GatherState>`; `register_systems` places it and `app` orders any other
  exclusive system of that pass against it with `Registration::order`. It runs once per
  frame after the frame's ticks, so one frame covers every tick since the previous
  gather, up to the catch-up cap, and never runs against a discarded tick.
- `GatherState::gather(world, job)` first decides full or incremental, before any row is
  read: with no mark yet, a stale mark (`World::is_current(mark)` false: a whole-world load
  or the change-stamp rebase, section 6) or a stale cursor (`StructuralLog::since` on a
  copy of the cursor returning `None`), the gather is a full pass, below. Otherwise it
  produces, into the frame `app` has not yet taken: for every type `world.registry()`
  holds whose `tracking().replicated` is set and whose role is `Component`, in ascending
  key order, every chunk of `World::columns_erased` whose column tick is not before the
  mark's tick is kept (the skip is by tick, before any fork, and costs nothing per row;
  only an incremental gather skips); the kept chunks are forked over inside `job`'s scope
  with `fork_range`; in each, the rows `ErasedColumn::changed_since(mark)` selects, by
  stamp and exactly as the typed `World::changed_since` would, go into the frame as
  (region, origin key, component, row tick, bytes); the children's partials are
  concatenated and the frame `finish`ed, so the result is in ascending (region, key,
  component) order whatever the chunk layout or worker count, and a gather with nothing
  changed since the last is empty. Then `StructuralLog::since(cursor)` is appended in log
  order. Then `World::mark()` and `StructuralLog::cursor()` are taken for the next gather.
  Replicated resources are not gathered (`TimeScale` travels in the frame's own field,
  region resources in the relevance message).
- The cost, measured in the first design review (release, 100k rows by 20 replicated
  components, 4,000 changed rows per tick, load about 5): skipping chunks by column tick
  and forking over the rest, about 0.8 ms per gather; forking over every one of the
  20,640 chunks, about 2.9 ms; a serial scan, about 0.9 ms. A full pass over the same
  world cost 220 to 280 ms with a stand-in that allocated per row, an overestimate.
- The full pass: `full` is set, every row of every replicated column is in the frame (no
  chunk is skipped, since the decision was made before the scan), `structural` is empty.
  A full frame replaces the packer's mirror and voids every client's acked ticks; the
  packer then resyncs every client with a full staged frame, never from the frame alone,
  since a despawn in the gap has no log entry.
- `SimulationBuilder::log_retention` must be at least the timer root's catch-up cap (the
  `TickPacer`'s, chosen by `app`; four in the sketches): a frame that runs several ticks
  trims entries older than the retention before the gather runs, so a retention below the
  cap makes the cursor stale at every gather after a catch-up frame and every gather a
  full pass. `build` cannot check it, since the cap is `app`'s; the sim README states the
  same rule.
- `GatherState` is the frame's owner. `gather` merges into the frame `app` has not taken
  (`StagingFrame::merge`: of two rows for one (key, component) the row with the later tick
  wins, and on equal ticks the newer frame's; structural entries append, a full frame
  supersedes; and the newer frame's structure applies to the older rows: a row of an
  entity it despawned or of a component it removed goes, a row of an entity it moved is
  re-homed to the region the entity resides in now), so a frame not taken loses nothing,
  the frame never holds two values of one row, and the merged frame equals what one
  gather over both windows would have produced; a full older frame stays full and keeps
  `structural` empty; `take` moves it out between runs,
  leaving an empty frame at the same tick, and the packer owns and drops it. Nothing else
  clears it. `app`'s between-run access is `Simulation::world_mut().service_mut::<
  GatherState>()`, an access to `net`'s output and not to the world's state (the sim README
  sentence on `world_mut` is amended by D121 to say so).
- `StagingFrame::region(r)` is the per-region dirty list of section 9.3: the rows residing
  in `r`, found by binary search over the sorted rows, in ascending (key, component)
  order; every client to whom `r` is relevant reads the same slice, so packing cost is
  clients times relevant rows. `structural` carries the `Despawned` entries that become
  tombstones, the `Spawned` entries that make an entity enter every interested client's
  set whole, the `Moved` entries that become `StagedFrame::moves` and change which dirty
  list a row is in, and the `Removed` entries of replicated components that become
  `StagedFrame::removed` and drop the row from the mirror (`Removed` of an unreplicated
  component and `Added` are carried and ignored).
- `push_row`, `push_structural` and `finish` are public so a test or the spike builds a
  frame by hand; `rows` and `region` on a frame pushed to and not finished are a
  `debug_assert` failure.

**The relevance message** (architecture sections 9.3 and 15.2; `crates/sim/README.md`,
"Regions")

- When a region enters a client's set, the server's `app` exports it between runs
  (`World::export_region`, the same export a save or an unload makes), runs `persist`'s
  diff against the cooked baseline into an `Overlay`, and builds `RegionRelevance::
  from_export(export, baseline.content_hash(), overlay, table)`: the seed, the persistent
  resources and the transient keys are copied from the export, the overlay is `Full`
  (`Cooked` when the diff is empty and the caller passes `None`), the table is the
  region's resolved `VersionTable` if it has one. Co-located clients entering the same
  region in one frame share one export and one overlay. `with_digest` replaces a `Full`
  overlay by `Digest(overlay.digest())` for a client the server knows holds a copy (it was
  sent earlier in the session, or the client said so on join); the client's cache policy
  is `net`'s. The message travels on `net`'s reliable ordered channel, with the time scale
  and the other control messages.
- `to_bundle(baseline, cached, assets)` on the client gives the `RegionBundle` for
  `TickInput::loaded`: `baseline`, fetched from the store by `baseline_hash`, which must
  equal its content hash (`VersionMismatch` otherwise); the overlay, `Full(overlay)`
  or for `Digest(d)` the `cached` copy, which must exist and have digest `d`
  (`VersionMismatch` otherwise: the client sends an `OverlayRequest`), either way after
  `Overlay::check_against(baseline)` (`VersionMismatch` if made against another baseline),
  or `None` for `Cooked`; `resources` and `transient_resources` as carried; `assets` as
  the client's `app` pinned it from the table, or `None` if the table has not arrived;
  `mode: RegionMode::Replicated`. The sim's load path then validates everything before
  `create_region`, as it does a save. The region's entities then arrive as spawns, which
  the hook applies as upserts over the rows the bundle loaded.
- A `RegionChange::Irrelevant` becomes `TickInput::unloaded`, which despawns the region's
  entities; the tombstones sent beside it ("The staged frame") release their slots and
  handles. `RegionChange::Assets(table)` is a table published or replaced after
  relevance (a hot reload, section 9.3), pinned by `app` into `TickInput::assets`.

**Lag compensation** (architecture section 9.5)

- `LAG_COMPENSATION_MAX_TICKS` is `TICK_RATE_HZ / 4`, 15 ticks, 250 milliseconds;
  `LagCompensationHistory` is the named place of the history, shaped at milestone 4 with
  `physics-api`'s types.

## Not in this crate yet

- The readiness hold (architecture section 14): holding a player at a region boundary
  moves simulation state, so readiness enters the server tick as a journaled input at
  milestone 4, a `lactose-sim` addition (`SessionEvent::Ready { region }` or a `TickInput`
  field); `lactose-net-host`'s `ClientReady` is the message and `ClientInterest::ready`
  the network's copy only.
- A `lactose-predict` build check that prediction-world systems read only replicated or
  derived components: a persistent component that is not replicated is never corrected,
  so a prediction system reading it diverges silently.
- The packer, the byte encodings, the jitter window and the per-client input queue, the
  clock-sync estimator, the region-origin table: `lactose-net` and `lactose-net-host`.
- The interpolation of anything but a translation and a rotation: `lactose-net-host`.

## Proof for milestone 1

Every row below holds under `cargo nextest run -p lactose-net-api`; the last column names
the test file under `tests/` that holds it (or the unit tests in `src/staging.rs`).
`tests/consumers.rs` also runs every dependent's sketch, and `tests/sour.rs` replicates a
scripted scene to a client under SOUR (cooked baseline rows, a bound predicted spawn,
untaken frames merged, a region unloaded and reloaded, a claim and its release), asserts
that every path of the hook ran, and records both worlds' hashes.

| What | How | Held by |
|---|---|---|
| The gather | Under SOUR's layout mode: a world with replicated rows changed on several ticks gives the same `StagingFrame` (rows, order, bytes, structural entries) under every chunk capacity and order and with one or many workers; a second gather with nothing changed is empty; a `world_mut` write between gathers appears exactly once; a `set_tick`-driven stamp rebase or a restore makes the next frame `full` with `structural` empty; a retention below the catch-up cap makes every gather after a catch-up frame full; a frame not taken merges with the next and holds one value per row, the newer; `region(r)` equals the rows of `r` in `rows()`; a `Moved` entry is in `structural` | `gather.rs` |
| The staged frame | `push_row` keeps the newest tick and returns `false` otherwise, also for a row after the frame's tick or of another length, and the arena does not grow; `rows` ascend by (key, component); `is_empty` is exact, `removed` included; `StagedInbox::push` refuses a frame not after the newest ever accepted, also after a pop, `due` and `pop_through` agree and hand frames up to a tick oldest first; `oldest_after` names the first frame `due` leaves out, also with a frame for the present still queued; after `set_tick` back to an earlier tick, `reset` lets the full frame and the next one in, which are refused without it; the sketch hook applied to a world leaves spawns bound, `RemoteSlot` set, a live entity upserted with its slot set or removed, moves applied once, removed components gone (and re-added by a row of the same frame), rows written and tombstoned entities gone, a `Left` tombstone skipped only for an entity residing in the region it names, an equal row leaves the row's stamp alone, and a failing entry leaves the rest of the frame applied; a region loaded by its bundle and then spawned by the stream ends with `RemoteSlot` set; a full frame over a client that holds stale rows ends with the frame's values and slots | `staged.rs` |
| The fold | Under the real sim: a frame for the tick the next step runs is folded and popped in that step (worlds built at 10, a full frame for 10, a delta for 12 loading a region and spawning into it: the region is loaded and the entity live after the step for 12); Relevant then Irrelevant in one step leaves the region unloaded; Irrelevant then Relevant unloads and loads it; a refused frame enters nothing; a full frame unloads every loaded region and reloads the relevant ones; a delta then a full frame due in one step load exactly the full frame's regions | `fold.rs` |
| The gather's decision | A stale cursor with a current mark gives a full frame with every row, none skipped; a current cursor with a stale mark the same | `gather.rs` (stale cursor), `src/staging.rs` (stale mark, which no public call reaches: every epoch change also resets the structural log) |
| The relevance message | `from_export` copies the seed, resources and transient keys; `to_bundle` builds the bundle the sim loads, and refuses the wrong baseline, a digest without a cache or with another digest, and an overlay against another baseline, carried or cached, each as `VersionMismatch` | `relevance.rs` |
