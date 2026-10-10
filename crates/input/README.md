# lactose-input

Player input: the quantised per-tick sample the simulation reads, its byte image on the
wire and in the journal, the applier that writes it at the input sync point, the
redundancy window, and the host-side places for the latest sample, device sources and
action mapping.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | outside; boundary crates may use the `sample`, `apply` and `history` modules (and their root re-exports) for types and for operations that are deterministic by construction (architecture section 8.2); `raw`, `latest`, `sources` and `mapping` are host-only |
| Unsafe | none, `#![forbid(unsafe_code)]` |

**Status: facade.** Every public signature is final and every body that decides anything is
`todo!()`; nothing here runs. This file is the contract the implementation lane meets, and
every sentence under "Behaviour the signatures cannot express" is one a dependent crate may
rely on. Architecture sections 4, 5.3, 6.1, 8.2, 9.2, 9.4, 15.1 and 15.5 are the design;
where this file and the architecture disagree, the architecture wins and this file is wrong.
Decision D120 records the choices those sections left open. `crates/sim/README.md` is the
contract this crate's applier meets ("The input sync point": `PlayerSample`,
`InputApplier`, the entity lookup, the writer declaration) and is not restated here.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `app`, `predict`, `net`, `presentation` and the
game crates are written against this surface. `tests/consumers.rs` is a compile-only sketch
of what each of them writes: a change that breaks it is a signature change to list.

## Surface

| Item | Purpose |
|---|---|
| `PlayerInput` | The sample: a `transient` component, key `input::player_input`, sixteen bytes of integers; what a kernel takes as `&PlayerInput` |
| `Axis2`, `Yaw`, `Pitch`, `Actions`, `Action` | Its value types: a movement vector in sixteen bits per axis, two angles in 1/65536 turn, a 32-bit action set and the named bits of milestone 1 |
| `PlayerInput::{aim, world_movement}`, `Yaw::radians`, `Pitch::radians`, `Axis2::to_vec2` | What a kernel derives from the integers, through `lactose_math`, when it reads them; nothing derived is stored |
| `PlayerInput::{to_sample_bytes, from_sample_bytes, validate, to_player_sample}`, `SAMPLE_SIZE` | The byte image `PlayerSample::sample` holds, the journal stores and the wire carries, and the frame entry |
| `PlayerInput::substitute`, `PlayerInput::IDLE` | The substitution rule for a sample lost after redundancy, and the zero sample a row reads without one |
| `PlayerInputApplier`, `player_entity` | The `InputApplier` `app` installs, and the one definition of a player's entity |
| `register_types`, `register_systems`, `sets::APPLY` | The registration pair `app` calls, and the set another crate's sync-point set is ordered against |
| `SampleHistory`, `SampleIter`, `REDUNDANT_SAMPLES` | The redundancy window: the last eight ticks' samples, filled by the client, packed into every packet, filled by the server from every packet |
| `raw::RawSample`, `RawSample::quantise` | Host-only: the frame's raw sample in floats, and the one step from floats to the integers both machines simulate |
| `latest::LatestSample` | Host-only: the latest raw sample, published by the input task and read by the late-latched camera without locking |
| `sources::Source` | Host-only: the shape of a device source (milestone 5) |
| `mapping` | Host-only: the named place of the action map (milestone 5); empty |
| `UNITS_PER_TURN`, `AXIS_SCALE`, `Pitch::LIMIT`, `Action::NAMED`, `Actions::BITS` | The constants of the encodings |

Hidden plumbing: none. The private fields of `SampleHistory`, `SampleIter`,
`PlayerInputApplier` and `LatestSample` are the implementation's to reshape.

## What boundary code may use

Architecture section 8.2 lets a boundary crate depend on this crate "only for types and
for operations that are deterministic by construction: no clocks, no floats computed
outside `math`". The modules keep the two sides apart so that a later lint can enforce it
on paths:

| Module | Side | Why it is deterministic by construction |
|---|---|---|
| `sample` (root re-exports) | boundary may use | Every stored value is an integer; every `Scalar` and vector is computed from integers through `lactose_math` in the stated order |
| `apply` (root re-exports) | boundary may use | Decodes integers, queries the world in key order, writes through the world; no float, no clock, no allocation beyond the ECS's own |
| `history` (root re-exports) | boundary may use | Integers and ticks; no boundary crate needs it at milestone 1, `net` does |
| `raw` | host-only | Floats: the raw sample and its quantisation |
| `latest` | host-only | A shared slot the render side reads outside the tick |
| `sources`, `mapping` | host-only | Devices and the action map, milestone 5 |

