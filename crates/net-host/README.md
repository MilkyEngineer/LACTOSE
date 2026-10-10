# lactose-net-host

The host side of networking: the quantised wire values, the handshake, the transport trait
a loopback or UDP implementation is a page of, the packet framing and the per-connection
handles both ends keep, the per-client interest state the packer keeps against the
gather's frame, the lock-free interpolation buffer the presentation schedule samples, and
the named places for clock sync and the control messages. The simulation side, what enters
and leaves a world, is `lactose-net-api`, inside the boundary.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | outside, and no boundary crate may depend on it: it is absent from `[boundary_internal]` in `crate-tiers.toml` on purpose |
| Unsafe | none, `#![forbid(unsafe_code)]`; the sequence lock is built from atomics alone |

**Status: facade.** Every public signature is final and every body that decides anything is
`todo!()`; nothing here runs. This file is the contract the implementation lane meets, and
every sentence under "Behaviour the signatures cannot express" is one a dependent crate may
rely on. Architecture sections 4, 5.1, 5.3, 9.1, 9.2, 9.3, 9.4, 11, 14 and 22 are the
design; where this file and the architecture disagree, the architecture wins and this file
is wrong. Decision D121 records the choices those sections left open, for this crate and
for `lactose-net-api` together. `crates/net-api/README.md` is the contract the frames
this crate's receiver builds and its packer reads meet, and is not restated here.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `app`, `lactose-net`, `lactose-presentation`
and the milestone 1 spike are written against this surface. `tests/consumers.rs` is a
compile-only sketch of what each of them writes: a change that breaks it is a signature
change to list.

## Surface

| Item | Purpose |
|---|---|
| `quant::{QuantisedPosition, QuantisedRotation, POSITION_UNITS_PER_METRE, ROTATION_SCALE}` | The wire values and their conversions through `lactose_math` |
| `handshake::{Handshake, ProtocolHash, BoundaryBuildId, CookedDataHash, HandshakeMismatch, PROTOCOL_VERSION}` | What each side sends first and the rule that refuses a connection |
| `transport::{Transport, ConnectionId, Datagram, TransportEvent, DisconnectReason, LinkStats, MAX_DATAGRAM_BYTES}` | Unreliable, unordered datagrams with a connection identity |
| `packet::{EntityHandle, ConnectionHandles, AssignedHandles, PacketSequence, AckField, TickHeader, PacketRecord, PacketRing}` | The framing shapes, the per-connection handle table and the packet-to-entity ring |
| `interest::{ClientInterest, EntityInterest, Priority, SendBudget, RelevancePolicy}` | The per-client state the gather's frame and the packer share |
| `interpolation::{InterpolationBuffer, InterpolationWriter, InterpolationReader, ReceivedState, PresentationTime, RemoteSample, SampleStatus, STATES_PER_SLOT}` | The lock-free buffer, its one writer and its readers, and the batch sampling call |
| `clock::{ClockEstimate, MAX_RATE_ADJUST_THOUSANDTHS}` | Named: what the timer root reads from clock sync |
| `message::{JoinAccepted, ClientReady, OverlayRequest, ResyncRequest, ResyncReason}` | The control messages, named |

