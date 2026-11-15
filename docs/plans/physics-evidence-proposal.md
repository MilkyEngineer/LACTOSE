# Milestone 1: the physics evidence nightly 43703297881 left open

Status: proposal, written at main `b2e2a41`, architecture revision 3.3, and revised after its
first review. Nothing of section 5 is applied until the owner answers section 4. This
document changes no code.

Nightly 43703297881 at `c50b688` found the three platforms' SOUR hash files byte-identical,
and `b2e2a41` closed criteria 2, 3 and 7 on it. It left three questions for the planner.
Section 1 settles the row "The further gaps of the `lactose-physics-api` (D119) row", site by
site. Section 2 settles the deserialisation claims of the row "The physics snapshot contract
rests on the backend deserialising Rapier's sets". Section 3 decides whether
`PhysicsSnapshot::hash` joins the world hash. Section 4 is the one question for the owner.
Section 5 is the text for the coordinator: the decision entry D145, one architecture
amendment, the plan amendments and the verification rows. Section 6 says what this document
could not verify.

In brief:

- The recorded scenes reach one of the listed sites, the broad phase's `log2`, and its value
  cannot differ between platforms in what it decides. Every other site is unreachable by what
  the backend builds, or reached once with its result discarded. No lane is owed. The eigen
  decomposition is re-gated to milestone 3, where a cooked collision mesh first keeps its
  result.
- The deserialisation claims hold as built and are held by tests on main. No path a milestone
  1 binary runs rests on them. The row closes on the coordinator's pass.
- `PhysicsSnapshot::hash` does not join the world hash, in milestone 1 or later. It is
  recorded beside it, per tick, by one small lane in tests alone.

Conventions:

- A trace figure is this document's: a count of calls into the platform maths library, taken
  on aarch64-apple-darwin at `b2e2a41` by the probe of the appendix. A count is exact for the
  run it was taken in.
- A probe figure is a time this document measured in a scratch copy of the tree at `b2e2a41`,
  in release, on the Apple M1 while other agents built. It carries its load averages. It
  steers and proves nothing (architecture section 20).
- A review figure is the first review's of this document. Its probes extend this document's,
  ran in the same scratch copy, and steer in the same way.
- A quiet figure is a figure of a result file under `docs/results`.
- A verification row is cited by its opening words. File and line references are to the tree
  at `b2e2a41`, to the vendored `rapier3d-f64` 0.36.0 under `vendor/`, and to `parry3d-f64`
  0.31.1 and `glamx` 0.3.1 as `Cargo.lock` pins them.

## 1. The further gaps of the D119 row

### 1.1 The trace

Which sites a scene reaches was measured, not read. The test binaries that hold the recorded
Rapier scenes import seven maths functions from the platform library: `acos`, `cos`, `exp`,
`fmod`, `log2`, `pow` and `sin` (`nm -u`; the spike's binary imports `atan2` as well, and none
imports `asin`). The probe interposes all of them, counts every call and keeps its calling
frames, which the binary's debug symbols name by function, file and line. Each recording test
ran once, alone, in the dev profile CI tests in, outside the runner: production layout,
default executor. SOUR's modes vary workers, layout and schedule, not what the backend
builds, so the reach of a scene does not depend on the mode.

Nine recording tests step Rapier. The other recording tests step the test backend or no
physics world. Each of the nine reaches `log2` and no other function from inside a step:

| Recording test | `log2` calls | Other platform calls |
|---|---|---|
| `lactose_physics_rapier::sour::a_server_scene` | 284 | none |
| `lactose_physics_rapier::sour::a_prediction_scene_with_a_rollback` | 524 | none |
| `lactose_physics_rapier::sour::a_limited_hinge_driven_against_its_limits` | 66 | none |
| `lactose_physics_rapier::sour::the_conformance_run` | 18 | none |
| `lactose_app::spike::a_dynamic_capsule_is_recorded` | 206 | `atan2` 1,791 and `fmod` 1,779, from the test's own yaw measurement |
| `lactose_app::spike::a_kinematic_character_is_recorded` | 2 | none |
| `lactose_app::spike::a_claim_in_contact_is_recorded` | 3,076 | none |
| `lactose_app::spike::fifty_thrown_bodies_are_recorded` | 4,658 | none |
| `lactose_app::headless::a_recorded_headless_run` | 238 | none |

Every `log2` call has one chain. `Rapier::step` (`crates/physics-rapier/src/backend.rs`, line
267) calls `PhysicsWorld::step_with_events`, then `PhysicsPipeline::step`, then `step_inner`
(`src/pipeline/physics_pipeline/substep.rs`, line 447), then `detect_collisions`. That
function, in `src/pipeline/physics_pipeline/solve.rs`, reaches `Bvh::optimize_incremental`
two ways: through `BroadPhaseBvh::update` (its line 71;
`src/geometry/broad_phase_bvh/update.rs`, line 282), and through `take_deferred_optimize`
(its line 83; `src/geometry/broad_phase_bvh/mod.rs`, lines 226, 152 and 127), the site of
D123's second hunk. `optimize_incremental` calls
`optimization_config` (`parry3d-f64`, `src/partitioning/bvh/bvh_optimize.rs`, line 245), whose
lines 26 and 27 call `f64::log2`.

The dynamic capsule's `atan2` and `fmod` are `yaw_of` and `wrap` in
`crates/app/tests/spike.rs` (lines 2123 and 2128). They measure the yaw error the spike
reports, outside the step, in a crate outside the boundary. They feed no input and no
recorded value.

The same probe ran over 50 other tests of `lactose-physics-rapier`: its unit tests and the
test targets `backend`, `conformance`, `copy`, `determinism`, `events`, `memory`, `queries`,
`reload`, `scenes` and `snapshot`. They reach `log2` and nothing else. So none of those tests,
recorded or not, reaches the eigen decomposition, the joint motor's `sin`, the soft-body
solver, the pseudo-normals or either `angle`.

### 1.2 Site by site

| Site | Reached by | Settled as |
|---|---|---|
| `Bvh::optimize_incremental`'s `log2` | All nine recorded scenes, by the chain above | Traced, and accepted: the value decides one rounded count, which is the same for any `log2` within two billion ulp (1.3) |
| glamx's symmetric eigen decomposition (`acos`, `cos`) | No recorded scene and no test. One transition through the public surface, its result discarded | Accepted for milestone 1 (1.4). Re-gated to milestone 3, where a mesh's mass properties keep the result |
| `GenericJointConstraint::lock_axes`'s `sin` | Nothing | Accepted as unreachable: the backend builds no multibody joint and no motor (1.5) |
| Trimesh pseudo-normals' `acos` | Nothing | Accepted as unreachable at milestone 1: the backend builds no trimesh. Re-gated to milestone 3 with the mesh (1.5) |
| The soft-body solver's `exp`, `pow` and `acos` | Nothing | Accepted as unreachable: the backend builds no soft body (1.5) |
| `RevoluteJoint::angle` and `VectorExt::angle` | Nothing; linked by no test, and banned in every boundary lint file | Accepted (1.5) |

### 1.3 The broad phase's `log2`

`optimization_config` computes, from the tree's leaf count `n` alone:

```
a = ceil(sqrt(n))            b = ceil(4 * sqrt(n))
cost = a * log2(a) / (b * log2(b))
target_optimized_subtree_count = max(0, round(ceil(5n / 100) / b - cost))
```

