# Proposal: physics is a function of history, and what prediction compares

Status: accepted by the owner on 2026-10-11 after three design reviews, as D124: the
recommendation below as decided, with the tolerance defaults kept at 2e-3 m, 2e-3 per
quaternion component, 2e-2 m/s and 2e-2 rad/s and the milestone 1 spike deciding any
doubling. The amendments to `docs/architecture.md` sections 1, 2, 9.4, 10, 15.5, 16 and 22,
the README amendments, the decision entry and the verification rows are applied by the
merge's doc-edit script (`merge-docs-d124.py`, against main f70c25f); until it runs, the
sections' "at full precision" and D122's "no tolerance" stand in the tree. The
`lactose-predict` implementation lane, running against D122 at 8a744fd, takes the changes
listed at the end, which are local to its comparison module and the write set that reuses
it.

## The problem

`lactose-physics-rapier` gives results that are a function of the whole row history, not of
the rows (D119, "History"; `crates/physics-rapier/README.md`;
`crates/physics-rapier/tests/determinism.rs`). The server's own determinism is intact,
because a replay and a re-simulation repeat the history. The client's prediction world does
not repeat it: it holds the local player, the claimed entities, the relevant entities as
kinematic proxies and the static collision, reached by its own path (architecture section
9.4; `crates/predict/README.md`, "The hook"). So the solver-written rows of a claimed
dynamic body on the two machines are not bit-equal in general, however good the inputs, and
D122's rule that any difference is a misprediction would correct what is not one.

Three questions follow, and this proposal decides each.

1. What prediction compares when it decides a correction.
2. Whether the backend must canonicalise its results per island.
3. Whether SOUR and the journal need anything more for the server's own determinism.

## What was measured

The backend was stepped directly through `PhysicsBackend`, as its own tests do (release,
Apple M1, one solver thread unless stated), and, for the reload paths, through
`physics-api`'s real sync with the Rapier backend. Rows are modelled as the write-back keeps
them, each body's last reported state carried forward, and the walk driver reads the rows,
corrected ones included; the first round's driver read the client's own previous step
instead, which the first review caught (its lone-capsule "bit-equal again" was that
artefact). Differences are per scalar component, as the proposed comparison takes them. The
planner's probes are `probe_history.rs` and `probe_d124_r2.rs` in the scratchpad, the
reviewer's `probe_d124.rs` (p1 to p14), `probe_d124_r2_calls.rs` and
`probe_d124_r2_reload.rs` beside them; none is committed.

**Where the difference comes from.** Three scenes, 1,800 ticks: a dynamic capsule with
rotation locked, walked by velocity writes in two-second legs along x and z with rests,
drifting across a floor (z from -3 to 12 m) with five static boxes it never reaches alone and
brushes for a few dozen ticks in the other two scenes; the same capsule pushing a dynamic
crate 4 m; the same capsule bumping a stack of three crates, which moves from tick 48 to
339. Each alone and then beside far-away awake bodies: a forty-ball pile a kilometre away
created before, created at tick 100, left awake throughout by a kinematic paddle sweeping
through it, and eight far capsules walked throughout; and after an arena history (bodies
created, stepped once and removed, their slots retaken at tick 40).

