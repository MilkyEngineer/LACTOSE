# lactose-anim

Animation inside the determinism boundary (architecture section 17): the state machine and
phase a character's clips play by, the events and root motion a clip carries, and the
deterministic sim skeleton that gives the simulation its sockets, hands, feet, head and
ragdoll bodies. Full pose evaluation and skinning are presentation and renderer work and are
not here.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | inside |
| Unsafe | none, `#![forbid(unsafe_code)]` |

**Status: facade.** Every public signature is final and every deciding body is `todo!()`;
the registration functions, the integer and compositional helpers are real. Every sentence
under "Behaviour the signatures cannot express" is one a dependent crate may rely on and
the implementation must meet. Architecture sections 4, 5.3, 6, 6.1, 8.2, 9.4, 13, 14, 15.1,
16, 17 and 22 are the design; where this file and the architecture disagree, the
architecture wins and this file is wrong. Decision D128 records the choices those sections
left open. `crates/ecs/README.md`, `crates/sim/README.md`, `crates/physics-api/README.md`,
`crates/predict/README.md` and `crates/asset-api/README.md` are the contracts this crate
builds on and never restates: what they say about commands, events, transient columns,
stages and sets, bodies and the write-back, the prediction world's filter and build check,
and pinned asset tables holds here unchanged.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `game-sim`, `predict`, `net`, `presentation`,
`present-api`, `cook` and `app` are written against this surface. `tests/consumers.rs` is
a compile-only sketch of what each of them writes, and its one test builds the schedule of
every world kind: a change that breaks it is a signature change to list.

## Surface

| Item | Purpose |
|---|---|
| `AnimRig` | Which state machine and sim skeleton a character animates with: two `AssetRef`s, persistent, replicated, extracted |
| `AnimState`, `Blend`, `StateIndex` | Where the state machine is: the state, its phase before and after this tick, the outgoing state and phase of a blend, ticks in state, entered and finished; `phase_crossed` is section 6.1's test; persistent, replicated, extracted, all integers |
| `AnimParams`, `ParamIndex`, `PARAM_SLOTS` | What the game tells the machine: eight scalar values, level flags, one-tick triggers; persistent, replicated |
| `RootMotion` | What the clip moved the root by this tick, in character space, for the movement system to apply; transient |
| `SimPose`, `BonePose`, `BoneIndex`, `Outgoing`, `MAX_SIM_BONES` | The evaluated sim skeleton, root-local, at most sixteen bones; `world` and `socket` place a bone by a root `Transform`; `evaluate` is the per-character work; transient, extracted |
| `Phase` | A clip fraction in units of `1 / 2^32`: wrapping for a loop, saturating at `END` for a one-shot; `rate`, `advanced`, `passed` |
| `AnimEvent`, `AnimEventKey` | An authored clip event the phase passed this tick, emitted to the character's own key |
| `Bones`, `BoneLink`, `BoneMoved` | A character's bone rows (kinematic bodies swept to the pose, or limp and driving it), the row's link back, and the event a limp bone reports its stepped transform with |
| `Clip`, `RootSample`, `ClipEvent`, `ClipLoader` | A clip cooked for one skeleton: parent-local samples per bone, a root track, events; kind `anim::clip` |
| `StateMachine`, `State`, `Transition`, `Condition`, `StateKey`, `StateMachineLoader` | States playing clips and the ordered transitions between them; kind `anim::state_machine` |
| `Skeleton`, `Bone`, `BoneRole`, `BoneBody`, `BoneJoint`, `Socket`, `BoneKey`, `SkeletonLoader` | The sim bones (parents first), their roles, rest poses and ragdoll bodies, and the sockets hung off them; kind `anim::skeleton` |
| `register_types`, `register_systems`, `sets::{DRIVE, ADVANCE, EVALUATE, REPORT, FIT, BONES}` | What `app` adds to a `SimulationBuilder`, and the six sets a game orders itself by |

