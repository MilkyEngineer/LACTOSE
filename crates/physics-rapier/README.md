# lactose-physics-rapier (QUARK)

The physics backend: `lactose-physics-api`'s `PhysicsBackend` over Rapier (`rapier3d-f64`
0.36.0 with `enhanced-determinism`, `parallel` and `serde-serialize`, built from the patched
copy in `vendor/`, see `vendor/README.md`).

| | |
|---|---|
| Tier | backend |
| Determinism boundary | inside |
| Unsafe | none, `#![forbid(unsafe_code)]` |

**Status: implemented.** It passes `physics-api`'s conformance run for pools of one, two and
four threads under two layouts, `physics-api`'s scenes re-run over it, and the SOUR scenes
under `[harness]`. `crates/physics-api/README.md` and the trait's documentation are the
contract; everything they say a backend does holds here and is not restated. This file states
what they leave to the backend. Decision D119 is the design.

## The one public item

`backend(pool: rayon::ThreadPool) -> Box<dyn PhysicsBackend>`. `app` builds the pool; each
of its threads enters `ThreadClass::PhysicsSolver` before the backend is returned (a refused
scheduling class is logged and the thread runs at its default priority).
Rapier's parallel work runs on that pool and nowhere else. The pool's size changes no output:
`tests/determinism.rs` and `tests/conformance.rs` compare one, two and four threads.

What the contract says is logged (refused values, unknown keys, dropped commands) goes to
target `physics`; what only this backend logs (a refused scheduling class, a joint whose
bodies are not in the solver) goes to target `quark`.

## Calls are recorded, the step applies them

Every trait call between steps changes only a bookkeeping keyed by `StableKey`. The step
first applies it to Rapier in ascending key order (removed joints, removed bodies, bodies
created or changed, joints waiting for a handle), then the step's commands in the order the
input holds them, then steps. Rapier's arenas, islands and broad-phase tree are therefore
independent of the order of the calls between two steps (`tests/determinism.rs` builds the
same scene in different orders, with noise, and compares the bytes), but they are a function
of the row history, not of the rows alone: an arena keeps its free list, and a body created
after a removal takes a freed slot, which changes the order of the solver's work. Unrelated
bodies that stay, whatever their keys, change nothing; unrelated bodies stepped once and
removed change the results of bodies created afterwards (`tests/determinism.rs`). Two
worlds holding different body sets or histories are not bit-equal for the bodies they share.

- A call with a value equal to the one recorded does nothing. Equality is bitwise on the
  canonical bits, so `0.0` and `-0.0` differ.
- A value that fails validation (a non-finite scalar, a rotation that is not unit, a
  negative mass, damping, density, friction, restitution or threshold, a non-positive shape
  extent, an unknown locked-axis bit, a zero axis on a revolute or prismatic joint, limits
  out of order, a joint between a body and itself) returns `InvalidData` from a creating call and is logged and dropped by
  a setting call; the bookkeeping is unchanged either way.
- Every honoured write and command wakes the body. A kinematic body ignores velocity
  writes; a dynamic body in a frozen region keeps the velocity it was given for when it
  thaws.
- A frozen body, and a collider without a `RigidBody`, is a fixed Rapier body at its last
  pose.
- `create_joint` needs both bodies held (`NotFound` otherwise); the joint reaches Rapier at
  the next step. Removing either body removes the joint from the backend too; the sync
  creates it again when the rows still want it.
- Body and joint handles are xxh64 of the key with a seed per kind. They are names only: no
  arena slot follows from them.

## How the rows map onto Rapier

- **Body type.** `Dynamic` is dynamic, `Kinematic` is position-based kinematic (its
  velocity is the one implied by the poses it is given), `Fixed`, a frozen body and a bare
  collider are fixed.
- **Mass.** A body with a collider takes `RigidBody::mass` as the collider's mass when it is
  positive, the collider's `density` otherwise. A body without a collider takes `mass` (one
  when it is zero) as a solid unit ball's.
- **Contact material.** `friction` and `restitution` go to Rapier as they are, combined by
  Rapier's default rule (the average). `material` and `impact_threshold` stay in the
  bookkeeping; Rapier never sees them.
- **Collision groups.** `membership` and `filter` are Rapier's interaction groups, tested
  in `And` mode: a pair interacts when each one's membership meets the other's filter.
- **Keys.** Rapier's `user_data` on every body, collider and joint is the key's code: a tag
  (network, predicted, placeholder) above the key's own bits. The codes ascend as the keys
  do and none is zero.
- **Joints.** `Fixed`, `Spherical`, `Revolute` and `Prismatic` are Rapier's joints of the same
  name, anchored at `anchor_a` and `anchor_b` on the joint's `axis`; `limited` sets the
  `limit_min`/`limit_max` limits on the free axes (all three angular axes of a spherical
  joint).

## What a step reports

- **Bodies.** Every body that was moving before the step or after it (awake and dynamic, or
  kinematic with a non-zero velocity), every body a write placed or swept, once, in key
  order. A body that came to rest in the step is reported once more, with its zero
  velocity. `teleported` is set for a body whose transform was written since the last step.
- **Contacts.** `Started` and `Stopped` go to both sides. The point is the deepest contact
  of the pair in world space and the normal points away from the target; a `Stopped` event
  carries zeros. `Impact` comes from Rapier's contact-force events (with Rapier's own
  threshold at zero) and goes to each side whose own collider has `events`, is not a
  sensor, and whose `impact_threshold` the pair's total impulse exceeds.
