# lactose-net-api

Network contracts: what the simulation side (`sim`, `predict`, `app`, the game crates) and
a transport agree on, so that the milestone 1 netcode spike is written against them and
milestone 4's `lactose-net` implements them. The staged frame a client applies at the
input sync point, the frame the server's replication gather fills, the relevance message,
the quantised wire values, the handshake, the transport trait, the per-connection handles
and interest state, and the lock-free interpolation buffer the presentation schedule
samples.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | outside; boundary crates may use the `staged`, `staging`, `relevance`, `quant` and `lag` modules (and their root re-exports) for types and for operations that are deterministic by construction (architecture section 8.2); `handshake`, `transport`, `packet`, `interest`, `interpolation`, `clock` and `message` are host-only |
| Unsafe | none, `#![forbid(unsafe_code)]`; the sequence lock is built from atomics alone |

**Status: facade.** Every public signature is final and every body that decides anything is
`todo!()`; nothing here runs. This file is the contract the implementation lane meets, and
every sentence under "Behaviour the signatures cannot express" is one a dependent crate may
rely on. Architecture sections 4, 5.1, 5.3, 6, 7, 8.2, 9.1 to 9.7, 10, 11, 14, 15.2, 19 and
22 are the design; where this file and the architecture disagree, the architecture wins and
this file is wrong. Decision D121 records the choices those sections left open.
`crates/sim/README.md` is the contract the hooks and the pass here meet ("The input sync
point": `StagedState`; "Runs": `run_passes` and `Pass::Gather`; "Regions": `RegionExport`
and the relevance overlay) and `crates/ecs/README.md` the storage contract they write
against; neither is restated here.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `app`, `lactose-net`, `lactose-predict`,
`lactose-presentation`, `lactose-persist` and the milestone 1 spike are written against
this surface. `tests/consumers.rs` is a compile-only sketch of what each of them writes: a
change that breaks it is a signature change to list.

## Surface

| Item | Purpose |
|---|---|
| `StagedFrame`, `StagedSpawn`, `Tombstone`, `StagedRow`, `StagedRows` | What a client received for one server tick, as the sync point applies it: spawns with the server's key and origin, rows by (origin key, component) with their server tick and precision, tombstones, the time scale, the processed-input acknowledgement, substituted inputs and region changes |
| `StagedInbox` | The service resource `app` fills between runs and a client world's `StagedState` hook drains inside the tick, without a lock |
| `RemoteSlot`, `InterpolationSlot` | The one component this crate adds to a client world: the entity's slot in the interpolation buffer, written by the hook from the staged spawn |
| `StagingFrame`, `GatheredRow`, `GatheredRows` | What the replication gather produced: every replicated row whose tick advanced, grouped by region, and the structural log since the last gather |
| `GatherState` | The gather's mark, cursor and frame under construction: the service resource the `net::gather` system takes and `app` takes the frame from |
| `RegionRelevance`, `OverlayPayload`, `RegionChange` | The message a region's relevance sends (overlay or digest, seed, resources, table), built from a `RegionExport`, and what the client builds its `RegionBundle` from |
| `QuantisedPosition`, `QuantisedRotation`, `POSITION_UNITS_PER_METRE`, `ROTATION_SCALE` | The wire values and their conversions through `lactose_math` |
| `register_types`, `setup`, `register_systems` | The registration triple `app` calls for every world kind |
| `handshake::{Handshake, ProtocolHash, BoundaryBuildId, CookedDataHash, HandshakeMismatch, PROTOCOL_VERSION}` | Host-only: what each side sends first and the rule that refuses a connection |
| `transport::{Transport, ConnectionId, Datagram, TransportEvent, DisconnectReason, LinkStats, MAX_DATAGRAM_BYTES}` | Host-only: unreliable, unordered datagrams with a connection identity |
| `packet::{EntityHandle, ConnectionHandles, AssignedHandles, PacketSequence, AckField, TickHeader, PacketRecord, PacketRing}` | Host-only: the framing shapes, the per-connection handle table and the packet-to-entity ring |
| `interest::{ClientInterest, EntityInterest, Priority, SendBudget, RelevancePolicy}` | Host-only: the per-client state the gather's frame and the packer share |
| `interpolation::{InterpolationBuffer, InterpolationWriter, InterpolationReader, ReceivedState, PresentationTime, RemoteSample, SampleStatus, STATES_PER_SLOT}` | Host-only: the lock-free buffer, its one writer and its readers, and the batch sampling call |
| `clock::{ClockEstimate, MAX_RATE_ADJUST_THOUSANDTHS}` | Host-only, named: what the timer root reads from clock sync |
| `lag::{LagCompensationHistory, LAG_COMPENSATION_MAX_TICKS}` | Named: the server's hit-validation history |
| `message::{ClientReady, ResyncRequest, ResyncReason}` | Host-only: the readiness acknowledgement and the resync request |

