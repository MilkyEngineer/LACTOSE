# lactose-input-host

The host side of input: the frame's raw sample in floats, the quantiser that turns it into
the one `PlayerInput` both machines simulate, the latest-sample slot the late-latched
camera reads, and the named places for device sources and action mapping (milestone 5).
The simulation side, the sample and its applier, is `lactose-input`.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | outside, and no boundary crate may depend on it: it is absent from `[boundary_internal]` in `crate-tiers.toml` on purpose |
| Unsafe | none, `#![forbid(unsafe_code)]` |

**Status: implemented.** Every behaviour below runs and is tested (the table at the end
names the tests); the sources and the action map are milestone 5's. Every sentence under
"Behaviour the signatures cannot express" is one a dependent crate may rely on. Architecture sections 4, 5.3 and 9.2 are the design; where this file and the
architecture disagree, the architecture wins and this file is wrong. Decision D120 records
the choices those sections left open, for this crate and for `lactose-input` together.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `app` (the input task) and `presentation` (the
late latch) are written against this surface, and the milestone 5 sources will be.
`tests/consumers.rs` is a compile-only sketch of what they write: a change that breaks it is
a signature change to list.

## Surface

| Item | Purpose |
|---|---|
| `RawSample` | The frame's raw sample: movement, absolute yaw and pitch in radians, the held actions, and the actions latched since the last tick; floats |
| `RawSample::{begin_frame, hold, offer_movement, turn}` | How the input task starts a frame and how sources fold their state in, so sources combine instead of overwriting each other |
| `Quantiser`, `Quantiser::next` | The one step from floats to the integer sample, once per tick, keeping the previous `held` for the edges and consuming the latch |
| `LatestSample` | The latest raw sample, published by the input task and read by the late latch, sequence-locked |
| `sources::Source` | The shape of a device source (milestone 5) |
| `mapping` | The named place of the action map (milestone 5); empty |

Hidden plumbing: none. The private fields of `Quantiser` and `LatestSample` are the
implementation's to reshape. The `loom` feature builds the loom models of the latest-sample
slot and changes nothing the crate ships.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| `app` (the input task) | Poll the sources once per frame into one raw sample; quantise once per tick, also for catch-up ticks, without a press firing twice; put the sample in the frame and the history | `RawSample::begin_frame`, `Source::poll`; `Quantiser::next` once per tick; `PlayerInput::to_player_sample` and `SampleHistory` from `lactose-input` |
| Presentation (milestone 2) | The latest input's yaw and pitch for the late latch, outside the boundary and the tick, without a lock | `LatestSample::{publish, read}`, `RawSample::{yaw, pitch}` |
| Sources (milestone 5) | A way to contribute held actions, movement and camera deltas that combines across devices | `Source::poll` with `RawSample::{hold, offer_movement, turn}`; `Actions::with_bit` and `Actions::GAME_FIRST` in `lactose-input` for the map's bits |

## Behaviour the signatures cannot express

**The raw sample** (architecture section 9.2)

- `RawSample` holds `movement_x`, `movement_y` in `[-1, 1]`, `yaw` and `pitch` in radians,
  `held`, and `latched`, the actions held at any poll since the last tick. `IDLE` is zero
  and `Default`. Nothing of it reaches a boundary crate: the quantiser's output is the only
  thing that does.
- A frame starts with `begin_frame`: `held` becomes empty and the movement zero; `yaw`,
  `pitch` and `latched` keep their values, since the angles are absolute and accumulate
  and the latch accumulates until a tick. The input task calls it before polling the
  sources, so a key released on one source cannot be held over by a stale value, and the
  sources combine whatever their poll order:
  - `hold(actions)` is `held = held.union(actions)` and `latched = latched.union(actions)`;
  - `offer_movement(x, y)` replaces the current movement only if the offered squared
    length is larger, so of a stick and the keys the larger movement wins and a centred
    stick does not cancel the keys; a non-finite component (NaN or infinite) counts as zero,
    in the offer and in the current movement, and is stored as zero if the offer wins; the
    squared lengths are compared in `f64`, so no huge offer overflows;
  - `turn(delta_yaw, delta_pitch)` adds the deltas: the yaw is wrapped into `[-PI, PI)`
    after every delta, so an accumulated yaw never grows coarser than a sample unit (past
    1024 rad an `f32` is, and at 8192 rad a mouse delta of 4e-4 rad vanishes), the bounds
    being the `f32` `PI`, and a sum already in range is only rounded, so a zero delta never
    moves the yaw; the pitch is clamped to `[-PI/2, PI/2]`; a non-finite delta is ignored,
    each of the two on its own.
