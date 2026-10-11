# Proposal: physics is a function of history, and what prediction compares

Status: proposal, not applied; `docs/architecture.md` sections 2 and 9.4 ("at full
precision") and D122's "full precision means canonical bytes per field, no tolerance" stand
until it is decided. It is decided before the `lactose-predict` implementation lane starts,
since that lane writes the comparison: adopted as an amendment to sections 1, 2, 9.4, 10,
15.5, 16 and 22 with a numbered decision entry, or declined with the reason recorded in the
decisions log. The physics backend changes nothing either way.

## The problem

`lactose-physics-rapier` gives results that are a function of the whole row history, not of
the rows: a body's bits depend on every body the world holds or held, far-away awake bodies
that never touch it included, and on the arena slots of bodies since removed (D119,
"History"; `crates/physics-rapier/README.md`; `crates/physics-rapier/tests/determinism.rs`).
The server's own determinism is intact, because a replay and a re-simulation repeat the
history. The client's prediction world does not: it holds the local player, the claimed
entities, the relevant entities as kinematic proxies and the static collision, never the
server's body set, created in a different order after a different history (architecture
section 9.4; `crates/predict/README.md`, "The hook"). So the solver-written rows of a claimed
dynamic body on the two machines are not bit-equal in general, however good the inputs,
and D122's rule that any difference is a misprediction would correct what is not one.

Three questions follow, and this proposal decides each.

1. What prediction compares when it decides a correction.
2. Whether the backend must canonicalise its results per island.
3. Whether SOUR and the journal need anything more for the server's own determinism.

## What was measured

The backend was stepped directly through `PhysicsBackend`, as its own tests do (release,
Apple M1, one and four solver threads; the probe is `probe_history.rs` in the planner's
scratchpad, not committed). Two things were measured: how large the history dependence is
in the scenes prediction compares, and how large and how long the residual is after a
correction, since D122 already notes that Rapier's solver state is not corrected.

Scenes, each 1,800 ticks, alone and then beside a forty-ball pile a kilometre away created
before the scene, the same pile created at tick 100, and an arena history (seven far bodies
created, stepped once and removed, ten far balls taking their slots at tick 40):

| Scene | Bits differ? |
|---|---|
| A dynamic capsule with rotation locked, walked at 3 m/s in a 6 m square across a row of five static boxes, jumping every 300 ticks, resting 60 ticks in 480 | No, in every variant, on one and four threads |
| The same capsule pushing a dynamic crate 4.1 m | No |
| The same capsule bumping a stack of three crates | No |
| Fifty dynamic balls 2.5 m apart, each given an impulse three times, touching at least once (closest centres 0.97 m) | No, over 900 ticks |
| A kinematic capsule swept along a curve with a downward shape cast and a forward ray each tick | No, and every query answer equal |
| The 150-body pile with two joints and a sweeping paddle of `tests/determinism.rs` | Yes: first at tick 238 by 2.7 mm, then 12, 38, 96 and 346 mm at ticks 250, 300, 350 and 400 |

The dependence is therefore a property of large contact graphs, where constraint order
between constraints that share a body is history, and not of a body on static geometry or
of a few bodies in contact. In the pile it is chaotic: fourfold growth every twelve ticks.

A correction, measured as `lactose-predict` would make it: run A is the server; run B
strays at tick *p* (its capsule written one centimetre along x and 0.1 m/s faster, as a
misprediction leaves it) and at tick *c* has A's `Transform` and `Velocity` for *c* written
into every body's rows through `set_transform` and `set_velocity`, which is what the sync
does after `corrected`; the solver's state is untouched.

| Scene | *p*, *c* | Residual after *c* | Bit-equal again |
|---|---|---|---|
| Lone capsule | 300, 330 | at most 3.7e-15 m, 1.0e-13 m/s | after 73 ticks |
| Lone capsule | 700, 730 | at most 3.6e-15 m, 1.2e-13 m/s | after 305 ticks |
| Lone capsule, in the air | 150, 156 | none | at once |
| Capsule pushing a crate | 300, 330 | at most 5.4e-4 m, 1.6e-3 m/s | not within 870 ticks (2.0e-4 m at the end) |
| Capsule pushing a crate | 700, 730 | at most 2.7e-5 m | 1.7e-9 m at the end |
| Capsule and the stack | 300, 330 | at most 3.8e-4 m, 2.9e-3 m/s | not within 870 ticks (1.6e-4 m at the end) |
| Capsule and the stack | 700, 730 | at most 2.4e-5 m | after 463 ticks |