Hidden plumbing: none. The private fields of every type are the implementation's to
reshape; the one `#[system]`, `net::gather`, is reached only through `register_systems`.

## What boundary code may use

Architecture section 8.2 lets a boundary crate depend on a non-boundary foundation crate
"only for types and for operations that are deterministic by construction: no clocks, no
floats computed outside `math`". The modules keep the two sides apart so a later lint can
enforce it on paths:

| Module | Side | Why it is deterministic by construction |
|---|---|---|
| `staged` (root re-exports) | boundary may use | Plain records, keys and ticks; a frame is read without allocating; `lactose-predict`'s hook reads it |
| `staging` (root re-exports) | boundary may use | Reads the world through `columns_erased` and the structural log, keys every row by `StableKey`, sorts by keys; no float |
| `relevance` (root re-exports) | boundary may use | Documents, keys and tables; the bundle it builds is what `TickInput::loaded` carries |
| `quant` (root re-exports) | boundary may use | Integers, and scalars computed through `lactose_math` in the stated order |
| `lag` | boundary may use | Ticks; hit validation runs inside the simulation |
| `handshake`, `transport`, `packet`, `interest`, `message` | host-only | The network thread's and the packer's; a transport reads a clock |
| `interpolation`, `clock` | host-only | Read by the presentation schedule and the timer root, outside the tick |

Proposed rule for the lint: a crate in `[boundary]` may name `lactose_net_api::{staged,
staging, relevance, quant, lag}` and the crate root, and nothing under
`lactose_net_api::{handshake, transport, packet, interest, interpolation, clock, message}`.
`crate-tiers.toml` must list `lactose-net-api` under `[boundary_internal].any` before
`lactose-predict` depends on it (proposed text in D121).

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| `app` (client, milestone 1 spike and 4) | Frames from the network thread into the inputs and both client worlds | `StagedFrame::{time_scale, regions}` into `TickInput::{time_scale, loaded, unloaded, assets}` through `RegionRelevance::to_bundle`; the `Arc<StagedFrame>` into each world's `StagedInbox` between runs |
| | The clock estimate to pace by | `clock::ClockEstimate`, as `net` publishes it |
| `app` (server) | Register the gather and take its frame | `register_systems`, `setup`; `GatherState::take` between runs, the frame to the packer |
| | The relevance message on a region entering a client's set | `RegionRelevance::from_export` over `RegionExport` and `persist`'s overlay; `with_digest` for a client that caches |
| `lactose-net` (milestone 4) | A transport to drive, a handshake to check, handles and packets to encode | `transport::Transport`, `handshake::Handshake::{to_bytes, from_bytes, check}`, `packet::*` |
| | Per-client state the packer keeps against the gather's frame | `interest::ClientInterest`, `StagingFrame::region` per relevant region, `PacketRing`, `ClientInterest::acknowledge` |
| | The client's `StagedState` hook's input and output | `StagedInbox::pop_through`, `StagedFrame`'s fields, `RemoteSlot`; rows written through the erased write proposed below, or typed per registered type until then |
| | The interpolation writer on the network thread | `InterpolationWriter::{allocate, push, release}`, the slot into `StagedSpawn::slot` |
| | Resync and late join as one path | `StagedFrame::resync`, `full` |
| The milestone 1 spike | Everything above over a throwaway transport, typed by hand for its few components | `tests/consumers.rs` sketches each piece: the loopback `Transport`, the client hook, the gather, the packer, the receiver, the sampler |
| `lactose-predict` | Binds for its own predicted spawns; the proxies' per-tick history; the reconciliation record; the processed-input acknowledgement; substituted ticks | `StagedSpawn::origin` with the local player; `StagedRow` with `exact: false` by `(key, tick)`; `StagedRow` with `exact: true` for claimed entities; `StagedFrame::{acked_input, substituted}` |
| `lactose-presentation` (milestone 2) | Which slot to sample per remote entity, one batch sample per frame, render space | `RemoteSlot` rows through extract; `InterpolationReader::sample_batch` into a caller's slice; `RemoteSample::position` into `RenderPosition::from_sim` |
| `lactose-persist` | Nothing new: the overlay it diffs is what `RegionRelevance` carries | `RegionRelevance::from_export(export, baseline_hash, overlay, table)` |
| Game crates | Nothing: no game system names this crate | `RemoteSlot` is never a system parameter |

