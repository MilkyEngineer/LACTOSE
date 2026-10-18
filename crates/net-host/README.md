# lactose-net-host

The host side of networking: the quantised wire values, the handshake, the transport trait
a loopback or UDP implementation is a page of, the packet framing and the per-connection
handles both ends keep, the per-client interest state the packer keeps against the gather's
frame, the lock-free interpolation buffer the presentation schedule samples, and the named
places for clock sync and the control messages. The simulation side, what enters and leaves
a world, is `lactose-net-api`, inside the boundary.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | outside, and no boundary crate may depend on it: it is absent from `[boundary_internal]` in `crate-tiers.toml` on purpose |
| Unsafe | none, `#![forbid(unsafe_code)]`; the sequence lock is built from atomics alone |

**Status: implemented for milestone 1.** Every body runs, and every row of "Proof for
milestone 1" is proved by the tests it names. The packer and the receiver that use these
types are `lactose-net`'s, milestone 4: "Handles and packets" and "Interest and the packer"
state the rules they keep, and `tests/replication.rs` runs a model of each, held to those
rules, between a real server and a real client world. Every sentence under "Behaviour the
signatures cannot express" is one a dependent crate may rely on. Architecture sections 4,
5.1, 5.3, 9.1, 9.2, 9.3, 9.4, 11, 14 and 22 are the design; where this file and the
architecture disagree, the architecture wins and this file is wrong. Decision D121 records
the choices those sections left open, for this crate and for `lactose-net-api` together.
`crates/net-api/README.md` is the contract the frames this crate's receiver builds and its
packer reads meet, and is not restated here.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `app`, `lactose-net`, `lactose-presentation` and
the milestone 1 spike are written against this surface. `tests/consumers.rs` is a
compile-only sketch of what each of them writes: a change that breaks it is a signature
change to list.

## Surface

| Item | Purpose |
|---|---|
| `quant::{QuantisedPosition, QuantisedRotation, POSITION_UNITS_PER_METRE, ROTATION_SCALE}` | The wire values and their conversions through `lactose_math` |
| `handshake::{Handshake, ProtocolHash, BoundaryBuildId, CookedDataHash, HandshakeMismatch, PROTOCOL_VERSION}` | What each side sends first and the rule that refuses a connection |
| `transport::{Transport, ConnectionId, Datagram, TransportEvent, DisconnectReason, LinkStats, MAX_DATAGRAM_BYTES}` | Unreliable, unordered datagrams with a connection identity |
| `packet::{EntityHandle, ConnectionHandles, AssignedHandles, PacketSequence, AckField, TickHeader, PacketRecord, PacketRing}` | The framing shapes, the per-connection handle table and the packet-to-entity ring; a record holds the entity states and the tombstones a packet carried |
| `interest::{ClientInterest, EntityInterest, Priority, SendBudget, RelevancePolicy}` | The per-client state the gather's frame and the packer share: `enter`, `leave`, `set_claimed`, `resend_whole`, `sent`, `acknowledge`, `resync`; per entity `claimed`, `reentering` and `sends_whole` |
| `interpolation::{InterpolationBuffer, InterpolationWriter, InterpolationReader, ReceivedState, PresentationTime, RemoteSample, SampleStatus, STATES_PER_SLOT}` | The lock-free buffer, its one writer and its readers, and the batch sampling call |
| `clock::{ClockEstimate, MAX_RATE_ADJUST_THOUSANDTHS}` | Named: what the timer root reads from clock sync |
| `message::{JoinAccepted, ClientReady, OverlayRequest, ResyncRequest, ResyncReason}` | The control messages, named |

