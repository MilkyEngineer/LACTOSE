# lactose-input

Player input as the simulation sees it: the quantised per-tick sample a kernel reads, its
byte image on the wire and in the journal, the applier that writes it at the input sync
point, and the redundancy window. The host side (raw samples, the quantiser, the
latest-sample slot, device sources and action mapping) is `lactose-input-host`.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | inside |
| Unsafe | none, `#![forbid(unsafe_code)]` |

**Status: implemented.** Every behaviour below runs and is tested (the table at the end
names the tests), and the tests that run a simulation record their hashes under SOUR.
Every sentence under "Behaviour the signatures cannot express" is one a dependent crate may
rely on. Architecture sections 4, 5.3, 6.1, 8.1, 8.2, 9.2, 9.4, 15.1 and 15.5 are the
design; where this file and the architecture disagree, the architecture wins and this file
is wrong. Decision D120 records the choices those sections left open and what the first
design review changed. `crates/sim/README.md` is the contract this crate's applier meets
("The input sync point": `PlayerSample`, `InputApplier`, the entity lookup, the writer
declaration) and is not restated here; `crates/ecs/README.md` gives `World::claimed`, the
index the lookup uses.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `app`, `predict`, `net`, `lactose-input-host`
and the game crates are written against this surface. `tests/consumers.rs` sketches what
each of them writes, and runs the sketches: a change that breaks it is a signature change
to list.

## Surface

| Item | Purpose |
|---|---|
| `PlayerInput` | The sample: a `transient` component, key `input::player_input`, twenty bytes of integers; what a kernel takes as `&PlayerInput` |
| `Axis2`, `Yaw`, `Pitch`, `Actions`, `Action` | Its value types: a movement vector in sixteen bits per axis, two angles in 1/65536 turn, a 32-bit action set and the named bits of milestone 1 |
| `PlayerInput::{aim, world_movement}`, `Yaw::{radians, rotation}`, `Pitch::radians`, `Axis2::to_vec2` | What a kernel derives from the integers, through `lactose_math`, when it reads them; nothing derived is stored |
| `PlayerInput::{to_sample_bytes, from_sample_bytes, validate, to_player_sample}`, `SAMPLE_SIZE` | The byte image `PlayerSample::sample` holds, the journal stores and the wire carries, and the frame entry |
| `PlayerInput::{substitute, after_substitute, IDLE}` | The substitution rule for a sample lost after redundancy, the repair of the edges a substitution swallowed, and the zero sample a row reads without one |
| `PlayerInputApplier`, `player_entity` | The `InputApplier` `app` installs, and the one definition of a player's entity |
| `register_types`, `register_systems`, `sets::APPLY` | The registration pair `app` calls, and the set another crate's sync-point set is ordered against |
| `SampleHistory`, `SampleIter`, `REDUNDANT_SAMPLES` | The redundancy window: the last eight ticks' samples, filled by the client, packed into every packet, filled by the server from every packet within the ticks it can still apply |
| `UNITS_PER_TURN`, `AXIS_SCALE`, `Pitch::LIMIT`, `Action::NAMED`, `Actions::BITS`, `Actions::GAME_FIRST` | The constants of the encodings |

Hidden plumbing: none. The private fields of `SampleHistory`, `SampleIter` and
`PlayerInputApplier` are the implementation's to reshape.

## The boundary