Hidden plumbing: none. The five systems live in a private module and `sets` is the only
way to order against them; `SimPose::set_bone` is crate-private, the ragdoll fit's write.
What the implementation may reshape without a signature change: the cooked byte form of
the three asset kinds (named only, below), and the bodies of `Phase::rate`,
`Phase::passed`, `AnimState::phase_crossed`, `Clip::sample`, `Clip::root_motion`,
`SimPose::evaluate`, `SimPose::rest`, `Bones::spawn` and `Bones::set_limp`, each of whose
contract is its doc comment and the sections below.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| Game crates (`game-sim`, milestone 5; the milestone 1 spike's predicted character) | A state machine and phase advanced per tick from simulation inputs | `AnimParams` written in `sets::DRIVE` (`Mut`, `set_if_neq`); `anim::advance` in `sets::ADVANCE` reads them and writes `AnimState`; the machine is the `AnimRig`'s asset |
| | Phase crossings and authored events as deterministic events | `AnimState::phase_crossed(mark)` in any system after `sets::ADVANCE`; `Events<AnimEvent>` with `for_target(*key)` ("Phase and crossings" below) |
| | Root motion as a displacement the movement system applies | `RootMotion` after `sets::ADVANCE`, `RootMotion::apply(&Transform)` for the kinematic sweep target or the velocity ("Root motion" below); this crate never moves the root |
| | Hand, foot, head and socket transforms for hit volumes and placement | `SimPose::world(&root, bone)` and `SimPose::socket(..)` with `Skeleton::bone_of(BoneRole)` and `Skeleton::socket(BoneKey)`, after `sets::EVALUATE` in `Stage::Simulation` or after `sets::BONES` in `Stage::Post` with the stepped root |
| | Everything in fixed point or `lactose_math` scalars, keyed by `StableKey`, under section 8.2 | `Phase` and every `AnimState` field are integers; values are `Scalar`; keys are origin keys ("Keys and units" below) |
| | A ragdoll, and bodies to hit | `Bones::spawn` and `Bones::set_limp` from the character's own kernel ("Bones and ragdolls" below) |
| `predict`, `net-api` | Which components are persistent and replicated, and that every simulation-read field compares at full precision | `AnimRig`, `AnimState`, `AnimParams`, `Bones` and `BoneLink` are `persistent, replicated`; `RootMotion` and `SimPose` are `transient`; the replicated integers compare by bytes, the parameter scalars by canonical bits ("Prediction and replication" below); nothing holds state outside rows and no channel exists |
| `physics-api` | The ragdoll bodies as bodies or colliders, and the direction | Bone rows with `Transform`, `RigidBody` and `Collider`, joints as `Joint` rows: the pose drives kinematic bodies through `Commands::set` of `Transform` in `Stage::Post`; limp, dynamic bodies drive the pose through `BoneMoved` events ("Bones and ragdolls" below) |
| Presentation (milestones 2 and 3) | The sim skeleton's evaluated transforms as rows the presentation schedule reads | `SimPose` (`extracted`, root-local) beside the extracted root `Transform`; `AnimRig` and `AnimState` extracted for the render pose ("Presentation" below) |
| Assets (`asset`, `cook`) | The authored clip, machine and skeleton data as `asset-api` kinds, and the cooked form named | `Clip`, `StateMachine`, `Skeleton` with `KIND` and `FORMAT_VERSION`; `ClipLoader`, `StateMachineLoader`, `SkeletonLoader` through `ErasedLoader::new`; the byte form is named only ("Assets" below) |
| `sim`, SOUR | Systems placed in the stages, readers after them, every type registered | `register_systems` puts `ADVANCE` and `EVALUATE` in `Stage::Simulation` and `REPORT`, `FIT`, `BONES` in `Stage::Post`; `register_types` ("The tick" below) |
| `app` | Registration beside every other crate's, in every world kind | `register_types` and `register_systems`; no setup function, no resource |

## The registration a game crate writes

```rust
use lactose_anim::{AnimParams, AnimState, RootMotion, SimPose, sets};
use lactose_ecs::{Mut, SystemSet, system};
use lactose_physics_api::Transform;
use lactose_sim::{Registration, Stage};

/// Readers of the state, the root motion and the events, in `Stage::Simulation`.
pub static MOVEMENT: SystemSet = SystemSet::new("game_sim::movement").after(&[&sets::EVALUATE]);
/// Readers of the stepped pose, in `Stage::Post`.
pub static HITS: SystemSet = SystemSet::new("game_sim::hits").after(&[&sets::BONES]);

/// The game's parameter writer goes in this crate's `DRIVE` set, before the advance.
#[system(name = "game_sim::drive_anim", worlds = [server, prediction], in_set = sets::DRIVE)]
fn drive_anim(params: Mut<AnimParams> /* , the game's inputs */) { /* params.set_if_neq(..) */ }

#[system(name = "game_sim::apply_root_motion", worlds = [server, prediction], in_set = MOVEMENT)]
fn apply_root_motion(motion: &RootMotion, pos: Mut<Transform>) {
    let next = motion.apply(&pos);
    pos.set_if_neq(next);
}

pub fn register_systems(registration: &mut Registration) {
    registration.stage(Stage::Simulation).sets(&[&MOVEMENT]).add(&DRIVE_ANIM).add(&APPLY_ROOT_MOTION);
    registration.stage(Stage::Post).sets(&[&HITS]);
}
```

`app` calls `lactose_anim::register_types` on the `TypeTable` beside
`lactose_physics_api::register_types` (the bone rows' components are physics-api's) and
adds `lactose_anim::register_systems` with `SimulationBuilder::systems`. There is no setup
function: this crate holds no resource. A character is spawned with the five components
(`AnimRig`, `AnimState::UNSET`, `AnimParams::ZERO`, `RootMotion::ZERO`, `SimPose::EMPTY`)
beside its `Transform`; `tests/consumers.rs` is the whole of it.

## Behaviour the signatures cannot express

**Keys and units** (architecture sections 7 and 8.2)

- Every entity named here is an origin `StableKey`: `AnimEvent::target` and
  `BoneMoved::owner` are what `StableKeyOf` yields, and `BoneLink::owner` and the keys
  `Bones` holds are `EntityRef`s written through commands, so storage holds origin keys and
  a bind changes no bytes. A bone, a state or a parameter is named by its index into the
  rig's assets, never by a key; the keys (`BoneKey`, `StateKey`, `AnimEventKey`, each the
  xxh64 of an authored name with seed zero) live in the assets, where a game looks an index
  up once.