Hidden plumbing: none. The private fields of every type are the implementation's to
reshape.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| `lactose-net` (milestone 4) | A transport to drive, a handshake to check, handles and packets to encode | `transport::Transport`, `handshake::Handshake::{to_bytes, from_bytes, check}`, `packet::*` |
| | Per-client state the packer keeps against the gather's frame, and the mirror it packs from | `interest::ClientInterest`, `StagingFrame::region` per relevant region, `PacketRing`, `ClientInterest::acknowledge`; the mirror rule under "Interest and the packer" |
| | The receiver on the network thread: handles mirrored, positions dequantised, states into the buffer, frames staged | `ConnectionHandles::{insert, release, clear}`, `QuantisedPosition::dequantise`, `InterpolationWriter::{allocate, push, release}`, `lactose_net_api::StagedFrame` |
| | The client's clock estimate to publish | `clock::ClockEstimate` |
| `app` (client) | The player and the start tick on join; the estimate to pace by | `message::JoinAccepted`; `clock::ClockEstimate` |
| The milestone 1 spike | Everything above over a throwaway transport | `tests/consumers.rs`: the loopback `Transport`, the packer, the receiver, the sampler, the handshake, the join |
| `lactose-presentation` (milestone 2) | One batch sample per frame, render space | `InterpolationReader::sample_batch` into a caller's slice, by the `RemoteSlot` rows extract read; `RemoteSample::position` into `RenderPosition::from_sim` |
| `lactose-input-host` | Nothing: the input packets carry `lactose_input::SampleHistory` windows, which `net` encodes | - |

## Behaviour the signatures cannot express

**Quantisation** (architecture section 9.3)

- `POSITION_UNITS_PER_METRE` is 1024, a power of two, about one millimetre.
  `QuantisedPosition::quantise(relative)` takes a simulation position with the region
  origin already subtracted (the region's world origin is a cook-time fact the packer
  looks up; the table is not in this crate) and computes, per coordinate, `(c *
  Scalar::from_i32(1024)).round().to_i32()`: nearest with ties away from zero, saturating,
  zero for NaN. `dequantise` computes `Scalar::from_i32(q) / Scalar::from_i32(1024)`, an
  exact division. Under the 64-bit scalar `quantise(dequantise(q)) == q` for every `q`;
  under a 32-bit scalar only for integers within 2^24 units (sixteen kilometres) of the
  origin, beyond which the scalar cannot hold the integer. The same bits come out on every
  target; both go through `lactose_math` only.
- `ROTATION_SCALE` is `i16::MAX`. `QuantisedRotation::quantise(q)` normalises `q`, makes
  its sign canonical (`Quat::canonical_sign`), and quantises each component as the position
  does with the scale 32767, so the product never leaves `i16`. `dequantise` divides each
  component by the scale (one true division each) and normalises. Rotations do not
  round-trip exactly: `quantise(dequantise(q))` differs from `q` by one unit in one
  component for about one percent of values, and the value is stable after one trip
  (`quantise(dequantise(q1)) == q1` for `q1 = quantise(dequantise(q))`). A zero or NaN
  quaternion, whose normalisation is NaN, quantises to the all-zero value and nothing else
  does; the all-zero value dequantises to `Quat::IDENTITY`. `IDENTITY` is `(0, 0, 0,
  32767)` and the type's `Default`.
- What is quantised and what is not: the packer quantises the translation and rotation of
  entities the receiving player has not claimed; every other field of theirs and every
  field of claimed entities travels as the component's bytes. The wire states which region
  each quantised position is relative to (the entity's region at the packet's tick), so
  the receiver adds the right origin after a move. Which component holds the transform is
  `physics-api`'s to say; this crate names no component of another crate.

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
  as `u32`, every integer little-endian; then, for every registered type `covers` accepts,
  in ascending component key order, `TypeInfo::write_schema`, the per-type encoding
  `lactose-reflect` states byte for byte and pins with a golden test. The covered-set rule
  is this crate's; the encoding is `reflect`'s.
- `covers(info)` is: `tracking().replicated`, or `persistence()` is `Persistent` or
  `Transient`. Replicated types cross the wire; persistent and transient types are what
  the world hash and a desync digest compare, so a client whose hashed layout differs
  cannot explain a mismatch. A `derived` type that is not replicated never crosses and is
  never compared, whatever crate declares it. Hence the rule every crate obeys: a type
  registered on one side only (a presentation type the server does not register) is
  `derived` and unreplicated, or the handshake refuses every connection. The input sample
  (`transient`) and `TimeScale` (`derived, replicated`) are covered; `ClaimedBy`,
  `StreamingAnchor`, `RemoteSlot` and `RegionPolicy` are not.