Proposed rule for the lint: a crate in `[boundary]` may name `lactose_input::{sample,
apply, history}` and the crate root, and nothing under `lactose_input::{raw, latest,
sources, mapping}`.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| Game crates | A component to take as `&PlayerInput` with movement, absolute yaw and pitch, the `throw` of section 6.1 and the `aim` it multiplies | `PlayerInput`; `input.pressed.contains(Action::Throw)`; `input.aim()`; `input.world_movement()` for movement; `input.held` for held actions; `contains_bit` for the game's own bits above `Action::NAMED` |
| | Every value a kernel reads built from integers by `lactose_math` | `Yaw::radians`, `Pitch::radians`, `Axis2::to_vec2`, `aim`, `world_movement`, each with its formula below |
| | A declared writer so a transient column may be read | `sets::APPLY` holds the exclusive writer declaration (`writes = [PlayerInput]`) in `Stage::Input` after `sim::sets::INPUT` |
| `app` (headless, milestone 1) | Register the type and the system, install the applier, for every world kind | `register_types`, `register_systems`, `SimulationBuilder::inputs(Box::new(PlayerInputApplier::new()))`; a test builds a frame with `PlayerInput { .. }.to_player_sample(tick)` |
| `lactose-sim` | An `InputApplier` that decodes `PlayerSample::sample` with the component's `CheckedBitPattern`, finds the entity from `ClaimedBy` plus `StreamingAnchor`, writes with `World::insert` | `PlayerInputApplier`, `player_entity`, "The applier" below |
| `lactose-predict` | The local player's own sample into the prediction world through the same applier; the same bytes in the ring | `PlayerInput::to_player_sample` into `TickInput::inputs`; the same `PlayerInputApplier`; `player_entity` resolves once `predict` writes `StreamingAnchor` beside the claim |
| `net` (milestone 4) | The last N samples per packet; a fixed-size image per sample; substitution as a pure function; a way to refuse garbage before it reaches a frame; the schema the protocol hash covers | `SampleHistory` on both ends, `iter` to pack and `insert` to merge; `to_sample_bytes`, `SAMPLE_SIZE`; `PlayerInput::substitute` with `PlayerSample::substituted` set by `net`; `from_sample_bytes`; `PlayerInput::SCHEMA` |
| Presentation (milestone 2) | The latest input's yaw and pitch for the late latch, outside the boundary and the tick | `latest::LatestSample::{publish, read}`, `RawSample::{yaw, pitch}` |
| Sources and mapping (milestone 5) | Named places; the action bits a map assigns | `sources::Source`, `mapping`; `Actions::with_bit` and `Action::NAMED` |

## The registration `app` writes

```rust
use lactose_core::Result;
use lactose_ecs::WorldKind;
use lactose_input::PlayerInputApplier;
use lactose_sim::{Simulation, SimulationBuilder, TypeTable};

fn build(kind: WorldKind) -> Result<Simulation> {
    let mut types = TypeTable::new();
    lactose_input::register_types(&mut types)?;
    // game_sim::register_types(&mut types)?; ...
    SimulationBuilder::new(kind, types)
        .systems(lactose_input::register_systems)
        // .systems(game_sim::register_systems) ...
        .inputs(Box::new(PlayerInputApplier::new()))
        .build()
}
```

The same three calls serve the server, the prediction world and a replay. This crate has
no `setup` function: it inserts no resource. A crate whose exclusive system must run at
the sync point after the samples are written declares its set `.after(&[&sim::sets::INPUT])`
and `app` orders it with `Registration::order(&lactose_input::sets::APPLY, &THEIRS)`.

## Behaviour the signatures cannot express

**The sample** (architecture sections 5.3, 6.1, 9.2 and 15.1)

- `PlayerInput` is `transient`: the ECS zeroes the column at every input sync point and
  the applier writes it there, inside `sim::apply_inputs`, for every player the frame
  carries a sample for. A row that received no sample this tick reads as
  `PlayerInput::IDLE`, the all-zero value, which is also `Default`: no movement, yaw zero,
  level, nothing held, nothing pressed. A kernel cannot tell "no sample" from "idle", and
  does not need to: on the server `net` substitutes before the frame (below), so a
  connected player always has an entry; in the prediction world the local player always
  has one; in a headless test an absent entry means idle.