This crate is inside the determinism boundary (architecture section 8.1): the floats a
kernel reads from a sample (`aim`, `world_movement`, `radians`, `to_vec2`) are computed
here, so they are computed under the boundary lint file (`clippy.toml`, a byte-identical
copy of `crates/core/clippy.toml`), inside the boundary build ID of section 9.1, and with
no dependency a boundary crate does not already have (`lactose-core`, `lactose-math`,
`lactose-reflect`, `lactose-ecs`, `lactose-sim`; nothing third-party). `PlayerInput` is
therefore a boundary component type, which the protocol hash of section 9.1 covers.
Everything that is floats or devices is in `lactose-input-host`, which no boundary crate
may depend on (`crate-tiers.toml`); the one thing that crosses from it is the integer
sample its quantiser produces.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| Game crates | A component to take as `&PlayerInput` with movement, absolute yaw and pitch, the `throw` of section 6.1 and the `aim` it multiplies | `PlayerInput`; `input.pressed.contains(Action::Throw)`; `input.aim()`; `input.world_movement()` for movement; `input.held` for held actions, `input.released` for release edges; `contains_bit(Actions::GAME_FIRST + k)` for the game's own bit `k` |
| | Every value a kernel reads built from integers by `lactose_math`; a facing rotation to compose with | `Yaw::radians`, `Yaw::rotation`, `Pitch::radians`, `Axis2::to_vec2`, `aim`, `world_movement`, each with its formula below |
| | A declared writer so a transient column may be read | `sets::APPLY` holds the exclusive writer declaration (`writes = [PlayerInput]`) in `Stage::Input` after `sim::sets::INPUT` |
| `app` (headless, milestone 1) | Register the type and the system, install the applier, for every world kind | `register_types`, `register_systems`, `SimulationBuilder::inputs(Box::new(PlayerInputApplier::new()))`; a test builds a frame with `PlayerInput { .. }.to_player_sample(tick)` |
| `lactose-sim` | An `InputApplier` that decodes `PlayerSample::sample` with the component's `CheckedBitPattern`, finds the entity from `ClaimedBy` plus `StreamingAnchor`, writes with `World::insert` | `PlayerInputApplier`, `player_entity` over `World::claimed`, "The applier" below |
| `lactose-predict` | The local player's own sample into the prediction world through the same applier; the same bytes in the ring; the lookup between ticks | `PlayerInput::to_player_sample` into `TickInput::inputs`; the same `PlayerInputApplier`; `player_entity(&World, ..)` resolves once `predict` writes `StreamingAnchor` beside the claim |
| `net` (milestone 4) | The last N samples per packet; a fixed-size image per sample; validation of every received image; a bounded merge the wire cannot steer; substitution and the repair after it as pure functions; the schema the protocol hash covers | `SampleHistory` on both ends, `iter` to pack and `insert_bounded` to merge; `to_sample_bytes`, `SAMPLE_SIZE`; `from_sample_bytes` on every received image, a failure treated as lost; `PlayerInput::substitute` and `after_substitute` with `PlayerSample::substituted` set by `net`; `PlayerInput::SCHEMA` |
| `lactose-input-host` | The sample to produce and the action set to fill; the invariants its quantiser must meet | `PlayerInput`, `Actions`, the constants; `validate` |

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
  level, nothing held, pressed or released. A kernel cannot tell "no sample" from "idle",
  and does not need to: on the server `net` substitutes before the frame (below), so a
  connected player always has an entry; in the prediction world the local player always
  has one; in a headless test an absent entry means idle.
- The component is not replicated and not extracted: inputs reach the server through the
  input path and the client through its own sampling, never through replication.
- The layout is `repr(C)`, twenty bytes, alignment four, no padding: `movement: Axis2`
  (`x: i16` at 0, `y: i16` at 2), `yaw: Yaw` (`u16` at 4), `pitch: Pitch` (`i16` at 6),
  `held: Actions` (`u32` at 8), `pressed: Actions` (`u32` at 12), `released: Actions`
  (`u32` at 16). Field IDs 1 to 6 in that order. The static assertions in `lib.rs` pin the
  size, the alignment, the policy and the plain-data traits.
- `Axis2` holds the movement stick or keys in the player's own frame, `x` right and `y`
  forward, each component in `[-AXIS_SCALE, AXIS_SCALE]` with `AXIS_SCALE = 32767`
  decoding to one. The quantiser limits the raw vector to the unit disc and `validate`
  enforces the disc plus the quantiser's rounding (below), so a decoded vector exceeds unit
  length only by rounding, at most one unit per axis, and a kernel may take
  `world_movement`'s length as at most one up to rounding.