- `BoundaryBuildId` and `CookedDataHash` are values `app` supplies: the first from the
  build tooling (the `Cargo.lock` entries, resolved feature set and source hash of the
  boundary crates, section 8.3, which now include `lactose-net-api`), the second from the
  asset store over the pack index entries of every `SimulationAsset` kind. Neither is
  computed here.

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
  (`tests/consumers.rs`). `stats` is `None` for a connection that is not established,
  `rtt: None` for a transport that does not measure (then `net` estimates from its
  acknowledgements), `Some(Duration::ZERO)` for a loopback.

**Handles and packets** (architecture section 9.3)

- `EntityHandle` is a sixteen-bit slot index and a sixteen-bit generation, four bytes;
  generation zero is `NONE`. `ConnectionHandles` is kept identically by the server, which
  `assign`s, and the client, which mirrors and never assigns: every spawn on the wire
  carries its handle, the client records it with `insert(handle, key)`, and a tombstone
  releases it with `release`. `assign(key)` returns the handle of an already-assigned key,
  else the most recently released slot, else the next never-used slot, at the slot's
  current generation (from one), and fails with `ResourceExhausted` at
  `EntityHandle::CAPACITY` (65536) assigned; `insert` takes the handle's generation for the
  slot, is a no-op for the same (handle, key) again, and fails with `InvalidData` if the
  slot holds another live key, if the slot's generation is newer than the handle's (a
  spawn reordered after the tombstone that released it; the comparison is wrapping, since
  generations wrap from `u16::MAX` to one, so "newer" is `slot.wrapping_sub(handle)` in
  `1..=32767`), or if the key has a different handle already; `release(handle)` returns
  the key and advances the slot's generation (wrapping from `u16::MAX` to one), so the
  released handle never names the slot's next entity; `clear` forgets every assignment
  and every generation, what the client does when it stages a full frame and only then;
  `key_of` and `release` give `None` for a stale or unassigned handle. `iter` is in
  ascending key order.
- The receiver's rules, which keep the mirror equal to the server's table and the inbox's
  newest-ever-accepted rule true by construction: the receiver stages a server packet
  only if its header tick is after the newest frame it has staged; any other packet (a
  late one, a reordered one, a duplicate) it drops whole and does not acknowledge, so the
  server resends what the packet carried, and no tombstone is applied before its spawn,
  no row before the row it would replace. From a `ResyncRequest` or `JoinAccepted` until
  the full frame is staged, it stages nothing and drops every packet whose tick is not
  after the full frame's (the full frame travels on the reliable channel, and deltas sent
  after it overtake it), and it clears its handle mirror and its interpolation slots when
  it stages that frame, not when it asks. On a resync the server clears that client's
  `ConnectionHandles` and re-enters every relevant entity, so the full frame's spawns
  carry fresh handles and no handle from before the resync names anything after it.
  Hence the frames the receiver hands `app` ascend strictly by tick and the full frame is
  the next frame after the request, and `StagedInbox::push` never refuses one.
- `PacketSequence` wraps at sixteen bits; `after` is the wrapping difference being
  positive and below 32768, so two sequences in play are less than that apart. `AckField
  { latest, bits }` acknowledges `latest` and, for bit `n` set, `latest - (n + 1)`, a
  window of 32 before it; `covers` says so. `TickHeader { sequence, tick, ack }` begins
  every packet: `tick` is the server tick the rows are no newer than, or on a client the
  newest input tick carried. Every entity entry a server packet carries names its handle
  and, for a quantised position, the region it is relative to.
- `PacketRing` keeps the last `capacity` (`DEFAULT_CAPACITY` 64) `PacketRecord`s in
  sequence order: `record` pushes and, when full, evicts and returns the oldest, treated
  as lost; `acknowledge(sequence)` takes the record if held; `expire(oldest_live)` takes
  every record before it, treated as lost. An acknowledgement therefore advances exactly
  the entities a packet carried, and a loss re-queues exactly them.