- The component is not replicated and not extracted: inputs reach the server through the
  input path and the client through its own sampling, never through replication.
- The layout is `repr(C)`, sixteen bytes, alignment four, no padding: `movement: Axis2`
  (`x: i16` at 0, `y: i16` at 2), `yaw: Yaw` (`u16` at 4), `pitch: Pitch` (`i16` at 6),
  `held: Actions` (`u32` at 8), `pressed: Actions` (`u32` at 12). Field IDs 1 to 5 in that
  order. The static assertions in `lib.rs` pin the size, the alignment, the policy and
  the plain-data traits.
- `Axis2` holds the movement stick or keys in the player's own frame, `x` right and `y`
  forward, each component in `[-AXIS_SCALE, AXIS_SCALE]` with `AXIS_SCALE = 32767`
  decoding to one. The quantiser limits the raw vector to the unit disc; a decoded vector
  exceeds unit length only by rounding, at most one unit per axis. `i16::MIN` is accepted
  and decodes to just below minus one.
- `Yaw` and `Pitch` are in units of 1/65536 of a turn (`UNITS_PER_TURN = 65536`, about
  0.0055 degrees). `Yaw` is a `u16` and wraps: 0 and 65536 are the same facing. `Pitch` is
  an `i16`, positive looking up, valid within `[-Pitch::LIMIT, Pitch::LIMIT]` with
  `LIMIT = 16384`, a quarter turn each way; outside it the sample is invalid.
- `Actions` is a 32-bit mask; bit `n` set means action `n` is in the set. Bits `0..5` are
  the named actions of `Action` in declaration order: `Jump`, `Throw`, `Interact`,
  `Sprint`, `Crouch`; `Action::NAMED` is 5. Bits `5..32` are the game's, assigned by the
  action map of milestone 5 and read with `contains_bit`. Naming more of them later adds
  variants to `Action` and changes no encoding. `contains_bit` and `with_bit` treat a bit
  at or above `Actions::BITS` as never set. These methods are the definition of the type
  and are final, not `todo!()`.
- `held` is the set of actions held during the tick. `pressed` is the set that became
  held since the previous sample: the edges, computed once by the quantiser on the client
  from the previous sample's `held` (`held.without(previous_held)`) and carried in the
  sample, so neither the server nor a replay keeps button history and a sample is a pure
  value. A kernel that acts once per press tests `pressed`; one that acts while held
  tests `held`. There is no `released` set: a kernel that needs release edges keeps the
  previous `held` in a component of its own.
- `PlayerInput::validate` checks the one range the layout cannot, the pitch limit, and
  fails with `InvalidData`. Every other field accepts every bit pattern of its width, so
  the only invalid sixteen-byte image is one with a pitch beyond the limit.

**The world frame** (architecture section 5.3: the simulation uses the sample's yaw and
pitch for movement direction)

- The frame is right-handed with `+Y` up. Yaw zero faces `-Z`; yaw is the rotation about
  `+Y` that `lactose_math::Quat::from_rotation_y` makes, so a positive yaw turns left and a
  quarter turn (16384 units) faces `-X`. Pitch is the rotation about the yawed `+X` axis,
  positive upward. Every direction a kernel reads from a sample comes from the two
  functions below, so no game crate re-derives the convention, and the camera rig
  (presentation) uses the same one.
- `Yaw::radians` is `Scalar::from_u32(units) / Scalar::from_u32(UNITS_PER_TURN) *
  Scalar::TAU`: the division by a power of two is exact, the product rounds once; the
  result is in `[0, TAU)`. `Pitch::radians` is the same with `Scalar::from_i32(units)`,
  in `[-PI/2, PI/2]` for a valid pitch. `Axis2::to_vec2` is
  `Scalar::from_i32(c) / Scalar::from_i32(AXIS_SCALE)` per component, one true division
  each.
- `PlayerInput::aim` is, with `(sin_y, cos_y) = yaw.radians().sin_cos()` and
  `(sin_p, cos_p) = pitch.radians().sin_cos()`, the vector
  `(-(cos_p * sin_y), sin_p, -(cos_p * cos_y))`, each product rounded once. It has unit
  length up to rounding, and is `-Vec3::Z` at the idle sample. It is the `aim` of the
  section 6.1 example.
