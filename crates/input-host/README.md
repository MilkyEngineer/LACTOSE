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

**Status: facade.** Every public signature is final and every body that decides anything is
`todo!()`; nothing here runs. This file is the contract the implementation lane meets, and
every sentence under "Behaviour the signatures cannot express" is one a dependent crate may
rely on. Architecture sections 4, 5.3 and 9.2 are the design; where this file and the
architecture disagree, the architecture wins and this file is wrong. Decision D120 records
the choices those sections left open, for this crate and for `lactose-input` together.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `app` (the input task) and `presentation` (the
late latch) are written against this surface, and the milestone 5 sources will be.
`tests/consumers.rs` is a compile-only sketch of what they write.

## Surface

| Item | Purpose |
|---|---|
| `RawSample` | The frame's raw sample: movement, absolute yaw and pitch in radians, the held actions; floats |
| `RawSample::{begin_frame, hold, offer_movement, turn}` | How the input task starts a frame and how sources fold their state in, so sources combine instead of overwriting each other |
| `Quantiser`, `Quantiser::next` | The one step from floats to the integer sample, once per tick, keeping the previous `held` for the edges |
| `LatestSample` | The latest raw sample, published by the input task and read by the late latch, sequence-locked |
| `sources::Source` | The shape of a device source (milestone 5) |
| `mapping` | The named place of the action map (milestone 5); empty |

Hidden plumbing: none. The private fields of `Quantiser` and `LatestSample` are the
implementation's to reshape.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| `app` (the input task) | Poll the sources once per frame into one raw sample; quantise once per tick, also for catch-up ticks, without a press firing twice; put the sample in the frame and the history | `RawSample::begin_frame`, `Source::poll`; `Quantiser::next` once per tick; `PlayerInput::to_player_sample` and `SampleHistory` from `lactose-input` |
| Presentation (milestone 2) | The latest input's yaw and pitch for the late latch, outside the boundary and the tick, without a lock | `LatestSample::{publish, read}`, `RawSample::{yaw, pitch}` |
| Sources (milestone 5) | A way to contribute held actions, movement and camera deltas that combines across devices | `Source::poll` with `RawSample::{hold, offer_movement, turn}`; `Actions::with_bit` and `Action::NAMED` in `lactose-input` for the map's bits |

## Behaviour the signatures cannot express

**The raw sample** (architecture section 9.2)

- `RawSample` holds `movement_x`, `movement_y` in `[-1, 1]`, `yaw` and `pitch` in radians,
  and `held`. `IDLE` is zero and `Default`. Nothing of it reaches a boundary crate: the
  quantiser's output is the only thing that does.
- A frame starts with `begin_frame`: `held` becomes empty and the movement zero; `yaw` and
  `pitch` keep their values, since they are absolute and accumulate. The input task calls
  it before polling the sources, so a key released on one source cannot be held over by a
  stale value, and the sources combine whatever their poll order:
  - `hold(actions)` is `held = held.union(actions)`;
  - `offer_movement(x, y)` replaces the current movement only if the offered squared
    length is larger, so of a stick and the keys the larger movement wins and a centred
    stick does not cancel the keys; a NaN component counts as zero;
  - `turn(delta_yaw, delta_pitch)` adds the deltas: the yaw is wrapped into `[-PI, PI)`
    after every delta, so an accumulated yaw never grows coarser than a sample unit (past
    1024 rad an `f32` is, and at 8192 rad a mouse delta of 4e-4 rad vanishes); the pitch is
    clamped to `[-PI/2, PI/2]`; a non-finite delta is ignored.
- A source never assigns the fields directly (`Source` below); the input task and the
  camera rig may.

**Quantisation** (architecture section 9.2: the client simulates the value it sends)

- `Quantiser` keeps the one piece of state the edges need, the previous tick's `held`, so
  the edges are computed once, on the client, and travel in the sample. It is called once
  per simulation tick, never once per frame: when the timer root runs several catch-up
  ticks from one frame's raw sample, the second and later samples carry no edges, so a
  press fires once on the client as it does on the server. A fresh quantiser's previous
  sample is idle, so every action held at the first tick is also pressed in it.