- A phase is a `u32` fraction of its clip in units of `1 / 2^32`; every other quantity is a
  `Scalar`, `Vec3` or `Quat` of `lactose_math`. No float of the platform library is
  computed, no clock is read, no map is iterated, and nothing but rows carries state across
  a tick: the assets are immutable, pinned by the region, and read through `Ctx::assets`.
- Character space is the simulation's: right-handed, `+Y` up, `-Z` forward at yaw zero, a
  positive yaw turning left (architecture section 5.3). A clip's samples, its root track
  and `RootMotion` are in it; a `SimPose` is in it relative to the root; the cooker turns
  glTF's `+Z` forward half a turn about `+Y` as it does for meshes.
- Every component is its scalars then a tail that is a multiple of eight bytes, sizes
  asserted in `lib.rs` for both scalar widths; a field added later takes the next free ID
  and never renumbers another.

**The tick** (architecture sections 5.2, 6.1 and 16; `crates/sim/README.md`, the stages)

Five systems, every one `worlds = [server, prediction]`, in six sets. In `Stage::Simulation`,
in this order: the game's parameter writers in `sets::DRIVE` (this crate places nothing
there); `anim::advance` in `sets::ADVANCE` (`Ctx`, `EventWriter<AnimEvent>`, `&AnimRig`,
`Mut<AnimParams>`, `Mut<AnimState>`, `Mut<RootMotion>`); `anim::evaluate` in
`sets::EVALUATE` (`Ctx`, `&AnimRig`, `&AnimState`, `Mut<SimPose>`). In `Stage::Post`, after
the physics write-back, which is exclusive and needs no edge: `anim::report_bones` in
`sets::REPORT` (`EventWriter<BoneMoved>`, `&BoneLink`, `&Transform`); `anim::fit_ragdoll`
in `sets::FIT` (`Events<BoneMoved>`, `StableKeyOf`, `&Bones`, `&Transform`,
`Mut<SimPose>`); `anim::drive_bones` in `sets::BONES` (`Commands`, `&Bones`, `&SimPose`,
`&Transform`, `writes = [Transform]`).

