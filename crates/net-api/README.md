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
against (`ErasedColumn::changed_since`, `World::move_to_region`,
`World::move_alone_to_region`, `World::spawn`, `bind`, `insert`, `get_mut`, `remove`,
`despawn`, `despawn_alone`); neither is restated here.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `app`, `lactose-net`, `lactose-net-host`,
`lactose-predict`, `lactose-persist` and the milestone 1 spike are written against this
surface. `tests/consumers.rs` sketches what each of them writes and runs the sketches: a
change that breaks it is a signature change to list.

## Surface

| Item | Purpose |
|---|---|
| `StagedFrame`, `StagedSpawn`, `Tombstone`, `TombstoneCause`, `StagedRow`, `StagedRows` | What a client received for one server tick, as the sync point applies it: spawns (or upserts) with the server's key and origin, the transient components of claimed entities by key, moves, component removals, rows by (origin key, component) with their server tick and precision, tombstones, the time scale, the processed-input acknowledgement, substituted inputs and region changes |
| `StagedInbox`, `DueFrames` | The service resource `app` fills between runs and a client world's `StagedState` hook drains inside the tick, without a lock; `due` is what `app` folds into the step's `TickInput` |
| `RemoteSlot`, `InterpolationSlot` | The one component this crate adds to a client world: the entity's slot in `lactose-net-host`'s interpolation buffer, written by the hook from the staged spawn |
| `StagingFrame`, `GatheredRow`, `GatheredRows` | What the replication gather produced: every replicated row changed since the last gather, grouped by region, the structural log since the last gather, and which entities have which unreplicated transient components |
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
| | The relevance message on a region entering a client's set | `lactose-persist`'s `RegionRecords::from_export` over the `RegionExport`, then `to_relevance(table)` (D129); `with_digest` for a client that caches |
| `lactose-net` (milestone 4) | The client's `StagedState` hook's input and output | `StagedInbox::pop_through`, `StagedFrame`'s fields, `RemoteSlot`; rows written and `removed` applied through `lactose-ecs`'s `World::insert_erased` and `World::remove_erased`, spawns through `World::spawn_erased`, moves and tombstones through `World::move_alone_to_region` and `World::despawn_alone` |
| | What to state of a claimed entity's transient components | `StagingFrame::transient` from the gather, kept in the packer's mirror; `StagedFrame::transient` beside every claimed spawn and every claimed entity's exact rows ("Transient presence") |
| | Resync and late join as one path | `StagedFrame::resync`, `full` |
| The milestone 1 spike | The same, typed by hand for its few components | `tests/consumers.rs`: the client hook, the prediction hook, `app` between runs on both ends, the gather |
| `lactose-predict` | Binds for its own predicted spawns; which entities the local player has claimed, as claims and releases arrive; the proxies' per-tick history; the reconciliation record; the processed-input acknowledgement; substituted ticks; moves and removals | `StagedSpawn::origin` with the local player; `StagedSpawn::slot` on a spawn of a live key (`None`: claimed, start predicting it; `Some`: released, stop); `StagedRow` with `exact: false` by `(key, tick)`; `StagedRow` with `exact: true` for claimed entities; `StagedFrame::{acked_input, substituted, moves, removed}`; `StagedFrame::transient` for the transient components it spawns or corrects a claimed entity with; `StagedInbox::oldest_after` for `Behind`, `StagedInbox::reset` on a re-join |
| `lactose-presentation` (milestone 2) | Which slot to sample per remote entity | `RemoteSlot` rows through extract, into `lactose-net-host`'s `InterpolationReader::sample_batch` |
| `lactose-persist` | The message's fields, `pub`, which its `RegionRecords::to_relevance` fills from a region's records (D129) | `RegionRelevance`, `OverlayPayload` |
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
  `#[must_use]`), so no row can go back in time across frames. Under the receiver rules of
  `crates/net-host/README.md` ("Handles and packets": a packet is staged only if its tick is
  after the newest frame staged; from a resync request or the join until its full frame
  nothing is staged) every frame the network thread hands over is accepted, and a refusal is
  a defect in the receiver, logged. `due(tick)` is the queued frames whose tick is not after
  `tick`, oldest first; `pop_through(tick)` hands the hook the same frames, one at a time,
  removing each. A frame's time scale and region changes go in the `TickInput` of the step
  whose hook pops that frame: `app` pushes every frame it received, then folds
  `due(Tick(simulation.tick().0 + 1))`, the frames due at the tick the next step runs, into
  that step's `TickInput`, and only accepted frames are ever folded. Folding
  `due(simulation.tick())` instead leaves out every frame that arrived in time, and its
  region loads are lost. `oldest_after(tick)` is the tick of the oldest queued frame after
  `tick`, the first one `due(tick)` leaves out; `lactose-predict` asks it with the present
  tick. `accepted_tick` is the newest tick ever accepted, queued or popped: `app` resyncs a
  client that fell behind, or met a frame older than its ring (`TooOld`), to at or beyond
  it, since a target short of a frame already queued is behind again at the next frame.
  `reset` empties the inbox and forgets the newest tick it accepted: a re-join that moves a
  client world's tick with `World::set_tick` calls `reset` on that world's inbox in the same
  between-run access, since the new session's ticks may lie before the old session's and
  `push` would refuse every one of them, the full frame included. Frames of the old session
  not yet pushed when `reset` is called are dropped, never pushed after it: one accepted
  after the reset would make `push` refuse the new session's.
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
  tombstone are reconciliation facts it never writes directly (D122). `transient` is read
  by the prediction world's hook and corrections alone, for the entities the local player
  has claimed ("Transient presence" below; D141).
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
  `World::move_alone_to_region(key, region)` unless `World::region_of(key)` is already
  `region` or the key is not live (D141); then each removal in
  ascending (key, component) order, `World::remove_erased` of that component, nothing if
  the entity or the component is absent; then each row in ascending (key, component)
  order, written as an upsert writes a component (so a row for a pair
  removed in the same frame re-adds the component with the newer value), nothing if the
  entity does not exist (its spawn was lost and the server resends it, since nothing of
  it is acknowledged); then each tombstone in ascending key order, `World::despawn_alone`
  if the entity exists (D141), nothing otherwise, since a region that became
  irrelevant took its rows with it, and nothing for a `TombstoneCause::Left` tombstone of an
  entity that `World::region_of` places in the tombstone's region. The hook despawns nothing
  else: it applies each move and each tombstone to the one entity named, never to what
  that entity owns ("One entity per entry" below). It writes replicated components alone:
  a component of a spawn, or a row, that the registry lacks or holds as unreplicated is a
  defect of the sender, logged and skipped with the rest of the spawn applied (D141),
  and it does not read `transient`. An entry that fails (a spawn into a region
  the world has not loaded, a move or a decode the world refuses) is logged and skipped,
  never returned as the hook's error: the frame is already popped, and the rest of it
  applies. A frame never names a key both in `spawns` and in `despawns`, never names a
  tombstoned key in `removed`, and never holds a row for a key it tombstones.