- `Yaw` and `Pitch` are in units of 1/65536 of a turn (`UNITS_PER_TURN = 65536`, about
  0.0055 degrees). `Yaw` is a `u16` and wraps: 0 and 65536 are the same facing. `Pitch` is
  an `i16`, positive looking up, valid within `[-Pitch::LIMIT, Pitch::LIMIT]` with
  `LIMIT = 16384`, a quarter turn each way; outside it the sample is invalid.
- `Actions` is a 32-bit mask; bit `n` set means action `n` is in the set. Bits
  `0..Actions::GAME_FIRST`, that is `0..16`, are the engine's: bits `0..5` are the named
  actions of `Action` in declaration order, `Jump`, `Throw`, `Interact`, `Sprint`,
  `Crouch` (`Action::NAMED` is 5), and bits `5..16` are reserved for actions the engine
  names later, which adds variants to `Action` (it is `non_exhaustive`) and moves no bit.
  Bits `16..32` are the game's, assigned by the action map of milestone 5: the game's bit
  `k` is `Actions::GAME_FIRST + k`, read with `contains_bit` and set with `with_bit`.
  `GAME_FIRST` never changes, so a game's bit keeps its number when the engine names
  more. `contains_bit` and `with_bit` treat a bit
  at or above `Actions::BITS` as never set. `union`, `intersection` and `without` are the
  set operations. These methods are the definition of the type and are final.
- `held` is the set of actions held during the tick. `pressed` is the set that became
  held since the previous sample and `released` the set that stopped being held: the
  edges, computed once by the quantiser on the client from the previous sample's `held`
  (`held.without(previous_held)` and `previous_held.without(held)`) and carried in the
  sample, so neither the server nor a replay keeps button history and a sample is a pure
  value. A kernel that acts once per press tests `pressed`, one that acts while held tests
  `held`, one that acts on release (a bow loosed, a charged attack, a hold-to-aim throw)
  tests `released`.
- `PlayerInput::validate` checks the invariants kernels are told to rely on, and fails
  with `InvalidData` on the first that does not hold: the pitch is within `Pitch::LIMIT`;
  neither movement component is `i16::MIN`, and in 64-bit integers
  `x² + y² <= AXIS_SCALE² + 2·AXIS_SCALE` (a vector in the unit disc rounded by at most
  half a unit per component never exceeds it); `pressed.without(held).is_empty()`, an edge
  names a held action; `released.intersection(held).is_empty()`, a release names an action
  not held. Every other field accepts every bit pattern of its width. The quantiser's
  output always passes, so a failure means a forged or corrupt image, and the server never
  simulates one: a modified client cannot get diagonal speed or an edge every tick.

**The world frame** (architecture section 5.3: the simulation uses the sample's yaw and
pitch for movement direction)

- The frame is right-handed with `+Y` up. Yaw zero faces `-Z`; yaw is the rotation about
  `+Y` that `lactose_math::Quat::from_rotation_y` makes, so a positive yaw turns left and a
  quarter turn (16384 units) faces `-X`. Pitch is the rotation about the yawed `+X` axis,
  positive upward. Every direction a kernel reads from a sample comes from the functions
  below, so no game crate re-derives the convention; the camera rig (presentation) uses the
  same one, and the cooker turns glTF assets, which face `+Z`, half a turn about `+Y` so a
  character faces `-Z` at yaw zero.
- `Yaw::radians` is `Scalar::from_u32(units) / Scalar::from_u32(UNITS_PER_TURN) *
  Scalar::TAU`: the division by a power of two is exact, the product rounds once; the
  result is in `[0, TAU)`. `Pitch::radians` is the same with `Scalar::from_i32(units)`,
  in `[-PI/2, PI/2]` for a valid pitch. `Axis2::to_vec2` is
  `Scalar::from_i32(c) / Scalar::from_i32(AXIS_SCALE)` per component, one true division
  each.
- `Yaw::rotation` is `Quat::from_rotation_y(self.radians())`: the facing as a rotation,
  for a kernel or the sim skeleton that composes rotations rather than reading a
  direction. It turns `-Vec3::Z` into the horizontal part of `aim` within rounding; `aim`
  itself is the stated formula, not this product.