Hidden plumbing: none. The private fields of every type are the implementation's to
reshape.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| `lactose-net` (milestone 4) | A transport to drive, a handshake to check, handles and packets to encode | `transport::Transport`, `handshake::Handshake::{to_bytes, from_bytes, check}`, `packet::*` |
| | Per-client state the packer keeps against the gather's frame, and the mirror it packs from | `interest::ClientInterest`, `StagingFrame::region` per relevant region, `PacketRing`, `ClientInterest::{sent, acknowledge}`; the mirror and region rules under "Interest and the packer" |
| | The receiver on the network thread: handles mirrored, positions dequantised, states into the buffer, frames staged | `ConnectionHandles::{insert, release, clear}`, `QuantisedPosition::dequantise`, `InterpolationWriter::{allocate, push, release}`, `lactose_net_api::StagedFrame`; the receiver rules under "Handles and packets" |
| | The client's clock estimate to publish | `clock::ClockEstimate` |
| `app` (client) | The player and the start tick on join; the estimate to pace by | `message::JoinAccepted`; `clock::ClockEstimate` |
| The milestone 1 spike | Everything above over a throwaway transport | `tests/consumers.rs`: the loopback `Transport`, the packer, the receiver, the sampler, the handshake, the join |
| `lactose-presentation` (milestone 2) | One batch sample per frame, render space | `InterpolationReader::sample_batch` into a caller's slice, by the `RemoteSlot` rows extract read; `RemoteSample::position` into `RenderPosition::from_sim` |
| `lactose-predict` | A claimed entity whole in every frame that carries a row of it, and a frame staged whole or not at all | The whole-send rule under "Interest and the packer" and the receiver rules under "Handles and packets" |
| `lactose-input-host` | Nothing: the input packets carry `lactose_input::SampleHistory` windows, which `net` encodes | - |

## Behaviour the signatures cannot express

**Quantisation** (architecture section 9.3)

- `POSITION_UNITS_PER_METRE` is 1024, a power of two, about one millimetre.
  `QuantisedPosition::quantise(relative)` takes a simulation position with the region
  origin already subtracted (the region's world origin is a cook-time fact the packer looks
  up; the table is not in this crate) and computes, per coordinate, `(c *
  Scalar::from_i32(1024)).round().to_i32()`: nearest with ties away from zero, saturating,
  zero for NaN. `dequantise` computes `Scalar::from_i32(q) / Scalar::from_i32(1024)`, an
  exact division. Under the 64-bit scalar `quantise(dequantise(q)) == q` for every `q`;
  under a 32-bit scalar only for integers within 2^24 units (sixteen kilometres) of the
  origin, beyond which the scalar cannot hold the integer. The same bits come out on every
  target; both go through `lactose_math` only.
- `ROTATION_SCALE` is `i16::MAX`. `QuantisedRotation::quantise(q)` divides `q` by its
  largest component's magnitude and then normalises it, so a quaternion of any finite
  length normalises without overflow or underflow; it quantises each component as the
  position does with the scale 32767, clamped to `-32767..=32767` so the value never holds
  `i16::MIN`; and it makes the sign canonical on the integers, negating all four unless the
  first nonzero of `w`, `x`, `y`, `z` is positive, so `q` and `-q` quantise alike even when
  a component rounds to zero. `dequantise` divides each component by the scale (one true
  division each) and normalises. Rotations do not round-trip exactly: over two million
  uniform draws `quantise(dequantise(q))` differed from `q` for 0.98 percent of values, by
  one unit in at most two components (two for about one value in ten thousand), and every
  value was stable after one trip (`quantise(dequantise(q1)) == q1` for `q1 =
  quantise(dequantise(q))`). A zero, NaN or infinite quaternion quantises to the all-zero
  value and nothing else does; the all-zero value dequantises to `Quat::IDENTITY`.
  `IDENTITY` is `(0, 0, 0, 32767)` and the type's `Default`.
- What is quantised and what is not: the packer quantises the translation and rotation of
  entities the receiving player has not claimed; every other field of theirs and every
  field of claimed entities travels as the component's bytes. The wire states which region
  each quantised position is relative to (the entity's region at the packet's tick), so the
  receiver adds the right origin after a move. Which component holds the transform is
  `physics-api`'s to say; this crate names no component of another crate.

**The handshake** (architecture section 9.1)

- `Handshake { protocol, build, cooked }` is what each side sends first, twenty-four bytes:
  the three hashes as little-endian `u64`s in field order (`to_bytes`); every byte string
  of that length decodes (`from_bytes`). `check(peer)` compares in field order and reports
  the first difference as a `HandshakeMismatch`, which converts into an `Error` of kind
  `VersionMismatch`; either side refuses the connection on an error, and a datagram that is
  not twenty-four bytes is refused before `check`.