| Scene, against | Bits differ? |
|---|---|
| Capsule alone, crate, stack: far bodies before, at tick 100, awake throughout, far walkers, arena history | No, in every variant (planner's probe; reviewer's p2) |
| Capsule pressing two crates against a static wall for 1,800 ticks, far bodies awake throughout | No (reviewer's p8) |
| Fifty bodies thrown into one area and interacting (16 to 43 moving at a time), 900 ticks, far pile awake throughout | No (reviewer's p7) |
| Fifty balls 2.5 m apart given impulses, 900 ticks, far pile before | No |
| A kinematic capsule swept along a curve with a shape cast and a ray each tick | No, and every query answer equal |
| The 150-body pile with two joints and a sweeping paddle of `tests/determinism.rs`, far pile before | Yes: 2.7 mm at tick 238, 12 mm at 250, 38 at 300, 96 at 350, 346 at 400 |
| A fresh backend built from the rows at tick *t0* against the original continuing | Yes from the first tick: crate 1.5 mm, stack mid-bump 1.6 m (the top crate topples differently), stack at rest 1.9 mm (reviewer's p5; planner's r2) |
| Through the backend alone: every body removed and recreated in key order on the same solver, against a fresh backend built the same way | Yes from the first tick: crate 0.4 mm, stack 0.12 m, the pile 7.6 m |
| Through the backend alone: the solver restored to the bytes it had empty at setup, then the bodies recreated in key order, against the fresh backend | No, in every case, the pile included |
| Through `physics-api`'s sync: `sim`'s in-place reload (every region unloaded and loaded in one frame, the path a client's full frame takes too) against a fresh process | Yes from the first tick, and equal to the untouched twin: the sync reads `Despawned` then `Spawned` for each key with the row present and removes nothing, so the solver keeps its whole state and the derived handle rows are lost. A floor, a pusher and a three-crate stack: 3.9e-3, 1.1e-2 and 1.0e-2 m at *t0* = 30, 60, 120; a 75-body pile with two joints across two regions: 1.24 and 1.40 m (reviewer's `probe_d124_r2_reload.rs`) |
| Through the sync: unload in one frame and load in the next, the setup bytes restored when the solver empties | No (stack and pile) |
| Through the sync, the trigger of item 7 below: a sync whose log despawned every body the solver held removes them, restores the setup bytes and creates from the rows by a full diff | No, on every path, scene and *t0*; handle rows present (reviewer's candidate, `candidate-fix.diff`) |

So the far-body and arena dependence of D119 is real but confined to large contact graphs
and to a solver that has freed its slots; in every scene of a few bodies it changed nothing
even with far bodies awake throughout. What the prediction world meets instead is solver
state the rows do not carry: warm-start impulses, contact manifolds and sleep counters.
Two backends with equal rows and different solver state diverge while bodies are in
contact, and the client's backend reaches such a state by three paths: a correction (rows
written, solver state kept), a claim of a body in contact (a kinematic proxy made dynamic),
and a load or rebuild (a fresh solver). None of these is cured by a canonical solver. The
planner's first measurement of the reload bypassed the sync, which is why its trigger was
wrong in the previous revision; the sync's own path is what the rows above record.

**Corrections, at 20 Hz frames.** The client mispredicts at tick *p* (its capsule written a
centimetre along x and 0.1 m/s faster) and from *c* = *p* + 6 compares a server frame every
three ticks for 200 frames, under three rules: D122's (any bit difference is a mismatch,
every fact written at the rollback), the first round's (a mismatch is a leaf beyond the
tolerance, only the mismatching components written), and the rule now proposed (a mismatch
is a leaf beyond the tolerance, every fact at the rollback tick written, as D122 writes).
Tolerance 2e-3 m, 2e-3 per quaternion component, 2e-2 m/s, 2e-2 rad/s. The probe's
correction pushes every key's rows, bit-equal ones included, which wakes bodies the shipped
sync leaves alone (it skips a value equal to the last write-back's), and it models no
rollback; the spike re-measures on the shipped path.

| Scene | *p* | Bit-exact | Tolerance, write beyond | Tolerance, write all |
|---|---|---|---|---|
| Lone capsule | 40, 60, 100, 125 (a jump), 300 | 36, 29, 16, 8, 29 | 1 each | 1 each |
| Capsule and crate | 40, 60, 100, 125, 300 | 200, 200, 200, 7, 49 | 2, 1, 1, 1, 1 | 2, 1, 1, 1, 1 |
| Capsule and stack | 40, 60, 100, 125, 300 | 200, 200, 200, 8, 28 | 18, 34, 1, 1, 1 | 7, 16, 1, 1, 1 |
| A ball rolled at a crate, a crate tipped over, a second ball (reviewer's p9 "tumble") | 20, 40, 80 | 1, 1, 200 | 1 each | 1 each |

The largest difference the tolerance absorbed: the lone capsule 2.4e-5 m (a jump landing),
the capsule and crate 7.5e-4 m and 5.3e-3 m/s, the tumble 1.2e-3 m, 1.1e-3 per quaternion
component and 3.4e-3 rad/s, the stack up to the tolerance itself while it was moving. A
corrected body is never bit-equal to the server again while it is in contact: the lone
capsule differed in 870 of 870 ticks after its correction, by 2.7e-5 m at most.

**A claim started in contact**, no misprediction at all: the crates are kinematic proxies
placed at the server's previous-tick rows, made dynamic with the server's rows at the claim
tick, then compared at 20 Hz for 167 frames.

| Scene | Claim tick | Corrections, write beyond | Corrections, write all | Last correction |
|---|---|---|---|---|
| Capsule and crate | 50, 60, 100, 200, 400 | 2, 1, 1, 1, 1 | 2, 1, 1, 1, 1 | the claim's own frame |
| Capsule and stack, moving | 50, 60, 100, 200 | 44, 36, 32, 37 | 26, 31, 22, 35 | ticks 291, 217, 254, 306 |
| Capsule and stack, at rest | 400 | 1 | 1 | the claim's own frame |

The crate claimed at tick 50 absorbed 1.9e-3 m in the frames after its one correction.

A derived replicated integer (the capsule's x floored to a centimetre or a millimetre)
compared bit for bit beside the tolerance: under the first round's rule 72 to 83
corrections in 298 frames, since the correction wrote the integer and not the absorbed
`Transform` it was recomputed from; under write-all, one (reviewer's p13).

Numbers cited rather than measured: a rebuild of Rapier's sets every step removed the
history dependence in every case the backend lane tried at about 1.3 ms a step plus a
cold-contact step of 3.7 ms against 1.2 ms at 2,000 bodies, and left the dependence on far
bodies present; a rebuild on membership change moved the dependence rather than removing it
(D119, "History"). The physics phase's budget is 3 ms at 2,000 awake bodies (section 22).

Two defects found by reading, independent of the tolerance. First: `physics-rapier` reports
a body whose transform was written since the last step as `teleported` (`set_transform`
places it through `set_position` and adds it to the step's `placed` set), and
`physics-api`'s write-back then writes the tick into `Teleported`, a persistent replicated
component. A correction writes `Transform`, `corrected` makes the sync push
`set_transform`, so the re-simulated tick marks the client's row teleported while the
server's row is unchanged; the next frame's exact `Teleported` fact mismatches and D122
corrects again, one frame later, for every corrected `Transform`. No boundary system reads
`Teleported`: the write-back writes it, extract and the gather compare it with the last
tick they covered, and `net-host`'s interpolation reads it outside the boundary. Second,
found by the second review: on a same-frame unload and load the sync sees `Despawned` then
`Spawned` for a key it holds and does nothing, so every `BodyHandle` and `JointHandle` row
is gone afterwards while the bookkeeping still holds the body, against the README's
"present exactly while the row is in the solver"; a partial same-frame region reload loses
them the same way.

One risk found by the first reviewer, not this proposal's to fix: on a four-thread solver
pool the lone-capsule scene took 20 to 100 ms per 1,800-tick run in most runs, 41 of 300
runs took 0.4 to 4.4 s, and the worst step was 39 ms, against 19 to 30 ms per run in most
runs on one thread (319 ms in the worst, a worst step of 14.5 ms); and it once hung in
Rapier's deferred
broad-phase optimisation (a worker blocked in `join_deferred_bvh_optimize` on a
`rayon::spawn` that never ran). The physics-rapier lane carries the D123 patch (always the
inline path). Until it lands the prediction world's pool is one thread, which its scene
size warrants anyway, and the spike's server carries a step watchdog.

## The options

**Canonical results per island in the backend.** Solve each island separately in a
canonical order so that a body's bits depend on its island's rows alone. The pinned Rapier
solves all awake bodies as one island with constraint order from world-wide structures
(D119); changing that is not a fifty-line patch, so it is a fork or another engine, which
D123 and the milestone plan rule out. It would also cure none of the three paths above:
solver state the rows do not carry differs after a correction, a claim in contact and a
load whatever the solve order, and an island holding a proxy, kinematic on the client and
dynamic on the server, differs whatever the order, which section 9.4 already expects.
Costs a physics engine; fixes the pile and nothing the prediction world meets.

**Rebuild the backend's sets every step.** Measured by the backend lane: 1.3 ms a step plus
a cold-contact step at 2,000 bodies, over the 3 ms phase budget with the step itself, and
it discards exactly the solver state whose absence is the divergence, every step. Rules out
the phase budget and converges nothing.

**Authority-only physics facts.** Never compare the solver-written rows of a claimed body;
apply the server's values as they arrive. A claimed dynamic body then has no prediction for
its motion and is a proxy half a round trip late; a thrown object is presented where it was,
not where it is. Rules out predicting thrown and carried objects, which the reference game
and the spike's fifty claimed bodies need.

**Keep D122's bit-exact rule and count the storm.** No code beyond D122. The correction
table says what it costs: a lone body on the ground 8 to 36 rollbacks per misprediction,
a body pushing a crate 7 to 200, every frame while a contact lasts. Milestone 4's criterion
of fewer than one correction a minute is then unreachable for any dynamic character, and
the re-simulation budget is spent on corrections nobody can see. Rules out the milestone 4
criterion for dynamic bodies.

**A tolerance that gates the rollback, with D122's rollback kept.** The recommendation,
below. Costs a comparison outcome and four scalars in `lactose-predict`, nothing in the
backend; one correction per misprediction for a lone body and for bodies in contact with
statics or one crate; 7 to 35 corrections while a stack of claimed crates is moving, which
no option above improves, since the divergence there is physical and must be corrected.

## The recommendation

The backend keeps its history dependence and the comparison keeps it out. Concretely:

1. A fact's `Transform` or `Velocity` component agrees when its canonical bytes equal the
   record's, as every component does; failing that, for a claimed entity whose `RigidBody`
   is `Dynamic` at slot *T* or at slot *T* - 1 (the fact's `RigidBody` if the frame carries
   one, else the record's; *T* alone when the record has no slot *T* - 1 for the entity, its
   first claimed slot or the slot after a resync), it agrees when every scalar leaf is
   within `PredictionTolerance` for its kind: 2e-3 m per translation component, 2e-3 per
   quaternion component, 2e-2 m/s and 2e-2 rad/s per velocity component by default. The
   margins over the largest absorbed difference measured are 80 times for a lone body on
   static geometry (2.4e-5 m), 2.7 times for a body pushing one crate (7.5e-4 m), 1.7 and
   1.8 times for a rolled ball (1.2e-3 m, 1.1e-3 per quaternion component), 1.05 times for
   a crate claimed mid-push (1.9e-3 m), and none for a moving stack. The defaults therefore
   bound what is corrected rather than guarantee an absorbed residual: a residual at the
   margin costs one correction and not a storm as long as the next frames' residual falls
   below it, which it did in every single-contact case measured, and a storm is the
   multi-body case of item 6 whatever the default within an order of magnitude. Every
   other field, kinematic and fixed bodies' rows included, agrees by canonical bytes only. A
   NaN leaf agrees with an equal NaN and with nothing else. The defaults are set under the
   64-bit scalar; the milestone 1 scalar decision re-measures them, since a 32-bit scalar's
   spacing at the map edge is of their order, and the milestone 1 spike re-sets them: a
   default whose single-contact absorbed maximum in the spike exceeds half of it is
   doubled. The probes' figures already meet that condition for translation (1.9e-3 and
   1.2e-3 m against 1e-3) and rotation (1.1e-3 per quaternion component against 1e-3), so if
   the spike reproduces them those two defaults go to 4e-3; the owner kept 2e-3 so that the
   spike measures the shipped path first, whose sync pushes fewer rows than the probes did.
   On a 1440p display with a 60 degree field of view a pixel at five metres is 4.0e-3 m, so
   2e-3 m of translation is half a pixel there and 4e-3 is a pixel; 2e-3 per quaternion
   component is about 4e-3 rad, 2 mm at the rim of a body of 0.5 m radius, and 4e-3 per
   component 4 mm. The facts that would change the values now: a visible jump at a
   correction below a default.
2. The tolerance gates the rollback and nothing else. A frame makes a correction point when
   any fact does not agree; the rollback then writes every fact at *k* exactly as D122 says,
   the absorbed ones included, re-takes the slot and re-simulates. Writing the absorbed
   facts too is what converges: a moving stack 7 to 16 corrections against 18 to 34, a stack
   claimed mid-bump 22 to 35 against 32 to 44, a derived integer 1 against 72 to 83.
3. `Teleported` of a claimed entity is never a fact: not recorded, not compared, not written
   by a correction. It is the client's own no-interpolate flag, set by its own steps and its
   own corrections. A proxy's `Teleported` is written by the hook as its position for the
   tick, as before.
4. A replicated field a game derives from a solver-written value compares by canonical
   bytes like every other field and is corrected when it differs; the correction converges
   because the rollback writes every fact at *k*. A derived replicated scalar carries the
   solver's rounding and so mismatches on every frame while a contact lasts; a game keeps
   derived replicated state integer (hit points, a grounded flag), and the build check
   cannot see it, so section 9.4 states it as a rule of authoring.
5. `PredictionStats` records what the tolerance absorbed: the largest absorbed difference
   per kind as canonical bits and the count of absorbed fields, so milestone 4 re-sets the
   defaults from the spike, not from this probe. The spike runs the character as a dynamic
   capsule and as a kinematic body, its fifty claimed bodies thrown and interacting, and
   claims a body in contact (a crate being pushed, a stack being bumped), which is the case
   that storms.
6. Claimed dynamic bodies in active contact with each other are corrected every few frames
   until they settle, under any rule: the divergence there is physical (a stack claimed
   mid-bump differs by centimetres and 0.1 m/s, a stack corrected mid-bump by 1.6 m without
   the later frames). Measured on the three-crate stack: 7 to 16 corrections in 200 frames
   after a misprediction while it moves and one after it settled, 22 to 35 in 167 frames
   after a claim mid-bump and one after a claim at rest. The lever is the claim, not the
   solver: `net`'s claim policy at milestone 4 claims a thrown object with its throw and
   releases it once it rests or touches an unclaimed body, and never claims a body that is
   in contact with another dynamic one. The fact that would change this: a game scene that
   needs a moving pile of claimed bodies for more than a few seconds.
7. An empty solver is a fresh solver. `physics-api` keeps the bytes the backend wrote at
   `setup`. A sync whose structural log since its last pass, the pending entries a restore
   carried included, despawned every body the solver held removes them all, restores those
   bytes and creates from the rows by a full diff in ascending key order; a sync whose
   removals leave the solver empty restores the bytes and makes the next sync a full diff.
   The first is the path of `sim`'s in-place reload and of a client's full frame (every
   region unloaded and loaded in one frame, which the sync sees as `Despawned` then
   `Spawned` per key and today leaves untouched), the second is an unload followed by a
   load. A world reloaded either way then has the physics state a fresh process loading the
   same rows has, bit for bit: measured through the sync on a three-crate stack mid-bump at
   three ticks and a 75-body pile with two joints across two regions, where the sync as it
   is today differed from the first tick by 3.9e-3 to 1.1e-2 m and 1.24 to 1.40 m. The same
   fix round re-inserts the `BodyHandle` or `JointHandle` row of a `Spawned` entry for a key
   the bookkeeping holds, since a same-frame reload, whole or of one region, loses them
   today. Section 15.5's first test holds with physics only under this rule; D119's "never
   clears the solver" narrows to a solver whose log kept a body. The facts that would change
   this: the full diff's cost on a server-sized reload, a one-off.
8. The server's determinism needs nothing from SOUR: its modes vary threads, layout and
   schedule under one row history, which is where bit equality holds and what the backend's
   tests already show. The journal does need one thing: a checkpoint carries the physics
   snapshot, because a replay whose backend starts fresh hashes equal only until the first
   contact after the checkpoint (the fresh-against-continued row above). `PhysicsSnapshot`
   today carries an `Arc<()>` identity compared by pointer
   (`crates/physics-api/src/service.rs`), fit for the ring and not for a file, and `sim`
   cannot name `physics-api`; so `journal::Journal` gains `services`, a map from a service
   key to opaque bytes, where the key is the xxh64 of the service's declared name
   (`"physics_api::Physics"`), collision-checked as a `SystemKey` is and never a `TypeId`;
   `physics-api` gains the byte form of its snapshot, which carries a format version, the
   backend's identity (the backend's name, the Rapier version and the hash of the D123
   patch set under `vendor/`), the `PhysicsConfig`, the tick, the backend's bytes and the
   bookkeeping, and a restore of it into a fresh `Physics` of a world at the checkpoint's
   tick that refuses a mismatch of any of those with `InvalidData` and rebuilds the
   `BodyHandle` and `JointHandle` rows from the bookkeeping, since rows loaded from a
   checkpoint carry none; the replay driver hands the bytes back after the regions load and
   before the first step. `sim` never reads the bytes and the journal is not hashed state.
   Until the byte form exists a replay is bit-exact from a session's start only. The fact
   that would change this: the snapshot's size at the milestone 1 scale, about 1.2 kB per
   fixed collider today.

Cost: the predict implementation lane absorbs items 1 to 5 as a change to its comparison
module and the write set that reuses it, about a quarter of a run and no extra review
round; item 7 is a `physics-api` fix round, about a third of a run; item 8 is the journal
tooling lane's, about half a run across `physics-api` and `sim`; the spike's
claim-in-contact and character scenes about half a run; the document edits are the
coordinator's.

## Architecture amendments (exact text)

### §1 Goals, item 2, replace the sentence "It does not by itself make client prediction correction-free, because the client's prediction world differs from the server's world."

It does not by itself make client prediction correction-free, because the client's
prediction world differs from the server's world and its physics solver holds state the rows
do not carry (section 16).

### §2 Decisions, Netcode row, replace "Local-player prediction in a separate client prediction world, reconciled at full precision."

Local-player prediction in a separate client prediction world, reconciled at full precision
within a stated tolerance on the solver-written rows of a dynamic body.

### §2 Decisions, Determinism scope row, replace "Reconciliation compares at full precision (section 9.4)."

Reconciliation compares at full precision, with a stated tolerance on the solver-written
rows of a dynamic body (section 9.4).

### §9.4 Prediction, replace the sentence "Rapier's internal solver state is not corrected, so a corrected dynamic object converges over a few ticks rather than exactly."

Every field agrees by canonical bytes; the `Transform` and `Velocity` the physics write-back
writes on a dynamic body also agree when each scalar is within a stated tolerance
(`lactose-predict`'s, two millimetres, two thousandths of a quaternion component, two
centimetres per second and two hundredths of a radian per second by default), because the
solver holds state the rows do not carry (section 16) and it is not corrected, so a
corrected dynamic body differs from the server by rounding for as long as it is in contact.
The tolerance decides whether a frame makes a correction; a correction writes every fact at
its tick, the absorbed ones included, which is what converges. `Teleported` of a claimed
entity is the client's own no-interpolate flag and is never compared or corrected. A
replicated field a game derives from a solver-written value compares by canonical bytes
and is corrected when it differs, so a game keeps derived replicated state integer. Claimed
dynamic bodies in contact with each other are corrected every few frames until they settle,
so the claim policy claims a thrown object with its throw and releases it at rest.

### §10 Snapshots, replace "The server never rolls back. Save snapshots copy persistent columns only (section 15.3)."

The server never rolls back. Save snapshots copy persistent columns only (section 15.3); a
journal checkpoint carries the physics snapshot beside them (section 15.5).

### §15.5 Enforcement, replace "Journal replay (checkpoint plus input journal) is a debug and replay tool, not the save system."

Journal replay (checkpoint plus input journal) is a debug and replay tool, not the save
system. A checkpoint carries the physics snapshot of section 10 as bytes, since the solver
holds state the rows do not carry (section 16) and a replay from a checkpoint without it
hashes equal only until the first contact; a replay from a session's start needs none.

### §16 Physics, append after "Milestone 1 measures the physics phase's share of the tick and the stall it imposes on render recording during the overlap."

The solver holds state the rows do not carry: warm-start impulses, contact manifolds and
sleep counters. Two worlds with equal rows and different solver state diverge while bodies
are in contact, by rounding for a body on static geometry and by whole outcomes for bodies
in contact with each other, and a client's solver reaches such a state after a correction, a
claim of a body in contact and a rebuild. The backend's results are also a function of the
world's row history (D119): with one awake island and world-wide constraint order, far-away
awake bodies and freed arena slots can change a body's bits in a large contact graph (a
150-body pile; nothing in scenes of a few or fifty bodies). Bit equality therefore holds
between two worlds only when their row histories and solver states are identical, the
server and its replay or a run and its re-simulation, never the server and a client's
prediction world. The backend does not canonicalise per island: the pinned Rapier cannot,
and no solve order carries the solver state across. The difference is kept out of the
comparison instead (section 9.4). An empty solver is a fresh solver: `physics-api` keeps the
bytes the backend wrote at setup; a sync whose structural log since its last pass, the
pending entries a restore carried included, despawned every body the solver held removes
them all, restores those bytes and creates from the rows by a full diff in ascending key
order; a sync whose removals leave the solver empty restores the bytes and makes the next
sync a full diff. So a world reloaded in place, in one frame or two, has the physics state a
fresh process loading the same rows has, which is what section 15.5's first test needs.

### §22 Milestones, row 1, replace "one predicted character, fifty claimed dynamic bodies; correction rate, rollback cost per tick and divergence recorded as the baseline that milestone 4's threshold is judged against."

one predicted character, run as a dynamic capsule and as a kinematic body, fifty claimed
dynamic bodies thrown and interacting, and a claim of a body in contact; correction rate
under the section 9.4 tolerance, the largest difference the tolerance absorbed per kind,
rollback cost per tick and divergence recorded as the baseline that milestone 4's threshold
is judged against.

## README amendments (exact text)

### `crates/predict/README.md`, "Surface", add a row after the `Reconciled` row

| `PredictionTolerance`, `Predictor::set_tolerance` | How far a dynamic body's solver-written `Transform` and `Velocity` may differ from the server's and still agree: a scalar per kind, defaults in the type, set by `app` between runs |

### `crates/predict/README.md`, "Hidden plumbing", replace "Everything public is keys, ticks and counts; nothing reads a clock or computes a float, and the crate carries the boundary `clippy.toml`."

Everything public is keys, ticks, counts and the tolerance's scalars; nothing reads a
clock, every float goes through `lactose_math`, and the crate carries the boundary
`clippy.toml`.

### `crates/predict/README.md`, "Reconciliation", replace "Full precision, no tolerance: the same build on the same bytes produces the same bits, so any difference is a different input, which is what a misprediction is."

Full precision, with one exception: a `Transform` or `Velocity` whose canonical bytes
differ still agrees, for an entity whose `RigidBody` is `Dynamic` at slot *T* or at slot
*T* - 1 (the fact's `RigidBody` if the frame carries one, else the record's; *T* alone when
the record has no slot *T* - 1 for the entity), when every scalar leaf is within the
predictor's `PredictionTolerance` for its kind (translation, rotation, linear, angular),
since the physics solver holds state the rows do not carry and the client's solver reached
its state by another path (architecture section 16, D124); a NaN leaf agrees with an equal
NaN and with nothing else. Every other field, the rows of kinematic and fixed bodies
included, agrees only by canonical bytes, since the same build on the same bytes produces
the same bits. `Teleported` of a claimed entity is the client's own no-interpolate flag:
never recorded, never compared, never written. The tolerance decides whether the frame
makes a correction point and nothing else: a rollback writes every fact at *k*, the
absorbed ones included. A field within tolerance that is not bit-equal is counted in
`PredictionStats::absorbed`, with the largest difference per kind as canonical bits, and is
not a mismatch.

### `crates/predict/README.md`, "Reconciliation", replace the bullet "Rapier's internal solver state is not corrected (section 9.4), so a corrected dynamic body converges over a few ticks rather than exactly: the frames of those ticks differ again by small amounts, each a correction under the rule above. The spike's correction rate counts them; whether milestone 4 adds a tolerance for dynamic bodies is decided against that baseline, never silently here."

- Rapier's internal solver state is not corrected (section 9.4), so a corrected dynamic
  body's rows differ from the server's by rounding for as long as it is in contact: a body
  on static geometry or pushing one crate by up to 7.5e-4 m and 5.3e-3 m/s, which the
  tolerance absorbs (one correction per misprediction against 8 to 200 without it); claimed
  bodies in contact with each other by whole outcomes, so a three-crate stack of claimed
  crates was corrected 7 to 16 times in 200 frames after a misprediction while it moved,
  22 to 35 times in 167 frames after a claim mid-bump, and once at rest (D124). Writing
  every fact at *k*, the absorbed ones included, is what makes those corrections converge.
  The spike records the corrections and the largest absorbed difference per kind, and
  milestone 4 re-sets the defaults from them; `net`'s claim policy claims a thrown object
  with its throw and releases it at rest.

### `crates/predict/README.md`, "Not in this crate yet", replace "- A tolerance for dynamic bodies, decided at milestone 4 against the spike's baseline."

- The tolerance's defaults re-set from the spike's `absorbed` figures and from the scalar
  decision, and `net`'s release of a thrown object at rest: milestone 1 and 4.

### `crates/predict/README.md`, "Proof for milestone 1", "The comparison" row, append

; a dynamic body's `Transform` within tolerance agrees and the frame makes no correction,
one past it is a mismatch, a kinematic body's agrees by bytes only, a body dynamic at *T* - 1
and kinematic at *T* takes the tolerance, one with no slot *T* - 1 is judged at *T*, a NaN
leaf agrees with an equal NaN only, `Teleported` differing is not a mismatch and is not
written at a correction, a rollback writes the absorbed facts too (the physics test backend
records the pushes), and `absorbed` holds the largest absorbed difference per kind

### `crates/physics-api/README.md`, "Bodies", replace "It never clears the solver: sleep and warm starts survive wherever the rows agree."

It never clears a solver whose log kept a body: sleep and warm starts survive wherever the
rows agree. This crate keeps the bytes the backend wrote at `setup`; a sync whose
structural log since its last pass, the pending entries a restore carried included,
despawned every body the solver held removes them all, restores those bytes and creates
from the rows by a full diff in ascending key order; a sync whose removals leave the solver
empty restores the bytes and makes the next sync a full diff. So an empty solver is a fresh
solver, and a world reloaded in place, in one frame (every
region unloaded and loaded, which the log shows as `Despawned` then `Spawned` per key) or in
two, has the physics state a fresh process loading the same rows has, bit for bit (D124;
architecture section 15.5's first test). A `Spawned` entry for a key the bookkeeping holds
re-inserts its handle row, so a same-frame reload of a region keeps `BodyHandle` and
`JointHandle` present exactly while the row is in the solver.

### `crates/physics-api/README.md`, "Snapshot and restore", replace "Rapier's internal solver state is not corrected (architecture section 9.4), so a corrected dynamic body converges over a few ticks."

Rapier's internal solver state is not corrected (architecture section 9.4), so a corrected
dynamic body's rows differ from the server's by rounding for as long as it is in contact,
up to 7.5e-4 m for a body on static geometry or pushing one crate and by whole outcomes for
bodies in contact with each other; `lactose-predict`'s tolerance decides whether to roll
back and its rollback writes every fact (D124). A corrected `Transform` is pushed as a
placement, so the re-simulated step reports the body `teleported` and the write-back sets
the client's `Teleported`, which `lactose-predict` therefore never compares. A journal
checkpoint carries this snapshot as bytes: a format version, the backend's identity (its
name, the Rapier version and the hash of the D123 patch set under `vendor/`), the
`PhysicsConfig`, the tick, the backend's bytes and the bookkeeping; a fresh process
restores them into a `Physics` of a world at the checkpoint's tick, refusing a mismatch of
any of those with `InvalidData` and rebuilding the handle rows from the bookkeeping, since
loaded rows carry none. The byte form and the restore are the journal tooling lane's.

### `crates/physics-rapier/README.md`, "Calls are recorded, the step applies them", append after "two worlds holding different body sets or histories are not bit-equal for the bodies they share."

The history dependence is a property of large contact graphs and of a solver that has freed
its slots, not of every body: a dynamic capsule on static geometry, one pushing a crate, one
bumping a three-crate stack, one pressing two crates against a wall and fifty thrown bodies
interacting were bit-equal over 900 to 1,800 ticks with far awake bodies present, arriving
mid-run and awake throughout; the 150-body pile first differed by 2.7 mm at tick 238 and
by 346 mm at tick 400, and every body removed and recreated on the same solver differed
from a fresh solver by 0.4 mm to 7.6 m (D124). What a prediction world meets is the solver
state the rows do not carry (warm starts, manifolds, sleep), which `lactose-predict`
compares within a tolerance; the backend does not canonicalise.

### `crates/sim/README.md`, "Journal replay", replace "the untaken `PhysicsCommands`, `World::channel`, are dropped at a checkpoint, as solver state is),"

and `services`, a map from a service key (the xxh64 of the service's declared name,
collision-checked as a system key is) to the opaque bytes `app` took from that service at
the checkpoint, which the replay driver hands back after the regions load and before the
first step: the physics snapshot of `lactose-physics-api` goes there, the backend's whole
world and the sync's pending work with the untaken `PhysicsCommands`, since the solver
holds state the rows do not carry and a replay without it hashes equal only until the first
contact; this crate never reads the bytes and they are not hashed state; until the byte form
exists, D124, a replay is bit-exact from a session's start only),

## Decision entry (exact text; number at merge)

**D124. The physics solver holds state the rows do not carry: reconciliation compares a
dynamic body's solver-written rows within a tolerance that gates the rollback, `Teleported`
of a claimed entity is never compared, an empty solver is a fresh solver, and a journal
checkpoint carries the physics snapshot** (§1, §2, §9.4, §10, §15.5, §16, §22,
`lactose-predict`, `lactose-physics-api`, `lactose-physics-rapier`, `lactose-sim`)
- Why: the client's prediction world is not bit-equal to the server for a dynamic body
  however good the inputs, and D122's "no tolerance" would correct what is not a
  misprediction and then correct its own corrections. The source is not, in the scenes
  prediction meets, the far-body and arena history of D119 (a capsule on static geometry,
  one pushing a crate, one bumping a stack, one pressing crates against a wall and fifty
  thrown interacting bodies were bit-equal with far bodies awake throughout; the 150-body
  pile and a solver that has freed its slots were not); it is the solver state the rows do
  not carry, warm starts, manifolds and sleep, which the client's solver reaches by another
  path after a correction, a claim in contact and a rebuild. The planner's and the
  reviewers' probes (`docs/verification.md`) bound it: after a correction a body on static
  geometry or pushing one crate differs by up to 7.5e-4 m for as long as it is in contact,
  and bodies in contact with each other by whole outcomes; a fresh solver against the
  continued one by 1.5 mm to 1.6 m; at 20 Hz a bit-exact rule costs 8 to 200 rollbacks per
  misprediction and the tolerance one, or 7 to 16 after a misprediction while a stack of
  claimed crates moves and 22 to 35 after a claim mid-bump.
  - *The backend keeps its history dependence.* Canonical results per island need Rapier
    to solve islands separately in a canonical order, which the pinned version cannot and
    no fifty-line patch adds (D123), and no solve order carries solver state across a
    correction, a claim or a rebuild; a rebuild every step discards exactly that state at
    1.3 ms a step plus a cold-contact step (D119). Rules out canonicalisation in
    `physics-rapier`, a second engine and a physics world per island.
  - *A tolerance on the solver-written rows of a dynamic body gates the rollback; the
    rollback is D122's.* `Transform` and `Velocity` of a claimed entity whose `RigidBody`
    is `Dynamic` at the compared slot or the one before (the compared slot alone when the
    record has no earlier one) agree by canonical bytes or, failing that, when each scalar
    leaf is within `PredictionTolerance` (2e-3 m, 2e-3 per quaternion component, 2e-2 m/s
    and 2e-2 rad/s by default, under a pixel at five metres; `app` sets them through
    `Predictor::set_tolerance`). The measured margins over the largest absorbed residual
    are 80 times for a lone body, 2.7 for a body pushing one crate, 1.7 and 1.8 for a
    rolled ball, 1.05 for a crate claimed mid-push and none for a moving stack, so the
    defaults bound what is corrected rather than guarantee an absorbed residual; a residual
    at the margin costs one correction, not a storm, while the next frames fall below it,
    which every single-contact case did; the scalar decision re-measures the defaults and
    the spike doubles one whose single-contact absorbed maximum exceeds half of it, a
    condition the probes already meet for translation and rotation, so those two are
    expected to go to 4e-3 (a pixel at five metres on a 1440p display at 60 degrees of
    view, and 4 mm at the rim of a body of 0.5 m radius); the owner kept 2e-3 so that the
    spike measures the shipped path first. Every
    other field, kinematic and fixed bodies' rows included, agrees by canonical bytes only,
    and a NaN leaf agrees with an equal NaN and nothing else. A frame makes a correction
    point when any fact does not agree, and the rollback then writes every fact at *k*,
    the absorbed ones included, as D122 says: writing them is what converges (a moving
    stack 7 to 16 corrections against 18 to 34 when only mismatching components were
    written, a derived integer 1 against 72 to 83). Rules out a tolerance on every field, a
    tolerance on discrete fields, a rollback that leaves absorbed facts as predicted, an
    authority-only rule that applies physics facts without comparing them, and a quantised
    comparison grid.
  - *`Teleported` of a claimed entity is never recorded, compared or corrected.* It is the
    client's own no-interpolate flag: `physics-rapier` reports a body whose transform was
    written as `teleported` and the write-back sets the tick, so a correction's placement
    set it on the client alone and every corrected `Transform` was a second correction one
    frame later; no boundary system reads it. A defect of D122 found by reading. A proxy's
    `Teleported` is still written by the hook as its position for the tick.
  - *A replicated field derived from a solver-written value compares by bytes and is
    corrected; a game keeps such state integer.* The correction converges because the
    rollback writes every fact at *k*; a derived replicated scalar would mismatch on every
    frame while a contact lasts. The build check cannot see it, so section 9.4 states it.
  - *The spike records what the tolerance absorbed and claims a body in contact.*
    `PredictionStats::absorbed` holds the largest absorbed difference per kind as canonical
    bits and the count of absorbed fields, so milestone 4 re-sets the defaults from
    measurement; the spike runs the character as a dynamic capsule and as a kinematic body
    (a kinematic body and its queries were bit-exact under every history), its fifty
    claimed bodies thrown and interacting, and a claim of a crate being pushed and of a
    stack being bumped, the case that storms.
  - *Claimed dynamic bodies in contact with each other are corrected every few frames
    until they settle, under any rule.* A three-crate stack was corrected 7 to 16 times in
    200 frames after a misprediction while it moved, 22 to 35 times in 167 frames when
    claimed mid-bump, and once when corrected or claimed at rest; the divergence is
    physical, so no tolerance holds it and no canonical solve would either. The lever is
    the claim: `net`'s policy at milestone 4 claims a thrown object with its throw, releases
    it once it rests or touches an unclaimed body, and never claims a body in contact with
    another dynamic one.
  - *An empty solver is a fresh solver.* `sim`'s in-place reload and a client's full-frame
    rebuild unload and load every region in one frame, which the sync sees as `Despawned`
    then `Spawned` per key and leaves untouched, so the solver kept its whole state, the
    reloaded world equalled the untouched twin and not a fresh process (a stack mid-bump by
    3.9e-3 to 1.1e-2 m, a 75-body pile with joints across two regions by 1.24 to 1.40 m),
    and section 15.5's first test did not hold with physics (D57 intended both sides equal);
    the handle rows were lost besides. `physics-api` keeps the bytes the backend wrote at
    `setup`; a sync whose structural log since its last pass, the pending entries a restore
    carried included, despawned every body the solver held removes them all, restores those
    bytes and creates from the rows by a full diff in ascending key order; a sync whose
    removals leave the solver empty restores the bytes and makes the next sync a full diff;
    a `Spawned` entry for a key the bookkeeping holds re-inserts its handle row. Measured
    through the
    sync bit-equal to the fresh process on the one-frame and the two-frame path, the stack
    and the pile with its joints. D119's "never clears the solver" narrows to a solver whose
    log kept a body. Rules out amending section 15.5's first test to a tolerance, and a
    trigger on the solver being empty after a sync alone, which the one-frame path never
    reaches.
  - *A journal checkpoint carries the physics snapshot.* A checkpoint that drops solver
    state replays equal only until the first contact, since the replay's solver is fresh
    (1.5 mm to 1.6 m against the continued one); the milestone 4 replay criterion is met
    from a checkpoint that restores the snapshot after the regions and before the first
    step, or from a session's start. `PhysicsSnapshot` carries an `Arc<()>` identity
    compared by pointer, for the ring, and `sim` cannot name `physics-api`, so
    `journal::Journal` gains `services`, keyed by the xxh64 of a service's declared name
    (never a `TypeId`) to opaque bytes `app` fills and hands back, and `physics-api` gains
    the byte form, carrying a format version, the backend's identity (name, Rapier version,
    the D123 patch set's hash), the `PhysicsConfig`, the tick, the backend's bytes and the
    bookkeeping, and its restore into a fresh `Physics` at the checkpoint's tick, which
    refuses a mismatch with `InvalidData` and rebuilds the handle rows: the journal tooling
    lane's, sized at the milestone 1 scale first. `sim` never reads the bytes and the
    journal is not hashed state.
  - *SOUR needs nothing.* Its modes vary threads, layout and schedule under one row
    history, which is where bit equality holds and what `tests/determinism.rs` shows; the
    sign-of-zero item and the snapshot-hash item stand as they are.
  - *Risk, not this decision's:* on a four-thread pool a small scene stepped in 39 ms at
    worst, most 1,800-tick runs took 20 to 100 ms and 41 of 300 took 0.4 to 4.4 s, against
    19 to 30 ms on one thread (whose own worst run was 319 ms and worst step 14.5 ms), and
    it once hung in
    Rapier's deferred broad-phase optimisation; the physics-rapier lane carries the D123
    patch, the prediction world's pool is one thread until it lands, and the spike's server
    carries a step watchdog.
  - Named and deferred: the defaults re-set at milestone 1 (scalar) and 4 (spike); `net`'s
    claim policy; the snapshot's byte form, `services` and their size; the full diff's cost
    on a server-sized reload.
- Driven by: the D119 History addendum, the planner's probes and the proposal's two design
  reviews, whose probes supplied the claim-start, in-contact, derived-field, fresh-load,
  far-awake and through-the-sync reload measurements and the candidate trigger
  (2026-10-11).
- Replaced: section 2's and 9.4's "at full precision" without qualification; D122's "Full
  precision means canonical bytes per field, no tolerance" for the solver-written rows of a
  dynamic body, its "a tolerance for dynamic bodies is decided at milestone 4" and its
  "this crate computes no float"; D119's "never a cleared solver" for a solver whose log
  despawned every body it held or whose removals emptied it (the two clauses of this
  decision's reload rule); `crates/sim/README.md`'s "dropped at a checkpoint, as solver
  state is".

## Verification items (exact text)

### `docs/verification.md`, section 2, the Rapier determinism claim row (**Done**), append to its second cell after "records all three"

; the D124 probes (2026-10-11, the backend stepped directly and through `physics-api`'s
sync, release, Apple M1) found the dependence confined to large contact graphs and to a
solver that has freed its slots: a dynamic capsule on static geometry, one pushing a crate,
one bumping a three-crate stack, one pressing two crates against a wall and fifty thrown
interacting bodies were bit-equal over 900 to 1,800 ticks with far awake bodies present,
arriving at tick 100 and awake throughout, a kinematic capsule's state and query answers
likewise, while the 150-body pile differed by 2.7 mm at tick 238 and by 12, 38, 96 and 346
mm at ticks 250, 300, 350 and 400, every body removed and recreated in key order on the
same solver differed from a fresh solver by 0.4 mm (a crate), 0.12 m (a stack) and 7.6 m
(the pile), and a same-frame in-place reload through the sync, which removes nothing,
equalled the untouched twin (3.9e-3 to 1.1e-2 m, a 75-body pile 1.24 to 1.40 m); under the
D124 rule (`physics-api` keeps the bytes the backend wrote at setup; a sync whose structural
log since its last pass, the pending entries a restore carried included, despawned every body
the solver held removes them all, restores those bytes and creates from the rows by a full
diff in ascending key order; a sync whose removals leave the solver empty restores the bytes
and makes the next sync a full diff) the reload was bit-equal to the fresh process on every
path

### `docs/verification.md`, section 4, add three rows

| The reconciliation tolerance (D124): the milestone 1 spike's corrections per minute under the defaults for a dynamic capsule character on static geometry at 100 ms and 1% loss, the same with a kinematic character, fifty thrown claimed bodies, and a claim of a crate being pushed and of a stack being bumped; the largest absorbed difference per kind (`PredictionStats::absorbed`), and whether any single-contact maximum exceeds half a default, which doubles it; and how long claimed bodies in mutual contact keep correcting after a throw. The probes' figures, at 20 Hz frames with every fact written at the rollback: a lone capsule one correction per misprediction (8 to 36 bit-exact), absorbing at most 2.4e-5 m; a capsule pushing a crate 1 to 2 (7 to 200), absorbing 7.5e-4 m and 5.3e-3 m/s; a rolled ball and a tipped crate 1 (1 to 200), absorbing 1.2e-3 m, 1.1e-3 per quaternion component and 3.4e-3 rad/s; a crate claimed mid-push 1 to 2, absorbing 1.9e-3 m; a three-crate stack 7 to 16 after a misprediction while it moves (8 to 200) and 22 to 35 in 167 frames when claimed mid-bump, once when corrected or claimed at rest. The probes' correction pushed bit-equal rows too and modelled no rollback; the spike measures the shipped path. The defaults are re-set from the scalar decision at milestone 1 and the spike's figures at milestone 4 | 1 and 4 |
| An empty solver is a fresh solver (D124): `physics-api` keeps the bytes the backend wrote at setup; a sync whose structural log since its last pass, the pending entries a restore carried included, despawned every body the solver held removes them all, restores those bytes and creates from the rows by a full diff in ascending key order; a sync whose removals leave the solver empty restores the bytes and makes the next sync a full diff; a `Spawned` entry for a key the bookkeeping holds re-inserts its handle row. The test runs through `sim`'s in-place reload with `physics-api`'s sync and the Rapier backend, never the backend alone: every region unloaded and loaded in one frame, and in two frames, against a fresh process, equal hashes for K ticks with a three-crate stack mid-bump and a 75-body pile with two joints across two regions, `BodyHandle` and `JointHandle` rows present after each reload and after a same-frame reload of one region; and the full diff's cost on a 100k-entity reload | 1 |
| The journal checkpoint's physics snapshot (D124): `journal::Journal::services` keyed by the xxh64 of a service's declared name; a byte form of `PhysicsSnapshot` carrying a format version, the backend's identity (name, Rapier version, the D123 patch set's hash), the `PhysicsConfig`, the tick, the backend's bytes and the bookkeeping, restored into a fresh `Physics` of a world at the checkpoint's tick with a mismatch refused and the handle rows rebuilt (today's snapshot carries an `Arc<()>` identity checked by pointer); its size at the milestone 1 scale (about 1.2 kB per fixed collider, so the backend's shared fixed body for bare colliders may come first); and the replay test: a session checkpointed with contacts live, replayed from the checkpoint and from its start, hashing equal to the recording at every tick | 4 |

## What the predict implementation lane changes, if D124 is adopted

Against the implementation at 8a744fd. The hook, the ring, the rollback, the re-simulation
and the substitution rules are untouched; no SOUR scene is needed; the one-thread
prediction pool is `app`'s. The change is in `compare.rs`, in the write set of `fact.rs`
that reuses it, and in the reporting:

1. `Comparison` gains a third outcome, `Absorbed { fields }`: the canonical bytes differ,
   the component is `Transform` or `Velocity`, the entity's `RigidBody` at slot *T* or
   *T* - 1 has `body_type == Dynamic`, and every scalar leaf is within
   `PredictionTolerance` for its kind through `lactose_math` (a NaN leaf agrees only with
   an equal NaN, which the bytes already decided). `check`'s mismatch scan treats
   `Absorbed` as agreeing, so the frame makes no correction point for it; `writes` returns
   true for it, so a rollback writes it as it writes `Differs` and `Missing`; `absorbed`
   counts its fields and keeps the largest difference per kind. This is the one change of
   structure: the write set reuses `compare`, so putting the tolerance into `Agrees` would
   have stopped writing absorbed facts, which was measured worse (the stack 18 to 34
   corrections against 7 to 16, a claim mid-bump 32 to 44 against 22 to 35).
2. The caller passes the entity's `RigidBody` at *T* (the fact's if the frame carries one,
   else the record's) and at *T* - 1 (the record's, `None` when the record has no slot
   *T* - 1 for the entity, in which case *T* alone decides); `compare(info, predicted,
   server)` takes neither today.
3. `Teleported` is excluded from the predictor's recorded set (`state.recorded`, which
   `Slot::take` reads through `get_erased`) and from `written_components`, and from nowhere
   else: `recorded()` stays the filter `compare_data` uses, and `compare_data` skips
   `Teleported` silently, since `compare` already returns `Ignored` for it and a warning
   there would report a misconfiguration that is not there, the server's facts carrying it
   for every claimed entity.