- **Triggers.** Sensor pairs, to both sides.
- Events are sorted by (target, other, phase, then the event's canonical bits) and kept once
  per (target, other, phase), so neither the solver's threads nor its pair order reach them.

## Queries

Over the state the last step left (Rapier's broad-phase tree is updated at the end of the
step). The tree finds candidates, the exact test decides, the answer is the least
(distance, key), and `overlap` returns keys in ascending order. Sensors are skipped unless
the filter asks for them; groups use the same `And` test as contacts; the excluded key is
compared through `user_data`.

## The snapshot bytes

`b"LPR1"`, then the postcard encoding of: the bookkeeping, gravity, the integration
parameters, the island manager, the broad phase, the narrow phase, the rigid-body set, the
collider set, the impulse-joint set, the multibody-joint set and the soft-body set, in that
order.

- **Shapes as values.** Rapier serialises a collider's shape through parry's
  `SharedShape`. The collider set is encoded with Rapier's own derive, decoded into an image
  of its layout whose shape type is a parameter, and each shape replaced by the `Shape` value
  it was built from; restore runs the exchange backwards. Postcard is not self-describing,
  so the image is correct exactly when it matches the pinned Rapier's field order and types;
  a unit test reads a collider set back byte for byte, and a Rapier bump that changes the
  layout fails it.
- **A fixed function of the value.** postcard writes integers as varints, floats as their
  eight little-endian bytes, sequences and maps in iteration order, and nothing else: no
  padding, pointers, capacities or hash seeds. Every map in the encoded state iterates in
  a stored order (`IndexMap` with a fixed hasher under `enhanced-determinism`, Rapier's
  arenas, `BTreeMap` in the bookkeeping), so equal values give equal bytes. The bookkeeping
  stores row values as their plain-data bytes and keys as their codes.
- **Equal bytes need equal histories.** Rapier's arenas keep their free lists, so two worlds
  with the same bodies reached through different removals encode differently. Within one
  run, and across a restore, the bytes are equal (`tests/snapshot.rs`,
  `tests/conformance.rs`).
- **Restore** decodes the bytes into a new world with a fresh physics pipeline, collision
  pipeline and CCD solver (Rapier marks all three `serde(skip)`), hands it the pool, and
  checks that the bookkeeping and the world name each other: every handle the bookkeeping
  holds is in the world under its key, and the world holds nothing else. Bytes that fail
  any step return `InvalidData` and leave the backend as it was.
- **What Rapier's own deserialisation does.** Every field Rapier marks `serde(skip)` comes
  back as its default: the pipelines' and solvers' workspaces, the narrow phase's and broad
  phase's per-update buffers, the previous step's contact clusters (rebuilt from the
  encoded current ones at the next update), the joint sets' per-step queues and assembly
  epochs (reset, which forces the next step to rebuild its joint assembly), the island
  manager's solve groups and sleep-scan buffers, and the rigid-body set's default fixed
  body. Restoring is exact because the backend writes to Rapier only at the start of a
  step: whenever a snapshot is taken, the last step has drained every per-step queue, and
  what is left at its default is a workspace or a cache the next step rebuilds. That
  argument is checked by tests, not proven for every Rapier path: CCD, contact clustering,
  multibody joints and soft bodies are never used by this backend and are not covered.

## Known gaps

- **The sign of zero in `min` and `max`** (architecture section 8.2): `f64::max` and the
  `wide` SIMD lanes can return zeros of different sign on x86-64 and AArch64. No patch under
  fifty lines closes it, so the cross-architecture property stays unproven until SOUR's
  architecture mode runs on CI (`docs/verification.md`).
- **Snapshot cost** (`tests/cost.rs` with `--release`, printed, not asserted; Apple M1,
  load average about 5). Fifty dynamic bodies beside fixed colliders:

  | Fixed colliders | Bytes | Write | Restore |
  |---|---|---|---|
  | 0 | 64,860 | 32 µs | 45 µs |
  | 25 | 95,597 | 47 µs | 66 µs |
  | 50 | 125,972 | 61 µs | 83 µs |
  | 75 | 156,347 | 76 µs | 100 µs |
  | 100 | 187,846 | 92 µs | 127 µs |
  | 1,000 | 1,299,298 | 654 µs | 1,225 µs |

  Restore reaches the 100 µs budget at about 75 fixed colliders beside 50 bodies; write stays
  under it to about 100. Each fixed collider adds about 1.2 kB, 0.6 µs to write and 1.0 to
  1.2 µs to restore. Of its bytes about 700 are the fixed Rapier body a collider without a
  `RigidBody` is attached to; attaching every such collider to one shared fixed body would
  save them. Rapier's own encoding of its sets is most of the bytes.
- A snapshot that decodes and passes the check but whose Rapier state is otherwise
  inconsistent (bytes not written by this backend) can make a later Rapier call panic.
- `MultibodyJointSet::insert` and `insert_kinematic`, and `Multibody::inverse_kinematics` and
  its two delta variants, are banned in every boundary `clippy.toml`: a multibody's mass
  assembly above five degrees of freedom and inverse kinematics reach `matrixmultiply`, whose
  kernels are chosen by CPU feature. The backend uses impulse joints only.