- Why an upsert: a region becoming relevant loads its baseline and overlay rows through
  the bundle, and the stream then spawns those same entities; a full frame spawns entities
  the client holds; a change of claim is sent as a spawn of the live entity with `slot`
  set (the player released it) or `None` (the player claimed it). In every case the row is
  live and the spawn carries the whole entity, so the hook writes it whole.
- One entity per entry (D141). `OwnedBy` is persistent and not replicated, so a
  client holds ownership edges only as each region's relevance document stated them, and
  what the server gives or releases afterwards does not reach it before the region's next
  document. `World::despawn` and `World::move_to_region` take the entities the target
  owns, by the edges the world holds; a hook that called them would take an entity the
  server released after the document along with its former owner, on the client alone, and
  no later row would bring it back, since a row of an entity that is not live is skipped.
  The hook therefore applies a tombstone with `World::despawn_alone` and a move with
  `World::move_alone_to_region`. Nothing is lost by that: the server's structural log holds
  a `Despawned` or a `Moved` entry for every entity of a tree it despawned or moved, and
  the packer turns each into an entry of its own for every client that holds the entity:
  a tombstone, a move, or a spawn where the move brings the entity into the client's set.
  The entries of one tree may reach a client in different frames, since a cap
  makes sends partial; until an owned entity's own entry arrives it stays where it was,
  its owner gone or elsewhere, its `OwnedBy` a reference to an entity that is not live or
  not there, which a client's world holds like any other. The two methods are a client's
  alone: a `Server` world refuses them, since a despawn that leaves an owned entity behind
  can leave a region that cannot be exported (`crates/ecs/README.md`, Regions), and a
  client exports nothing. An owner that comes back, by a re-entry spawn or a region's next
  document, owns again what the client's stale edge says it owns, so a released entity can
  again be taken by a prediction system's cascade (`docs/verification.md`), never by the
  hook. An entity a region's document placed whose handle the client does not hold has
  no tombstone coming, owned or not (`crates/net-host/README.md`, "Interest and the
  packer", the open item). A game that needs the server's ownership on a client needs
  `OwnedBy` replicated: a change to this contract, which must first settle how a client
  applies a row that closes a cycle against an edge it has not yet seen removed
  (`docs/verification.md`).
- Transient presence (D141). A row the stream builds holds the replicated
  components the frames carry. A region document also restores the presence of every
  transient component (D129), and a kernel runs on a row only if the row has every
  component it names; so a claimed entity that the stream built, and no document placed,
  would never be run by a kernel that names a transient component the server's entity has.
  `StagedFrame::transient` carries that presence for the entities the receiving player has
  claimed, the only rows a kernel runs on in a client's worlds: by origin key, the set of
  the entity's components whose persistence is transient and that are not replicated. A
  statement is whole (the server's entity has exactly these) and carries no value, since
  such a column is zero at every input sync point on both machines. The packer keeps each
  entity's set in its mirror from `StagingFrame::transient` and puts a statement, the empty
  set for an entity that has none, beside every spawn whose `slot` is `None` and beside
  every claimed entity's exact rows; so a frame that states a claimed entity's replicated
  components states its transient ones too, and `lactose-predict` builds or corrects the
  entity from that frame alone. It states nothing of an entity the player has not claimed,
  and a statement of one is ignored. The bound: a statement names a component once at most
  (it is a set) and names only registered components that are transient and not
  replicated, at most as many as the registry holds; the receiver refuses a packet whose
  statement names any other, as it refuses a row of another length, and a hook skips such
  a key with a log. On the wire one bit per such component is enough, in the registry's
  ascending key order, which is the same on both ends because the protocol hash covers
  every transient type's key, persistence and tracking; the encoding is `lactose-net`'s.
  No registered type changes, so the protocol hash does not move. The prediction world's
  hook and corrections apply a statement (`crates/predict/README.md`, "What a row built
  from the stream has"). The replicated world's hook does not read it, since no kernel
  runs there: a replicated-world row that the stream built lacks the transient components
  a document would have given it, as it lacks every other unreplicated one. A frame with
  no statement for an entity says nothing of its unreplicated transient components, and
  they are left as they are. A replicated transient component needs no statement: it is
  among a spawn's
  components and among a claimed entity's exact rows. A derived component is never
  stated: the system that owns it rebuilds it.
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
- A tombstone's `cause` separates a destruction (`Destroyed`, naming the region the entity
  was destroyed in) from an entity alive on the server that left the client's set (`Left`,
  naming the region it resides in: the one leaving the set, the one it moved into that the
  client may not name, or, for an entity that already owed a tombstone, any region it moved
  into). The hook skips a `Left` tombstone when the entity resides in the region it names
  (D121 addendum, review r1 of `lactose-net-host`, B2). The server has the entity in that
  region, since the tombstone it owes follows the entity on every move (review r3 of
  `lactose-net-host`, N5); so that region settles the client's copy: its unload is still to
  come, or the server names the region and the entity enters again. A load of the region
  after the entity left holds it with the cooked rows the stream never sends, which a
  despawn followed by the re-entry spawn would lose. The prediction world's hook also
  requires the entity to have resided there after the frames last placed it
  (`lactose-predict`'s `StagedRegion`, D122's addendum), so a proxy a prediction system
  moved into that region is despawned as the replicated world's copy is. In the N1 case the
  entity resides in the region it came from, so the tombstone still takes it out. A later
  tombstone of the entity replaces a `Left` one the packer still owes, and the receiver
  stages it under the key the `Left` one released (review r2 of `lactose-net-host`, N1), so
  an entity destroyed, or moved to a region the client does not hold, before the server saw
  its `Left` tombstone acknowledged is despawned even where the hook kept it.
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
  previous input (section 9.2); the server's `app` knows them from
  `PlayerSample::substituted` in the frames it built, and `lactose-predict` re-simulates
  from the oldest whose substitute, or the repair after it, changes the sample the
  prediction world ran; one equal to it byte for byte rolls nothing back. A substituted tick
  *t* is in `substituted` of the first frame the inbox accepts at or after *t* that does not
  hold region changes alone, unless a full frame is accepted at or after *t* first (its
  rebuild supersedes the substitution), and may be repeated in later frames: `lactose-net`'s
  sender puts *t* in every packet whose tick is at or after *t* until it sees acknowledged a
  packet that carried it, and the receiver acknowledges only a packet it staged and stages
  none not after the newest it staged (`crates/net-host/README.md`, "Handles and packets");
  a frame of region changes alone is the receiver's split of two changes of one region,
  which drops the packet unacknowledged. So with frames accepted in ascending tick, no frame
  of a tick at or after *t* that carries a spawn, move, removal, row or tombstone is popped
  before the first that names *t*, unless a full frame at or after *t* comes first.
  `lactose-predict` acts on the first report and skips a repeat for at least ten seconds of
  ticks after *t*; a later repeat may cost a resync. A first report later than such a frame
  would roll the prediction world back before that frame's claim or release, which a
  re-simulated tick never re-applies and no later call could restate once an earlier one had
  reconciled it (D122, addendum of the claim replay).

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
- Transient presence (D141). The gather also fills `StagingFrame::transient`, for
  the packer's mirror: which components that are transient and not replicated each entity
  has, by origin key; presence and never a value, since the gather copies rows of
  replicated columns alone and such a column is zero at every input sync point. A full
  pass states every entity that has at least one,
  and an entity it does not state has none. An incremental gather states, of the entities
  live at the gather, each one the structural log since the cursor spawned that has at
  least one, and each one whose set that log changed (an `Added` or a `Removed` entry of
  such a component), with the set the entity has at the gather, which for a changed one
  may be empty; an entity it does not state has none if the log spawned it and is
  unchanged otherwise. So the packer's mirror gives an entity it first meets no such
  component until a frame states one, forgets an entity's set with the entity at its
  `Despawned` entry, and applies a frame's structural entries before its statements. The
  set is read by key and never by chunk or row, at most one `World::get_erased` for each
  entity the window spawned or changed and each registered component that is transient and
  not replicated (three in the engine: the input sample, the root motion and the sim
  pose), so a statement is the same under every chunk layout and worker count; a full
  pass reads each such column's keys once. The gather reads the set because a `Spawned`
  entry names no component, so the packer could not derive it from the log. `merge` keeps
  the newer
  frame's statement of an entity, drops the statement of an entity the newer frame
  despawned, takes a full newer frame's statements alone, and leaves a full frame with no
  empty set (a full frame states an entity that has none by not stating it), so a merged
  frame states what one gather over both windows would.
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
  component and `Added` are carried and ignored by the packer; the gather has already
  read those of an unreplicated transient component into `transient`).