The two `log2` values enter nothing else. The other three fields of the configuration come
from the pass counter and from `sqrt` and `ceil`, which IEEE 754 fixes. So a platform's `log2`
can change a result only by moving `ceil(5n / 100) / b - cost` across a rounding boundary.

It cannot. A probe evaluated that expression for every leaf count from 1 to 20,000,000. Its
least distance from a boundary that matters is 2.43e-7, at 11,677,041 leaves. `cost` is at
most 0.2147, so an error of one ulp in each `log2` moves the expression by less than
9.6e-17. The count is therefore the same for any `log2` within 2.5e9 ulp of the true value,
on any tree of up to twenty million leaves. An empty tree returns before the call. A probe
that perturbed both `log2` values by up to 2^30 ulp changed the count for none of the first
2,000,000 leaf counts.

This is a proof for the pinned parry, not evidence for nine scenes. The site is closed for
every scene milestone 1 to 4 can build. What reopens it is a parry bump that changes
`optimization_config`, which the Rapier bump's review reads as it reads the patch.

### 1.4 The eigen decomposition

`glamx`'s `DSymmetricEigen3` calls `acos` once and `cos` twice (`src/eigen3.rs`, lines 101,
105 and 106) when the matrix is not diagonal. A diagonal matrix returns before them (line
65). parry reaches it from `MassProperties::with_inertia_matrix`
(`src/mass_properties/mass_properties.rs`, line 215), which the sum of two non-zero mass
properties calls (line 484) and which compound, convex-polyhedron, trimesh and voxel mass
properties call.

The backend never keeps such a sum. `Desc::mass_plan` (`crates/physics-rapier/src/book.rs`,
lines 97 to 109) gives a body with a collider the collider's mass or density, and a body
without one the mass of a unit ball as additional mass properties. A body has at most one
collider (`Live`, line 124). Rapier's recompute starts from zero and adds the collider's
properties, then the additional ones (`src/dynamics/rigid_body_components.rs`, lines 453 to
471). `MassProperties::add` returns the other operand when one is zero, so one term never
reaches `with_inertia_matrix`. The backend builds a ball, a capsule or a cuboid and nothing
else (`shared_shape`, `crates/physics-rapier/src/convert.rs`, lines 122 to 133).

One transition does reach it. A row with a `RigidBody` and no `Collider` holds the unit
ball's mass. When it gains a `Collider`, `Rapier::update` inserts the collider before it
clears the additional mass (`backend.rs`, lines 484 to 487, then 535 to 541). The insert
attaches the collider and adds its mass to the body's (`ColliderSet::insert_with_parent`,
`src/geometry/collider_set.rs`, line 234; `attach_collider`, `rigid_body_components.rs`, line
1257). Both terms are non-zero there. The trace of a probe test shows it: one `acos` and two
`cos`, called from `eigen3.rs` under `with_inertia_matrix`, `add`, `attach_collider`,
`insert_with_parent`, `PhysicsWorld::insert_collider`, `update`, `flush` and `step`. The
collider must be offset or tilted, or the summed matrix is diagonal.

The result is discarded before anything reads it. The same `update` ends by calling
`recompute_mass` (`backend.rs`, lines 615 to 617), which rebuilds the body's mass from zero
with the additional mass now cleared. The probe confirms it by answering wrongly: with the
interposed `acos` and `cos` scaled by 1 + 1e-6, the scene's bytes after 180 ticks hash to the
value they hash to unperturbed (e26c59e58981ae9d), while the same sum kept by a direct call
changes all three principal inertias. A control body created with its collider reports the
same final transform bit for bit and makes no call.

So the site is accepted for milestone 1 on two grounds: no result of it reaches the solver,
and the one reach is a transition whose result is overwritten in the call that produced it.
The acceptance rests on the order of `Rapier::update`, and no test holds that order today.
One is owed: a body that gains an offset, tilted collider reports, from that tick on, the
states of a body created with it. The body is pushed (an impulse and a torque impulse) in
the tick it gains the collider and lands on a floor, so that a dropped closing rebuild fails
it as a kept sum does. The push is what a dropped rebuild needs. Without the closing rebuild
Rapier's own user-change pass still rebuilds the mass at the start of the step, so the sum
would be live only for the commands of the gaining tick, which the backend applies between
the two. The first review's probe shows both halves: with the closing rebuild removed, a
body that falls, spins, is pushed a tick later or lands stays equal to its control to tick
180, and only one pushed in the gaining tick differs, from tick 6; with the sum kept, every
variant differs. The test rides the lane of section 3, or the next lane that edits the
backend's tests.

It is re-gated to milestone 3. `ShapeKind::Mesh` arrives there with cooked collision
(`crates/physics-api/README.md`, "Not in this crate yet"), and a trimesh's mass properties
call `with_inertia_matrix` on every collider with a density
(`src/mass_properties/mass_properties_trimesh3d.rs`, line 33). A second collider on one body
is the other route, and is not built either. The lane that adds either closes the route
first. The recommended closure is a patch of `glamx` 0.3.1 under architecture section 8.2:
the three calls go through simba's `ComplexField`, which `libm_force` routes to `libm`, as
the Rapier patch does for `sin_cos`. `glamx` already depends on simba, `src/eigen3.rs` holds
no `unsafe`, and the change is three lines and their notices. A recorded scene would be
evidence for its own inertia matrices alone. Milestone 1 does not carry the patch: nothing
it builds keeps the result, and a patch travels with every bump.

### 1.5 The sites the backend cannot reach

`GenericJointConstraint::lock_axes`. The row says its `sin` serves locked or limited angular
axes. It does not: the one platform call is `(motor_params.target_pos / 2.0).sin()` in
`motor_angular_generic` (`src/dynamics/solver/joint_constraint/generic_joint_constraint_builder.rs`,
line 769), inlined into `lock_axes`. A joint takes the generic path only when one of its
bodies is a multibody link (`src/dynamics/solver/categorization.rs`, lines 14 to 17). The
backend inserts impulse joints alone (`backend.rs`, line 418), every boundary lint file bans
`MultibodyJointSet::insert` and `insert_kinematic`, and a snapshot whose multibody set holds
a joint is refused (`snapshot.rs`, line 466). The backend also sets no motor. The two-body
path it does use computes its angles through simba, and the recorded hinge, which runs both
limits, reaches no platform call but `log2`.

The soft-body solver. Its `exp`, `pow` and `acos` run per soft body. The backend inserts
none, and a snapshot whose soft-body set is not empty is refused (`snapshot.rs`, line 467).
No test makes one call of the three.

Trimesh pseudo-normals. Their `acos` is `VectorExt::angle`'s, from `src/shape/trimesh.rs`,
lines 1451 to 1453. The backend builds no trimesh, and its write and its copy refuse a shape
it did not build (`snapshot.rs`, lines 166 and 307). It goes with the mesh to milestone 3.

`RevoluteJoint::angle` and `VectorExt::angle`. No test binary imports `asin`, and the lint
file bans both by name.

What holds these reasons in place is one fact: among boundary crates only
`lactose-physics-rapier` may name `rapier3d-f64` (`crate-tiers.toml`), and it exposes one
item, `backend(pool)`. Nothing outside it can put a shape, a joint or a soft body into the
solver. So the acceptance is a property of one crate's source, and D145 binds its lanes: a
lane that widens what the backend builds re-runs the trace over its tests and closes each
site they newly reach, before a recorded scene is its only net.