- `PlayerInput::aim` is, with `(sin_y, cos_y) = yaw.radians().sin_cos()` and
  `(sin_p, cos_p) = pitch.radians().sin_cos()`, the vector
  `(-(cos_p * sin_y), sin_p, -(cos_p * cos_y))`, each product rounded once. It has unit
  length up to rounding, and is `-Vec3::Z` at the idle sample. It is the `aim` of the
  section 6.1 example.
- `PlayerInput::world_movement` is, with `m = movement.to_vec2()`, the vector
  `(m.x * cos_y - m.y * sin_y, 0, -(m.x * sin_y + m.y * cos_y))`, evaluated in that
  order: `m.y` along the yaw's forward `(-sin_y, 0, -cos_y)` and `m.x` along its right
  `(cos_y, 0, -sin_y)`. For a valid sample its length is at most one up to rounding.
- Both are computed when called and never stored: the sample carries integers only, so
  the client and the server derive the same bits from the same bytes, and a derived float
  never enters the journal or the wire. `sin_cos` is `lactose_math`'s, bit-identical to
  the two calls, on every target; the golden bits of a small table are pinned by
  `tests/sample.rs`, run in debug and in release, on aarch64 and on x86-64-v3 (linux-x64
  and windows-x64 in CI and in SOUR's architecture mode; `docs/verification.md`).

**The byte image** (architecture sections 9.1, 9.2 and 15.5)

- `to_sample_bytes` is the twenty memory bytes of the value, which on every supported
  target (x86-64 and aarch64) is each field little-endian at its offset above. The crate
  refuses to build on a big-endian target (`compile_error!`) rather than encode
  differently there. `PlayerSample::sample` holds exactly these bytes; the journal stores
  them verbatim and the wire carries them; `SAMPLE_SIZE` is 20.
- The image carries no version byte. Its version is the component's schema version
  (`PlayerInput::SCHEMA`, version 1), which the protocol hash of architecture section 9.1
  covers, since this crate is a boundary crate and `PlayerInput` a boundary component
  type, so a layout change refuses the connection. A journal replays on the build that
  wrote it; the checkpoint records no schema (`crates/sim/README.md`).
- `from_sample_bytes` accepts exactly `SAMPLE_SIZE` bytes, reads them through the
  component's own `CheckedBitPattern` (`lactose_reflect::decode`) and then `validate`s;
  anything else is `InvalidData`. `decode(to_sample_bytes(x)) == x` for every valid `x`.
  `net` must validate every received sample with `from_sample_bytes` and treat a failure
  as lost (substitute): a malformed image then never reaches a frame, where the applier's
  drop would leave the row at `IDLE` and snap the player's yaw to zero for that tick.
- `to_player_sample(input_tick)` is `PlayerSample { input_tick, substituted: false,
  sample: to_sample_bytes().to_vec() }`. The one allocation of the client path, outside
  the tick.

**The applier** (`crates/sim/README.md`, "The input sync point")

- `PlayerInputApplier::apply(world, player, sample)` does, in this order and nothing
  else: `PlayerInput::from_sample_bytes(&sample.sample)?`; then `player_entity(world,
  player)`, and with `None` returns `Ok(())` without writing, since a player whose `Join`
  is held while its region streams in has samples before it has a row and that is not an
  error worth a log line per tick (the facade logs nothing; the implementation may log at
  debug at most); then `World::insert(key, input)`. It reads neither `sample.input_tick`,
  which `sim` has already applied through `World::set_input_tick`, nor
  `sample.substituted`, which is the journal's. An error is logged by `sim` at target
  `curd` and that entry is dropped; the tick runs.
- `World::insert` adds the component to a row that lacks it, an archetype change the
  structural log records, once per player; afterwards it overwrites. Because the column
  is transient the row keeps the component across ticks. A kernel that takes
  `&PlayerInput` therefore runs for an entity from the tick of its first sample on; a
  session applier that wants the kernels from the first tick spawns the player's entity
  with `PlayerInput::IDLE` among its components, as `tests/consumers.rs` does. No `get_mut`
  path exists, since there is no reason to refuse a first sample.
- `player_entity(world: &World, player)` is the first key of `World::claimed(player)`,
  the rows with `ClaimedBy { player }` in ascending origin-key order from the world's
  claim index, whose row also carries `StreamingAnchor` by `World::get`: with two such
  rows, a game's bug, the lesser origin key is the answer on every machine and under every
  layout. It costs the player's claims, allocates nothing, takes a shared world and bumps
  no stamp clock, so `net` and `predict` may call it between ticks. It is never cached: a
  side table in the session applier is what `sim` forbids, and a rollback would leave it
  stale. It is the one definition of the player's entity: the session applier's `Leave`
  and `net`'s and `predict`'s lookups use it, so the appliers cannot disagree.
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
  newest inserted, one slot per tick, a slot possibly empty. `insert(tick, input)`, the
  client's path: a tick newer than the newest held moves the window forward to it and
  drops the slots that fall out; a tick inside the window fills its slot if empty, and is
  ignored (returning `false`) if the slot holds a sample already, so the first delivery of
  a tick wins; a tick older than the window is ignored. `insert_bounded(consumed, tick,
  input)`, the server's path for every entry of every packet, stores `tick` only if it
  lies in `(consumed, consumed + CAPACITY]` by wrapping difference, where `consumed` is the
  last tick applied for the player, and otherwise returns `false` and changes nothing: a
  forged or stale tick off the wire can neither move the window away from the ticks still
  to come (one entry at `newest + 2^30` would otherwise make every genuine later tick
  "older than the window" for years) nor resurrect a tick already applied. `net` keeps
  the consumed tick, and may keep a buffer of its own depth in front of this one; the
  window's capacity ties nothing to how far ahead the server may buffer. `get`, `newest`,
  `len` and `is_empty` read it; `iter` yields `(Tick, &PlayerInput)` in ascending tick
  order, skipping empty slots. The client inserts its own sample every tick and packs
  `iter()`.
- Ticks compare by wrapping difference, `tick.0.wrapping_sub(other.0) as i32`, so the
  window is correct across the `u32` wrap as long as no two ticks in play are more than
  2^31 apart, the ECS's own rule.
- `PlayerInput::substitute` is the substitution rule, refining section 9.2's "replaced by
  the previous input": the previous sample applied, with `pressed` and `released` cleared,
  so a held action stays held and no edge repeats; `IDLE.substitute()` is `IDLE`, the
  substitute for a player with no previous sample. A pure function. `net` decides when a
  tick's sample is lost (its jitter window), builds the frame entry with
  `substitute().to_player_sample(tick)`, sets `substituted: true` so the journal records
  it, and tells the client; this crate decides only what is substituted.
- `PlayerInput::after_substitute(applied)` repairs the edges a substitution swallowed:
  the client computed the first delivered sample's edges against its own previous sample,
  which was lost, so a press whose first sample was lost would otherwise never fire for the
  whole hold. `net` applies it once, to the first delivered sample after one or more
  substitutes, with `applied` the last substitute applied, and it replaces the sample's
  edges rather than adding to them: `pressed = held.without(applied.held)` and
  `released = applied.held.without(held)`. Replacing is what keeps every applied sample's
  edges consistent with the applied `held` sequence: a union would keep the client's own
  edges, and so release an action no applied sample held (a hold-to-aim throw firing with
  no aim) or re-press one the substitute already held. What it costs is a tap, or a
  release and re-press, wholly inside the gap. The result passes `validate` when the
  input does, and the journal records the applied bytes, so a replay needs no rule of its
  own. The press fires at most a few ticks late, on the server and, once the correction
  arrives, on the client.

## Not in this crate yet

- The sources, the quantiser, the latest-sample slot and the action map: `lactose-input-host`.
- The jitter window, when a tick's sample counts as lost, a deeper server-side buffer, the
  packet format around the window, delta coding of consecutive samples and telling the
  client about a substitution: `net`, milestone 4. This crate gives the window type, the
  image, the validation and the rules.
- The tick the client saw (architecture section 9.5, lag compensation): simulation input,
  so it must travel in the journaled sample bytes; it goes in as a field of schema version
  2 at milestone 4, with its ID and the validation rule decided there.
- Forwarding nearby players' inputs into the prediction world (architecture section 9.4,
  "later"): the same `PlayerSample` per player; nothing changes here.