- `PlayerInput::world_movement` is, with `m = movement.to_vec2()`, the vector
  `(m.x * cos_y - m.y * sin_y, 0, -(m.x * sin_y + m.y * cos_y))`, evaluated in that
  order: `m.y` along the yaw's forward `(-sin_y, 0, -cos_y)` and `m.x` along its right
  `(cos_y, 0, -sin_y)`. Its length is at most one up to rounding.
- Both are computed when called and never stored: the sample carries integers only, so
  the client and the server derive the same bits from the same bytes, and a derived float
  never enters the journal or the wire. `sin_cos` is `lactose_math`'s, bit-identical to
  the two calls, on every target.

**The byte image** (architecture sections 9.2 and 15.5)

- `to_sample_bytes` is the sixteen memory bytes of the value, which on every supported
  target (x86-64 and aarch64) is each field little-endian at its offset above. The crate
  refuses to build on a big-endian target (`compile_error!`) rather than encode
  differently there. `PlayerSample::sample` holds exactly these bytes; the journal stores
  them verbatim and the wire carries them; `SAMPLE_SIZE` is 16.
- The image carries no version byte. Its version is the component's schema version
  (`PlayerInput::SCHEMA`, version 1), which the protocol hash of architecture section 9.1
  covers, so a layout change refuses the connection, and which the journal's checkpoint
  records with the registry it was written against.
- `from_sample_bytes` accepts exactly `SAMPLE_SIZE` bytes, reads them through the
  component's own `CheckedBitPattern` (`lactose_reflect::decode`) and then `validate`s;
  anything else is `InvalidData`. `decode(to_sample_bytes(x)) == x` for every valid `x`.
- `to_player_sample(input_tick)` is `PlayerSample { input_tick, substituted: false,
  sample: to_sample_bytes().to_vec() }`. The one allocation of the client path, outside
  the tick.

**The applier** (`crates/sim/README.md`, "The input sync point")

- `PlayerInputApplier::apply(world, player, sample)` does, in this order and nothing
  else: `PlayerInput::from_sample_bytes(&sample.sample)?`, then
  `player_entity(world, player)` or `NotFound`, then `World::insert(key, input)`. It reads
  neither `sample.input_tick`, which `sim` has already applied through
  `World::set_input_tick`, nor `sample.substituted`, which is the journal's. An error is
  logged by `sim` at target `curd` and that entry is dropped; the tick runs.
- `World::insert` adds the component to a row that lacks it, an archetype change the
  structural log records, once per player; afterwards it overwrites. Because the column
  is transient the row keeps the component across ticks. A kernel that takes
  `&PlayerInput` therefore runs for an entity from the tick of its first sample on; a
  session applier that wants the kernels from the first tick spawns the player's entity
  with `PlayerInput::IDLE` among its components, as `tests/consumers.rs` does. No `get_mut`
  path exists, since there is no reason to refuse a first sample.
- `player_entity(world, player)` is the row with `ClaimedBy { player }` that also carries
  `StreamingAnchor`, found through `World::query::<(&ClaimedBy, &StreamingAnchor)>`,
  which yields in ascending key order: with two such rows, a game's bug, the lesser origin
  key is the answer on every machine and under every layout. It allocates nothing beyond
  the query's own. It is the one definition of the player's entity: the session applier's
  `Leave` and `net`'s and `predict`'s lookups use it, so the appliers cannot disagree.
- The same applier serves the server (samples from `net`), the prediction world (the
  local player's own sample, resolving because `lactose-predict` writes `StreamingAnchor`
  beside its claim) and a replay (samples from the journal). It is `Send + Sync` and holds
  no state: the world is its only input, so a replay from a checkpoint needs nothing of
  it.
- The writer declaration `input::player_input_writer` is an exclusive system in
  `sets::APPLY`, `Stage::Input`, after `sim::sets::INPUT`, `worlds = [server, prediction]`,
  declaring `writes = [PlayerInput]` and doing nothing: the write happened inside
  `sim::apply_inputs`. The ECS counts an exclusive system's declaration as satisfying
  readers in later stages, which is what lets a kernel in `Stage::Simulation` take
  `&PlayerInput`. Its body is final. `register_systems` adds the set and the system;
  `register_types` registers the component with `TypeTable::register::<PlayerInput>()`,
  which builds its hasher: a transient column is hashed with whatever the tick left in it
  (`crates/sim/README.md`, "The world hash"), so two machines that applied different
  samples hash apart at the next sync point, which is what a desync diff should show.