- The rule a game follows, which the schedule enforces by refusing the build on an
  unordered conflict: a `Stage::Simulation` system that touches `AnimParams`, `AnimState`,
  `RootMotion` or `SimPose` is in `sets::DRIVE` (a parameter writer) or in a set declared
  `.after(&[&sets::ADVANCE])` or `.after(&[&sets::EVALUATE])`; a `Stage::Post` system that
  touches `Transform`, `SimPose` or `Bones` is in a set declared `.after(&[&sets::BONES])`.
  `sets::DRIVE` is the one set another crate's systems are placed in; `app` may instead
  order the game's own set before `sets::ADVANCE` with `Registration::order`.
- `RootMotion` and `SimPose` are transient: zeroed at the input sync point, written by
  `anim::advance` and `anim::evaluate` through `Mut` every tick, and the schedule refuses a
  reader not ordered after one of those writers (`TransientBeforeWriter`), so no system
  reads a zeroed pose by mistake. `anim::fit_ragdoll` is a second declared writer of
  `SimPose`, in `Stage::Post`, so a `Stage::Post` reader after `sets::BONES` is after a
  writer in its own stage as well.
- A `Client` world runs none of the five (it runs no simulation system); `register_systems`
  there declares the sets alone, so one registration function serves every kind. In the
  prediction world each runs over the claimed rows: the local player's character and what
  it has claimed, bone rows included, since a bone spawned from a claimed kernel inherits
  the claim.
- `anim::advance` stamps `AnimState` every tick it advances (the phase changes) and
  `AnimParams` only when it clears a trigger, through `set_if_neq`; `anim::evaluate` and
  `anim::fit_ragdoll` stamp `SimPose` every tick they write it. A game writer of
  `AnimParams` uses `set_if_neq` so an unchanged tick replicates nothing.

**The state machine** (architecture sections 17 and 18)

`anim::advance` does the following for one character, in this order, and nothing else:

1. Looks the rig's `StateMachine` up in the region's pinned table (`Ctx::assets`), and
   then the current state's `Clip` (and the outgoing state's, during a blend). If the table
   has not entered or any of them is missing or of the wrong kind, it leaves `AnimState`
   and `AnimParams` as they are, writes `RootMotion::ZERO`, emits nothing and logs at
   target `anim` ("What is logged" below). The tick runs.
2. If the state is `UNSET` or its index is out of the machine's range, enters
   `StateMachine::initial` at `Phase::ZERO`: entered, ticks in state zero (one after step
   4), no blend, not finished. No transition is tried on that tick.
3. Otherwise tries the transitions in the machine's order, with `AnimState` and
   `AnimParams` as they are before this tick's advance (the phase of the previous tick, the
   triggers the game set since). A transition applies when `from` is the current state, or
   `from` is `None` and `to` is not the current state; it is taken when every one of its
   conditions holds (`Condition` names each); the first taken ends the search, so at most
   one transition is taken per tick. Taking it: the state becomes `to`, entered, with its
   phase at `entry_phase` or, with `keep_phase`, at the phase it had; `previous_phase` is
   set to that entry phase; finished is cleared; with `blend_ticks` above zero the outgoing
   state and its phase are kept as the blend, with the blend's length, else the blend is
   none. Not taking any: entered is cleared and `previous_phase` becomes the phase.
4. `ticks_in_state` is incremented, saturating.
5. The phase advances by `Phase::rate(clip.duration_ticks, state.speed * value)` where
   `value` is `AnimParams::values[speed_param]` or one, wrapping for a looping clip and
   saturating at `END` otherwise (`Phase::advanced`); a one-shot that holds `END` after the
   advance is finished, and stays so until the state changes. During a blend the outgoing
   state's phase advances likewise by its own rate, and the blend ends, its outgoing state
   set equal to the state, on the tick `ticks_in_state` reaches the blend's length.
6. `RootMotion` is written as `clip.root_motion(previous_phase, phase)`; during a blend it
   is blended with the outgoing clip's root motion over its own advance, translation and
   yaw each interpolated by the outgoing weight.
7. One `AnimEvent` is emitted for every `ClipEvent` of the current clip the advance passed
   ("Phase and crossings" below), in ascending phase order, to the character's own key;
   none for the outgoing clip.
8. If any trigger was set, `AnimParams` is rewritten with `triggers` zero through
   `set_if_neq`; values and flags are never written by this crate.