- A source never assigns the fields directly (`Source` below); the input task and the
  camera rig may.

**Quantisation** (architecture section 9.2: the client simulates the value it sends)

- `Quantiser` keeps the one piece of state the edges need, the previous tick's `held`, so
  the edges are computed once, on the client, and travel in the sample. It is called once
  per simulation tick, never once per frame: when the timer root runs several catch-up
  ticks from one frame's raw sample, the second and later samples carry no edges, so a
  press fires once on the client as it does on the server. A fresh quantiser's previous
  sample is idle, so every action held at the first tick is also pressed in it.
- `Quantiser::next(&mut raw)` produces, in integers: `movement` as the raw vector scaled
  down to the unit disc if longer (a non-finite component, NaN or infinite, counts as zero;
  the length is computed in `f64`, where no square of an `f32` overflows, so a huge finite
  stick quantises to unit length in its own direction), each component times `AXIS_SCALE`,
  rounded to nearest with ties away from zero, saturating to `i16`; `yaw` as the raw yaw
  divided by `TAU`, its fraction of a turn in `[0, 1)` (the yaw reduced into `[0, TAU)`)
  times `UNITS_PER_TURN`, rounded, modulo `UNITS_PER_TURN` (a non-finite yaw gives zero);
  `pitch` as the raw pitch clamped to `[-PI/2, PI/2]`, divided by `TAU`, times
  `UNITS_PER_TURN`, rounded, clamped to `Pitch::LIMIT` (a NaN pitch gives zero, an infinite
  one the limit of its sign); `held` as `raw.held.union(raw.latched)`, after which
  `raw.latched` is empty; `pressed` as `held.without(previous_held)`; `released` as
  `previous_held.without(held)`; and then `previous_held` becomes `held`. The result always
  passes `PlayerInput::validate`.
- One raw sample quantises to the same integers on every target: each value is computed in
  `f64` from the exact widening of its `f32`, through IEEE basic operations (`+`, `-`, `*`,
  `/`, `sqrt`, each correctly rounded) and exact ones (`floor`, `round`, the casts) only,
  with no library function and no fused multiply-add. `tests/golden.rs` pins the images of
  a table of raw samples, generated on aarch64 and checked by CI on x86-64.
- The latch is why a tick never misses a tap: a tick sees the `held` set of the last poll
  before it, so a press and release between two polls (a tap under 16.7 ms at 60 Hz, or
  under 33 ms on a 30 FPS client running two catch-up ticks per frame) would otherwise
  reach no sample. Because `hold` also adds to `latched` and only `next` clears it, such a
  tap becomes one tick of `held`: pressed at that tick, released at the next.
- Quantisation happens before the sample enters any frame, so the client simulates the
  quantised value, the value it sends, and never the raw one.

**The latest sample** (architecture section 5.3)

- `LatestSample` is one slot the input task publishes a `RawSample` into every frame and
  the late latch reads immediately before submit. `publish` and `read` never lock and
  never block a reader. The slot is a sequence-locked copy of the sample's six 32-bit
  words, as `net-api`'s interpolation buffer is. `publish`: compare-exchange (`Acquire`)
  the sequence from even to odd, then `fence(Release)`, then the words `Relaxed`, then
  store the next even value (`Release`); so a publisher may wait for another publisher but
  never for a reader, two concurrent publishers cannot interleave their words, and the
  fence orders every word write after the sequence was taken (without it a reader's
  acquire fence has nothing to synchronise with and a single publisher tears under the
  memory model, as the second review showed in loom). `read`: load the sequence `Acquire`,
  the words `Relaxed`, `fence(Acquire)`, reload the sequence, and retry on mismatch or
  odd; so it returns a sample some `publish` wrote and never a mix of two. `read` returns
  `RawSample::IDLE` before the first `publish`. The sequence is 64 bits wide, so it never
  wraps and two equal loads of it always bracket no publish. A retry spins briefly and then
  yields the thread, since the publisher it waits for may have been preempted mid-write;
  neither side takes a lock or makes a system call other than that yield. The words are the
  fields' bits, so a NaN payload is published unchanged. With several publishers the last to finish
  wins. It is `Send + Sync` and `const`-constructible, so it can be a static or live in a
  presentation resource.