So a correction under a bit-exact rule breeds corrections: a lone body's rows differ by
rounding for 70 to 300 ticks, 25 to 100 server frames at 20 to 30 Hz, each a rollback; a
body in contact with another never returns to the bit while the contact lasts. No rotation
or angular residual appeared (the capsule's rotation is locked and the crates slid without
turning); the spike measures rolling bodies.

Numbers cited rather than measured: a rebuild of Rapier's sets every step removed the
history dependence in every case the backend lane tried at about 1.3 ms a step plus a
cold-contact step of 3.7 ms against 1.2 ms at 2,000 bodies, and left the dependence on far
bodies present, which the prediction world lacks; a rebuild on membership change moved the
dependence rather than removing it (D119, "History"). The physics phase's budget is 3 ms at
2,000 awake bodies (section 22).

One defect found by reading, independent of the tolerance: `physics-rapier` reports a body
whose transform was written since the last step as `teleported` (`set_transform` places it
through `set_position` and adds it to the step's `placed` set), and `physics-api`'s
write-back then writes the tick into `Teleported`, a persistent replicated component. A
correction writes `Transform`, `corrected` makes the sync push `set_transform`, so the
re-simulated tick marks the client's row teleported while the server's row is unchanged; the
next frame's exact `Teleported` fact mismatches and D122 corrects again, one frame later,
for every corrected `Transform`.

## The options

**Canonical results per island in the backend.** Solve each island separately in a
canonical order so that a body's bits depend on its island's rows alone. The pinned Rapier
solves all awake bodies as one island with constraint order from world-wide structures
(D119); changing that is not a fifty-line patch, so it is a fork or another engine, which
D123 and the milestone plan rule out, a milestone of work by itself. It would also not help
the one case that matters in the prediction world beyond small scenes: an island that holds
a proxy, kinematic on the client and dynamic on the server, differs whatever the order, and
section 9.4 already expects that mismatch. Rules out nothing useful now; costs a physics
engine.

**Rebuild the backend's sets every step.** Measured by the backend lane: 1.3 ms a step plus
a cold-contact step at 2,000 bodies, over the 3 ms phase budget with the step itself, and
the dependence on far bodies present stays, so the comparison would need a tolerance anyway.
Rules out the phase budget.

**Authority-only physics facts.** Never compare the solver-written rows of a claimed body;
apply the server's values as they arrive. A claimed dynamic body then has no prediction for
its motion and is a proxy half a round trip late; a thrown object is presented where it was,
not where it is. Rules out predicting thrown and carried objects, which the reference game
and the spike's fifty claimed bodies need.

**Keep D122's bit-exact rule and count the storm.** No code beyond D122. The residual table
says what it costs: 25 to 100 rollbacks after each correction of a body on the ground, and
a rollback every frame while a corrected body touches another. Milestone 4's criterion of
fewer than one correction a minute is then unreachable for any dynamic character, and the
re-simulation budget is spent on corrections nobody can see. Rules out the milestone 4
criterion for dynamic bodies.

**A tolerance on the solver-written rows of a dynamic body, bit-exact everywhere else.**
The recommendation, below. Costs a comparison rule and four scalars in `lactose-predict`,
nothing in the backend; absorbs the residual in every scene measured with a margin of about
four; leaves large contact graphs correcting every frame while awake, which no option above
avoids either, since their divergence is chaotic.

## The recommendation

The backend keeps its history dependence and the comparison keeps it out. Concretely:

1. A fact's `Transform` and `Velocity` agree, for a claimed entity whose `RigidBody` is
   `Dynamic` at the compared slot, when each scalar leaf is within `PredictionTolerance`:
   2e-3 m per translation component, 2e-3 per quaternion component, 2e-2 m/s per linear
   velocity component and 2e-2 rad/s per angular one by default. These are about four times
   the largest residual measured and under a pixel at five metres on a 1440p display with a
   60 degree field of view. Every other field, the rows of kinematic and fixed bodies
   included, compares by canonical bytes as D122 says. The facts that would change the
   values: the spike's largest absorbed difference within a factor of two of a default, or
   a visible jump at a correction below it.
2. A fact within tolerance is not written at a correction. Writing it would place the body,
   wake it and discard its warm start for a difference nobody can see, and would start the
   residual again. A component is written whole when any compared leaf of it mismatches.
3. `Teleported` is never compared and never written by a correction. It is the client's own
   no-interpolate flag, set by its own steps and its own corrections.
4. A replicated field a game computes from a solver-written value (damage from an impulse,
   a distance) is an integer or quantised, or it is corrected on the solver's rounding. The
   build check cannot see it; it is a rule of authoring.
5. `PredictionStats` records what the tolerance absorbed, the largest difference per kind
   and the count of absorbed fields, so milestone 4 re-sets the defaults from the spike's
   measurement, not from this probe. The spike runs the character as a dynamic capsule and
   as a kinematic body, and its fifty claimed bodies are thrown and interacting, not heaped.
6. A heap of claimed bodies is corrected every frame while it is awake, under any rule. The
   lever is the claim, not the solver: `net`'s claim policy at milestone 4 releases a thrown
   object once it rests or touches an unclaimed body. The fact that would change this: a
   game scene that needs a claimed heap awake for more than a few seconds.
7. The server's determinism needs nothing from SOUR: its modes vary threads, layout and
   schedule under one row history, which is where bit equality holds and what the backend's
   tests already show. The journal does need one thing: a checkpoint carries the physics
   snapshot, because a replay whose backend starts empty has a different history and hashes
   equal only until the first contact after the checkpoint. `PhysicsSnapshot` today carries
   an `Arc<()>` identity compared by pointer (`crates/physics-api/src/service.rs`), fit for
   the ring and not for a file, so the journal tooling lane adds a byte form that a fresh
   process restores into a `Physics` of a world at the checkpoint's tick. Until it exists a
   replay is bit-exact from a session's start only. The fact that would change this: the
   snapshot's size at the milestone 1 scale, about 1.2 kB per fixed collider today, which
   may need the backend's shared fixed body for bare colliders first.

Cost: the predict implementation lane absorbs items 1 to 5 in its brief, about a third of a
run and no extra review round; the journal tooling lane absorbs item 7, about half a run
in `physics-api` and `sim`; the spike's two extra scenes are about half a run; the document
edits are the coordinator's.

## Architecture amendments (exact text)

### §1 Goals, item 2, replace the sentence "It does not by itself make client prediction correction-free, because the client's prediction world differs from the server's world."

It does not by itself make client prediction correction-free, because the client's
prediction world differs from the server's world and the physics backend's results are a
function of the whole world's history (section 16).

### §2 Decisions, Netcode row, replace "Local-player prediction in a separate client prediction world, reconciled at full precision."

Local-player prediction in a separate client prediction world, reconciled at full precision
within a stated tolerance on the rows the solver writes.

### §2 Decisions, Determinism scope row, replace "Reconciliation compares at full precision (section 9.4)."

Reconciliation compares at full precision, with a stated tolerance on the rows the solver
writes (section 9.4).

### §9.4 Prediction, replace the sentence "Rapier's internal solver state is not corrected, so a corrected dynamic object converges over a few ticks rather than exactly."

Every field compares bit for bit except the `Transform` and `Velocity` the physics
write-back writes on a dynamic body, which agree when each scalar is within a stated
tolerance (`lactose-predict`'s, two millimetres, two thousandths of a quaternion component,
two centimetres per second and two hundredths of a radian per second by default), because
the backend's results are a function of the world's whole history (section 16) and Rapier's
internal solver state is not corrected, so a corrected dynamic body converges to within the
tolerance at once and to the bit only eventually. A field within tolerance is not written
by a correction. `Teleported` is the client's own no-interpolate flag and is never compared
or corrected. A replicated field a game computes from a solver-written value is an integer
or quantised, or it is corrected on the solver's rounding.

### §10 Snapshots, replace "The server never rolls back. Save snapshots copy persistent columns only (section 15.3)."

The server never rolls back. Save snapshots copy persistent columns only (section 15.3); a
journal checkpoint carries the physics snapshot beside them (section 15.5).

### §15.5 Enforcement, replace "Journal replay (checkpoint plus input journal) is a debug and replay tool, not the save system."

Journal replay (checkpoint plus input journal) is a debug and replay tool, not the save
system. A checkpoint carries the physics snapshot of section 10 as bytes, since the
backend's results are a function of its history (section 16) and a replay from a checkpoint
without it hashes equal only until the first contact; a replay from a session's start needs
none.

### §16 Physics, append after "Milestone 1 measures the physics phase's share of the tick and the stall it imposes on render recording during the overlap."

The backend's results for a body are a function of the whole world's row history, not of
the body's own rows: with one awake island and world-wide constraint order, far-away awake
bodies and the arena slots of bodies since removed can change a body's bits (D119). Bit
equality therefore holds between two worlds only when their row histories are identical,
the server and its replay or a run and its re-simulation, never the server and a client's
prediction world. The backend does not canonicalise per island: the pinned Rapier cannot,
and a rebuild every step costs 1.3 ms a step without removing the dependence on bodies
present. The dependence is kept out of the comparison instead (section 9.4). In scenes of
one to four dynamic bodies on static geometry and fifty scattered bodies it changed nothing
over 1,800 ticks; in a 150-body pile it first appeared at 2.7 mm and grew fourfold every
twelve ticks, so a heap of claimed bodies is corrected every frame while it is awake and
the claim policy, not the solver, bounds that.

### §22 Milestones, row 1, replace "one predicted character, fifty claimed dynamic bodies; correction rate, rollback cost per tick and divergence recorded as the baseline that milestone 4's threshold is judged against."

one predicted character, run as a dynamic capsule and as a kinematic body, fifty claimed
dynamic bodies thrown and interacting rather than heaped; correction rate under the section
9.4 tolerance, the largest difference the tolerance absorbed per kind, rollback cost per
tick and divergence recorded as the baseline that milestone 4's threshold is judged against.

## README amendments (exact text)

### `crates/predict/README.md`, "Surface", add a row after the `Reconciled` row

| `PredictionTolerance`, `Predictor::set_tolerance` | How far a dynamic body's solver-written `Transform` and `Velocity` may differ from the server's and still agree: a scalar per kind, defaults in the type, set by `app` between runs |

### `crates/predict/README.md`, "Hidden plumbing", replace "Everything public is keys, ticks and counts; nothing reads a clock or computes a float, and the crate carries the boundary `clippy.toml`."

Everything public is keys, ticks, counts and the tolerance's scalars; nothing reads a
clock, every float goes through `lactose_math`, and the crate carries the boundary
`clippy.toml`.

### `crates/predict/README.md`, "Reconciliation", replace "Full precision, no tolerance: the same build on the same bytes produces the same bits, so any difference is a different input, which is what a misprediction is."

Full precision, with one exception: the `Transform` and `Velocity` of an entity whose
`RigidBody` is `Dynamic` at slot *T* (the fact's `RigidBody` if the frame carries one,
else the record's) agree when each scalar leaf is within the predictor's
`PredictionTolerance` for its kind (translation, rotation, linear, angular), since the
physics backend's results are a function of the whole world's history and the client's
world has a different one (architecture section 16, D124); every other field, the rows of
kinematic and fixed bodies included, agrees only bit for bit, since the same build on the
same bytes produces the same bits. A NaN leaf never agrees. `Teleported` is the client's
own no-interpolate flag: never compared, never written. A field within tolerance that is
not bit-equal is counted in `PredictionStats::absorbed`, with the largest difference per
kind, and is not a mismatch.

### `crates/predict/README.md`, "Reconciliation", the rollback's step 2, replace "Every fact at *k* is written, stamped:"

Every fact at *k* that does not agree is written, stamped (a component whole when any
compared leaf of it mismatches; a fact within tolerance is left as predicted, since writing
it would place the body, wake it and discard its warm start for a difference nobody can
see):

### `crates/predict/README.md`, "Reconciliation", replace the bullet "Rapier's internal solver state is not corrected (section 9.4), so a corrected dynamic body converges over a few ticks rather than exactly: the frames of those ticks differ again by small amounts, each a correction under the rule above. The spike's correction rate counts them; whether milestone 4 adds a tolerance for dynamic bodies is decided against that baseline, never silently here."

- Rapier's internal solver state is not corrected (section 9.4), so a corrected dynamic
  body's rows differ from the server's by rounding afterwards: a lone body by about 1e-15 m
  for up to 300 ticks, a body in contact with another by up to half a millimetre for as
  long as the contact lasts (D124). The tolerance absorbs it; without one every correction
  would breed 25 to 100 more. A large contact graph of claimed bodies diverges chaotically
  under any tolerance and is corrected every frame while awake; `net`'s claim policy
  releases a thrown object once it rests. The spike records the corrections and the
  largest absorbed difference per kind, and milestone 4 re-sets the defaults from them.

### `crates/predict/README.md`, "Not in this crate yet", replace "- A tolerance for dynamic bodies, decided at milestone 4 against the spike's baseline."

- The tolerance's defaults re-set from the spike's `absorbed` figures, and `net`'s release
  of a thrown object at rest: milestone 4.

### `crates/predict/README.md`, "Proof for milestone 1", "The comparison" row, append

; a dynamic body's `Transform` within tolerance agrees and is not written, one past it is
a mismatch of the component, a kinematic body's is bit-exact, a NaN leaf never agrees,
`Teleported` differing is not a mismatch and is not written at a correction, and `absorbed`
holds the largest absorbed difference per kind

### `crates/physics-api/README.md`, "Snapshot and restore", replace "Rapier's internal solver state is not corrected (architecture section 9.4), so a corrected dynamic body converges over a few ticks."

Rapier's internal solver state is not corrected (architecture section 9.4), so a corrected
dynamic body's rows differ from the server's by rounding afterwards, a lone body by about
1e-15 m for up to 300 ticks and a body in contact with another by up to half a millimetre
for as long as the contact lasts; `lactose-predict`'s tolerance absorbs it (D124). A
corrected `Transform` is pushed as a placement, so the re-simulated step reports the body
`teleported` and the write-back sets the client's `Teleported`, which `lactose-predict`
therefore never compares.

### `crates/physics-rapier/README.md`, "Calls are recorded, the step applies them", append after "two worlds holding different body sets or histories are not bit-equal for the bodies they share."

The dependence is a property of large contact graphs, not of every body: a dynamic capsule
walking on static geometry, one pushing a crate, one bumping a three-crate stack and fifty
scattered balls were bit-equal over 1,800 ticks with and without far awake bodies, bodies
arriving mid-run and an arena history, on one and four threads; the 150-body pile first
differed by 2.7 mm at tick 238 and fourfold every twelve ticks after (D124). The backend
does not canonicalise; `lactose-predict` compares solver-written rows within a tolerance.

### `crates/sim/README.md`, "Journal replay", replace "the untaken `PhysicsCommands`, `World::channel`, are dropped at a checkpoint, as solver state is),"

and the physics snapshot of `lactose-physics-api`, the backend's whole world and the
sync's pending work with the untaken `PhysicsCommands`, as bytes a fresh process restores
after the regions and before the first step, since the backend's results are a function of
its history and a replay without it hashes equal only until the first contact; until the
byte form exists, D124, a replay is bit-exact from a session's start only),

## Decision entry (exact text; number at merge)

**D124. Physics is a function of history: reconciliation compares solver-written rows within
a tolerance, `Teleported` is never compared, and a journal checkpoint carries the physics
snapshot** (§1, §2, §9.4, §10, §15.5, §16, §22, `lactose-predict`, `lactose-physics-api`,
`lactose-physics-rapier`, `lactose-sim`)
- Why: the Rapier backend's results for a body depend on every body the world holds or held
  (D119, "History"), so the client's prediction world, which never holds the server's body
  set, is not bit-equal to the server for a dynamic body however good the inputs, and D122's
  "no tolerance" would correct what is not a misprediction and then correct its own
  corrections, since Rapier's uncorrected solver state leaves a rounding residual. The
  planner's probe (`docs/verification.md`) bounds both: a capsule on static geometry, one
  pushing a crate, one bumping a stack and fifty scattered balls were bit-equal whatever the
  far bodies over 1,800 ticks; after a correction a lone body differed by at most 3.7e-15 m
  for up to 305 ticks and a body in contact by at most 5.4e-4 m for as long as the contact
  lasted; a 150-body pile first differed by 2.7 mm and grew fourfold every twelve ticks.
  - *The backend keeps its history dependence.* Canonical results per island need Rapier
    to solve islands separately in a canonical order, which the pinned version cannot and
    no fifty-line patch adds (D123); a rebuild of its sets on every membership change moved
    the dependence, and a rebuild every step removed it at 1.3 ms a step plus a cold-contact
    step of 3.7 ms at 2,000 bodies, over the phase budget, without removing the dependence
    on bodies present, which the prediction world lacks. Rules out canonicalisation in
    `physics-rapier`, a second engine and a physics world per island.
  - *Bit-exact everywhere but the solver-written rows of a dynamic body.* `Transform` and
    `Velocity` of a claimed entity whose `RigidBody` is `Dynamic` at the compared slot agree
    when each scalar leaf is within `PredictionTolerance` (2e-3 m, 2e-3 per quaternion
    component, 2e-2 m/s and 2e-2 rad/s by default, about four times the largest residual
    measured and under a pixel at five metres; `app` sets them through
    `Predictor::set_tolerance`); every other field, kinematic and fixed bodies' rows
    included, compares by canonical bytes as D122 says, and a NaN leaf never agrees. A fact
    within tolerance agrees and is not written at a correction, since writing it would
    place the body, wake it and discard its warm start for a difference nobody can see and
    start the residual again; a component is written whole when any compared leaf
    mismatches. Rules out a tolerance on every field, a tolerance on discrete fields, an
    authority-only rule that applies physics facts without comparing them (a claimed body
    would be a proxy half a round trip late), and a quantised comparison grid (two values a
    ULP apart straddle a grid line).
  - *`Teleported` is never compared or corrected.* It is the client's own no-interpolate
    flag: `physics-rapier` reports a body whose transform was written as `teleported` and
    the write-back sets the tick, so a correction's placement set it on the client alone
    and every corrected `Transform` was a second correction one frame later. A defect of
    D122 found by reading, with or without a tolerance.
  - *A replicated field computed from a solver-written value is an integer or quantised.*
    The solver's rounding would otherwise reach it and be corrected bit for bit; the build
    check cannot see it, so section 9.4 states it as a rule of authoring.
  - *The spike records what the tolerance absorbed.* `PredictionStats::absorbed` holds the
    largest absorbed difference per kind and the count of absorbed fields, so milestone 4
    re-sets the defaults from measurement; the spike runs the character as a dynamic
    capsule and as a kinematic body (the probe found a kinematic body and its queries
    bit-exact under every history), and its fifty claimed bodies are thrown, not heaped.
  - *A heap of claimed bodies is corrected every frame while awake, under any rule.* Its
    divergence is chaotic, so no tolerance holds it and no canonical solve would either
    once a proxy is in it; the lever is the claim, released by `net` once a thrown object
    rests or touches an unclaimed body, a milestone 4 policy.
  - *A journal checkpoint carries the physics snapshot.* A checkpoint that drops solver
    state replays equal only until the first contact, since the replay's backend has a
    different history; the milestone 4 replay criterion is met from a checkpoint that
    restores `physics-api`'s snapshot after the regions and before the first step, or
    from a session's start. `PhysicsSnapshot` carries an `Arc<()>` identity compared by
    pointer, for the ring, so a byte form a fresh process restores is a `physics-api`
    addition of the journal tooling lane, sized at the milestone 1 scale first.
  - *SOUR needs nothing.* Its modes vary threads, layout and schedule under one row
    history, which is where bit equality holds and what `tests/determinism.rs` shows; the
    sign-of-zero item and the snapshot-hash item stand as they are.
  - Named and deferred: the defaults re-set at milestone 4; `net`'s release of a thrown
    object at rest; the snapshot's byte form and its size; a rotation and angular residual,
    unmeasured since the probe's bodies did not turn.
- Driven by: the D119 History addendum, the planner's probe and the predict facade's
  comparison rule (2026-10-11).
- Replaced: section 2's and 9.4's "at full precision" without qualification; D122's "Full
  precision means canonical bytes per field, no tolerance" for the solver-written rows of a
  dynamic body, and its "a tolerance is a milestone 4 decision against the spike's
  baseline"; `crates/sim/README.md`'s "dropped at a checkpoint, as solver state is".

## Verification items (exact text)

### `docs/verification.md`, section 2, the Rapier determinism claim row (**Done**), append to its second cell after "records all three"

; the planner's probe (2026-10-11, the backend stepped directly, release, Apple M1, one and
four threads) found the dependence confined to large contact graphs: a dynamic capsule on
static geometry, one pushing a crate, one bumping a three-crate stack and fifty scattered
balls were bit-equal over 1,800 ticks with far awake bodies, bodies arriving at tick 100 and
an arena history, a kinematic capsule's state and query answers likewise, while the 150-body
pile differed by 2.7 mm at tick 238 and by 12, 38, 96 and 346 mm at ticks 250, 300, 350 and
400

### `docs/verification.md`, section 4, add two rows

| The reconciliation tolerance (D124): the milestone 1 spike's corrections per minute under the defaults for a dynamic capsule character on static geometry at 100 ms and 1% loss, the same with a kinematic character, and fifty thrown claimed bodies; the largest absorbed difference per kind (`PredictionStats::absorbed`), including rotation and angular velocity on rolling bodies, which the planner's probe did not exercise; and how long a fifty-body heap stays awake after a throw, which bounds its storm of corrections. The probe's residual after a correction that wrote the server's rows into a client a centimetre astray: a lone capsule at most 3.7e-15 m and 1.2e-13 m/s, bit-equal again within 73 to 305 ticks; a capsule pushing a crate and one bumping a stack at most 5.4e-4 m and 2.9e-3 m/s, still 2e-4 m apart after 870 ticks. The defaults are re-set at milestone 4 from the spike's figures | 4 |
| The journal checkpoint's physics snapshot (D124): a byte form of `PhysicsSnapshot` a fresh process restores into a `Physics` of a world at the checkpoint's tick (today's carries an `Arc<()>` identity checked by pointer), its size at the milestone 1 scale (about 1.2 kB per fixed collider, so the backend's shared fixed body for bare colliders may come first), and the replay test: a session checkpointed with contacts live, replayed from the checkpoint and from its start, hashing equal to the recording at every tick | 4 |

## What the predict implementation lane does differently

1. Adds `PredictionTolerance { translation, rotation, linear, angular }: Scalar` with
   `DEFAULT` (2e-3, 2e-3, 2e-2, 2e-2) and `Predictor::set_tolerance(&mut self,
   PredictionTolerance)` between runs; `Predictor::new` keeps its signature and starts at
   the default.
2. Compares a fact's `Transform` and `Velocity` leaves within the tolerance when the
   entity's `RigidBody` at slot *T* (the fact's if the frame carries one, else the
   record's) has `body_type == Dynamic`; every other component and every other body type by
   canonical bytes as the facade says; a NaN leaf never agrees. The subtraction and the
   compare go through `lactose_math`; the crate's "computes no float" sentence goes.
3. Skips `Teleported` in the record and in the facts: not compared, not written, not
   counted. The second-correction case above is a test: a corrected `Transform` followed by
   a frame with the server's unchanged `Teleported` makes no correction.
4. Writes at a correction only the facts that do not agree: a component whole when any
   compared leaf mismatches, nothing for one within tolerance; `corrected` still receives
   exactly the keys whose rows were written.
5. Adds `PredictionStats::absorbed: Absorbed { fields: u64, translation, rotation, linear,
   angular: Scalar }`, the largest absorbed difference per kind and the count of absorbed
   fields; `Divergence` stays counts of mismatches beyond tolerance.
6. Extends `tests/cost.rs` with nothing; extends the comparison's golden cases and the
   corrections' cases as the README amendment lists; the SOUR prediction scene adds a
   claimed dynamic body whose facts differ within tolerance at one tick and beyond it at
   another, recording that the first makes no correction and the second one.
7. Writes the spike's two character runs and the thrown bodies into the spike's brief, not
   this lane's: the lane proves the rule, the spike measures it.