## Proof for milestone 1

The tests in `tests/`, each a nextest test that asserts on values, counts, orders, bytes
and hashes and never on timing. `tests/common/mod.rs` is a small game written as a game
crate writes it: a kernel in `Stage::Simulation` that records into a persistent component
everything it derives from `&PlayerInput` (so the world hash covers the derived floats), a
session applier that spawns and despawns through `player_entity`, a scripted client per
player, and the frames of a server program (players joined with and without `IDLE`, a
skipped tick, a malformed image of every kind, a held join, a stranger, a player with two
anchored rows and a lesser unanchored one, a leave and a rejoin) and of a prediction
program. `tests/sour.rs` records both programs under SOUR, which runs the package in
workers, repeat, layout, chaos and (nightly) architecture modes. The review probes of the
design rounds are ported as tests of the real code where they apply: the forgeries, the
quantiser sweep, the world frame against `Quat`, the exhaustive repair check and the
writer declaration.

| What | How |
|---|---|
| The image | `sample.rs`: `from_sample_bytes(to_sample_bytes(x)) == x` and `to_sample_bytes(x) == bytemuck::bytes_of(&x)` over a sweep of 20,000 valid samples; each field little-endian at its offset; 0, 1, 19, 21 and 40 bytes fail with `InvalidData`; `to_player_sample` carries the same bytes with `substituted: false` |
| Validation | `sample.rs`: each rule refused on its own, through `validate` and through `from_sample_bytes`: pitch `±(LIMIT + 1)`; a movement component of `i16::MIN`; `(AXIS_SCALE, AXIS_SCALE)`, and the disc's bound exactly (`(AXIS_SCALE, 255)` accepted, `(AXIS_SCALE, 256)` refused); `pressed` 0b10 with `held` 0; `released` overlapping `held`; every bit pattern of the other fields accepted; the sweep accepted; a stand-in quantiser (an `f32` one of the same shape as `lactose-input-host`'s, not its rule), over 200,000 directions at five lengths up to 1e30, the diagonals, `f32::MAX` components, NaN and infinity, all accepted, worst `x² + y² - AXIS_SCALE²` within `2·AXIS_SCALE`, and every input at least one long quantised to unit length (within `2·AXIS_SCALE` of `AXIS_SCALE²`) |
| The action bits | `src/sample.rs` (unit test): every `Action` variant, listed by a wildcard-free match, has bit equal to its position, below `Actions::GAME_FIRST`, and `NAMED` counts them; the game's first bit is set and read by `with_bit` and `contains_bit` and is none of the named ones; `GAME_FIRST == 16` is a const assertion in `lib.rs` |
| Decoding | `sample.rs`: `Yaw(0)`, `Yaw(16384)`, `Yaw(32768)` and `Yaw(49152)` give `0`, `PI/2`, `PI` and `3PI/2` within one ulp; `Pitch(±LIMIT)` give `±PI/2` exactly; `Axis2 { x: AXIS_SCALE, y: 0 }.to_vec2()` is `(1, 0)` exactly; `radians`, `to_vec2`, `rotation`, `aim` and `world_movement` equal the stated formulas written out, bit for bit, over the sweep |
| Directions | `sample.rs`: `IDLE.aim() == -Vec3::Z`; `Yaw(16384)` aims at `-X`, `Yaw(32768)` at `+Z` and `Yaw(49152)` at `+X`, and `Pitch(±LIMIT)` at `±Y`, within `Scalar::EPSILON`; `Yaw::rotation` rotates `-Z` onto the level `aim` within 1e-15; `world_movement` of `(0, AXIS_SCALE)` at yaw zero is `(0, 0, -1)` and of `(AXIS_SCALE, 0)` is `(1, 0, 0)`, at a quarter turn `-X` and `-Z`; `aim` has unit length and `world_movement` at most `1 + 1/AXIS_SCALE` up to rounding over the sweep; `aim` is the yaw quaternion times the pitch quaternion applied to `-Z`, and `world_movement` the stick along the yawed forward and right, within 1e-12 over a grid of yaw and pitch; golden bits of twelve derived values for six samples, pinned (generated on aarch64, checked on every target CI tests, in debug and in CI's release step), and the same values hashed by `sour.rs` under the architecture mode |
| The applier | `applier.rs`: a kernel taking `&PlayerInput` reads, bit for bit, the sample of every tick that carried one and `IDLE` on every tick without one or with a malformed image; a player spawned without the component gets it from its first sample and the kernel runs from then on; the program with its malformed images hashes equal, every tick, to the program with those entries absent; samples for a held join and for a player who never joined hash equal to their absence, and the applier returns `Ok(())` for them; called directly, every malformed image is `InvalidData` and writes nothing, and `substituted` and `input_tick` are not read; the lesser of two anchored rows is the entity, and the answer follows an anchor or claim removed; the prediction world's records equal the server's byte for byte on every tick both simulate the player; the hashes of both programs are equal under the production layout and SOUR's layout mode for `Rows(1)`, `Rows(7)`, `Rows(64)` and `Bytes(16384)` under three seeds each, on one and four workers, with the lesser key the entity throughout. `lookup.rs`, in a binary with the tagged global allocator: among 0, 1,000 and 100,000 other rows `player_entity` names the right row for 64 players and allocates nothing, and the applier allocates exactly what the same `World::insert` calls allocate; an ignored test times the lookup by hand (about 50 ns at both sizes on an M1) |
| The writer declaration | `applier.rs`: on a server and a prediction world, the schedule with the kernel builds with `register_systems` and fails without it with `InvalidData` whose source is `TransientBeforeWriter { reader: "test::observe", component: PlayerInput::KEY }`; a client world builds with it |
| The history | `history.rs`: eight ticks in order, then a ninth: the first is gone; a duplicate returns `false` and keeps the first; out-of-order inserts yield ascending from `iter`; a tick older than the window is ignored; a jump past the window empties it; `insert_bounded` refuses `consumed`, `consumed - 1`, `consumed + CAPACITY + 1`, `newest + 2^30` and `newest + 2^31` and accepts `consumed + 1` to `consumed + CAPACITY`; the same from four bases across the `u32` wrap; and 20,000 random inserts around a moving newest tick, across the wrap, against a model |
| Substitution | `sample.rs`: `substitute` keeps `held`, movement, yaw and pitch and clears `pressed` and `released`; `IDLE.substitute() == IDLE`; `after_substitute` of a sample holding `Throw` after a substitute that did not sets `pressed` to exactly `Throw` and passes `validate`; of a sample that stopped holding what the substitute held sets `released` to exactly that; of a sample whose own `released` names an action the substitute never held, or whose own `pressed` names one it already held, drops that edge; over every valid (sample, applied) pair of a four-action set, the result's edges equal the differences of the two `held` sets, the rest is the sample's, and `validate` accepts it; `net`'s procedure over 20,000 ticks with a quarter lost keeps every applied sample's edges equal to the differences of the applied `held` sequence |
| The consumers | `consumers.rs`: the sketches compile against the surface, every world kind builds as `app` builds it, and a player joined through the sketched session applier and driven through the sketched `net` path with one tick lost presses `Throw` on the repaired tick, the sketched kernels spawn the bomb and write the velocity, and the sketched leave despawns the entity |
| SOUR | `sour.rs`: the server program for 60 ticks, and the prediction program rolled back to tick 30 and re-simulated, built from the session and recording `(tick, hash)` after every step |