### 1.6 Cost, and what would change this

No lane. The row's text is the coordinator's (section 5). The trace needed a probe, and this
document ran it: it cost one build of three test binaries. It is a probe a reviewer can run
again from the appendix, not a lane and not a coordinator task.

What would change it: a recorded scene or a backend change that reaches another site, which
the trace shows in a minute on the Mac; a tree of more than twenty million leaves; a parry
bump that changes `optimization_config`; an edit of `Rapier::update` that drops or moves its
closing `recompute_mass`. The trace is not a gate. It runs on macOS alone, by hand.

## 2. The snapshot contract's deserialisation claims

### 2.1 What is built

The row dates from the facade. It asked whether Rapier's deserialisation could rebuild shapes
from the carried `Shape` values without setting a change flag, waking a body or updating a
mass, and guessed that a custom collider `Deserialize` might be needed. The backend answered
it in code, and its README records the reading ("The snapshot bytes", "What Rapier's own
deserialisation does"). The row was never brought up to date.

No custom `Deserialize` was needed. `shapes_in` (`crates/physics-rapier/src/snapshot.rs`,
lines 171 to 178) validates each carried `Shape`, builds it with `shared_shape`, the
constructor `set_collider` uses, and puts it into `SetImage`, an image of the collider set's
serde layout whose shape type is a parameter (lines 45 to 123). Rapier's own derive then
decodes the image as a `ColliderSet`. The exchange happens in the encoded form. No setter of
Rapier runs.

Rapier's sets and its arena derive `Deserialize`. No hand-written impl in the closure calls a
Rapier method: parry's three (`SharedShape`, `SimdAabb` and `ContactManifoldsWorkspace`),
glam's, and Rapier's `deserialize_from_vec_tuple`, which rebuilds two maps whose order
nothing reads (`BroadPhaseBvh::pairs` and `PersistentIslands::joint_link_locs`). A derive
assigns the fields it decodes and gives each `serde(skip)` field its default. It cannot set
a flag it did not decode, wake a body or recompute a mass, because it calls no method of the
type. `read` then builds `PhysicsWorld`
as a struct literal with fresh pipelines and checks the bookkeeping against it (lines 382 to
407).

### 2.2 Claim by claim

| Claim | The reading | The test that holds it |
|---|---|---|
| Shapes are rebuilt from the carried `Shape` values | `shapes_in`, as above | `the_collider_set_reads_back_byte_for_byte_with_its_shapes_from_their_values` (`snapshot.rs`, line 537): Rapier's own encoding of the restored set equals the original's, shapes included |
| No change flag is set | A collider's `changes`, a body's and both modified lists are encoded fields, decoded as written | The same test for the collider set. `tests/snapshot.rs`, `a_restored_backend_writes_the_same_bytes_and_steps_on_identically`: a pile of 60 bodies with two joints, with and without pending work, restored on pools of one and four threads, writes back the bytes it restored |
| Nothing is woken | A body's activation is an encoded field | The same test, and `tests/copy.rs`, `a_backend_restored_from_a_copy_steps_as_one_restored_from_its_bytes`, whose restore points include sleeping bodies: a byte restore steps as the run it was taken from |
| No mass is updated | Mass properties are encoded fields; no recompute runs | The same two. A recompute that returned the values it had would not be observable, and would not be a breach |
| A custom collider `Deserialize` may be needed | It was not: `SetImage` | The first test, which a Rapier bump that changes the layout fails |

The tests have teeth. In a scratch copy, a restore mutated to wake every body it decoded
fails `a_restored_backend_writes_the_same_bytes_and_steps_on_identically` at "the bytes of
what they restored", `bytes_that_are_not_a_snapshot_are_refused_and_change_nothing` and the
`copy.rs` test. A restore mutated to replay each shape through `set_shape` fails those three
and the unit test.

### 2.3 What milestone 1 rests on

Nothing a milestone 1 binary runs rests on a claim that could be false. Under both mutations
the three tests of `tests/conformance.rs`, the three reload tests and every test of
`tests/sour.rs` still pass. The conformance run and the SOUR scenes restore through the copy,
the reload tests restore the empty world, and `conformance::validity` compares one byte
restore with another:

- Prediction's restore is `PhysicsBackend::restore_copy`
  (`crates/physics-api/src/service.rs`, line 448), which for Rapier clones the copy's sets
  (`backend.rs`, lines 322 to 330). It decodes nothing.
- Criterion 5's slot is `PhysicsBackend::copy` (`service.rs`, line 371). It encodes nothing
  and decodes nothing.
- D143's copy encodes only when its size or hash is asked. That write decodes the collider
  set's own encoding into the image (`shapes_out`, lines 159 to 167). A mismatch there is a
  property of the build, which the unit test holds.
- Shipped crates restore bytes at three sites: the fresh solver's
  (`crates/physics-api/src/sync.rs`, line 451), which alone runs in a binary; the trait
  default `restore_copy` (`crates/physics-api/src/backend.rs`, lines 292 to 299), which
  Rapier overrides; and `conformance::step_from` (`crates/physics-api/src/conformance.rs`,
  lines 543 to 553), which only tests call. The fresh solver's restores the empty world's
  bytes (D124), which hold no shape, body or joint, so the four claims are vacuous there.
  What it rests on is that the empty bytes restore to a world that steps as a new one,
  which `conformance::empty_restore` holds on every pool size and layout.

Were a claim false, a reader would see it in `tests/snapshot.rs` on every CI platform, at
the first assertion, and nowhere in a milestone 1 binary. The first restore of a world that
holds bodies that a binary runs is milestone 4's journal checkpoint. Its row already owes the test
that would show it: a session checkpointed with contacts live, replayed from the checkpoint,
hashing equal to the recording at every tick.

### 2.4 Who, when, and what would change this

The reading is this section, made against the vendored source at `b2e2a41`; the proposal's
reviewer attacks it. The tests are on main and run under nextest on linux-x64, windows-x64
and macos-arm64. The row is marked Done at milestone 1 by the coordinator's pass, with the
text of section 5 and the CI run that passed the three tests. No lane, no run.

Not covered, as the backend's README says: CCD, contact clustering, multibody joints and soft
bodies, none of which the backend uses. A lane that enables one owes the restore test for
it, as D143 already says of CCD across a copy.

What would change it: a Rapier bump, which re-runs the unit test and re-reads the
`serde(skip)` list; a hand-written `Deserialize` appearing on one of Rapier's sets.

## 3. The physics hash and the world hash

### 3.1 What a join would be

Section 8.3 now says `PhysicsSnapshot::hash` "may now join the world hash". The world hash
of that section is `Simulation::world_hash(region)`: a region's rows, resources, seed, mode
and asset table. The backend's bytes are one world's, with one physics world per process
(section 16). They belong to no region. A join can therefore only be a fold into
`Simulation::hash`, the value over the tick and every loaded region that SOUR records
(`crates/sim/src/simulation.rs`, line 486). `lactose-sim` cannot name `lactose-physics-api`,
which depends on it, so a fold needs a registration hook in `lactose-sim` for a service's
hash.

### 3.2 What it would buy

SOUR would compare the solver's whole state at every recorded tick, where today it compares
the rows the solver wrote back. State that has not yet moved a row would show at the tick it
diverged: warm-start impulses, manifolds, sleep counters, the broad phase's tree. That bears
on a contract already in force. D119 obliges the backend to give equal bytes under any pool
size and on x86-64 and aarch64, and `lactose-physics-api` froze on it. The evidence for the
bytes across platforms is one value: the conformance run's, after ten ticks. The evidence for
the sign of zero in SIMD `min` and `max`, the one determinism gap closed by evidence alone,
is rows over nine scenes and that one value.

No milestone 1 criterion needs it. Criteria 2, 3 and 7 are met on rows and on the conformance
run's bytes. The first milestone that needs solver state compared is milestone 4: a recorded
session replayed from a checkpoint that carries the physics snapshot, and a solver-state
baseline sent to a client (the row "Solver history departure in the prediction world").

All of that is bought by recording the hash. None of it needs the fold.

### 3.3 What a fold would cost

It would empty a figure of criterion 7's result file. `Client::hash` is `Simulation::hash` of
the prediction world, "what the spike compares against the server's hash of the same tick"
(`crates/app/src/client.rs`, lines 139 to 144; `crates/app/tests/spike.rs`, lines 2278 to 2300
and 2507). `docs/results/spike-net-apple-m1-2026-11-15.md` carries the count as
`hash_agreement.agreed`: 35,197 of 35,553 compared ticks for the dynamic capsule under the
clock script, 35,520 of 35,556 for the kinematic character, 16,022 of 19,150 for fifty thrown
bodies. The two worlds can agree because the hash is a function of rows. Their solvers never
agree: bit equality holds "never the server and a client's prediction world" (section 16),
the client's solver is rebuilt at a join and at a resync, and the bytes carry state as plain
as the count of optimisation passes a world's broad phase has run, which for a client starts
at its join. A folded hash makes the spike's count zero in every scene, by construction.

It would move 8,711 of the 10,334 lines the nightly recorded, in 14 of its 25 recording
tests: every world hash of a test whose world holds a `Physics`, the test backend's included.
The conformance run's eleventh line is the bytes' hash itself and stays. Both counts are the
nightly's, at `c50b688`. Main at `b2e2a41` lists 27 recording tests, and the two it added
hold no `Physics`, so the 14 stand. The spike's digest
8db4551c007e6410 would move with them. The merges of `c9b-renames` and `slot-copy` used it to
show they changed no behaviour (the rows "Codenames on test glue" and "The prediction slot as
a copy (D143)"). So would the headless binary's 0x92ff315464f84575, which session B compares
on x86-64. The fold itself could then not be shown neutral by the digest.

It would put a backend write on every hashed tick. Criterion 1 excludes the world hash,
"which SOUR pays at every step and a shipped binary only when it asks" (section 22), so the
cost is SOUR's and the headless binary's, but it is not small:

| Scale | Bytes | Write | `xxh64` | Per hashed tick | Against |
|---|---|---|---|---|---|
| The spike's thrown scene, the prediction world at the bound of 64 | 251,041 | 135.2 µs (quiet) | about 80 µs (probe rate) | about 215 µs, and a copy of 79 µs more through `PhysicsSnapshot::hash` | Criterion 5's 100 µs |
| 2,000 awake bodies, 40 ticks in | 3,456,816 | 2,129 µs | 1,087 µs | 3.2 ms | A step of about 1 ms |
| 2,000 awake bodies, 200 ticks in, toppled | 6,507,386 | 5,388 µs | 2,386 µs | 7.8 ms | |
| 2,000 awake bodies beside 50,000 fixed boxes, the physics-phase scene's count | 65,243,938 | 47,324 µs | 21,524 µs | 68.8 ms | The quiet phase of 1,954 µs, 35 times over |

The last three rows are probe figures (load averages 7.75 8.36 7.28, four solver threads).
The first is the first quiet window's write and the probe's hashing rate, 3.2 GB/s.

It would amend `lactose-sim`'s surface, a facade other crates are written against, and it
would have to switch `Client::hash` to a second, rows-only hash to keep the spike's figure.
Of the lanes main does not yet hold, it meets `schema-digest`, which changes every baseline's
content hash (plan decision 5.6) and may move recorded lines of its own: the two would merge
in a stated order, each proving itself over the other's moved lines. It also meets
`readme-sweep` in the two READMEs that name the join.

### 3.4 The options

- **A. Recorded beside, from milestone 1.** A recording test that steps a backend records
  `PhysicsSnapshot::hash` of each world it steps as a second stream under its own name, every
  tick, as `the_conformance_run` does for one tick today. `Simulation::hash` stays a function
  of rows. No interface changes: `lactose_physics_api::snapshot(world)?.hash()` is public. No
  recorded line moves. Rules out one number for a world's whole state; a caller that wants
  one hashes the two itself.
- **B. Recorded beside, from milestone 4.** The same form, decided now, built with the
  journal checkpoint. Rules out any evidence for the bytes across platforms beyond the
  conformance run's one value until then.
- **C. Folded into `Simulation::hash`, with a rows-only hash for comparisons between worlds.**
  One number per world. Rules out the spike's digest and every world hash of 14 tests as
  they stand, and needs the `lactose-sim` hook, a second public hash and the switch of
  `Client::hash`.

### 3.5 Recommendation

A. `PhysicsSnapshot::hash` is never folded into `Simulation::hash` or `world_hash`. It is
recorded beside them, from milestone 1, by lane `physics-records`.

The lane edits tests and one policy list. In `crates/physics-rapier/tests/sour.rs` the server
scene, the prediction scene with its rollback and the limited hinge record the hash after
every tick, the prediction scene's re-simulated ticks again. In `crates/app/tests/spike.rs`
the four recorded scenes record one value per frame over the server's world and the client's
prediction world, and `crates/app/tests/headless.rs` records the server's. The ten-minute
replays compute no physics hash, so `spike-net`'s figures do not move. Each stream has its
own name, the test's with `::physics` appended, opens its own recorder and, under a chaos
seed, writes the test's execution log under its own name, since the runner asks an applied
line and a log of every name that records (`tools/xtask/src/harness/mod.rs`, lines 543 to
572). `tools/xtask/ci-policy.toml` lists the eight names under `[harness]`. The lane also
carries the test of section 1.4 and corrects the three sentences that call `hash` the place
where the bytes join the world hash.

Its proof:

- `cargo xtask sour run --nightly` on the M1: every new stream equal in every mode, and the
  prediction scene's re-simulated physics hashes equal to its first run's.
- Every line the existing names record is byte-identical to main's at the lane's base, the
  spike's digest among them.
- One mutation of solver state that moves no row at the tick it is made fails a physics
  stream at that tick. The lane picks it and says why the world stream cannot see it then.
- `cargo xtask sour run --pr` fails when a listed physics stream records nothing.
- `cargo xtask measure spike-net --short` before and after, as steering.
- The test time the new streams add to one SOUR launch. It is about one second, most of it
  the thrown scene: about ten seconds a nightly, which is ten launches, and three a pull
  request, which is three. The first review measured it: 0.9 to 1.1 s more for the thrown
  scene in each of six modes, in the test profile, at load averages 1.9 to 2.8. What it adds
  to the wall clock depends on how nextest runs the tests side by side.
- The first nightly that holds the lane: equal on three platforms, or a finding.

Cost: five runs, as a small lane, and a floor like every count of the plan. It joins queue 2
and starts when no lane in flight edits `crates/app/tests/spike.rs` or the `[harness]` list.
Of the lanes main does not yet hold, none is briefed to edit the spike, and edits to
`ci-policy.toml` are ordered by merge (the plan's second rule). It moves no recorded line, so
it needs no order against `schema-digest`.

Why now and not at milestone 4: the nightly is the only place x86-64 and aarch64 are
compared, the contract the bytes are held to is already frozen, and each nightly between now
and milestone 4 is evidence the lane makes free. A difference found now is a finding about
nine small scenes. Found at milestone 4 it is a replay that hashes apart with `net` under it.

What would change this: the owner preferring one number per world (C); the lane's nightly
showing bytes that differ across platforms where rows agree, which comes back to the planner
to decide whether the difference is closed or the backend's obligation is restated to rows;
the new streams adding more than three seconds of test time to one SOUR launch, which cuts
the spike's streams to the server's world.

## 4. The question for the owner

One question. The coordinator asks it in these words; the recommended option is first.

> Nightly 43703297881 showed Rapier's serialised bytes hash the same on all three platforms,
> at one tick of one scene, and the architecture now says that hash "may now join the world
> hash". Folding it in would zero a figure of criterion 7's baseline: the spike counts the
> ticks at which the prediction world's hash equals the server's, and their solvers never
> agree. How does the physics hash enter SOUR?
>
> 1. **Recorded beside the world hash, from milestone 1 (Recommended):** One small lane,
>    physics-records, of five runs, in tests alone. Each recorded Rapier scene records the
>    hash of the solver's bytes every tick as a stream of its own. No recorded hash moves and
>    no interface changes. It adds about one second of test time per SOUR launch, most of it
>    the thrown scene: about ten seconds a nightly and three a pull request. Every nightly
>    from then on compares solver state across x86-64 and aarch64, which the backend's
>    contract already promises and one value proves today.
> 2. **Recorded beside it, from milestone 4:** No lane now. The form is decided, and the
>    recording is built with the journal checkpoint. Until then the conformance run's one
>    value is the only evidence for the bytes across platforms.
> 3. **Folded into the world hash:** One number per world. It moves 8,711 of the 10,334 lines
>    the nightly recorded, the spike's digest and the headless hash, needs a hook in
>    lactose-sim and a second rows-only hash for the spike, and comes back to the planner for
>    that design.

D145 also records what needs no choice: the platform-library sites of section 1 as settled
there, and the snapshot contract's claims as closed by section 2.

## 5. Text for the coordinator (exact)

D145 is the next free number. The text is for answer 1, and the coordinator writes the date
of the owner's answer where it says `<date>`. The sentences that change under answer 2 follow
each block. Every anchor was checked at `b2e2a41` and again at main `26e743c`, which adds the
`timer-windows` merge, with line breaks and doc-comment markers read as spaces: each occurs
once, inside the row or file named, and D145 is free at both.

### Decision entry

```
**D145. The physics hash is recorded beside the world hash and never folded into it; the platform-library sites inside Rapier and parry are settled by what the backend builds and what the recorded scenes reach** (§8.2, §8.3, §10, §16, §22, `lactose-physics-api`, `lactose-physics-rapier`, `lactose-app`, `docs/plans/milestone-1.md`)
- Why: nightly 43703297881 closed criteria 2, 3 and 7 and left three things open (`b2e2a41`):
  which of the platform-library sites that `nm -u` found inside Rapier and parry the recorded
  scenes reach; whether the snapshot contract's deserialisation claims hold; and section
  8.3's new sentence that `PhysicsSnapshot::hash` may join the world hash.
  `docs/plans/physics-evidence-proposal.md` holds the trace, the reading and the figures;
  these are the decisions.
  - *The physics hash is recorded beside the world hash.* A recording test that steps a
    backend records `PhysicsSnapshot::hash` of each world it steps as a stream of its own,
    under its own name, at every tick, and SOUR compares it in every mode. `Simulation::hash`
    and `Simulation::world_hash` stay functions of rows. The world hash is a region's, and
    the backend's bytes are the whole world's. The bytes are a function of the row history
    (section 16), and `Simulation::hash` is compared between worlds whose histories differ by
    design: the netcode spike's baseline counts the ticks at which the prediction world's
    hash equals the server's (35,197 of 35,553 for the dynamic capsule under the clock
    script, `docs/results/spike-net-apple-m1-2026-11-15.md`), and a folded hash makes that
    count zero in every scene. A fold would also move 8,711 of the 10,334 lines the nightly
    recorded at `c50b688`, the spike's digest and the headless binary's hash, and put a
    backend write on
    every hashed tick: about 215 µs on the spike's thrown scene and 69 ms beside the
    physics-phase scene's 50,000 fixed boxes (a probe under load). Rules out a hook in
    `lactose-sim` for a service's hash, a second rows-only hash, and one number for a world's
    whole state: a caller that wants one hashes the two itself.
  - *The recording lands in milestone 1, in tests alone*: lane `physics-records`, over the
    three scenes of `lactose-physics-rapier`'s SOUR test, the spike's four and the headless
    run. The backend's obligation of equal bytes on x86-64 and aarch64 (D119) is frozen and
    rests on one value, the conformance run's; the nightly is the only place it is measured.
    A difference between platforms in a physics stream where the rows agree is a finding and
    returns to the planner.
  - *The broad phase's `log2` is accepted.* All nine recorded scenes that step Rapier reach
    it, through `Bvh::optimize_incremental`, and reach no other platform function from a
    step. Its two values decide one rounded count. For every leaf count from 1 to 20,000,000
    the rounded expression stays 2.4e-7 or more from a boundary, and one ulp of `log2` moves
    it by less than 9.6e-17, so the count is the same for any `log2` within 2.5e9 ulp. Rules
    out a patch and a recorded scene for it.
  - *glamx's eigen decomposition is accepted for milestone 1 and gated to milestone 3.* No
    recorded scene and none of the backend's tests reaches it. The backend gives a body one
    collider or a unit ball's mass, never both, and builds a ball, a capsule or a cuboid, so
    no sum and no shape keeps its result. One transition reaches it, a `RigidBody` row that
    gains an offset or tilted `Collider`, and `Rapier::update` rebuilds the mass from zero in
    the same call: the scene's bytes are unchanged with `acos` and `cos` perturbed. A test
    owed by `physics-records` pins that order: a body that gains an offset, tilted collider
    reports, from that tick on, the states of a body created with it. The body is pushed (an
    impulse and a torque impulse) in the tick it gains the collider and lands on a floor, so
    that a dropped closing rebuild fails it as a kept sum does. The lane that adds
    `ShapeKind::Mesh` or a second collider on a body closes the route first, by a patch of
    glamx under section 8.2 or by refusing the feature. Rules out the patch in milestone 1,
    which builds nothing that keeps the result.
  - *The joint motor's `sin`, the soft-body solver, the trimesh pseudo-normals,
    `RevoluteJoint::angle` and `VectorExt::angle` are accepted as unreachable.* The `sin` is
    the target of an angular motor on a joint that touches a multibody link, not a locked or
    limited axis. The backend inserts impulse joints without motors, three shapes and no soft
    body; the lint file bans the multibody inserts and both `angle` functions; a snapshot
    that holds a multibody joint or a soft body is refused; a write and a copy refuse a shape
    the backend did not build. Among boundary crates only `lactose-physics-rapier` may name
    `rapier3d-f64`, and it exposes `backend(pool)` alone.
  - *A lane that widens what the backend builds owes the trace.* A shape kind, a second
    collider on a body, a joint motor, a multibody joint, a soft body or CCD: the lane runs
    the platform-library trace over its tests and closes each site they newly reach, by a
    patch under D123 or by refusing the feature, before a recorded scene is its only net.
  - *The snapshot contract's deserialisation claims are closed.* No custom collider
    `Deserialize` was needed: the shapes are exchanged inside an image of the collider set's
    serde layout, and Rapier's sets derive `Deserialize`, with no hand-written impl in the
    closure that calls a Rapier method, so decoding calls no setter. Three tests hold the
    claims and fail under a restore mutated to wake a body or to replay a shape. No path a
    milestone 1 binary runs restores a world that holds a body from bytes: a slot restores
    the backend's copy (D143), and the fresh solver restores the empty world's bytes (D124).
    The first such restore is the journal checkpoint's, at milestone 4.
- Driven by: nightly 43703297881 and `b2e2a41`; the planner's trace of the platform
  library's callers on aarch64-apple-darwin, its arithmetic over the leaf counts, its
  perturbation probe and its two mutations of the byte restore; the owner's answer of
  `<date>`, which chose the recording from milestone 1.
- Replaced: section 8.3's "so `PhysicsSnapshot::hash` may now join the world hash"
  (`b2e2a41`). D119's "hashing the snapshot bytes" among its deferred items, as a join.
  D119's "until then the property is unproven, not excused" for the sites this entry
  accepts; for the sign of zero in SIMD `min` and `max` it stands.
```

Under answer 2, the second bullet reads:

```
  - *The recording lands in milestone 4, with the journal checkpoint*, whose replay is the
    first consumer of solver state compared across processes. Until then the conformance
    run's one value is the evidence for the backend's obligation of equal bytes on x86-64 and
    aarch64 (D119). Rules out a lane in milestone 1.
```

The eigen bullet's "A test owed by `physics-records` pins that order" reads "A test owed by
the next lane that edits the backend's tests pins that order", and "Driven by" ends "which
chose the recording from milestone 4".

### Architecture amendment

`docs/architecture.md`, section 8.3. The same under answers 1 and 2.

```
--- section 8.3: replace ---
equal across platforms in nightly 43703297881, so `PhysicsSnapshot::hash` may now join the world hash.
--- with ---
equal across platforms in nightly 43703297881. `PhysicsSnapshot::hash` is recorded beside the world hash, as a stream of its own that SOUR compares in every mode, and is never folded into it: the bytes are the whole world's and a function of its row history (section 16), where the world hash is a region's, a function of its rows, and is compared between a prediction world and its server, whose solvers never agree (D145).
```

### Plan amendments

`docs/plans/milestone-1.md`, for answer 1.

```
--- section 2, the row of row 88: replace ---
| 88 | The physics snapshot contract | `sour-evidence` | The recorded physics hash, and the reading of Rapier's deserialisation |
--- with ---
| 88 | The physics snapshot contract | `sour-evidence`, D145 | The recorded physics hash. The reading of Rapier's deserialisation is `crates/physics-rapier/README.md`, "The snapshot bytes", re-read at `b2e2a41` with the tests that hold it (`docs/plans/physics-evidence-proposal.md`, section 2) |
| new | The further gaps of the `lactose-physics-api` (D119) row | D145 | The trace of the platform library's callers over the nine recorded Rapier scenes: `log2` alone, accepted by arithmetic; the other sites unreachable by what the backend builds, the eigen decomposition re-gated to milestone 3 (`docs/plans/physics-evidence-proposal.md`, section 1) |
| new | The physics hash beside the world hash (D145) | `physics-records` | Its streams equal in every SOUR mode on the M1, then on three platforms in the first nightly that holds it |

--- section 3, the second lane table: add a row after the row of `timer-windows` ---
| `physics-records` | The physics hash recorded beside the world hash (D145), in tests alone. Each recording test that steps Rapier opens a second recorder, named as the test with `::physics` appended, and records after every tick `lactose_physics_api::snapshot(world)?.hash()` of each world it steps: `lactose-physics-rapier`'s `a_server_scene`, `a_prediction_scene_with_a_rollback`, its re-simulated ticks again, and `a_limited_hinge_driven_against_its_limits`; `lactose-app`'s four spike scenes, the server's world and the client's prediction world as one value per frame, and `a_recorded_headless_run`. `the_conformance_run` keeps its one value. The ten-minute replays compute no physics hash. Each `::physics` name opens its own recorder and, under a chaos seed, writes the test's execution log under its own name. `tools/xtask/ci-policy.toml` lists the eight names under `[harness]`. One test in `crates/physics-rapier/tests/backend.rs`: a body with a `RigidBody` and no `Collider` that gains an offset, tilted collider reports, from that tick on, the states of a body created with it. The body is pushed (an impulse and a torque impulse) in the tick it gains the collider and lands on a floor, so that a dropped closing rebuild fails it as a kept sum does. The sentences of `crates/physics-api/README.md`, `crates/sim/README.md` and the doc comment of `PhysicsSnapshot::hash` that call it the place where the bytes join the world hash | No lane in flight edits `crates/app/tests/spike.rs` or the `[harness]` list | `cargo xtask sour run --nightly` on the M1: every new stream equal in every mode, and the prediction scene's re-simulated physics hashes equal to its first run's. Every line the existing names record is byte-identical to main's at the lane's base, the spike's digest 8db4551c007e6410 among them. One mutation of solver state that moves no row at the tick it is made fails a physics stream at that tick; the lane picks it and says why the world stream cannot see it then. `cargo xtask sour run --pr` fails when a listed physics stream records nothing. `cargo xtask measure spike-net --short` before and after, as steering. The test time the new streams add to one SOUR launch, about one second by the first review's measure. Then the first nightly that holds the lane: equal on three platforms, or a finding for the planner | 5 |

--- section 3, after the paragraph that begins "D144 adds `timer-windows`": add a paragraph ---
D145 adds `physics-records`, five runs, and six for its proposal: its writing, two reviews, two
revisions and its application.

--- section 3, the queue table: replace ---
| 2, the client and then the save path | `client-fix`, `sour-evidence`, `spike-net`, `stream-impl`, `slot-copy`, `data-path`, `binds-design`, `binds-impl` | 56 |
--- with ---
| 2, the client and then the save path | `client-fix`, `sour-evidence`, `spike-net`, `stream-impl`, `slot-copy`, `data-path`, `binds-design`, `binds-impl`, `physics-records` | 61 |
```

The queue's total counts lane runs alone, as D143's change from 49 to 56 did for
`slot-copy`'s seven. So the lane's five take queue 2 from 56 to 61 and the three queues from
161 to 166, and the proposal's six runs stand in the paragraph, as D143's five and D144's
six do.

Under answer 2 the third added row of section 2, the lane's row, the queue change and the
lane's five runs are left out, and the added paragraph reads "D145 adds no lane, and six
runs for its proposal: its writing, two reviews, two revisions and its application." The
three sentences and the test of section 1.4 then go to `readme-sweep` and to the next lane
that edits `lactose-physics-rapier`'s tests.

### Verification rows

`docs/verification.md`. Each change is an exact replacement inside the row its opening words
name, or a new row at the end of section 4.

```
--- "| The further gaps of the `lactose-physics-api` (D119) row": replace ---
`GenericJointConstraint::lock_axes`'s `sin`, for locked or limited angular axes;
--- with ---
`GenericJointConstraint::lock_axes`'s `sin`, for the target of an angular motor on a joint that touches a multibody link (`motor_angular_generic`, inlined; not for locked or limited axes, as first written);

--- "| The further gaps of the `lactose-physics-api` (D119) row": replace ---
The same nightly's scenes reach some of these sites, the broad phase's `log2` in every step among them, and no difference showed; which sites they reach has not been traced one by one, so at the sites no recorded scene reaches the property is unproven, not excused, and each scene recorded under architecture mode extends the evidence. Open, for the planner: trace which of these sites the recorded scenes reach, then accept the rest with a reason, add a scene that reaches them, or re-gate them
--- with ---
Traced (`docs/plans/physics-evidence-proposal.md`, section 1; aarch64-apple-darwin at `b2e2a41`, the seven maths functions the test binaries import from the platform library interposed and every call counted with its calling frames): the nine recording tests that step Rapier, the four of `lactose-physics-rapier`, the spike's four and the headless run, reach `log2` and no other from a step, 2 to 4,658 calls each, all from `optimization_config` under `Bvh::optimize_incremental`, `BroadPhaseBvh::update` or `take_deferred_optimize`, and `PhysicsPipeline::detect_collisions`; the dynamic capsule's `atan2` and `fmod` are the spike's own yaw measurement, outside the step; 50 other tests of the backend reach `log2` alone. **Done** (D145), site by site. `log2`: accepted. Its two values decide one rounded count, `target_optimized_subtree_count`; for every leaf count from 1 to 20,000,000 the rounded expression stays 2.4e-7 or more from a boundary and one ulp of `log2` moves it by less than 9.6e-17, so the count is the same for any `log2` within 2.5e9 ulp; a parry bump that changes `optimization_config` reopens it. The eigen decomposition: accepted for milestone 1 and gated to milestone 3 (the row "glamx's symmetric eigen decomposition calls the platform library"). The backend gives a body one collider or a unit ball's mass, never both, and builds a ball, a capsule or a cuboid; a `RigidBody` row that gains an offset or tilted `Collider` reaches it once (`attach_collider` before `Rapier::update` clears the additional mass), and the same call rebuilds the mass from zero: with `acos` and `cos` scaled by 1 + 1e-6 the scene's bytes after 180 ticks are unchanged. The motor's `sin`, the soft-body solver, the pseudo-normals and the two `angle` functions: accepted as unreachable. The backend inserts impulse joints without motors and no soft body, a snapshot that holds a multibody joint or a soft body is refused, a write and a copy refuse a shape the backend did not build, and among boundary crates only `lactose-physics-rapier` may name `rapier3d-f64`. The trace is not a gate: it runs by hand on macOS. A lane that widens what the backend builds (a shape kind, a second collider on a body, a joint motor, a multibody joint, a soft body, CCD) runs it over its tests and closes each site they newly reach (D145)

--- "| The physics snapshot contract rests on the backend deserialising Rapier's sets": replace ---
unverified against Rapier 0.36's deserialisation (a custom collider `Deserialize` may be needed).
--- with ---
read against the vendored Rapier 0.36.0 at `b2e2a41` (`docs/plans/physics-evidence-proposal.md`, section 2): no custom collider `Deserialize` was needed, since `shapes_in` (`crates/physics-rapier/src/snapshot.rs`) exchanges each carried `Shape` for the shape `set_collider`'s constructor builds inside an image of the collider set's serde layout, which Rapier's own derive then decodes; Rapier's sets and its arena derive `Deserialize`, and no hand-written impl in the closure calls a Rapier method (parry's three, glam's, and Rapier's `deserialize_from_vec_tuple`, which rebuilds two maps whose order nothing reads), so decoding assigns the encoded fields, gives each `serde(skip)` workspace its default and calls no setter.

--- "| The physics snapshot contract rests on the backend deserialising Rapier's sets": replace ---
`PhysicsSnapshot::hash` may now join the world hash (section 8.3), a change not yet made. Still unverified: the deserialisation claims above
--- with ---
`PhysicsSnapshot::hash` is recorded beside the world hash and never folded into it (D145, section 8.3). **Done:** the claims are held by the unit test `the_collider_set_reads_back_byte_for_byte_with_its_shapes_from_their_values` (Rapier's own encoding of the restored set, change flags and mass properties included, equals the original's), by `tests/snapshot.rs` `a_restored_backend_writes_the_same_bytes_and_steps_on_identically` (a pile of 60 bodies with two joints, with and without pending work, restored on pools of one and four threads, writes back the bytes it restored and steps on as the run does) and by `tests/copy.rs` `a_backend_restored_from_a_copy_steps_as_one_restored_from_its_bytes`, whose restore points include sleeping bodies. In a scratch copy at `b2e2a41` a restore mutated to wake every body fails the last two and `bytes_that_are_not_a_snapshot_are_refused_and_change_nothing`, and one mutated to replay each shape through `set_shape` fails all four; the conformance run, the reload tests and every SOUR scene pass under both, since none decodes a world that holds a body, and `conformance::validity` passes because it compares one byte restore with another. A recompute of a mass that returned the values it had would not be observable. No path a milestone 1 binary runs restores such a world from bytes: a prediction slot restores the backend's copy (D143), and of the three sites where shipped crates restore bytes only the fresh solver's runs in a binary (D124), on the empty world's bytes, held by `conformance::empty_restore`; Rapier overrides the trait default `restore_copy`, and only tests call `conformance::step_from`. The first is the journal checkpoint's (the row "The journal checkpoint's physics snapshot (D124)"). Not covered, as the backend's README states: CCD, contact clustering, multibody joints and soft bodies, none of which the backend uses

--- section 4: add three rows at the end ---
| glamx's symmetric eigen decomposition calls the platform library's `acos` and `cos` (`glamx` 0.3.1, `src/eigen3.rs`) whenever parry diagonalises an inertia matrix that is not diagonal: `MassProperties::with_inertia_matrix`, from the sum of two non-zero mass properties and from compound, convex-polyhedron, trimesh and voxel mass properties. At milestone 1 no result of it reaches the solver (D145; the row "The further gaps of the `lactose-physics-api` (D119) row"). The lane that adds `ShapeKind::Mesh` for cooked collision meshes, or a second collider on a body, closes the route before its first recorded scene: a patch of glamx under architecture section 8.2 that sends the three calls through simba's `ComplexField`, as the Rapier patch does for `sin_cos` (glamx depends on simba, and `src/eigen3.rs` holds no `unsafe`), or the feature is refused; a recorded scene alone is evidence for its own inertia matrices. The same lane traces the trimesh's pseudo-normals (`parry3d-f64` 0.31.1, `src/shape/trimesh.rs`, through `VectorExt::angle`'s `acos`) | 3 |
| The acceptance of the eigen decomposition at milestone 1 rests on the order of `Rapier::update` (`crates/physics-rapier/src/backend.rs`): it inserts a gained collider while the body still holds its unit ball's mass, and ends by rebuilding the mass from zero (D145). No test holds it. Owed by lane `physics-records`: a body with a `RigidBody` and no `Collider` that gains an offset, tilted collider reports, from that tick on, the states of a body created with it. The body is pushed (an impulse and a torque impulse) in the tick it gains the collider and lands on a floor, so that a dropped closing rebuild fails it as a kept sum does: without the closing rebuild Rapier's own user-change pass still rebuilds the mass at the start of the step, so only a command of the gaining tick meets the sum (the first review's probe: with the rebuild removed, only the body pushed in that tick differs from its control, from tick 6; with the sum kept, every variant differs) | 1 |
| The physics hash beside the world hash (D145): lane `physics-records` records `PhysicsSnapshot::hash` after every tick, as a stream of its own, in the three scenes of `lactose-physics-rapier`'s SOUR test, the spike's four and the headless run. Its proof: every new stream equal in every SOUR mode on the M1; every line the existing names record byte-identical to main's, the spike's digest 8db4551c007e6410 among them; a mutation of solver state that moves no row failing a physics stream; the test time the streams add to one SOUR launch, about one second, most of it the thrown scene, which is about ten seconds a nightly and three a pull request (the first review's measure: 0.9 to 1.1 s for the thrown scene in each of six modes). Then the first nightly that holds it: equal across linux-x64, windows-x64 and macos-arm64, which extends the rows "Rapier's serialised snapshot is iteration-order stable enough to hash" and "`lactose-physics-api` (D119) is implemented" from one value and from rows to the solver's state at every recorded tick; a difference where the rows agree is a finding for the planner. To size it: a backend write and its hash cost about 215 µs a tick on the spike's thrown scene, 3.2 to 7.8 ms for 2,000 awake bodies and 69 ms beside 50,000 fixed boxes (the planner's probe, release, Apple M1, load averages 7.75 8.36 7.28), so no shipped tick hashes the bytes | 1 |
```

Under answer 2 the second new row ends "Owed by the next lane that edits
`lactose-physics-rapier`'s tests", and the third opens "The physics hash beside the world
hash (D145): decided, and not built. With the journal checkpoint, a recording test that
steps a backend records `PhysicsSnapshot::hash` after every tick, as a stream of its own.",
keeps its sizing sentence, drops the lane's proof and is gated 4.

### README and doc-comment sentences

For `physics-records`, or for `readme-sweep` under answer 2. Text only.

```
--- crates/physics-api/src/service.rs, the doc comment of `PhysicsSnapshot::hash`: replace ---
the place where architecture section 8.3 adds the physics world to the world hash, which does not include it yet.
--- with ---
what a recording test records beside the world hash and never folds into it (architecture section 8.3, D145).

--- crates/physics-api/README.md, "Snapshot and restore": replace ---
`hash` is the named place where section 8.3 adds the bytes to the world hash;
--- with ---
`hash` is what a recording test records beside the world hash, never folded into it (section 8.3, D145);

--- crates/physics-api/README.md, "Snapshot and restore": replace ---
`the_conformance_run` records its first value, and no world hash includes it yet.
--- with ---
`the_conformance_run` records its first value.

--- crates/physics-api/README.md, "Not in this crate yet": replace ---
- Hashing the snapshot bytes into the world hash: `PhysicsSnapshot::hash` is the place; the verification item decides when.
--- with ---
- A fold of the snapshot bytes into the world hash: not built, by decision. `PhysicsSnapshot::hash` is recorded beside it (D145).

--- crates/sim/README.md, "Not in this crate yet": replace ---
- Hashing `physics-api::snapshot` bytes into the world hash (architecture section 8.3). Rapier's serialisation was shown order-stable in nightly 43703297881.
--- with ---
- A fold of `physics-api::snapshot` bytes into the world hash: not built, by decision. A recording test records `PhysicsSnapshot::hash` beside this crate's hash (architecture section 8.3, D145).
```

## 6. What this document could not verify

- The trace ran on aarch64-apple-darwin alone. Which sites a scene reaches is a property of
  its control flow, and the three platforms' equal hashes say the flow is the same, but no
  x86-64 trace was taken. On x86-64 a function that is an instruction on aarch64 can be a
  library call; the row's own reading by `nm -u` is the check for that, and this document
  did not repeat it for x86-64.
- Each scene was traced once, in the default configuration. Under SOUR's modes the backend
  receives the same calls in the same key order; no mode was traced.
- The margin of `log2` was computed in double precision, to 20,000,000 leaves. The least
  margin is nine orders of magnitude above the arithmetic's own error.
- The perturbation probe covers one path of `Rapier::update`: a collider gained with no
  change of type. The argument for the other arms is the reading of section 1.4.
- The wake mutation of section 2.2 reaches each body through `get_mut`, which also marks it
  modified, so it breaks the flag claim with the wake claim. A wake that left no mark was not
  tried.
- The CI run that passed the three tests of section 2 on each platform is the coordinator's
  to cite; this document read no CI log.
- The figures of section 3.3 at the 2,000-body scale are probe figures, taken under load.
  The test time the lane's streams add to a SOUR launch is a review figure for the thrown
  scene alone; the three smaller spike scenes, the backend's scenes, the headless run and
  the wall clock under nextest were not timed.
- The first review ran the backend's three streams and the spike's four in the modes it
  tried and found each equal, the prediction scene's after `corrected` among them. The
  spike's streams under the 64-row layout and chaos seeds 2 and 3, the headless run's stream
  and the comparison across platforms are the lane's and its nightly's to show.
- A by-product for the row "Float `%` inside the boundary": the trace saw no `fmod` from a
  step in any recorded scene or backend test. The row stays as it is.

## Appendix: the probes

They are kept, with their traces, in the coordinator's scratch area under
`planner-physics-probes`, and are not part of the tree.

The trace is a dynamic library loaded with `DYLD_INSERT_LIBRARIES`. For each of `acos`,
`asin`, `atan2`, `cos`, `exp`, `fmod`, `log2`, `pow` and `sin` it holds a wrapper in the
`__DATA,__interpose` section that takes `backtrace()`, counts the call under its return
addresses and calls the real function. At exit it writes each distinct chain with its count
and the executable's load address, and `atos -i` names the frames from the test binary's
debug symbols (`dsymutil`). With a function named in an environment variable the wrapper
scales its result by 1 + 1e-6, which is the perturbation of section 1.4.

```
cargo test -p lactose-physics-rapier --test sour --no-run
cargo test -p lactose-app --test spike --test headless --no-run
DYLD_INSERT_LIBRARIES=<interposer> PLANNER_PHYSICS_TRACE_OUT=<file> \
    <test binary> --exact <test> --test-threads=1
```

The leaf-count arithmetic is section 1.3's three lines evaluated for each count, with the
distance of the rounded expression from the nearest half-integer at or above one half.

The transition probe, the cost probe and the two mutations ran in a scratch copy of the
tree. The transition probe is a test of `lactose-physics-rapier` that creates a body with a
`RigidBody` alone, steps five ticks, sets a cuboid collider offset by (0.3, 0.2, 0.1) and
turned 0.7 rad about (1, 2, 3), and steps to tick 180. The cost probe writes and hashes the
backend's bytes of `tests/support`'s pile of 2,000, 21 times each, median. The mutations add
to `snapshot::read`, after the world is built, a `wake_up` of every body, or a `set_shape` of
every collider with its own shape.