- `ProtocolHash::of(registry)` is xxh64, seed zero, over one byte stream: first the
  protocol constants, `PROTOCOL_VERSION` as `u32`, `TICK_RATE_HZ` as `u32`,
  `POSITION_UNITS_PER_METRE` as `i32`, `ROTATION_SCALE` as `i32` and `MAX_DATAGRAM_BYTES`
  as `u32`, every integer little-endian; then, for every registered type `covers` accepts,
  in ascending component key order, `TypeInfo::write_schema`, the per-type encoding
  `lactose-reflect` states byte for byte and pins with a golden test. The covered-set rule
  is this crate's; the encoding is `reflect`'s. The hash of the engine's own registry
  (`TypeTable::new` with the types of `lactose-net-api`, `lactose-input` and
  `lactose-physics-api`) is pinned in `tests/handshake.rs`, so a change to any engine type
  that crosses the wire or is hashed is seen as the protocol break it is.
- `covers(info)` is: `tracking().replicated`, or `persistence()` is `Persistent` or
  `Transient`. Replicated types cross the wire; persistent and transient types are what the
  world hash and a desync digest compare, so a client whose hashed layout differs cannot
  explain a mismatch. A `derived` type that is not replicated never crosses and is never
  compared, whatever crate declares it. Hence the rule every crate obeys: a type registered
  on one side only (a presentation type the server does not register) is `derived` and
  unreplicated, or the handshake refuses every connection. The input sample (`transient`)
  and `TimeScale` (`derived, replicated`) are covered; `ClaimedBy`, `StreamingAnchor`,
  `RemoteSlot` and `RegionPolicy` are not.
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
  a connection that is not established, `Network` for a failed socket. `poll` appends every
  ready event in the order observed and returns at once.
- A datagram arrives whole or not at all, possibly reordered or duplicated, never
  corrupted; `net` builds reliability, ordering and acknowledgement on top. For one
  connection, `Connected` is the first event, `Received` comes only between it and
  `Disconnected`, `Disconnected` is the last, and the transport never reuses the ID.
  `disconnect` on an established connection makes a later `poll` yield `Disconnected` with
  `Local`; on an unknown one it does nothing.
- How connections are established (listening, dialling) is a transport's constructor's; the
  trait sees connections as events, which is what lets a loopback be one page
  (`tests/consumers.rs`). `stats` is `None` for a connection that is not established, `rtt:
  None` for a transport that does not measure (then `net` estimates from its
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
  slot (a slot the mirror never recorded accepts any), is a no-op for the same (handle,
  key) again, and fails with `InvalidData` for `NONE`, if the slot holds another live key,
  if the slot's generation is newer than the handle's (a spawn reordered after the
  tombstone that released it; the comparison is wrapping, since generations wrap from
  `u16::MAX` to one, so "newer" is `slot.wrapping_sub(handle)` in `1..=32767`), or if the
  key has a different handle already; `release(handle)` returns the key and advances the
  slot's generation (wrapping from `u16::MAX` to one), so the released handle never names
  the slot's next entity; `clear` forgets every assignment and every generation, what the
  client does when it stages a full frame and only then; `key_of` and `release` give `None`
  for a stale or unassigned handle. `iter` is in ascending key order. After `clear` a
  handle from before it may name a slot the full frame's inserts reused; the receiver never
  looks one up, since every packet it stages after the full frame was packed after the
  server's resync.
- The receiver's rules, which keep the mirror equal to the server's table and the inbox's
  newest-ever-accepted rule true by construction: the receiver stages a server packet only
  if its header tick is after the newest frame it has staged; any other packet (a late one,
  a reordered one, a duplicate) it drops whole and does not acknowledge, so the server
  resends what the packet carried, and no tombstone is applied before its spawn, no row
  before the row it would replace. From a `ResyncRequest` or `JoinAccepted` until the full
  frame is staged, it stages nothing and drops every packet whose tick is not after the
  full frame's (the full frame travels on the reliable channel, and deltas sent after it
  overtake it), and it clears its handle mirror and its interpolation slots when it stages
  that frame, not when it asks. On a resync the server clears that client's
  `ConnectionHandles` and re-enters every relevant entity, so the full frame's spawns carry
  fresh handles and no handle from before the resync names anything after it. Hence the
  frames the receiver hands `app` ascend strictly by tick and the full frame is the next
  frame after the request, and `StagedInbox::push` never refuses one. A re-join's
  `JoinAccepted` restarts the newest staged tick: the receiver forgets it and drops what it
  has not handed `app`, and `app` resets each client world's inbox (`StagedInbox::reset`).