**Interest and the packer** (architecture sections 9.3, 9.4 and 14)

- `ClientInterest` is everything the server keeps per connected client. `enter(key)`
  makes an entity relevant: a handle from `handles` and an `EntityInterest` with `acked:
  None`, so the entity is sent whole until one state of it is acknowledged, then by the
  rows changed since `acked`. An entity the receiving player has claimed is sent whole in
  every packet that carries it (section 9.4: every simulation-read field at full precision
  on each update), whatever `acked` says. `leave(key, tick)` owes a tombstone at `tick`;
  the entity stays until the tombstone is acknowledged, then is forgotten and its handle
  released, so the client's mirror releases the same slot at the same point. `acknowledge
  (ack)` applies the peer's field: every packet it covers is taken from the ring and each
  entity it carried advances `acked` to the carried tick if newer and clears `in_flight`
  if it names that packet; records older than `latest - 32` are expired and their
  entities' `in_flight` cleared, so they are sent again. `Priority` accumulates each
  frame from distance and staleness (the weights are `net`'s) and resets when the entity
  is sent, so an entity starved by the cap rises until it goes.
- The packer keeps a mirror of every replicated row of every relevant region, built from
  the first (full) `StagingFrame` and every later one: rows replace, `Despawned` removes
  the entity's rows, `Removed` of a replicated component drops that row, `Moved`
  re-homes; a full frame replaces the mirror outright, since it carries no structural
  entries and an entity despawned in the gap would otherwise stay and be resent as a
  spawn on the resync. Whole sends when an entity enters, resends of lost rows that have
  not changed since, and the every-field updates of claimed entities all read the mirror;
  a delta frame holds only changed rows and is never enough on its own. A `Removed` entry
  becomes `StagedFrame::removed` for every client to whom the entity is relevant, and a
  `Despawned` entry the tombstone.
- `regions` is the client's relevant region set under `RelevancePolicy` (region membership
  within `radius` regions of the player's, plus the `always` set); a region entering is
  sent as a `RegionRelevance` and every entity residing in it enters; a region leaving is
  sent as `Irrelevant` and its entities leave without tombstones. `ready` holds the
  regions the client acknowledged with `ClientReady`, the network's copy; the hold itself
  moves simulation state and enters the server tick as a journaled input at milestone 4
  (a `lactose-sim` addition, `SessionEvent::Ready { region }` or a `TickInput` field).

**The interpolation buffer** (architecture sections 5.3 and 9.3)

- `InterpolationBuffer::split(capacity)` makes one buffer of `capacity` slots, allocating
  once, and hands out its one `InterpolationWriter` and an `InterpolationReader`, which
  clones. The writer is the network thread's; a reader is the `net-api` resource of
  section 5.3 that the presentation schedule holds. The buffer never feeds a simulation
  row: `Replicated` rows get the same states through the staged frame.
- Each slot is a sequence lock over a ring of the `STATES_PER_SLOT` (8) newest states by
  tick, every word an atomic, so the lock needs no `unsafe`. The orderings, exactly: the
  writer stores the odd sequence (`Relaxed`), issues `fence(Release)`, stores the words
  (`Relaxed`), and stores the even sequence with `Release`; a reader loads the sequence
  with `Acquire`, loads the words with `Relaxed`, issues `fence(Acquire)`, and reloads the
  sequence, retrying when the first load was odd or the two differ. A first load left
  `Relaxed` admits torn reads (the first design review measured 3 to 84 per three-second
  aarch64 run); `Acquire` admits none in 2.3 billion. The slot's `generation` and `count`
  are words read inside that window, and `allocate` and `release` bump the sequence as
  `push` does. No lock, no allocation on either side; a reader never observes a
  half-written ring, and a writer is never delayed by readers. A reader's retry is bounded
  by the writer's rate (one push per received state), never by other readers.
- `allocate` takes the most recently released slot, else the next never-used, at a new
  generation (never zero, wrapping past `u32::MAX` to one), empty; `ResourceExhausted`
  when every slot is allocated. `push(slot, state)` keeps the eight newest states by tick:
  a state older than the oldest kept is dropped, one with the tick of a kept state replaces
  it, so arrival order does not matter. `release(slot)` makes the slot read as missing
  under the old generation from then on. `push` and `release` fail with `NotFound` for a
  stale or unallocated slot. Each state is the tick and the teleport tick packed in one
  word, the position's three scalars and the rotation's four, each as canonical bits
  widened to 64; the network thread hands the writer world-space values (the wire position
  dequantised plus the region origin).
- `ReceivedState::teleported` is the `Teleported` value of the entity's row (D119: the
  last tick the entity was placed without a sweep, zero for never), a tick rather than a
  flag so a teleport is told from its states whatever order they arrive in. `sample(slot,
  at)`: `Interpolated` when two kept states bracket `at` (the earlier's tick not after
  `at`, the later's after it), position by `Vec3::lerp` and rotation by `Quat::nlerp` with
  the parameter `(at - earlier.tick) / (later.tick - earlier.tick)`, `at` taken as `tick +
  fraction / 65536`; `Held` at the newest state when `at` is not before it (the entity's
  updates are late), at the oldest when `at` is before every kept state, and around a
  teleport: with `later.teleported` in `(earlier.tick, later.tick]`, at `earlier` while
  `at` is before `later.teleported` and at `later` from then on, so nothing blends across
  a teleport; `Missing` when the slot holds no state or `slot`'s generation is not the
  slot's, in which case the caller keeps what it showed last. `sample_batch(slots, at,
  out)` is `sample` for each `slots[i]` into `out[i]`, `out` at least as long, allocating
  nothing; the presentation schedule computes `at` from `ClockEstimate`, about two send
  intervals behind the newest received tick.
- The presentation schedule learns which slot to sample from the `RemoteSlot` rows
  (through extract or the renderer's slot table), not from a lookup by key: there is no
  key-to-slot directory to lock, and a reused slot reads as missing under the old value.
  A slot is about 530 bytes; five thousand remote entities are about 2.6 MB.

**Clock sync and control messages** (architecture sections 9.2, 9.3 and 14)

- `ClockEstimate` is what `net` publishes to the timer root: the server's tick now, the
  lead (half the round trip plus the jitter margin, in ticks under the replicated time
  scale), the rate adjustment clamped to `1000 ± MAX_RATE_ADJUST_THOUSANDTHS` (30, three
  percent), and `snap` when the drift exceeds what the catch-up cap can recover. The
  estimator is `net`'s; how the value reaches the timer root is `app`'s.
- Control messages (`JoinAccepted`, `RegionRelevance`, the time scale, `ClientReady`,
  `OverlayRequest`, `ResyncRequest`) travel on a reliable ordered channel `lactose-net`
  builds over the transport's datagrams at milestone 4; an overlay and a full frame
  exceed one datagram, so that channel fragments. `JoinAccepted { player, tick }` is the
  first message after the handshake and before the full frame of the late join: the
  client builds its worlds at `tick`, so the full frame for `tick` is due at their first
  sync point, and `player` is what the prediction world's hook and the input packets
  name. `ClientReady { region, tick }` is the readiness acknowledgement. `OverlayRequest
  { region }` follows a digest the client cannot match, and is answered with the same
  `RegionRelevance`, `Full`. `ResyncRequest { tick, reason }` asks for a full frame, for
  an `AssetFailure` (the tick-stamped event of section 14, from
  `RegionLifecycle::take_asset_failures`), `BehindCatchUp` (section 11) or `Divergence`.
  The sequencing of a join or a resync is the receiver rule above: nothing staged from
  the message until the full frame, every packet not after the full frame's tick dropped,
  the mirror and slots cleared at the full frame, the server's handle table for the
  client cleared and refilled. Their encoding is `net`'s.

## Not in this crate yet

- `JoinAccepted::entity: StableKey` (D122): the player's own entity, as the session applier
  spawned or re-claimed it, read by the server's `app` with `player_entity` between runs
  after the join applied; what `lactose-predict`'s `LocalPlayer::entity` takes. And a
  milestone 4 control message, `PlayerEntity { entity: StableKey }`, sent when the server
  names another entity for the player (a re-join after a load), which `app` passes to
  `Predictor::set_local` between runs.
- The packer itself: filling each packet by priority up to `SendBudget`, delta coding,
  the byte encoding of every type here and of the staged frame's records, the jitter
  window and the per-client input queue of `lactose_input::SampleHistory` windows, the
  reliable ordered channel, the clock-sync estimator, the region-origin table the packer
  subtracts and the receiver adds: `lactose-net`, milestone 4.
- The interpolation of anything but a translation and a rotation (a scale, an animation
  phase): added to `ReceivedState` and the slot's words when presentation needs it.
- The server's hold of a player at a region boundary until `ClientReady`: a journaled
  input of `lactose-sim` at milestone 4, with `ClientInterest::ready` as the network's
  copy.

## Proof for milestone 1

Nothing runs yet. The implementation lane replaces every `todo!()` and proves:

| What | How |
|---|---|
| Quantisation | `quantise(dequantise(q)) == q` for positions over a sweep under the 64-bit scalar; saturation at the `i32` and `i16` limits; NaN to zero; `q` and `-q` quantise alike; a rotation stable after one trip; the zero and NaN quaternions to all-zero and back to `IDENTITY`; golden bits of `dequantise` for a small table, equal on x86-64 and aarch64 under SOUR's architecture mode |
| The handshake | Golden `ProtocolHash` for a small registry, pinned; adding a replicated or persistent type, raising a version, changing a field's ID, offset, size, tag or kind each change it; adding a derived unreplicated type does not; the constants block changes it; `check` reports the first difference in field order; `from_bytes(to_bytes(h)) == h` |
| Handles and packets | A released slot is reused with the next generation and the old handle is stale, across the generation wrap; 65536 assignments then `ResourceExhausted`; a client mirror driven by the receiver rules (`insert` and `release` for staged packets, late and reordered packets dropped whole) equals the server's table after spawns and tombstones delivered in any order, a tombstone overtaking its spawn included, with the reordered spawn dropped and resent; `clear` at the full frame then its inserts, with a pre-resync handle naming nothing; a full frame overtaken by later deltas is staged first and the deltas dropped and resent; `after` and `covers` across the wrap; the ring evicts the oldest when full, `acknowledge` takes exactly one record, `expire` takes exactly the older ones |
| Interest | `enter` sends whole until an acknowledgement; a claimed entity is whole in every packet; `acknowledge` advances exactly the carried entities and clears exactly the acknowledged `in_flight`; an expired packet re-queues its entities; an acknowledged tombstone releases the handle; a resync clears and refills the client's handles; the mirror after a sequence of frames equals the server's replicated rows, a removed component gone, and a full frame replaces it with an entity despawned in the gap gone |
| The interpolation buffer | A loom model or a stress test with a checksum word: under one writer and several readers every sample is a state that was pushed, never a mix, with the stated orderings; `sample` cases: bracketed, held newest, held oldest, held on either side of a teleport by `teleported`, missing after release and under a stale generation; a push older than the ring is dropped and an equal tick replaces; out-of-order pushes give the same ring; `sample_batch` equals `sample` per slot and allocates nothing (a counting allocator in the test) |
| The transport | The loopback sketch under the trait's contract: event order per connection, the size limit, `NotFound` after `disconnect`, `rtt` zero |