So a trigger set by the game in `sets::DRIVE` fires a transition in the same tick's
advance and is gone by the next; a `Finished` transition is taken the tick after the
one-shot reached its end, which is one tick of holding the last frame; and a state the
game never leaves loops forever.

**Phase and crossings** (architecture section 6.1)

- `Phase::rate(duration_ticks, speed)` is `2^32 * speed / duration_ticks` through
  `lactose_math`, truncated toward zero, clamped to `[0, u32::MAX]`, zero for a speed that
  is not finite, negative or zero or for a zero duration. The truncation is part of the
  contract: at speed one a clip of `d` ticks wraps one tick late once every `2^32 / d`
  cycles, which is never within a session, and a change of rounding would desynchronise the
  phases of two builds.
- `Phase::passed(from, to, mark)` is `mark` in `(from, to]` when `to >= from`, and in
  `(from, END] ∪ [ZERO, to]` when `to < from` (a loop that wrapped; an advance wraps at most
  once). `AnimState::phase_crossed(mark)` is `passed(previous_phase, phase, mark)`, except on
  the tick the state was entered, when a mark equal to the entry phase also counts (a
  footstep authored at phase zero plays when the cycle starts). Each pass over a mark is
  therefore reported exactly once, by the system that reads the state this tick, whatever
  the rate, and a tick with no advance (rate zero, a missing asset) reports nothing.
- `ClipEvent`s are passed by the same rule, so an authored event and a game's own mark at
  the same phase fire on the same tick.
- A phase is a fraction, not a frame, so a re-cooked clip with another sample count or
  another `duration_ticks` entering at the input sync point (architecture section 14)
  changes what a stored phase samples and how fast it moves, never whether it is valid, and
  a saved or replicated `AnimState` is valid against any version of its machine's assets
  whose state count covers its index.

**Root motion** (architecture section 17)