4. `PredictionTolerance { translation, rotation, linear, angular }: Scalar` with
   `DEFAULT` (2e-3, 2e-3, 2e-2, 2e-2) and `Predictor::set_tolerance(&mut self,
   PredictionTolerance)` between runs; `Predictor::new` keeps its signature and starts at
   the default.
5. `PredictionStats::absorbed: Absorbed { fields: u64, translation: u64, rotation: u64,
   linear: u64, angular: u64 }`, the count of absorbed fields and the largest absorbed
   difference per kind as canonical bits, so the derives stay; `Divergence` counts
   mismatches only and `Correction` is unchanged.
6. Docs: `compare.rs`'s module comment ("a tolerance for dynamic bodies ... changes this
   module and nothing else") and its "Full precision, no tolerance"; `reconcile.rs`'s
   `Divergence` doc ("this crate computes no float"); the README sentences above.
7. Tests, each mandatory: the comparison's golden cases listed in the README amendment; a
   corrected `Transform` followed by a frame with the server's unchanged `Teleported` makes
   no correction; a frame whose only difference is within tolerance makes no correction and
   counts in `absorbed`; a frame with one leaf beyond it rolls back and writes the absorbed
   facts too, which the physics test backend's recorded pushes show; a body dynamic at
   *T* - 1 and kinematic at *T* takes the tolerance, and one with no slot *T* - 1 is judged
   at *T*.