**Redundancy and substitution** (architecture section 9.2)

- `REDUNDANT_SAMPLES` is 8: every input packet carries the samples of the last eight
  ticks, 133 ms at 60 Hz, so at a 30 Hz send rate three consecutive lost packets lose no
  sample. `net` may amend the number with its bandwidth model; it is not part of any
  schema or hash.
- `SampleHistory` holds the samples of the `CAPACITY` ticks up to and including the
  newest inserted, one slot per tick, a slot possibly empty. `insert(tick, input)`: a tick
  newer than the newest held moves the window forward to it and drops the slots that fall
  out; a tick inside the window fills its slot if empty, and is ignored (returning `false`)
  if the slot holds a sample already, so the first delivery of a tick wins and a
  retransmitted packet changes nothing; a tick older than the window is ignored. `get`,
  `newest`, `len` and `is_empty` read it; `iter` yields `(Tick, &PlayerInput)` in ascending
  tick order, skipping empty slots. The client inserts its own sample every tick and packs
  `iter()`; the server inserts every entry of every packet, in any order.
- Ticks compare by wrapping difference, `tick.0.wrapping_sub(newest.0) as i32`, so the
  window is correct across the `u32` wrap as long as no two ticks in play are more than
  2^31 apart, the ECS's own rule.
- `PlayerInput::substitute` is the substitution rule: the previous sample applied, with
  `pressed` cleared, so a held action stays held and no edge repeats; `IDLE.substitute()`
  is `IDLE`, the substitute for a player with no previous sample. A pure function. `net`
  decides when a tick's sample is lost (its jitter window), builds the frame entry with
  `substitute().to_player_sample(tick)`, sets `substituted: true` so the journal records
  it, and tells the client; this crate decides only what is substituted. A press lost
  this way is a misprediction the client's reconciliation corrects.

**Raw samples and quantisation** (host-only, architecture section 9.2)

- `RawSample` is the frame's raw sample in host floats: `movement_x`, `movement_y` in
  `[-1, 1]`, `yaw` and `pitch` in radians (any yaw; pitch clamped at quantisation), and
  `held`. `IDLE` is zero and `Default`.
- `RawSample::quantise(previous_held)` is the one step from floats to integers, and the
  integers are what both machines simulate: the movement vector scaled down to the unit
  disc if longer (a NaN component counts as zero), each component times `AXIS_SCALE`,
  rounded to nearest with ties away from zero, saturating to `i16`; the yaw reduced into
  `[0, TAU)`, divided by `TAU`, times `UNITS_PER_TURN`, rounded, modulo `UNITS_PER_TURN`
  (a non-finite yaw gives zero); the pitch clamped to `[-PI/2, PI/2]`, divided by `TAU`,
  times `UNITS_PER_TURN`, rounded, clamped to `Pitch::LIMIT` (a NaN pitch gives zero);
  `held` as given and `pressed = held.without(previous_held)`. The result always passes
  `validate`. The caller keeps the previous sample's `held`; the first sample's previous
  is `Actions::EMPTY`, so every action held at start is also pressed.
- Quantisation happens before the sample enters any frame, so the client simulates the
  quantised value, the value it sends, and never the raw one.

**The latest sample** (host-only, architecture section 5.3)

- `LatestSample` is one slot the input task publishes a `RawSample` into every frame and
  the late latch reads immediately before submit. `publish` and `read` never block each
  other and never lock: the implementation is a sequence-locked copy, as `net-api`'s
  interpolation buffer is; a reader that observes a torn write retries, a publisher is
  never delayed. `read` returns the latest completed `publish`, never a mix of two, and
  `RawSample::IDLE` before the first. One publisher is the expected use; with several,
  the last to finish wins. It is `Send + Sync` and `const`-constructible, so it can be a
  static or live in a presentation resource.
- It holds the raw sample, not the quantised one: the latch wants the freshest rotation,
  sampled after the tick's sample was taken, and the quantisation step (1/65536 turn) is
  far below what a frame shows. The simulation never reads it.