- `push_row`, `push_structural` and `finish` are public so a test or the spike builds a
  frame by hand; `rows` and `region` on a frame pushed to and not finished are a
  `debug_assert` failure.

**The relevance message** (architecture sections 9.3 and 15.2; `crates/sim/README.md`,
"Regions")

- When a region enters a client's set, the server's `app` exports it between runs
  (`World::export_region`, the same export a save or an unload makes), runs
  `lactose_persist::RegionRecords::from_export(export, &baseline, &schemas)` and builds the
  message with `RegionRecords::to_relevance(table)`, which fills the fields `from_export`
  here fills: the seed, the resources (canonical, as a save holds them) and the transient
  keys from the records, the overlay `Full`, or `Cooked` when the diff was empty (D129), the
  table is the region's resolved `VersionTable` if it has one. Co-located clients entering
  the same region in one frame share one export and one overlay. `with_digest` replaces a
  `Full` overlay by `Digest(overlay.digest())` for a client the server knows holds a copy
  (it was sent earlier in the session, or the client said so on join); the client's cache
  policy is `net`'s. The message travels on `net`'s reliable ordered channel, with the time
  scale and the other control messages.
- `to_bundle(baseline, cached, assets)` on the client gives the `RegionBundle` for
  `TickInput::loaded`: `baseline`, fetched from the store by `baseline_hash`, which must
  equal its content hash (`VersionMismatch` otherwise); the overlay, `Full(overlay)` or for
  `Digest(d)` the `cached` copy, which must exist and have digest `d` (`VersionMismatch`
  otherwise: the client sends an `OverlayRequest`), either way after
  `Overlay::check_against(baseline)` (`VersionMismatch` if made against another baseline),
  or `None` for `Cooked`; `resources` and `transient_resources` as carried; `assets` as the
  client's `app` pinned it from the table, or `None` if the table has not arrived; `mode:
  RegionMode::Replicated`, the replicated world's mode. A prediction world's regions are
  `Simulated` instead once their table is published, since it runs its kernels and steps its
  bodies only there: `app` sets `RegionBundle::mode` to `Simulated` on the bundle it folds
  into the prediction world when `assets` is `Some`, `Frozen` when it is `None` (the
  `RegionChange::Assets` that brings the table sets it `Simulated`), and builds that world
  under `RegionControl::Directed` so the lifecycle loads the mode the bundle names. A region
  the server holds `Frozen` with its table published is simulated by the client's
  prediction world, because `RegionChange` carries no mode. Each update corrects a claimed
  entity there, since the server sends a claimed entity whole in every packet
  (`lactose-net-host`'s whole-send rule, architecture section 9.4); a sender of changed rows
  alone would leave it uncorrected until the next full frame. A mode variant is a change to
  this crate, decided before `lactose-net`'s packer sends region changes. The sim's load
  path then validates everything before `create_region`, as it does a save. The region's
  entities then arrive as spawns, which the hook applies as upserts over the rows the
  bundle loaded.
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

## Proof for D141

Each row is a test and the sentence it holds; the first two are the whole-project review's
S3 probe as a test of this crate's hook sketch (`tests/common/mod.rs`, `ClientHook`), which
applies moves and tombstones through `World::move_alone_to_region` and
`World::despawn_alone`, as the sketches of `tests/consumers.rs` do.

| Test | What it holds | File |
|---|---|---|
| `a_released_item_survives_its_former_owners_tombstone` | A `Client` world loads a region document in which an item is `OwnedBy` an owner. The server then releases the item, which no frame carries, and destroys the owner: one frame, one tombstone, the owner's. After the step the owner is gone and the item is live in its region with every component the document gave it, its `OwnedBy` as written; the server's next row of the item is written. With `World::despawn` in the hook the item is gone and the row skipped, which is the failure the test names | `staged.rs` |
| `a_released_item_stays_when_its_former_owner_moves` | The same document; the frame moves the owner to another loaded region. The owner resides there and the item where it was; a later frame's move of the item moves it | `staged.rs` |
| `every_entity_of_a_destroyed_tree_leaves_by_its_own_tombstone` | An owner and what it owns, both destroyed on the server, their tombstones in two frames: the owned entity is live between them, its owner not, and gone after its own; the structural log holds one `Despawned` entry for each | `staged.rs` |
| `the_hook_writes_replicated_components_alone` | A spawn that carries, beside its replicated components, a derived one and an unreplicated transient one as values gives the row neither, logs both and applies the rest; a row of an unreplicated component is skipped; a frame's `transient` statements change nothing in a `Client` world | `staged.rs` |
| `a_statement_is_part_of_a_frame` | Two frames that differ in one statement are unequal; a frame that holds a statement alone is not empty | `staged.rs` |
| `the_gather_states_what_it_spawned_and_what_changed` | Under every layout and with one and several workers, equal frames: an incremental gather states an entity spawned in its window that has such a component, with its set, and states nothing of one spawned without any; it restates an entity that gained or lost one (the empty set after the last), states nothing of an entity whose rows alone changed, and never names a replicated, persistent or derived component; an entity spawned and despawned in one window is not stated, and one despawned and spawned again without any leaves no statement | `gather.rs` |
| `a_full_gather_states_every_entity_that_has_one` | After a stale mark: every entity with an unreplicated transient component is stated with its set, and no other entity | `gather.rs` |
| `a_merged_frame_states_what_one_gather_would` | Two frames not taken: the newer statement of an entity wins, the statement of an entity the newer frame despawned is gone, a full newer frame's statements stand alone, an empty set merged into a full frame leaves no statement, and the result equals one gather over both windows | `gather.rs` |