- A clip's root track is the root's pose per frame in character space at the clip's start,
  `frames + 1` entries for a loop (the last where the cycle ends: sample zero displaced by
  the cycle's total motion) and `frames` for a one-shot, or empty. `Clip::root_motion(from,
  to)` samples the track at both phases by the sampling rule of `Clip::sample` and
  subtracts, adding the cycle's total motion once when `to < from`; so a looping walk's
  motion sums to its total over one cycle whatever the rate, and a one-shot's to its
  track's end.
- The displacement is the character's: `RootMotion::apply(&root)` turns the translation
  into world space by the root's rotation and adds it, and applies the yaw about the
  world's `+Y` before the root's rotation. A kinematic character writes that transform to
  its `Transform` row (the physics sync sweeps it there during the step); a dynamic one
  divides the translation by `TickDelta` into its `Velocity`; a game that steers by input
  ignores or scales it. This crate moves no root and declares no write of `Transform` in
  `Stage::Simulation`.

**The sim skeleton** (architecture sections 13 and 17)

- `SimPose::evaluate(skeleton, clip, phase, outgoing)` is, per bone in skeleton order:
  `clip.sample(phase, bone)`, blended toward `outgoing.clip.sample(outgoing.phase, bone)`
  by `outgoing.weight` when a blend is given (`BonePose::blend`: translation lerp,
  rotation nlerp along the shorter arc, never a slerp), composed onto the parent's
  already-composed pose (`BonePose::compose`), bone zero onto nothing. The result is
  root-local: a function of the state and the assets alone, valid for the whole tick, and
  a reader places a bone by the root `Transform` of the moment with `SimPose::world`, the
  stepped root in `Stage::Post`. `Clip::sample` reads the two frames the phase lies between
  (`phase * intervals / 2^32` in integer arithmetic, `intervals` the frame count for a
  loop and one less for a one-shot) and blends them by the fraction, one `lactose_math`
  division; `END` on a one-shot samples the last frame exactly. Allocates nothing: the
  pose is a fixed array, the clip is read in place.
- A clip is evaluated only against the skeleton it was cooked for (`Clip::skeleton`), with
  the skeleton's bone count; any other clip gives `SimPose::EMPTY`, as does a state that is
  `UNSET`. A character without `SimPose` is not evaluated at all: a non-player with a state
  machine and no skeleton spawns without the component and pays nothing.
- A socket is an offset on a bone, not a bone: `SimPose::socket` composes it on demand, so
  sockets are free at evaluation and do not count against `MAX_SIM_BONES`. Hands, feet
  and the head are bones found by `Skeleton::bone_of(BoneRole)`.
- The per-character work is bounded by `MAX_SIM_BONES`: sixteen bones, each two sample
  reads, a lerp and an nlerp, the same again and a blend during a cross-fade, and one
  compose; then a 904-byte row write at eight-byte scalars. Two binary searches in the
  pinned table find the assets. That is what "Proof for milestone 1" measures against the
  0.5 ms budget for 200 characters.

**Bones and ragdolls** (architecture sections 16 and 17; `crates/physics-api/README.md`,
"Bodies" and "Joints")

- The direction: while a character is animated, the pose drives the bodies; when it is
  limp, the bodies drive the pose. Chosen because physics cannot be told a pose except
  through rows (a kinematic body sweeps to its `Transform` row during the step) and a kernel
  reads its own row only, so the only deterministic cross-row paths are commands and
  events: the character writes its bones' transforms through `Commands::set`, and a bone
  reports its stepped transform through an event. The alternative, the bones evaluating
  themselves, would need every bone kernel to read its owner's state and clips, which no
  kernel can.
- A bone row is spawned by `Bones::spawn` from the character's own kernel: for every
  skeleton bone with a `BoneBody`, a row with `Transform` (the bone placed by the root at
  the pose given), the body's `RigidBody` made kinematic, its `Collider`, `OwnedBy` the
  character (so it migrates and despawns with it) and a `BoneLink` back; for every such
  bone whose `BoneJoint` has an ancestor with a body, a `Joint` row owned by the bone row;
  and `Bones` written onto the character naming the rows, placeholders resolved at apply.
  The bone rows exist from the next sync point on and are bodies from the next physics
  sync point on.
- `anim::drive_bones`, in `Stage::Post` after the write-back, records for an animated
  character `Commands::set` of each bone row's `Transform` at `SimPose::world` of the
  character's stepped root. The commands apply at the run's closing sync point and the
  next tick's physics sync sweeps the kinematic bodies there during its step: the bodies
  are where the pose was at the end of the previous tick, one step behind the root, which
  is the same lag a write in `Stage::Simulation` would have (the root moves during the
  step either way) without a `Transform` writer in the stage the game moves the root in.
  A kernel reading hit volumes from `SimPose::world` sees no lag; the bodies are for
  contacts, queries and being hit.
- `Bones::set_limp(.., true)` rewrites `Bones`, every bone row's `RigidBody` as the
  skeleton's body made dynamic and every `BoneLink` as limp, all through commands; the
  physics sync sees the body change at the next physics sync point and the joints hold
  the ragdoll together. From then on `anim::drive_bones` does nothing for the character,
  `anim::report_bones` emits a `BoneMoved` from every limp bone row's stepped `Transform`
  in `Stage::Post`, and `anim::fit_ragdoll` writes each reported bone into the character's
  `SimPose` in root-local space against the character's own stepped `Transform`, leaving a
  bone no row reported at the evaluation's pose. What the character's root does when it is
  limp (following the hips, dropping its capsule) is the game's: a `Commands::set` of its
  own `Transform` from the hips bone's `BoneMoved`, which the game may read too.
  `set_limp(.., false)` reverses all of it; the pose then sweeps the bodies back from
  wherever they lie, with no blend.
- An animated character's bone rows emit no event (`BoneLink::limp` is false), so a
  character that never ragdolls costs its bone commands alone; a character without
  `Bones` costs nothing in `Stage::Post`.
- The cost of bodies: each animated character with bones records one command per bone
  body per tick, which the closing sync point applies as a row write and the next physics
  sync pushes as a kinematic sweep. On today's ECS a `Commands::set` row write is about
  0.8 µs (D119's 1.6 ms for 2,000 rows), so 200 characters with eleven bodies each are
  about 2 ms of a 4 ms tick before the sync's own scan and the solver's sweeps. The levers
  are the number of bones with a `BoneBody` (hit volumes need few; a skeleton may give
  bodies to the head and hands alone) and whether a character has `Bones` at all; the sim
  pose itself costs none of it. `tests/cost.rs` records the bone commands beside the
  evaluation.
- Limited joints wait on the backend's verification item, as `physics-api` says; a
  `BoneJoint` with `limits` is authored data that the backend may refuse until then.

**Assets** (architecture section 14; `crates/asset-api/README.md`)

- Three simulation asset kinds: `anim::clip` (`Clip`), `anim::state_machine`
  (`StateMachine`) and `anim::skeleton` (`Skeleton`), each `FORMAT_VERSION` one. Their
  loaders are pure decoders of their bytes, bit-identical on every host, which is the
  promise `SimulationAsset` makes, and each validates what it decoded with the type's
  `validate` (the doc comments list the checks) and returns `InvalidData` otherwise.
  A clip's one dependency is its skeleton; a machine's are its states' clips; a skeleton
  has none.
- The cooked byte form is named and not fixed: the implementation lane fixes it together
  with the cooker (RICOTTA) and records it here. Nothing a kernel reads depends on it: the
  runtime form above is the contract. A clip is cooked for one skeleton, in that skeleton's
  bone order and count, with its samples already in character space convention and as the
  simulation scalar; a change of scalar width is a cook change, as it is for every
  simulation asset.
- A rig's machine and skeleton, and the machine's clips, are read from the region's
  exact-version table, so they are in the region's asset set (architecture section 14; the
  open item on spawned entities' assets in `docs/verification.md` applies to a character
  spawned at run time). Until the table has entered, or while an asset is missing, the
  character holds: step 1 of the advance.
- A hot reload of any of the three enters as a new table at the input sync point, where a
  stored `AnimState` stays valid as "Phase and crossings" says.

**Prediction and replication** (architecture sections 9.4 and 15.1;
`crates/predict/README.md`)

- Policies: `AnimRig` `persistent, replicated, extracted`; `AnimState` `persistent,
  replicated, extracted`; `AnimParams`, `Bones` and `BoneLink` `persistent, replicated`;
  `RootMotion` `transient`; `SimPose` `transient, extracted`. Every component a
  `[server, prediction]` system of this crate names therefore passes
  `lactose_predict::check_prediction_reads`, which `tests/consumers.rs` runs over them.
- What the server sends for a predicted character compares at full precision and by bytes
  on every field but the parameter values, which compare by canonical bits as every scalar
  does: `AnimState` is integers, so a predicted state and phase agree with the server's or
  are corrected to it, never absorbed by a tolerance. The animation phase is persistent
  state (section 15.1) and saved with the row.
- The transient components are rebuilt every tick from the persistent ones and the assets,
  so a rollback restores `AnimState` and `AnimParams` and re-simulates `RootMotion` and
  `SimPose`; nothing of this crate is in a snapshot but rows, and no channel exists (D126).
- The RNG is not used: nothing here draws.
- The milestone 1 netcode spike's predicted character carries the five components; its
  root motion reaches the physics sync through the game's `Transform` write, so a
  mispredicted phase is corrected as a mismatch of `AnimState`, and the step that follows
  sweeps the capsule from the corrected pose.

**Presentation** (architecture sections 5.3 and 13)

- Extract copies `AnimRig`, `AnimState` and `SimPose` beside the root `Transform`.
  Presentation evaluates the render pose from the state, its phase (`Phase::fraction`) and
  its blend (`AnimState::blend`) against the render clips, which is its full pose
  evaluation; it places the sim bones and sockets by the root it interpolates through
  `SimPose::world` and `SimPose::socket`, so a weapon mesh sits where the simulation says
  the grip is and the render skeleton can be fitted to the sim feet and hands. A root-local
  pose interpolates correctly between two snapshots of a moving character, which a
  world-space pose would not.
- Remote characters are not in the snapshot (section 5.3): their `AnimState` arrives
  through replication and presentation samples it from the interpolation buffer, whose
  interpolation of an animation phase is `lactose-net-host`'s deferred item. Nothing here
  changes for it.

**What is logged**

- At target `anim`: a rig whose table has not entered or whose assets are missing or of the
  wrong kind (at trace, since it repeats every tick), a transition the loader should have
  refused (at error, once per advance that meets it), a `Bones::spawn` on a character that
  already has bones (at warn). The facade carries no `tracing` dependency: the allowlist
  entry for it is proposed with D128.

## Not in this crate yet

- Blend trees and one-dimensional blend spaces (walk to run by speed): a state plays one
  clip; `State::speed_param` scales its rate, and a blend space is a `State` variant when a
  game needs one.
- Layers, masks and additive clips (an upper-body swing over a walk): one state machine per
  character; a second machine on a second rig component when needed.
- Inverse kinematics for foot placement and hand grabs (architecture section 17 names them
  among what needs bone transforms): the game's, from `SimPose` and physics queries, at
  milestone 5; a sim-side IK pass that writes `SimPose` after `sets::EVALUATE` is where it
  would go.
- Getting up: a limp ragdoll blended back into a clip. `set_limp(.., false)` cuts.
- The interpolation of a remote character's phase (`lactose-net-host`, milestone 4) and the
  visual blend of a correction (presentation, milestone 2).
- The cooked byte form of the three kinds, the cooker's processors and the format's test:
  the implementation lane with RICOTTA.
- A SOUR test and the `[harness]` entry: the implementation's, proposed with D128.
- Clip compression and streaming of clip data: milestone 3's budgets.

## Proof for milestone 1

Today: `tests/consumers.rs`, compile-only, whose one test builds the schedule of a server,
a prediction and a client world with this crate's systems beside a game sketch (parameters
driven from a sketch input, a footstep from `phase_crossed(Phase::HALF)` and an authored
event, root motion applied by a movement sketch, a hit test and a ragdoll from the stepped
pose, presentation reading the extracted rows), the prediction world under
`check_prediction_reads`. The implementation lane adds the tests below, each a nextest
test asserting on counts, orders, bytes and hashes and never on timing, the SOUR test
recording under every mode.

| What | How |
|---|---|
| The state machine | A machine with a loop, a one-shot, a `keep_phase` transition and a trigger: the transition order, one per tick, the entry tick's fields, a trigger consumed, `Finished` one tick after `END`, a blend's weight per tick and its end; under three layouts and one and four workers, hashes equal |
| Phase | `rate` for the duration and speed cases in the contract; `advanced` wrapping and saturating; `passed` across the wrap and at `END`; `phase_crossed` once per pass at every rate, on the entry tick at the entry phase, never on a held tick |
| Events | Authored events emitted in phase order once per pass, to the character's key, read by a later system; none from the outgoing clip |
| Root motion | A loop's motion over one cycle equals the track's total at every rate; a one-shot's reaches its end; a blend interpolates; `apply` against a hand-composed transform |
| Evaluation | `evaluate` equals a hand-composed pose for a two-bone chain at authored phases, with and without a blend, bit for bit; `rest`; a clip for another skeleton gives `EMPTY`; the same bits under every layout and worker count |
| Bones | `spawn` records the stated rows and the `Bones` write, placeholders resolved; `drive_bones` sweeps the kinematic bodies to the previous tick's pose, seen by the physics test backend; `set_limp` makes them dynamic, `report_bones` emits from limp rows alone, `fit_ragdoll` rewrites the pose; a character without bones costs no command |
| Prediction | The build check over this crate's systems; a predicted character rolled back and re-simulated with the ring's inputs hashes equal to the straight run; a corrected `AnimState` re-simulates from the server's phase |
| SOUR | `tests/sour.rs`: 200 characters with a sixteen-bone skeleton, a looping clip with events and root motion and a machine with triggers, 300 ticks, hashes recorded under workers, repeat, layout and chaos modes; `lactose-anim` listed under `[harness]` |
| The budget | `tests/cost.rs`, an ignored test run by hand in release on the M1 and the x86 desktop: 200 characters, each a sixteen-bone skeleton with a body on every bone and a 60-sample looping clip, half of them mid cross-fade; (a) `SimPose::evaluate` over the 200 for 1,000 ticks, median and best microseconds per tick; (b) `Simulation::step` of the world holding them with and without `anim::evaluate` registered, one and four workers, the difference per tick; reported with `uptime`'s load averages. The criterion is (b) at most 0.5 ms; (a) says how much of it is the evaluation and how much the ECS's dispatch. If (b) misses, the first levers are the row write (skip `set_if_neq`-equal bones, which a holding character has) and the sample reads (a frame index cached per state), never the bone cap |