- The receiver stages a packet whole or not at all. Before it changes anything it checks
  the whole packet against a copy of its handle mirror and the registry: each spawn's
  `insert`, then each row's, move's and removal's handle (a spawn in the same packet
  counts), and the length of each row and of each spawn's component against the component's
  size in the registry (`push_row` refuses a length other than the row it holds, and
  nothing else checks the first row's). A packet that fails any check is dropped whole as
  malformed and not acknowledged, and leaves the mirror, the interpolation slots and the
  newest staged tick as they were. A staged packet stages every row it carries: a row equal
  to one an earlier frame staged is no defect, since a claimed entity is sent whole in
  every packet that carries it and an unacknowledged row is sent again. A tombstone for a
  handle the mirror does not hold (a resend of one already staged, or one for an entity
  whose spawn never arrived) is ignored. The receiver allocates an interpolation slot for
  each spawn of an entity the player has not claimed (keeping the slot it holds for a live
  one) and releases it at the entity's tombstone or claim.
- Region changes arrive on the reliable channel, each stamped with the server tick it
  applies from, and the receiver puts each into the first frame it stages at or after that
  tick, once the change has arrived, in channel order. When one frame would hold two
  changes of one region (an `Irrelevant` and a `Relevant` both pending when a packet
  arrives), it stages a frame for the packet's tick holding only the changes before the
  second, and drops the packet unacknowledged: the second change goes into the next frame,
  and the fold applies the pair as an unload then a load. A full frame drops every pending
  change stamped at or before its tick, which it subsumes.
- `PacketSequence` wraps at sixteen bits; `after` is the wrapping difference being positive
  and below 32768, so two sequences in play are less than that apart. `AckField { latest,
  bits }` acknowledges `latest` and, for bit `n` set, `latest - (n + 1)`, a window of 32
  before it; `covers` says so. `TickHeader { sequence, tick, ack }` begins every packet:
  `tick` is the server tick the rows are no newer than, or on a client the newest input
  tick carried. Every entity entry a server packet carries names its handle and, for a
  quantised position, the region it is relative to. A peer that has received no packet
  sends no field: `AckField::default()` covers sequence zero.
- `PacketRing` keeps the last `capacity` (`DEFAULT_CAPACITY` 64) `PacketRecord`s in
  sequence order: `record` pushes and, when full, evicts and returns the oldest, treated as
  lost; `acknowledge(sequence)` takes the record if held; `expire(oldest_live)` takes every
  record before it, treated as lost. A record lists the entity states a packet carried,
  each with the tick it was packed at, and apart from them the tombstones, each with its
  tick. An acknowledgement therefore advances exactly the entities a packet carried, and a
  loss re-queues exactly them.

**Interest and the packer** (architecture sections 9.3, 9.4 and 14)

- `ClientInterest` is everything the server keeps per connected client. `enter(key)` makes
  an entity relevant: a handle from `handles` and an `EntityInterest` with `acked: None`,
  so the entity is sent whole until one state of it is acknowledged, then by the rows
  changed since `acked`. An entity the receiving player has claimed (`set_claimed`) is sent
  whole in every packet that carries it (section 9.4: every simulation-read field at full
  precision on each update), whatever `acked` says; `sends_whole` says which. `leave(key,
  tick)` owes a tombstone at `tick`; the entity stays until the tombstone is acknowledged,
  then is forgotten and its handle released, so the client's mirror releases the same slot
  at the same point. An entity entered again while its tombstone is owed is marked
  `reentering` and keeps its old handle until the tombstone is acknowledged, then re-enters
  under a new one: the client released the old handle when it staged the tombstone, so
  cancelling the tombstone instead would leave the two tables apart. `set_claimed` and
  `resend_whole(key)` void what was acknowledged of the entity, including every record
  already in the ring, so a late acknowledgement of an earlier packet never marks it
  acknowledged again. `sent(record)` records a packet: each entity and tombstone it carried
  is in flight in it and its priority resets. `acknowledge(ack)` applies the peer's field:
  every packet it covers is taken from the ring and each entity it carried advances `acked`
  to the carried tick if newer and clears `in_flight` if it names that packet; a tombstone
  it carried releases the handle if it is the one owed; records older than `latest - 32`
  are expired and their entities' `in_flight` cleared, so they are sent again.
  `resync(tick)` is the server's side of a resync: the handles and the ring start over,
  every entity whose tombstone is owed is forgotten unless it is re-entering, and every
  other entity re-enters under a fresh handle, assigned in ascending key order, with its
  claim kept and `acked` at `tick`, the tick of the full frame that carries each of them
  whole. `Priority` accumulates each frame from distance and staleness (the weights are
  `net`'s) and resets when the entity is sent, so an entity starved by the cap rises until
  it goes.
- The packer keeps a mirror of every replicated row of every relevant region, built from
  the first (full) `StagingFrame` and every later one: rows replace, `Despawned` removes
  the entity's rows, `Removed` of a replicated component drops that row, `Moved` re-homes;
  a full frame replaces the mirror outright, since it carries no structural entries and an
  entity despawned in the gap would otherwise stay and be resent as a spawn on the resync.
  Whole sends when an entity enters, resends of lost rows that have not changed since, and
  the every-field updates of claimed entities all read the mirror; a delta frame holds only
  changed rows and is never enough on its own. A `Removed` entry becomes
  `StagedFrame::removed` for every client to whom the entity is relevant, and a `Despawned`
  entry the tombstone.
- What a packet carries for each entity of the client's set: the tombstone, if one is owed,
  and nothing else of it; else, while nothing of it is acknowledged, every replicated
  component from the mirror as a spawn, beside a move to the region it resides in, since
  the hook's upsert of a live entity does not move it; else, for a claimed entity, an exact
  row of every replicated component it has, in every packet, so a frame that holds an exact
  row of an entity holds every replicated component the server's entity has at the frame's
  tick; else the rows changed and the components removed since its `acked` tick, if any. A
  row is sent stamped with the tick of the gathered frame that last changed it.
- `regions` is the client's relevant region set under `RelevancePolicy` (region membership
  within `radius` regions of the player's, plus the `always` set). A region entering is
  sent as a `RegionRelevance` on the reliable channel, stamped with the tick it enters at;
  the packer names no entity of it (no spawn, row or move into it) until the channel has
  delivered that message and while the region is still relevant, and then every entity
  residing in it enters. A full frame's regions count as delivered. With the receiver's
  rule for region changes, a frame names an entity in a region only if the region is
  relevant to the client as of that frame, by its own region changes at the latest, and the
  client loads a frame's region changes in the step that pops it.
- A region leaving is sent as `Irrelevant`, stamped with the tick it leaves at, and every
  entity of the client's set residing in it leaves with a tombstone at that tick (D121, the
  decision on region leave). The unload despawns them on the client; the tombstones release
  their handles and slots on both ends, which nothing else does; and when the step that
  pops the frames folds a region's `Relevant` and then `Irrelevant` into no load, the
  tombstone takes out an entity the frames moved into the region, which would otherwise
  stay where it was in both client worlds until a full frame.
- Moves: an entity moving from a region the packer may name into one it may not leaves with
  a tombstone at the move's tick; one moving the other way enters; one moving between two
  such regions is sent whole beside its move (`resend_whole`), since the client may not
  hold it to move: the step that applies the move may unload the region it left first.
- What this costs, accepted: a region that leaves and returns within about one round trip
  has its entities re-enter only once their tombstones are acknowledged, so after the
  client reloads the region the tombstones despawn what it loaded until the re-entry spawns
  arrive, a gap of about one round trip that converges. And a claimed entity whose region
  leaves the client's set gets a tombstone, which `lactose-predict` reconciles as its
  destruction.
- `ready` holds the regions the client acknowledged with `ClientReady`, the network's copy;
  the hold itself moves simulation state and enters the server tick as a journaled input at
  milestone 4 (a `lactose-sim` addition, `SessionEvent::Ready { region }` or a `TickInput`
  field).

**The interpolation buffer** (architecture sections 5.3 and 9.3)

- `InterpolationBuffer::split(capacity)` makes one buffer of `capacity` slots, allocating
  once, and hands out its one `InterpolationWriter` and an `InterpolationReader`, which
  clones. The writer is the network thread's; a reader is the `net-api` resource of section
  5.3 that the presentation schedule holds. The buffer never feeds a simulation row:
  `Replicated` rows get the same states through the staged frame.
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
  generation (never zero, wrapping past `u32::MAX` to one), empty; `ResourceExhausted` when
  every slot is allocated. `push(slot, state)` keeps the eight newest states by tick: a
  state older than the oldest kept is dropped, one with the tick of a kept state replaces
  it, so arrival order does not matter. `release(slot)` makes the slot read as missing
  under the old generation from then on. `push` and `release` fail with `NotFound` for a
  stale or unallocated slot. Each state is the tick and the teleport tick packed in one
  word, the position's three scalars and the rotation's four, each as canonical bits
  widened to 64; the network thread hands the writer world-space values (the wire position
  dequantised plus the region origin).
- `ReceivedState::teleported` is the `Teleported` value of the entity's row (D119: the last
  tick the entity was placed without a sweep, zero for never), a tick rather than a flag so
  a teleport is told from its states whatever order they arrive in. `sample(slot, at)`:
  `Interpolated` when two kept states bracket `at` (the earlier's tick not after `at`, the
  later's after it), position by `Vec3::lerp` and rotation by `Quat::nlerp` with the
  parameter `(at - earlier.tick) / (later.tick - earlier.tick)`, `at` taken as `tick +
  fraction / 65536`; `Held` at the newest state when `at` is not before it (the entity's
  updates are late), at the oldest when `at` is before every kept state, and around a
  teleport: with `later.teleported` in `(earlier.tick, later.tick]`, at `earlier` while
  `at` is before `later.teleported` and at `later` from then on, so nothing blends across a
  teleport; `Missing` when the slot holds no state or `slot`'s generation is not the
  slot's, in which case the caller keeps what it showed last. `sample_batch(slots, at,
  out)` is `sample` for each `slots[i]` into `out[i]`, `out` at least as long, allocating
  nothing; the presentation schedule computes `at` from `ClockEstimate`, about two send
  intervals behind the newest received tick.
- The presentation schedule learns which slot to sample from the `RemoteSlot` rows (through
  extract or the renderer's slot table), not from a lookup by key: there is no key-to-slot
  directory to lock, and a reused slot reads as missing under the old value. A slot is
  about 530 bytes; five thousand remote entities are about 2.6 MB.

**Clock sync and control messages** (architecture sections 9.2, 9.3 and 14)

- `ClockEstimate` is what `net` publishes to the timer root: the server's tick now, the
  lead (half the round trip plus the jitter margin, in ticks under the replicated time
  scale), the rate adjustment clamped to `1000 ± MAX_RATE_ADJUST_THOUSANDTHS` (30, three
  percent), and `snap` when the drift exceeds what the catch-up cap can recover. The
  estimator is `net`'s; how the value reaches the timer root is `app`'s.
- Control messages (`JoinAccepted`, `RegionRelevance`, the time scale, `ClientReady`,
  `OverlayRequest`, `ResyncRequest`) travel on a reliable ordered channel `lactose-net`
  builds over the transport's datagrams at milestone 4; an overlay and a full frame exceed
  one datagram, so that channel fragments, and it tells the sender when each message is
  delivered, which the region rule above waits on. `JoinAccepted { player, tick }` is the
  first message after the handshake and before the full frame of the late join: the client
  builds its worlds at `tick`, so the full frame for `tick` is due at their first sync
  point, and `player` is what the prediction world's hook and the input packets name.
  `ClientReady { region, tick }` is the readiness acknowledgement. `OverlayRequest { region
  }` follows a digest the client cannot match, and is answered with the same
  `RegionRelevance`, `Full`. `ResyncRequest { tick, reason }` asks for a full frame, for an
  `AssetFailure` (the tick-stamped event of section 14, from
  `RegionLifecycle::take_asset_failures`), `BehindCatchUp` (section 11) or `Divergence`.
  The sequencing of a join or a resync is the receiver rule above: nothing staged from the
  message until the full frame, every packet not after the full frame's tick dropped, the
  mirror and slots cleared at the full frame, the server's handle table for the client
  cleared and refilled. Their encoding is `net`'s.

## Not in this crate yet

- `JoinAccepted::entity: StableKey` (D122): the player's own entity, as the session applier
  spawned or re-claimed it, read by the server's `app` with `player_entity` between runs
  after the join applied; what `lactose-predict`'s `LocalPlayer::entity` takes. And a
  milestone 4 control message, `PlayerEntity { entity: StableKey }`, sent when the server
  names another entity for the player (a re-join after a load), which `app` passes to
  `Predictor::set_local` between runs.
- The packer and the receiver themselves: filling each packet by priority up to
  `SendBudget`, delta coding, the byte encoding of every type here and of the staged
  frame's records, the jitter window and the per-client input queue of
  `lactose_input::SampleHistory` windows, the reliable ordered channel and its delivery
  acknowledgements, the clock-sync estimator, the region-origin table the packer subtracts
  and the receiver adds: `lactose-net`, milestone 4. `tests/replication.rs` holds models of
  the packer and the receiver that keep the rules above, which `lactose-net`'s must keep as
  well; a server tick split over several packets, each staged as its own frame under the
  receiver's tick rule, is `lactose-net`'s to design.
- The interpolation of anything but a translation and a rotation (a scale, an animation
  phase): added to `ReceivedState` and the slot's words when presentation needs it.
- The server's hold of a player at a region boundary until `ClientReady`: a journaled input
  of `lactose-sim` at milestone 4, with `ClientInterest::ready` as the network's copy.

## Proof for milestone 1

| What | How |
|---|---|
| Quantisation | `tests/quant.rs`: `quantise(dequantise(q)) == q` for positions over a sweep under the 64-bit scalar, ties away from zero; saturation at the `i32` and `i16` limits; NaN to zero; `q` and `-q` quantise alike, a component rounding to zero included; non-idempotence measured and a rotation stable after one trip; the zero, NaN and infinite quaternions to all-zero and back to `IDENTITY`; golden bits of `dequantise` for a small table. `tests/sour.rs` records the quantised and dequantised wire values of every entity of a replication under SOUR, whose architecture mode compares them between x86-64 and aarch64 |
| The handshake | `tests/handshake.rs`: the hash is xxh64 of the stated stream; golden `ProtocolHash` for the empty registry, a small one and the engine's own, pinned; adding a replicated, persistent or transient type, raising a version, changing a field's ID, offset, size, tag or kind each change it; adding a derived unreplicated type does not; the constants block changes it; `check` reports the first difference in field order and converts to `VersionMismatch`; `from_bytes(to_bytes(h)) == h` |
| Handles and packets | `tests/handles.rs`: a released slot is reused with the next generation and the old handle is stale, across the generation wrap; 65536 assignments then `ResourceExhausted`; `insert`'s refusals; a client mirror driven by the receiver rules equals the server's table after spawns and tombstones delivered in any order under loss, delay and duplication, a tombstone overtaking its spawn included, with the reordered spawn dropped and resent; `clear` at the full frame then its inserts, with a pre-resync handle naming nothing; a full frame overtaken by later deltas is staged first and the deltas dropped and resent; `after` and `covers` across the wrap; the ring evicts the oldest when full, `acknowledge` takes exactly one record, `expire` takes exactly the older ones. `tests/replication.rs`: a malformed packet (a row of another length, a row of an unknown handle, a spawn component of another length) changes nothing and is not acknowledged, and a claimed entity's rows are staged all or none |
| Interest | `tests/interest.rs`: `enter` sends whole until an acknowledgement; a claimed entity is whole in every packet; `acknowledge` advances exactly the carried entities and clears exactly the acknowledged `in_flight`; a lost, expired or evicted packet re-queues its entities; an acknowledged tombstone releases the handle and a re-entry waits for it; `resend_whole` is not undone by an older acknowledgement; a resync clears and refills the client's handles. `tests/replication.rs`: the mirror after a sequence of gathered frames equals the server's replicated rows, a removed component gone, and a full gather replaces it with an entity despawned in the gap gone; every frame holding an exact row of an entity holds every replicated component the server's entity has at the frame's tick; the client world converges to the server's relevant regions under churn, loss, reordering, duplication, resyncs and claims, with equal handle tables, a slot holding a state for every unclaimed entity and none for a claimed one, no region load refused and no hook entry skipped; an entity moved into a region made relevant and irrelevant again in one step leaves with the region; an entity moved out of a region that unloads in the same step arrives whole; two changes of one region due for one frame are staged in two |
| The interpolation buffer | `tests/interpolation.rs`: a stress test with words derived from one counter, one writer and three readers, sampling single and batched while slots are reallocated: every sample is a state that was pushed, never a mix, with the stated orderings (no loom model); `sample` cases: bracketed, held newest, held oldest, held on either side of a teleport by `teleported`, missing after release and under a stale generation; a push older than the ring is dropped and an equal tick replaces; out-of-order pushes give the same ring; `sample_batch` equals `sample` per slot. `tests/interpolation_alloc.rs`: `sample` and `sample_batch` allocate nothing, counted by the tagged global allocator |
| The transport | `tests/consumers.rs`: the loopback sketch under the trait's contract: event order per connection, the size limit, `NotFound` after `disconnect`, `rtt` zero |