- It holds the raw sample, not the quantised one: the latch wants the freshest rotation,
  sampled after the tick's sample was taken, and the quantisation step (1/65536 turn) is
  far below what a frame shows. The simulation never reads it.

**Sources and mapping** (milestone 5)

- `Source::poll(&mut self, raw: &mut RawSample)` folds a device's state for the frame into
  the raw sample through `hold`, `offer_movement` and `turn` only. A source reads no clock
  and produces nothing the simulation sees except through `Quantiser::next`. No source
  exists yet.
- `mapping` is the named place of the action map, from device controls to `Action` bits
  and movement axes, loaded as an asset. It is empty: the game's bits, from
  `Actions::GAME_FIRST` up, are what a map assigns.

## Not in this crate yet

- The sources and the action map themselves, and anything that touches a device or a
  clock: milestone 5. The input task that polls sources, quantises, fills the history,
  publishes the latest sample and builds the frame is `app`'s; its shape is in
  `tests/consumers.rs`.
- Mouse-look sensitivity, dead zones, inversion and other per-player settings: the
  sources' and the map's, milestone 5.

## Proof for milestone 1

The tests in `tests/` and the loom models in `src/models.rs`, each asserting on values,
bits, counts and orders and never on timing.

| What | How |
|---|---|
| Quantisation | `quantise.rs`: every combination of 24 values per field (in range, edges, huge, subnormal, signed zeros, NaN, both infinities) passes `validate`, and every input with a finite component at least one long quantises to unit length within `2·AXIS_SCALE`; 50,000 directions at seven lengths up to 1e30 likewise; `(2, 0)`, `(0, -2)` and `(f32::MAX, 0)` quantise to `AXIS_SCALE` on their axis and the huge diagonals to the unit diagonal; `±0.5` to `±16384` (ties away from zero); yaw `x`, `x ± TAU` and `x + 4·TAU` alike, the quarter turns exact, the largest `f32` below a turn to zero; pitch `PI/2`, 1.6, 2, 100, `f32::MAX` and infinity clamp to `±LIMIT`; the edges against the previous call over a sequence of held sets, the game's bits included; a second and later call on the same raw sample carry no edges; NaN and infinite stick components count as zero, a non-finite yaw and a NaN pitch give zero; the quantised angles decode through `lactose-input` to within half a unit |
| The latch | `quantise.rs`: a `hold` followed by a `begin_frame` with no `hold` and then `next`: held and pressed in that sample and released in the next; `next` leaves `latched` empty and `held` as it was; an action held across the tick, through a catch-up tick and later frames, is never released |
| The raw sample | `quantise.rs`: `begin_frame` keeps the angles and the latch and clears the rest; `hold` unions into both sets; `offer_movement` keeps the larger whatever the order, a centred stick cancels nothing, an equal offer keeps the first, a huge offer compares without overflow, and a non-finite component counts as zero in the offer and in the current movement; `turn` keeps the yaw in `[-PI, PI)` over 200,000 turns of six deltas, each wrapped yaw quantising within one unit of the same turn unwrapped; a 4e-4 rad delta still turns after 9,000 rad; a zero turn leaves the yaw, and a sum rounding up to the `f32` `PI` becomes `-PI`; the pitch clamps; a non-finite delta is ignored, each on its own |
| Golden bits | `golden.rs`: the 20-byte images of 16 raw samples and the angles six sequences of turns leave, generated on aarch64-apple-darwin, checked on every target CI tests |
| The latest sample | `latest.rs`: `IDLE` before the first `publish`, from `new`, `default` and a `static`; `read` after `publish` returns the published sample, NaN payload included, and a read does not consume; a static slot shared between threads; one and two publisher threads of 1,000,000 stamped samples against three readers, every read whole by a checksum word and, per publisher, never older than the reader's previous read, and the slot holding a last publish after the joins. `src/models.rs` (`cargo test -p lactose-input-host --features loom --release`): the shipped `publish` and `read` over loom's atomics, one publisher publishing twice and two publishers once each against a reader at preemption bound 2, every read whole and the final state whole; the same models with the publisher's `fence(Release)` removed find a torn read |