## The registration `app` writes

```rust
use lactose_core::Result;
use lactose_ecs::WorldKind;
use lactose_sim::{Simulation, SimulationBuilder, StagedState, TypeTable};

fn build(kind: WorldKind, hook: Option<Box<dyn StagedState>>) -> Result<Simulation> {
    let mut types = TypeTable::new();
    lactose_net_api::register_types(&mut types)?;
    // game_sim::register_types(&mut types)?; ...
    let mut builder = SimulationBuilder::new(kind, types)
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
  (origin key, component key) and returns `false` for an older or equal one. Rows live in
  one byte arena: reading a frame allocates nothing, and a frame's `rows` iterate in
  ascending (origin key, component key) order.
- `tick` is the server tick the frame is for; no row in it is from a later tick, and rows
  may be from earlier ones. The client's replicated world steps once per server tick, in
  order, under `Directed` control, so the `tick` its hook receives is a server tick;
  `pop_through(tick)` hands the hook every queued frame whose tick is not after it, oldest
  first, and a frame delayed by loss applies at the first sync point after it arrives.
  The prediction world runs ahead of the server, so its hook receives every frame as
  soon as `app` queued it. `StagedInbox::push` refuses a frame whose tick is not after the
  newest queued, since the network thread emits frames in order.
- Who reads what: `app`, between runs, reads `time_scale` (into `TickInput::time_scale`
  and its pacer) and `regions` (into `TickInput::{loaded, unloaded, assets}` under
  `Directed` control, through `RegionRelevance::to_bundle`), then queues the frame in both
  inboxes. The replicated world's hook (`lactose-net`'s) applies `spawns`, the rows and
  `despawns`. The prediction world's hook (`lactose-predict`'s) reads `spawns` whose
  `origin` names the local player (binds), the rows (`exact: false` into the proxies'
  per-tick history, `exact: true` into the reconciliation record), `acked_input` and
  `substituted`. `TimeScale` is a replicated world resource, and resources are not
  gathered: the server's `app` puts its own value in the frame.
- What the replicated world's hook does, in this order and nothing else, for each frame
  `pop_through` hands it: for a `full` frame, despawns every entity the frame does not
  spawn; then each spawn in ascending `NetId` order, `World::spawn` under
  `StagedSpawn::key(net)` in its region with its components, then `World::bind(origin,
  net)` if `origin` is `Some`, then `World::insert(key, RemoteSlot { slot })` if `slot` is
  `Some`; then each row in ascending (key, component) order, written whole so that an
  equal value does not dirty the row (`Mut::set_if_neq`, or the erased write proposed
  below), added if the row lacks the component, dropped and logged if the entity does not
  exist (its spawn was lost and the server resends it, since nothing of it is
  acknowledged); then each tombstone in ascending key order, `World::despawn` if the
  entity exists, nothing otherwise, since a region that became irrelevant took its rows
  with it. A frame never names a key both in `spawns` and in `despawns`, and never holds a
  row for a key it tombstones.
- `exact` is true for the rows of entities the receiving player has claimed: the server
  sends every simulation-read field of predicted entities at full precision on each update
  (section 9.4), as the component's memory image. Every other row is a dequantised image:
  the network thread dequantises the wire position and adds the region origin before
  staging, so the hook writes one shape of bytes whatever the precision, and the
  prediction world's proxies and the replicated rows hold the same values.
- `StagedSpawn::slot` is `None` for an entity the receiving player has claimed, which the
  prediction world presents; `Some` for every other entity, which the network thread
  allocated from its `InterpolationWriter` before staging the spawn, and whose states it
  pushes from then on. The hook writes it as `RemoteSlot`, `derived` and `extracted`, so
  extract learns the slot and the renderer samples by it; the slot is released by the
  network thread when it stages the tombstone.
- `full` marks the full relevance-set snapshot, the one message late join and resync share
  (section 9.3): `regions` names every relevant region as `Relevant` (`app` puts every
  other loaded region in `TickInput::unloaded` of the same frame, and the sim applies
  unloads before loads), `spawns` names every relevant entity with every replicated
  component, `time_scale` and `acked_input` are `Some`. `StagedFrame::resync(tick)` is the
  constructor. The hook's despawn-everything step is what makes the same apply path serve
  both.
- `substituted` holds the receiving player's input ticks for which the server applied the
  previous input (section 9.2); the server's `app` knows them from `PlayerSample::
  substituted` in the frames it built, and `lactose-predict` re-simulates from the oldest.

**The gather** (architecture sections 5.3, 6 and 9.3; `crates/sim/README.md`, "Runs")

- `net::gather` is an exclusive system in `Pass::Gather`, `worlds = [server]`, taking
  `Job` and `ResMut<GatherState>`; `register_systems` places it and `app` orders any other
  exclusive system of that pass against it with `Registration::order`. It runs once per
  frame after the frame's ticks, so one frame covers every tick since the previous
  gather, up to the catch-up cap, and never runs against a discarded tick.
- `GatherState::gather(world, job)` produces, into the frame `app` has not yet taken: for
  every type `world.registry()` holds whose `tracking().replicated` is set and whose role
  is `Component`, in ascending key order, every chunk of `World::columns_erased` whose
  column tick is not before the gather's mark tick, and in it every row whose row tick is
  not before the mark tick, as (region, origin key, component, row tick, bytes); one child
  per chunk inside `job`'s scope, the children's partials concatenated and the frame
  `finish`ed, so the result is in ascending (region, key, component) order whatever the
  chunk layout or worker count. Then `StructuralLog::since(cursor)` is appended in log
  order. Then `World::mark()` and `StructuralLog::cursor()` are taken for the next
  gather. Replicated resources are not gathered (`TimeScale` travels in the frame's own
  field, region resources in the relevance message).
- The comparison is by tick, not by stamp position, since erased columns expose ticks: a
  row written directly between ticks before a gather (`world_mut`, which `app` does not do
  to a replicated column) is gathered twice. The packer and the client treat a row as
  idempotent, so a repeated row costs bandwidth and nothing else.
- With no mark yet, a mark `World::is_current` rejects, or a cursor `since` rejects (a
  whole-world load or the change-stamp rebase, section 6), the gather is a full pass:
  `full` is set, every row of every replicated column is in the frame, `structural` is
  empty. The packer treats a full frame as every client's acked ticks voided: each is
  resent whole, or resynced with a `full` staged frame.
- `GatherState` is the frame's owner. `gather` merges into the frame `app` has not taken
  (`StagingFrame::merge`: a newer row replaces an older one for the same (key, component),
  structural entries append, a full frame supersedes), so a frame not taken loses nothing
  and the frame never holds two values of one row; `take` moves it out between runs,
  leaving an empty frame at the same tick, and the packer owns and drops it. Nothing else
  clears it. `app`'s between-run access is `Simulation::world_mut().service_mut::<
  GatherState>()`, an access to `net`'s output and not to the world's state (the sim README
  sentence on `world_mut` is amended by D121 to say so).
- `StagingFrame::region(r)` is the per-region dirty list of section 9.3: the rows residing
  in `r`, found by binary search over the sorted rows, in ascending (key, component)
  order; every client to whom `r` is relevant reads the same slice, so packing cost is
  clients times relevant rows. `structural` carries the `Despawned` entries that become
  tombstones, the `Spawned` entries that make an entity enter every interested client's
  set whole, and the `Moved` entries that change which dirty list a row is in.
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
  is `net`'s.
- `to_bundle(baseline, cached, assets)` on the client gives the `RegionBundle` for
  `TickInput::loaded`: `baseline`, fetched from the store by `baseline_hash`, which must
  equal its content hash (`VersionMismatch` otherwise); `Full(overlay)` after
  `Overlay::check_against(baseline)` (`VersionMismatch` if made against another baseline),
  or for `Digest(d)` the `cached` copy, which must exist and have digest `d`
  (`VersionMismatch` otherwise: the client requests the full overlay), or `None` for
  `Cooked`; `resources` and `transient_resources` as carried; `assets` as the client's
  `app` pinned it from the table, or `None` if the table has not arrived; `mode:
  RegionMode::Replicated`. The sim's load path then validates everything before
  `create_region`, as it does a save.
- A `RegionChange::Irrelevant` becomes `TickInput::unloaded`; the region's entities need
  no tombstones, since the unload despawns them, and the network thread releases their
  slots and handles. `RegionChange::Assets(table)` is a table published or replaced after
  relevance (a hot reload, section 9.3), pinned by `app` into `TickInput::assets`.

**Quantisation** (architecture section 9.3)

- `POSITION_UNITS_PER_METRE` is 1024, a power of two, about one millimetre.
  `QuantisedPosition::quantise(relative)` takes a simulation position with the region
  origin already subtracted (the region's world origin is a cook-time fact the packer
  looks up; it is not in this crate) and computes, per coordinate, `(c *
  Scalar::from_i32(1024)).round().to_i32()`: nearest with ties away from zero, saturating,
  zero for NaN. `dequantise` computes `Scalar::from_i32(q) / Scalar::from_i32(1024)`, an
  exact division, so `quantise(dequantise(q)) == q` for every `q` and the same bits come
  out on every target. Both go through `lactose_math` only.
- `ROTATION_SCALE` is `i16::MAX`. `QuantisedRotation::quantise(q)` normalises `q`, makes
  its sign canonical (`Quat::canonical_sign`), and quantises each component as the position
  does with the scale `32767`, so the product never leaves `i16`. `dequantise` divides each
  component by the scale (one true division each) and normalises; the all-zero value,
  which no `quantise` produces, gives `Quat::IDENTITY`. `IDENTITY` is `(0, 0, 0, 32767)`
  and the type's `Default`.
- What is quantised and what is not: the packer quantises the translation and rotation of
  entities the receiving player has not claimed; every other field of theirs and every
  field of claimed entities travels as the component's bytes. Which component holds the
  transform is `physics-api`'s to say; this crate names no component of another crate.

**The handshake** (architecture section 9.1)

- `Handshake { protocol, build, cooked }` is what each side sends first, twenty-four bytes:
  the three hashes as little-endian `u64`s in field order (`to_bytes`); every byte string of
  that length decodes (`from_bytes`). `check(peer)` compares in field order and reports the
  first difference as a `HandshakeMismatch`, which converts into an `Error` of kind
  `VersionMismatch`; either side refuses the connection on an error, and a datagram that is
  not twenty-four bytes is refused before `check`.
- `ProtocolHash::of(registry)` is xxh64, seed zero, over one byte stream: first the protocol
  constants, `PROTOCOL_VERSION` as `u32`, `TICK_RATE_HZ` as `u32`,
  `POSITION_UNITS_PER_METRE` as `i32`, `ROTATION_SCALE` as `i32` and `MAX_DATAGRAM_BYTES`
  as `u32`; then, for every registered type `covers` accepts, in ascending component key
  order: the key as `u64`, the schema version as `u32`, the role as `u8` (0 component, 1
  resource), the persistence as `u8` (0 persistent, 1 transient, 2 derived), the tracking
  as `u8` (bit 0 replicated, bit 1 extracted), then the layout: size as `u32`, alignment
  as `u32`, the count of non-padding fields as `u32`, and each non-padding field in
  declaration order as its ID as `u16`, offset as `u32`, size as `u32`, its type tag
  (`Field::NAME`) as a `u32` length and UTF-8 bytes, and its kind as a tag byte (0 `Bool`,
  1 `Unsigned`, 2 `Signed`, 3 `Float`, 4 `Scalar`, 5 `EntityRef`, 6 `AssetRef`, 7 `Enum`,
  8 `Struct`, 9 `Array`) followed, for `Enum`, by its size as `u8`, its variant count as
  `u32` and each variant's value as `u32` in declaration order; for `Struct`, by the nested
  layout in the same form; for `Array`, by its length as `u32`, the element's type tag and
  the element's kind in the same form. Everything little-endian. Type, field and variant
  names and retired IDs are excluded; the type tag is the one name included, since it is
  the stable tag of the schema dump and tells `u32` from `RegionId`.
- `covers(info)` is: `tracking().replicated`, or `persistence()` is `Persistent` or
  `Transient`. Replicated types cross the wire; persistent and transient types are what
  the world hash and a desync digest compare, so a client whose hashed layout differs
  cannot explain a mismatch. A `derived` type that is not replicated never crosses and is
  never compared, whatever crate declares it: presentation types differ between client and
  server by design (section 9.1), and so may `derived` boundary types. The input sample
  (`transient`) and `TimeScale` (`derived, replicated`) are covered; `ClaimedBy`,
  `StreamingAnchor`, `RemoteSlot` and `RegionPolicy` are not.
- `BoundaryBuildId` and `CookedDataHash` are values `app` supplies: the first from the
  build tooling (the `Cargo.lock` entries, resolved feature set and source hash of the
  boundary crates, section 8.3), the second from the asset store over the pack index
  entries of every `SimulationAsset` kind. Neither is computed here.

**The transport** (architecture sections 5.1 and 9.2)

- A `Transport` is owned and driven by the dedicated network thread; it is `Send` and
  nothing in it blocks. `send` hands one payload to the link as one datagram or drops it
  and never waits for delivery; it fails with `InvalidData` for a payload over
  `MAX_DATAGRAM_BYTES` (1200, under the usual path MTU) and nothing is sent, `NotFound` for
  a connection that is not established, `Network` for a failed socket. `poll` appends
  every ready event in the order observed and returns at once.
- A datagram arrives whole or not at all, possibly reordered or duplicated, never
  corrupted; `net` builds reliability, ordering and acknowledgement on top. For one
  connection, `Connected` is the first event, `Received` comes only between it and
  `Disconnected`, `Disconnected` is the last, and the transport never reuses the ID.
  `disconnect` on an established connection makes a later `poll` yield `Disconnected`
  with `Local`; on an unknown one it does nothing.
- How connections are established (listening, dialling) is a transport's constructor's;
  the trait sees connections as events, which is what lets a loopback be one page
  (`tests/consumers.rs`, section 10). `stats` is `None` for a connection that is not
  established, `rtt: None` for a transport that does not measure (then `net` estimates
  from its acknowledgements), `Some(Duration::ZERO)` for a loopback.

**Handles, packets and interest** (architecture section 9.3)

- `EntityHandle` is a sixteen-bit slot index and a sixteen-bit generation, four bytes;
  generation zero is `NONE`. `ConnectionHandles` is kept identically by the server, which
  assigns, and the client, which mirrors from spawns and tombstones: `assign(key)` returns
  the handle of an already-assigned key, else the most recently released slot, else the
  next never-used slot, at the slot's current generation (from one), and fails with
  `ResourceExhausted` at `EntityHandle::CAPACITY` (65536) assigned; `release(handle)`
  returns the key and advances the slot's generation (wrapping from `u16::MAX` to one), so
  the released handle never names the slot's next entity; `key_of` and `release` give
  `None` for a stale or unassigned handle. `iter` is in ascending key order.
- `PacketSequence` wraps at sixteen bits; `after` is the wrapping difference being
  positive and below 32768, so two sequences in play are less than that apart. `AckField
  { latest, bits }` acknowledges `latest` and, for bit `n` set, `latest - (n + 1)`, a
  window of 32 before it; `covers` says so. `TickHeader { sequence, tick, ack }` begins
  every packet: `tick` is the server tick the rows are no newer than, or on a client the
  newest input tick carried.
- `PacketRing` keeps the last `capacity` (`DEFAULT_CAPACITY` 64) `PacketRecord`s in
  sequence order: `record` pushes and, when full, evicts and returns the oldest, treated
  as lost; `acknowledge(sequence)` takes the record if held; `expire(oldest_live)` takes
  every record before it, treated as lost. An acknowledgement therefore advances exactly
  the entities a packet carried, and a loss re-queues exactly them.
- `ClientInterest` is everything the server keeps per connected client. `enter(key)`
  makes an entity relevant: a handle from `handles` and an `EntityInterest` with `acked:
  None`, so the entity is sent whole (every replicated component) until one state of it
  is acknowledged, then by changed rows since `acked`. `leave(key, tick)` owes a tombstone
  at `tick`; the entity stays until the tombstone is acknowledged, then is forgotten and
  its handle released, so the client's mirror releases the same slot at the same point.
  `acknowledge(ack)` applies the peer's field: every packet it covers is taken from the
  ring and each entity it carried advances `acked` to the carried tick if newer and clears
  `in_flight` if it names that packet; records older than `latest - 32` are expired and
  their entities' `in_flight` cleared, so they are sent again. `Priority` accumulates each
  frame from distance and staleness (the weights are `net`'s) and resets when the entity
  is sent, so an entity starved by the cap rises until it goes.
- `regions` is the client's relevant region set under `RelevancePolicy` (region membership
  within `radius` regions of the player's, plus the `always` set); a region entering is
  sent as a `RegionRelevance` and every entity residing in it enters; a region leaving is
  sent as `Irrelevant` and its entities leave without tombstones. `ready` holds the
  regions the client acknowledged with `ClientReady`; the server holds the player at the
  boundary of a relevant region not in it (section 14), which is `net`'s and the game's to
  do with the position.

**The interpolation buffer** (architecture sections 5.3 and 9.3)

- `InterpolationBuffer::split(capacity)` makes one buffer of `capacity` slots, allocating
  once, and hands out its one `InterpolationWriter` and an `InterpolationReader`, which
  clones. The writer is the network thread's; a reader is the `net-api` resource of
  section 5.3 that the presentation schedule holds. The buffer never feeds a simulation
  row: `Replicated` rows get the same states through the staged frame.
- Each slot is a sequence lock over a ring of the `STATES_PER_SLOT` (8) newest states by
  tick, every word an atomic: the writer makes the slot's sequence odd, stores the words,
  and makes it even (a release fence before and after the stores); a reader loads the
  sequence, then the words, then the sequence again (an acquire fence between), and
  retries when the first load was odd or the two differ. No `unsafe`, no lock, no
  allocation on either side; a reader never observes a half-written ring, and a writer is
  never delayed by readers. A reader's retry is bounded by the writer's rate (one push per
  received state), never by other readers.
- `allocate` takes the most recently released slot, else the next never-used, at a new
  generation (never zero, wrapping past `u32::MAX` to one), empty; `ResourceExhausted`
  when every slot is allocated. `push(slot, state)` keeps the eight newest states by tick:
  a state older than the oldest kept is dropped, one with the tick of a kept state replaces
  it, so arrival order does not matter. `release(slot)` makes the slot read as missing
  under the old generation from then on. `push` and `release` fail with `NotFound` for a
  stale or unallocated slot. Each state is the tick and the teleport flag packed in one
  word, the position's three scalars and the rotation's four, each as canonical bits
  widened to 64; the network thread hands the writer world-space values (the wire position
  dequantised plus the region origin).
- `sample(slot, at)`: `Interpolated` when two kept states bracket `at` (the earlier's tick
  not after `at`, the later's after it), position by `Vec3::lerp` and rotation by
  `Quat::nlerp` with the parameter `(at - earlier.tick) / (later.tick - earlier.tick)`,
  `at` taken as `tick + fraction / 65536`; `Held` at the newest state when `at` is not
  before it (the entity's updates are late), at the oldest when `at` is before every kept
  state, or at the state before a teleport while `at` is before the teleport's tick, so a
  teleport is never blended across; `Missing` when the slot holds no state or `slot`'s
  generation is not the slot's, in which case the caller keeps what it showed last.
  `sample_batch(slots, at, out)` is `sample` for each `slots[i]` into `out[i]`, `out` at
  least as long, allocating nothing; the presentation schedule computes `at` from
  `ClockEstimate`, about two send intervals behind the newest received tick.
- The presentation schedule learns which slot to sample from the `RemoteSlot` rows
  (through extract or the renderer's slot table), not from a lookup by key: there is no
  key-to-slot directory to lock, and a reused slot reads as missing under the old value.

**Clock sync, lag compensation and messages** (architecture sections 9.2, 9.5 and 14)

- `ClockEstimate` is what `net` publishes to the timer root: the server's tick now, the
  lead (half the round trip plus the jitter margin, in ticks under the replicated time
  scale), the rate adjustment clamped to `1000 ± MAX_RATE_ADJUST_THOUSANDTHS` (30, three
  percent), and `snap` when the drift exceeds what the catch-up cap can recover. The
  estimator is `net`'s; how the value reaches the timer root is `app`'s.
- `LAG_COMPENSATION_MAX_TICKS` is `TICK_RATE_HZ / 4`, 15 ticks, 250 milliseconds;
  `LagCompensationHistory` is the named place of the history, shaped at milestone 4 with
  `physics-api`'s types.
- `ClientReady { region, tick }` is the readiness acknowledgement; `ResyncRequest { tick,
  reason }` asks for a `full` frame, for an `AssetFailure` (the tick-stamped event of
  section 14, from `RegionLifecycle::take_asset_failures`), `BehindCatchUp` (section 11) or
  `Divergence`. Their encoding is `net`'s.

## Not in this crate yet

- An erased row write in `lactose-ecs`: the client's hook writes every registered
  replicated type from bytes, and the ECS has only typed writes (`World::insert`,
  `get_mut`, `columns_mut`), so milestone 4's hook needs either a per-type writer
  monomorphised at registration (as `lactose-sim`'s `TypeTable` monomorphises hashers) or
  the ECS methods proposed in D121: `World::insert_erased(key, &ComponentData) ->
  Result<bool>`, validating through the registry and stamping only when the bytes differ,
  and `World::spawn_erased(key, region, &[ComponentData])`, the direct form of what
  `load_region` already does. The spike's hook is typed by hand (`tests/consumers.rs`).
- The packer itself: filling each packet by priority up to `SendBudget`, delta coding,
  the byte encoding of every type here, the jitter window and the per-client input queue
  of `lactose_input::SampleHistory` windows, the clock-sync estimator, the region-origin
  table the packer subtracts: `lactose-net`, milestone 4.
- The server's hold of a player at a region boundary until `ClientReady`: `net` and the
  game, with `ClientInterest::ready` as the record.
- The interpolation of anything but a translation and a rotation (a scale, an animation
  phase): added to `ReceivedState` and the slot's words when presentation needs it.
- A lint that enforces the module split on boundary crates; the rule is proposed under
  "What boundary code may use".

## Proof for milestone 1

Nothing runs yet. The implementation lane replaces every `todo!()` and proves:

| What | How |
|---|---|
| The gather | Under SOUR's layout mode: a world with replicated rows changed on several ticks gives the same `StagingFrame` (rows, order, bytes, structural entries) under every chunk capacity and order and with one or many workers; a second gather with nothing changed is empty; a `world_mut` write between gathers appears once (or twice, documented) and never is lost; a `set_tick`-driven stamp rebase or a restore makes the next frame `full` with `structural` empty; a frame not taken merges with the next and holds one value per row, the newer; `region(r)` equals the rows of `r` in `rows()` |
| The staged frame | `push_row` keeps the newest tick and returns `false` otherwise; `rows` ascend by (key, component); `is_empty` is exact; `StagedInbox::push` refuses out-of-order frames and `pop_through` hands frames up to a tick oldest first; the sketch hook applied to a world leaves spawns bound, `RemoteSlot` set, rows written and tombstoned entities gone, and an equal row leaves the row's stamp alone |
| The relevance message | `from_export` copies the seed, resources and transient keys; `to_bundle` builds the bundle the sim loads, and refuses the wrong baseline, a digest without a cache or with another digest, and an overlay against another baseline, each as `VersionMismatch` |
| Quantisation | `quantise(dequantise(q)) == q` over a sweep; saturation at the `i32` and `i16` limits; NaN to zero; `q` and `-q` quantise alike; golden bits of `dequantise` for a small table, equal on x86-64 and aarch64 under SOUR's architecture mode |
| The handshake | Golden `ProtocolHash` for a small registry, pinned; adding a replicated or persistent type, raising a version, changing a field's ID, offset, size, tag or kind each change it; adding a derived unreplicated type does not; the constants block changes it; `check` reports the first difference in field order; `from_bytes(to_bytes(h)) == h` |
| Handles and packets | A released slot is reused with the next generation and the old handle is stale; 65536 assignments then `ResourceExhausted`; `after` and `covers` across the wrap; the ring evicts the oldest when full, `acknowledge` takes exactly one record, `expire` takes exactly the older ones |
| Interest | `enter` sends whole until an acknowledgement; `acknowledge` advances exactly the carried entities and clears exactly the acknowledged `in_flight`; an expired packet re-queues its entities; an acknowledged tombstone releases the handle |
| The interpolation buffer | A loom model or a stress test with a checksum word: under one writer and several readers every sample is a state that was pushed, never a mix; `sample` cases: bracketed, held newest, held oldest, held before a teleport, missing after release and under a stale generation; a push older than the ring is dropped and an equal tick replaces; `sample_batch` equals `sample` per slot and allocates nothing (a counting allocator in the test) |