- `Quantiser::next(raw)` produces, in integers: `movement` as the raw vector scaled down to
  the unit disc if longer (a NaN component counts as zero), each component times
  `AXIS_SCALE`, rounded to nearest with ties away from zero, saturating to `i16`; `yaw` as
  the raw yaw reduced into `[0, TAU)`, divided by `TAU`, times `UNITS_PER_TURN`, rounded,
  modulo `UNITS_PER_TURN` (a non-finite yaw gives zero); `pitch` as the raw pitch clamped
  to `[-PI/2, PI/2]`, divided by `TAU`, times `UNITS_PER_TURN`, rounded, clamped to
  `Pitch::LIMIT` (a NaN pitch gives zero); `held` as given; `pressed` as
  `held.without(previous_held)`; `released` as `previous_held.without(held)`; and then
  `previous_held` becomes `held`. The result always passes `PlayerInput::validate`.
- Quantisation happens before the sample enters any frame, so the client simulates the
  quantised value, the value it sends, and never the raw one.

**The latest sample** (architecture section 5.3)

- `LatestSample` is one slot the input task publishes a `RawSample` into every frame and
  the late latch reads immediately before submit. `publish` and `read` never lock and
  never block a reader. The slot is a sequence-locked copy of the sample's five 32-bit
  words, as `net-api`'s interpolation buffer is: `publish` takes the sequence by
  compare-exchange from even to odd, writes the words, and releases it to the next even
  value, so a publisher may wait for another publisher but never for a reader, and two
  concurrent publishers cannot interleave their words; `read` loads the sequence with
  `Acquire`, the words `Relaxed`, then `fence(Acquire)` and reloads the sequence, retrying
  while the sequence was odd or changed, so it returns a sample some `publish` wrote and
  never a mix of two. `read` returns `RawSample::IDLE` before the first `publish`. With
  several publishers the last to finish wins. It is `Send + Sync` and `const`-constructible,
  so it can be a static or live in a presentation resource.
- It holds the raw sample, not the quantised one: the latch wants the freshest rotation,
  sampled after the tick's sample was taken, and the quantisation step (1/65536 turn) is
  far below what a frame shows. The simulation never reads it.

**Sources and mapping** (milestone 5)

- `Source::poll(&mut self, raw: &mut RawSample)` folds a device's state for the frame into
  the raw sample through `hold`, `offer_movement` and `turn` only. A source reads no clock
  and produces nothing the simulation sees except through `Quantiser::next`. No source
  exists yet.
- `mapping` is the named place of the action map, from device controls to `Action` bits
  and movement axes, loaded as an asset. It is empty: the generic bits above
  `Action::NAMED` are what a map assigns.

## Not in this crate yet

- The sources and the action map themselves, and anything that touches a device or a
  clock: milestone 5. The input task that polls sources, quantises, fills the history,
  publishes the latest sample and builds the frame is `app`'s; its shape is in
  `tests/consumers.rs`.
- Mouse-look sensitivity, dead zones, inversion and other per-player settings: the
  sources' and the map's, milestone 5.

## Proof for milestone 1

Nothing runs yet. The implementation lane replaces every `todo!()` and proves:

| What | How |
|---|---|
| Quantisation | `next` of a sweep of raw samples passes `validate`; a vector of length two quantises to length one; yaw `TAU + 0.1` and `0.1` quantise alike; pitch beyond a quarter turn clamps; `pressed` and `released` are the edge sets against the previous call; a second call on the same raw sample carries no edges; NaN and infinite inputs give the stated zeros |
| The raw sample | `begin_frame` keeps the angles and clears the rest; `hold` unions; `offer_movement` keeps the larger; `turn` wraps the yaw into `[-PI, PI)` across many turns and clamps the pitch; the wrapped yaw quantises to the same units as the unwrapped one would within one unit |
| The latest sample | `read` after `publish` returns the published sample; under two publisher threads and reader threads, every read is a sample that was published, never a mix (the reviewer's stress test with a checksum word, and the loom model of two publishers and a reader at preemption bound 2) |