**Sources and mapping** (host-only, milestone 5)

- `sources::Source::poll(&mut self, raw: &mut RawSample)` folds a device's state for the
  frame into the raw sample, which arrives holding the previous frame's values so a delta
  source (the mouse) can accumulate. A source reads no clock and produces nothing the
  simulation sees except through `quantise`. No source exists yet.
- `mapping` is the named place of the action map, from device controls to `Action` bits
  and movement axes, loaded as an asset. It is empty: the generic bits above
  `Action::NAMED` are what a map assigns.

## Not in this crate yet

- The sources and the action map themselves, and anything that touches a device or a
  clock: milestone 5. The input task that polls sources, quantises, fills the history,
  publishes the latest sample and builds the frame is `app`'s.
- The jitter window, when a tick's sample counts as lost, the packet format around the
  window, delta coding of consecutive samples and telling the client about a substitution:
  `net`, milestone 4. This crate gives the window type, the image and the rule.
- Release edges (`released`): a kernel that needs them keeps the previous `held` itself;
  added as a sixth field with a new ID if the game shows it needs them.
- Forwarding nearby players' inputs into the prediction world (architecture section 9.4,
  "later"): the same `PlayerSample` per player; nothing changes here.
- A lint that enforces the module split above on boundary crates; the rule is proposed
  under "What boundary code may use".
- Mouse-look sensitivity, dead zones, inversion and other per-player settings: the
  sources' and the map's, milestone 5.

## Proof for milestone 1

Nothing runs yet. The implementation lane replaces every `todo!()` and proves:

| What | How |
|---|---|
| The image | `from_sample_bytes(to_sample_bytes(x)) == x` for a sweep of valid samples; `to_sample_bytes(x) == bytemuck::bytes_of(&x)`; the offsets above; a 15-byte or 17-byte slice, and an image with pitch `LIMIT + 1`, fail with `InvalidData`; `to_player_sample` carries the same bytes with `substituted: false` |
| Decoding | `Yaw(0)`, `Yaw(16384)`, `Yaw(32768)` and `Yaw(49152)` give `0`, `PI/2`, `PI` and `3PI/2` within one ulp; `Pitch(±LIMIT)` give `±PI/2`; `Axis2 { x: AXIS_SCALE, y: 0 }.to_vec2()` is `(1, 0)` exactly; the formulas are the stated ones, checked against them written out |
| Directions | `IDLE.aim() == -Vec3::Z`; `Yaw(16384)` aims at `-X` and `Yaw(49152)` at `+X` within one ulp; `Pitch(LIMIT)` aims at `+Y`; `world_movement` of `(0, AXIS_SCALE)` at yaw zero is `(0, 0, -1)` and of `(AXIS_SCALE, 0)` is `(1, 0, 0)`; both have length at most one up to rounding over a sweep; golden bits for a small table, pinned, equal on x86-64 and aarch64 under SOUR's architecture mode |
| Quantisation | `quantise` of a sweep of raw samples passes `validate`; a vector of length two quantises to length one; yaw `TAU + 0.1` and `0.1` quantise alike; pitch beyond a quarter turn clamps; `pressed` is the edge set; NaN and infinite inputs give the stated zeros |
| The applier | Under SOUR: a server with a joined player, a frame with that player's sample, then a kernel taking `&PlayerInput` sees the value; a tick without a sample sees `IDLE`; a malformed sample or a player without an entity leaves the world unchanged and the tick runs; two claimed rows resolve to the lesser key under every layout; the prediction world applies the same bytes to the same bits |
| The writer declaration | A schedule with a kernel reading `PlayerInput` in `Stage::Simulation` builds with `register_systems` and fails with `TransientBeforeWriter` without it |
| The history | Insert eight ticks in order, then a ninth: the first is gone; a duplicate returns `false` and keeps the first; out-of-order inserts yield ascending from `iter`; a tick older than the window is ignored; across the `u32` wrap the same |
| Substitution | `substitute` keeps `held`, movement, yaw and pitch and clears `pressed`; `IDLE.substitute() == IDLE` |
| The latest sample | `read` after `publish` returns the published sample; under a publisher thread and reader threads, every read is a sample that was published, never a mix (a loom model or a stress test with a checksum field) |
