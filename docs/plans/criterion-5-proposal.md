# Milestone 1 after the quiet window: the criteria against their figures, and criterion 5

Status: proposal, not applied. Written at main `1d8042e`, architecture revision 3.3, for the
owner's decision D143.

The quiet window ran `cargo xtask measure --all` on the Apple M1 at the measurement commit
`bef3455`, and `1d8042e` checked in its 22 files (`docs/results/*-apple-m1-2026-11-15.*`). This
document reads each milestone 1 criterion of architecture section 22 against them (section 1).
One criterion is not met: the prediction-world snapshot is 140.8 µs against 100 µs. Section 2
says where that time goes, section 3 what can be done about it, section 4 what this document
recommends and section 5 the question for the owner. Section 6 is the text for the coordinator:
the decision entry D143, the amendments to `docs/architecture.md` and
`docs/plans/milestone-1.md`, the verification rows and the README sentences. Nothing of section
6 is applied until the owner answers. This document changes no code.

Conventions:

- A quiet figure is a figure of one of those result files, cited by the file's measurement
  name. It was taken with no other build on the machine, its file carries its load averages,
  and it is what a criterion is read against (architecture section 20).
- A probe figure is the planner's. It was taken in an instrumented copy of the tree at
  `1d8042e`, in release, on the same M1 while other agents built, and it carries its load
  averages. A probe figure steers and proves nothing. Section 2 describes the probe.
- A verification row is cited by its opening words.
- `slot-copy` is this document's handle for a lane (section 4). It is not a crate name.

## 1. The criteria against the quiet window

The criteria are numbered as `docs/plans/milestone-1.md` section 1 numbers them. Thresholds are
those of architecture section 22, read as its paragraph under the table and the plan's section 4
read them.

| # | Criterion | Quiet figure on the M1 | Verdict |
|---|---|---|---|
| 1 | 100k entities simulate a tick in at most 4 ms on 8 performance cores | `tick-100k`: T₁ 10,193.0 µs, T₄ 3,344.6 µs, S 1,061.8 µs, projected to eight workers 2,203.2 µs; 7·T₄ − T₁ is 13.2 ms against the 24 ms that steers; hash 0x11c1a653b85d7d3d; load 1.47 2.11 3.72 | Proven only by session B. The M1's projection steers, and it is inside by 45%; T₄ alone is under 4 ms |
| 2 | SOUR passes in workers, repeat, layout and chaos modes | None: not a timing | Not the quiet window's. The exit's reviewer reads the nightly |
| 3 | Physics snapshot conformance | None: not a timing | Not the quiet window's. D143 keeps the run and adds a leg to it (section 6) |
| 4 | Physics phase at most 3 ms for 2,000 awake bodies | `physics-phase`: 1,954.3 µs (sync 554.3, step 985.4, write-back 419.0) on four frame workers and three solver threads; load 1.43 2.09 3.71 | Met on the M1, with a third to spare. The x86 desktop's figure is session B's |
| 5 | Prediction-world snapshot at most 100 µs | `prediction-slot`: snapshot 140.8 µs (world 5.6, physics 135.2); record 54.4 µs; restore 193.0 µs (world 12.3, physics 180.6); load 1.51 2.09 3.69 | Not met: 41% over. Sections 2 to 6 |
| 6 | Sim-skeleton evaluation at most 0.5 ms for 200 characters | `sim-skeleton`, figure (b): the step with the skeleton 229.0 µs on one worker and 136.0 µs on four, 180.8 and 98.4 µs more than without it; (a) 160.5 µs; load 1.51 2.09 3.69 | Met on the M1, by more than half. The x86 desktop's figure is session B's |
| 7 | Netcode spike: correction rate, absorbed differences, rollback cost per tick and divergence recorded | `rollback-seven-ticks`: 3,185.9 µs for seven ticks, 455.1 µs a tick; load 1.51 2.09 3.69 | Recorded; it has no budget. The spike's other figures have no result file (below) |
| 8 | Scalar decided by the section 16 rule | None: not a timing | Not the quiet window's. The probe of 14 November closed it: 32 bits fails the precision half and 64 bits stays (the row "Scalar precision") |
| 9 | Edit-to-playing at most 30 s per platform | `edit-to-playing`: slower median 1.56 s (sketch system 1.51 s, `app` function 1.56 s); load 3.79 7.12 5.87 before and 8.10 7.89 6.19 after, the task benchmark's and the measurement's own builds | Met on macos-arm64, 19 times inside. The `measure` workflow's runners gave 3.21 s on linux-x64 and 5.83 s on windows-x64 at `bef3455` (run 43659357193, not checked in). The desktop's figures are session B's |

Of the five timing criteria, four are inside their thresholds on the M1, criterion 1 by a
projection that only steers. Criterion 5 is not.

What the quiet window measured beside the criteria:

| Measurement | Threshold the documents give it | Quiet figure on the M1 | Verdict |
|---|---|---|---|
| The composed tick (`composed-tick`) | None. D139 records it and rules out reading criterion 1's 4 ms against it. Section 16 asks milestone 1 for the physics phase's share of the tick | T₁ 13,808.9 µs, of which physics 3,156.1 µs; T₄ 5,772.5 µs, of which physics 2,265.8 µs; S 3,093.7 µs, projected to eight workers 4,433.1 µs; load 1.43 2.09 3.71 | Recorded. The share section 16 asks for is 23% on one worker and 39% on four. On four frame workers the tick leaves 10.9 ms of a 16.67 ms period |
| The timer wake (`whisk`): milestone 0's tick start, carried by D138 | Under 20 µs at P99 at 60 Hz (section 22, milestone 0; `crates/task/README.md`) | P50 2.6 µs, P99 27.0 µs, max 69.6 µs, none of 5,000 over 1 ms; load 1.51 2.07 3.66 before, 4.04 7.23 5.90 after, the benchmark's own | Not met, for the third time on this machine: 24.5 and 29.9 µs on 9 and 10 October. D138 has decided what follows: one bounded attempt, `keeper`, after session A's numbers, then a restatement at 35 µs, which 27.0 µs is inside. Nothing new is asked |
| WHISK's other three | Makespan over its lower bound at most 1.25 at P50 and 1.5 at P99; wake from an idle pool under 35 µs at P99; steal under 1 µs | At most 1.143 and 1.235; 31.6 µs; 167 ns | Met on the M1, as on 9 and 10 October. The x86 half is session A's |
| A 100k reload (`reload-100k`) | None: recorded (the row "An empty solver is a fresh solver (D124)") | The reload tick's step 392.5 ms, its sync 127.0 ms, against 2.27 ms and 0.57 ms for a normal tick; load 1.43 2.09 3.71 | Recorded |
| Bone bodies on Rapier (`bone-bodies`) | None: recorded outside criterion 6 (D128) | 7,382.5 µs of bodies per tick on one worker and 6,098.2 µs on four, for 3,200 bodies; load 1.56 2.10 3.70 | Recorded. The lever is the number of `BoneBody`s |
| The headless run (`headless-120`) | Its hash on x86-64 equals the M1's at the same commit | 0x92ff315464f84575, 120 ticks in 121 frames | The reference for session B |

What this reading leaves open, and whose it is:

- **Session B.** No file of it is in `docs/results`. It owes criterion 1's proof on eight frame
  workers, the x86 figures of criteria 4, 6 and 9, the headless hash, and the slot's x86
  figure, which the plan records and does not judge. The owner's.
- **Criterion 5.** The owner's decision, D143, below.
- **Criterion 7's baseline has no result file.** The plan's section 1 names a result file
  `spike-net`. `cargo xtask measure` writes none: it runs the slot and the rollback, and the
  spike's nine ten-minute measurement tests are not in it. The correction rates, the absorbed
  maxima and the divergence are in the rows "The reconciliation tolerance (D124)" and
  "`lactose-predict` is implemented (D122 addendum)", from the `spike-net` lane's runs. They are
  counts under seeded links and scripted clocks, so the machine's load does not move them, but
  section 20 asks for a file. Recommended: a small lane adds a `spike-net` measurement to
  `cargo xtask measure` (five runs), merged before the second quiet window of section 4 so that
  window writes the file. It is a gap between the plan and the tree, not a design choice, so
  the coordinator's to route.
- **The runners' edit-to-playing files are not checked in.** Decision 5.9 lets a runner's
  figure stand for an operating system the desktop does not boot. Once session A or B shows
  which that is, the coordinator checks that runner's file in.

## 2. Criterion 5: where the time goes

**What the criterion reads.** Section 22 says "prediction-world snapshot at most 100 µs". The
plan reads it (section 1, row 5; section 4) as the take of one ring slot, `World::snapshot` plus
`physics_api::snapshot`, through `lactose-predict` with Rapier on the spike's scene, with the
restore recorded. The scene is `cost_of_a_prediction_slot`'s: the character and fifty thrown
bodies 97 ticks after the throw, among five fixed colliders and 59 unclaimed bodies the
prediction world holds as kinematic proxies, the 64 rows D139 admits. The file's `snapshot_us`
is that sum: 140.8 µs. Its `take_us`, 195.3 µs, adds the record `lactose-predict` copies of
every claimed entity's replicated components, 54.4 µs, which the criterion does not read.
Section 10 budgets the restore too, at "about 92 µs at that count": it is 193.0 µs.

The world's part is 5.6 µs. The other 135.2 µs is `physics_api::snapshot`, and all but about
5 µs of that is the backend's `write` (`crates/physics-rapier/src/snapshot.rs`): postcard's
encoding of Rapier's whole world, with each collider's shape exchanged for its `Shape` value.

**The probe.** `snapshot.rs` and `crates/app/tests/spike.rs` at `1d8042e`, instrumented in the
planner's worktree, built in release and reverted before this file was committed. On the scene
above it first takes the slot's figures as the cost test does. Then, in 200 further snapshots,
it encodes each part of the world alone into a reserved buffer and decodes it again, runs
`shapes_out` and `shapes_in` step by step, and clones each part. Every figure is the median of
200. Other agents were building throughout. The measuring thread therefore entered the frame
workers' scheduling class (`ThreadClass::FrameCritical`), which the predictor's thread has in a
shipped client, and each scene was run three to ten times; the figures below are of each
scene's calmest run, at one-minute loads of 5.5 to 9.0. On the criterion's scene the ten runs
gave a physics snapshot of 138.4 to 145.2 µs, a record of 53.0 to 55.9 µs and a restore of
195.3 to 204.8 µs, against the quiet window's 135.2, 54.4 and 193.0 µs. The probe's times are
within a few percent of the quiet window's where the two can be compared.

**The write, by part** (the calmest run of the criterion's scene: load averages 6.83 15.67
16.88; physics snapshot 138.4 µs, of which the backend's write is 133.7 µs):

| Part of the backend's world | Bytes | µs | Share of the write |
|---|---|---|---|
| The narrow phase: 263 contact pairs, each a manifold, 77 of them touching, and the solver's graph over them | 101,179, of which the contact graph is 96,666 | 48.5, of which the contact graph is 42.7 | 36% |
| The collider set's shape exchange, `shapes_out`: Rapier's set encoded (11.9 µs), decoded as an image (11.4 µs) and each shape swapped (3.8 µs), because the set's fields are private | none of its own | 27.2 | 20% |
| The collider set's image, encoded: 115 colliders | 28,106 | 8.8 | 7% |
| The bodies: 51 dynamic, 59 kinematic, 5 fixed | 78,670 | 24.6 | 18% |
| The broad phase's tree and pairs | 14,955 | 11.2 | 8% |
| The islands | 3,448 | 6.6 | 5% |
| The backend's bookkeeping, keyed by `StableKey` | 24,410 | 4.5 | 3% |
| Joints, soft bodies and parameters, all empty here | 269 | 0.3 | under 1% |
| Beside the write, `lactose-physics-api`'s own part: the sync's bookkeeping and pending work | not in the bytes | 4.7 | |

The bytes sum to the file's 251,041 with the four-byte tag. The parts' times sum to 131.7 µs of
the 133.7 µs write measured beside them, so the growth of the output vector costs nothing that
shows.

**What each part follows.** The collider set, the bodies, the broad phase and the bookkeeping
follow the count of rows, claimed or not: about 1.3 kB and 0.6 µs for each fixed collider or
proxy, which is D139's figure. The narrow phase follows the contact pairs, and only claimed
bodies make them here: a pair of kinematic proxies, or of a proxy and a fixed collider, makes
none. A pair costs 367 B whether it touches or not, since Rapier keeps a manifold for every pair
whose bounding boxes overlap. The thrown block makes 32 pairs at ten bodies, 98 at 25, 263 at
fifty and 685 at a hundred, so the pairs grow faster than the bodies, and they follow where the
bodies are at the tick, not how many rows the world holds.

The same scene at other counts (probe; each time the lowest median of three to ten runs, at
one-minute loads of 5.5 to 9.0; the bytes and the pairs are exact):

| Thrown bodies | Contact pairs | With 59 proxies: bytes | physics snapshot, µs | Without proxies: bytes | physics snapshot, µs |
|---|---|---|---|---|---|
| 0 | 1 | 84,671 | 50.5 | 8,958 | 10.8 |
| 10 | 32 | 110,073 | 65.3 | 34,372 | 21.5 |
| 25 | 98 | 154,168 | 87.0 | 78,539 | 46.9 |
| 50 | 263 | 251,041 | 138.4 | 175,344 | 103.2 |
| 100 | 685 | 469,928 | 308.5 | 391,780 | 250.3 |

The row of fifty with proxies is the criterion's scene, 135.2 µs in the quiet window. Without a
single proxy the fifty bodies take 103.2 µs, and 106 µs with the world's part: over the budget
before D139's bound counts anything. The 59 proxies add 35 µs, 0.6 µs each. Beside all 64 rows
the budget holds about 25 bodies in flight: 87.0 µs, and 92 µs with the world's part.

**The restore, by part** (the same run: `read` 162.6 µs and the drop of the world it replaces
14.2 µs, of a physics restore of 183.7 µs). The narrow phase is 64.7 µs. The collider set is
37.3 µs: 10.1 µs to decode its image and 27.2 µs for `shapes_in`, which encodes and decodes it
once more. The bodies are 25.0 µs, the bookkeeping 16.0 µs, the broad phase 12.8 µs, the
islands 5.0 µs, and the fresh pipelines and the check 1.4 µs. Nothing in it is specific to
restoring: it is the write read back.

**Why D139's count was wrong.** D139 sized the bound from `crates/physics-rapier/tests/cost.rs`:
fifty bodies that touch nothing, beside N fixed colliders, 32 µs to write at none and 92 µs at a
hundred. That scene has no contact pair. The spike's fifty bodies in flight have 263, and those
pairs with the fifty bodies' own rows put the write over the budget before any proxy is
counted. The 59 proxies cost what D139 said they would, about 35 µs of the 135.

## 3. The options

**A. A cheaper snapshot in `lactose-physics-rapier`: the slot holds a copy, not bytes.**

Every part of Rapier's world that the bytes carry derives `Clone` in the pinned crate
(`RigidBodySet`, `ColliderSet`, `IslandManager`, `BroadPhaseBvh`, `NarrowPhase`,
`ImpulseJointSet`, `MultibodyJointSet`, `SoftBodySet`), and `PhysicsWorld`'s fields are public.
The backend can therefore copy its world with no patch under `vendor/`. A copy shares each
collider's shape with the live world, since a `SharedShape` is a reference-counted immutable
value, so the whole shape exchange goes, in both directions.

- *What it buys, by the probe.* On the criterion's scene a copy of every part took 28.2 µs
  where the write took 133.7 µs: the narrow phase 14.8 µs, the bodies 4.9 µs, the bookkeeping
  3.6 µs, the broad phase and the colliders 1.8 µs each, the islands 1.3 µs. At every count of
  the table above the copy took between a fifth and a quarter of the write; at a hundred thrown
  bodies among the 64 rows, twice the spike's claim, it took 68.9 µs where the write took
  272.6 µs. With the world's 5.0 µs and `lactose-physics-api`'s 4.7 µs the snapshot of the
  criterion's scene comes to about 38 µs, against 100.
- *Whether the restore can share it.* It can. A restore from a copy is one more copy out of the
  slot, which stays in the ring, a fresh pipeline as a restore from bytes builds, and the drop
  of the world it replaces: 30.9 µs and 13.4 µs in the same run, 44.3 µs where the byte form
  took 176.8 µs. With `lactose-physics-api`'s part and the world's 12.2 µs the restore comes to
  about 63 µs, against the run's 196.0 µs.
- *What it does to a rollback.* On the thrown scene without proxies, the scene of
  `rollback-seven-ticks`, the probe's slot is a snapshot of 106 µs and a record of 46 µs, a
  third of that file's 455.1 µs a tick. A copy takes about 80 µs off each re-simulated tick.
- *Whether a copy is exact.* The probe checked what a planner can. It stashed a copy beside
  each snapshot's bytes and served every restore whose bytes it held from the copy, with a
  fresh physics pipeline, collision pipeline and CCD solver. The spike's four scenes and its
  claim on approach, each run for 1,800 ticks under the drifting clock, gave the byte restore's
  record at every frame (both worlds' hashes and every datagram) and its stream hash: 546
  corrections and 4,217 re-simulated ticks between them, the deepest 39 ticks, 541 restores
  from a copy. With the same switch every test of `lactose-physics-rapier` and `lactose-app`
  that is not ignored passed in release: 156 tests, 25 of which restored from a copy, among
  them the conformance run on every pool size and layout, both in-place reloads,
  `a_restored_backend_writes_the_same_bytes_and_steps_on_identically` and the spike's
  `the_records_are_the_same_under_another_layout_and_worker_count`. SOUR's own modes were not
  run, and nothing ran on x86-64 (section 7).
- *Which parts.* All of them, at once. Two narrower fixes do not reach the budget. Removing the
  shape exchange alone saves about 27 µs and leaves the snapshot near 114 µs. A lighter image
  of the narrow phase, without the pairs that do not touch, is not available: the broad phase
  reports a pair once, when its boxes first overlap (`geometry/broad_phase_bvh/update.rs` of
  the vendored crate), so a narrow phase restored without a pair never gets it back, and
  section 8.2 refuses a patch to the vendored crate that is an optimisation.
- *What it costs.* One lane, `slot-copy`, seven runs, and a second quiet window for two
  measurements. In memory a copy is larger than the bytes it replaces. On the criterion's scene
  a copy weighs 537 kB in 789 allocations, where the encoded bytes hold 262 kB of capacity for
  251,041 written (probe, under an accounting allocator). With the sync's bookkeeping, 102 kB
  either way, a slot's physics is about 640 kB in place of 364 kB, and a ring of sixty slots
  about 38 MB in place of 22 MB.
- *What it changes in the documents.* `lactose-physics-api`'s backend trait gains a copy and a
  restore from one, beside `snapshot` and `restore`, whose byte form and contract stay: the
  fresh solver's bytes (section 16, D124), the journal checkpoint (section 15.5) and the
  physics hash (section 8.3) are bytes and stay bytes. That is an amendment to an interface
  section 20 freezes once its first backend conforms; section 20 permits it in the change that
  needs it, called out, and D143 calls it out. Architecture section 10 changes in two places,
  section 22 gains one sentence under its table, and section 23's COW row changes one word.
  Section 22's row is not touched. The plan gains a lane and a rule (section 6).
- *What it rules out.* A slot that leaves the process without being encoded first. The
  checkpoint and a solver baseline sent with a full frame (the row "Solver history departure")
  encode when they are taken, at the byte form's cost, and neither is taken every tick.

**B. A smaller claimed set, or another bound in D139.**

D139's bound limits fixed colliders and proxies, and no value of it reaches the budget: with no
proxy and five fixed colliders the fifty bodies' snapshot is 106 µs (probe), so the byte form
misses at a bound of zero. What fits is a smaller claimed set: beside all 64 rows, about 25
bodies in flight, half of what section 22's spike throws. Even that counts the wrong thing. The
cost follows contact pairs, 263 for these fifty at this tick and 685 for a hundred.

- *What it costs.* No lane. Criterion 5 would be read on a scene the netcode spike does not
  run, and section 22's "fifty claimed dynamic bodies thrown and interacting" would hold only
  outside the budget its own milestone sets.
- *What it changes.* D139's bound, section 10, the plan's reading of criterion 5, and the claim
  policy `lactose-net` is to write at milestone 4, which would have to count contact pairs.
- *What it rules out.* A prediction world that holds what the spike throws. A bound a game can
  author against: the cost follows contact pairs, and no content rule counts those.

**C. A changed budget.**

Only the owner grants it. The quiet figure is 140.8 µs, so a budget of 150 µs passes with 6% to
spare and breaks at the next Rapier bump or the next scene; 200 µs is the number that holds.

- *What the budget protects.* Not the predicted tick: 140.8 µs is under a hundredth of a 60 Hz
  frame. It protects the rollback. A re-simulated tick takes a slot again, and
  `rollback-seven-ticks` puts a tick of the thrown scene at 455.1 µs, a third of it the slot's
  snapshot and record (option A). The ring's depth is the frame's re-simulation budget divided
  by that cost (section 9.4), and the spike's ordinary corrections are eight or nine ticks
  deep.
- *What it costs.* No lane, and no second quiet window.
- *What it changes.* Section 22's row, which only the owner may change, and the sentence of
  section 10 that names the budget and the restore.
- *What it rules out.* Nothing now. It leaves the encode, decode and exchange in every
  predicted and every re-simulated tick, and it leaves the restore, which section 10 also
  budgets, at 193.0 µs.

## 4. Recommendation

**A: the slot holds the backend's world as a copy, and section 22's 100 µs stands.** The
criterion is missed because a slot pays to serialise a world that never leaves the process. A
copy removes that cost and leaves the budget, the bound and the spike's scene as they are. The
probe puts the copied snapshot near 38 µs and the restore near 63 µs, and it re-simulated every
scene of the spike to the records the byte restore gives.

The lane, as the plan's section 3 states lanes:

| Lane | What it does | Needs | Proof it owes | Runs |
|---|---|---|---|---|
| `slot-copy` | The prediction slot holds the backend's world as a copy (D143). `lactose-physics-api`: the backend trait gains a copy and a restore from one, with the byte form as their default, and `PhysicsSnapshot` holds the copy and encodes it only when asked for its bytes or its hash. `lactose-physics-rapier`: the copy is a clone of every set and of the bookkeeping, shapes shared, and a restore from it builds a fresh pipeline. The interface amendment is called out in its merge (section 20). The three READMEs' slot sentences | `c9b-renames` merged, which renames test glue in the crates it edits, and `stream-impl` merged, which rewrites `crates/predict/README.md` | `physics-api`'s conformance run through the copy, and again through the bytes of a copy, on every pool size and layout. The spike's four recording tests record the hashes they recorded before the change, in the pull-request, layout and chaos modes. A copy that leaves out a part fails conformance. `cargo xtask measure prediction-slot rollback-seven-ticks` before and after, with load, as steering. The ring's memory per slot | 7 |

It merges after the measurement commit and edits a measured crate, so the plan's fifth rule
binds it as well: it reports `physics-phase`, `composed-tick`, `reload-100k` and `bone-bodies`
as steering. It needs no facade first. The amendment is two methods with default bodies and
one opaque type, and its one consumer is `PhysicsSnapshot`. It adds no thread and no `unsafe`,
so `reviewer` reads it alone.

Criterion 5 is then proven by a second quiet window, the coordinator's, at the first commit of
main that holds `slot-copy`: `cargo xtask measure prediction-slot rollback-seven-ticks`, two
files that replace the measurement commit's for criteria 5 and 7. Every other file of the
measurement commit stands. The same window confirms D139's bound of 64 or changes it, as
section 10 already says the measurement does.

Cost: seven runs for the lane, two for this document's review and answer, and the second quiet
window.

What would change the recommendation:

- A SOUR mode in which a restore from a copy records another hash than a restore from bytes:
  layout, chaos, or architecture mode on x86-64. The copy is then not exact, the lane stops, and
  the budget comes back to the owner as option C.
- The second quiet window putting the copied snapshot over 100 µs. The copy stays, since it is
  the cheaper form, and the budget comes back to the owner once. No second lane is spent on it.

Two things this document does not ask for, and where they go:

- **The slot's record.** `lactose-predict` copies every claimed entity's replicated components
  into a map of maps after each tick: 54.4 µs for 51 entities, more than the copied snapshot
  would be. The criterion does not read it, and it is private to the crate, so a flat buffer
  changes no signature. It is a verification row for milestone 4, when `app` sizes the ring's
  depth from the measured cost of a tick (D122).
- **The bound's enforcement.** It stays a content rule that nothing enforces, until milestone 3
  designs what replaces it (section 10).

## 5. The question for the owner

The coordinator asks it; the recommended option is first.

> Criterion 5 of milestone 1, the prediction-world snapshot at most 100 µs, measured 140.8 µs
> on the quiet M1. The slot serialises Rapier's whole world every tick, and the fifty claimed
> bodies alone put it over the budget before any proxy is counted. How is it closed?
>
> 1. **Hold the slot as a copy; keep 100 µs (recommended).** One lane of seven runs changes
>    what a slot holds, with a small amendment to `lactose-physics-api`'s backend trait; a
>    probe puts the copied snapshot near 38 µs and re-simulated every scene of the spike to the
>    same hashes as today's restore. Criterion 5 is measured again in a second quiet window. If
>    it is still over, the budget comes back to you.
> 2. **Restate the budget at 200 µs.** No lane. Section 22's row changes and the bound of 64
>    is confirmed as measured. Every predicted and re-simulated tick keeps paying the
>    serialisation, and the restore stays at 193 µs.
> 3. **Shrink what a prediction world may claim.** No lane. The spike's fifty thrown bodies no
>    longer fit the budget their milestone sets; it comes back to the planner for the new
>    bound.

## 6. Text for the coordinator (exact)

D143 is the next free number. The first block is for the recommended answer, and the
coordinator writes the date of the owner's answer where it says `<date>`. Each amendment is
anchored on the sentence it replaces, as it stands at `1d8042e`.

### Decision entry

```
**D143. The prediction slot holds the backend's world as a copy, not as bytes; criterion 5 keeps its 100 µs and is measured again** (§8.3, §10, §15.5, §16, §20, §22, §23, `lactose-physics-api`, `lactose-physics-rapier`, `lactose-predict`, `docs/plans/milestone-1.md`)
- Why: the quiet window on the Apple M1 at `bef3455` put the prediction-world snapshot at
  140.8 µs against section 22's 100 µs (`docs/results/prediction-slot-apple-m1-2026-11-15.md`):
  the world's 5.6 µs and the physics world's 135.2 µs, on the spike's thrown scene, fifty
  claimed bodies among five fixed colliders and 59 proxies, the 64 D139 admits. The restore was
  193.0 µs, where section 10 said about 92. D139 sized the bound from fifty bodies that touched
  nothing. The spike's bodies in flight hold 263 contact pairs, and Rapier's bytes carry a
  manifold for each. A probe at `1d8042e` (`docs/plans/criterion-5-proposal.md`, section 2)
  splits the backend's write: the narrow phase 36%, the collider set 27%, three quarters of
  that the encode, decode and exchange that Rapier's private fields force on the shape swap,
  the bodies 18%, the broad phase, the islands and the bookkeeping the rest. Without a single
  proxy the fifty bodies' snapshot is still 106 µs, so no bound on rows brings the bytes inside
  the budget. The cost is serialising a world that never leaves the process.
  - *A slot holds the backend's world as a copy the backend makes.* `PhysicsBackend` gains a
    copy of its world, as a value the backend defines, and a restore from one, beside
    `snapshot` and `restore`; a backend with nothing cheaper keeps their default, which goes
    through its bytes. `PhysicsSnapshot` holds the copy beside the sync's bookkeeping and
    pending work, and encodes it only when its bytes or its hash are asked for. The Rapier
    backend's copy is a clone of every set and of its bookkeeping; a collider's shape is
    shared with the live world, since a shape is immutable once built. No path that runs every
    tick encodes; what the ring reports as a slot's physics size is the lane's to state. Rules
    out a slot that pays for an encoding nobody reads, and a patch to the vendored crate.
  - *The byte form stays, with its contract.* The bytes `physics-api` keeps from setup and
    restores into an emptied solver (D124), the journal checkpoint (section 15.5) and the
    physics hash (section 8.3) are the backend's bytes, as D119 defined them. A world restored
    from the bytes of a copy steps as the copy does, and the conformance run holds both.
  - *A restore from a copy builds a fresh physics pipeline, collision pipeline and CCD solver*,
    as a restore from bytes does. Rules out a restore that keeps the live pipeline: its solver
    keeps caches keyed by the island manager's epoch, which a restored world can repeat.
  - *This amends a frozen interface.* `lactose-physics-api` froze when the Rapier backend
    passed its conformance run (section 20). The amendment is two methods with default bodies
    and one opaque type, made in the change that needs it, and called out here.
  - *Section 22's 100 µs stands, and the copied slot is measured in a second quiet window*, at
    the first commit of main that holds the lane: `cargo xtask measure prediction-slot
    rollback-seven-ticks`, whose two files replace the measurement commit's for criteria 5 and
    7. The probe's figures, a copy at 28 µs where the write took 134 µs and a restore from it
    at 44 µs where the byte restore took 177 µs, steer and prove nothing.
  - *D139's bound of 64 fixed colliders and proxies stands until that measurement*, which
    confirms it or changes it. The reading of criterion 5 is the plan's, now in section 22: the
    world snapshot and the physics snapshot of one slot on the spike's thrown scene at the
    bound, with the slot's record and its restore recorded beside it.
  - *If the copied slot is over 100 µs on the quiet M1, the budget comes back to the owner
    once.* Rules out a second lane on the slot's cost in milestone 1.
  - *Not decided here*: the slot's record of the claimed entities, 54.4 µs of every take, which
    the criterion does not read (a row gates it at milestone 4); what enforces the bound, which
    stays milestone 3's.
- Driven by: the quiet window's result files (`1d8042e`); the second review of lane `measure`,
  which found the claimed bodies' contact graph and the shape exchange under the cost; the
  planner's probe; the owner's answer of <date>.
- Replaced: D139's "about 0.6 µs each to write and about 1.1 µs for a fixed collider to
  restore" as the whole cost of a slot, and D140's "The bound keeps a slot's take inside
  section 22's 100 µs budget": the bound is not what holds it. D119's "The bytes are the
  backend's whole world" as what a slot holds; the bytes and their contract stand for what
  leaves the process. Section 10's "its restore, about 92 µs at that count".
```

### If the owner restates the budget instead

The entry is this, section 22's row replaces "prediction-world snapshot at most 100 µs" with
"prediction-world snapshot at most 200 µs", and in section 10 "which keeps a slot's take inside
section 22's 100 µs budget with the pinned backend, and its restore, about 92 µs at that count,
under it" becomes "at which a slot's snapshot is 140.8 µs on a quiet Apple M1, inside section
22's 200 µs budget with the pinned backend; its restore, 193.0 µs, is recorded and not
budgeted". Of the rest of this section only the verification rows apply, with four changes.
In the row "Prediction-world snapshot cost" everything from "Decided (D143):" on becomes
"Decided (D143): section 22's budget is restated at 200 µs, inside which the snapshot is, and
the bound of 64 is confirmed." In the row "Section 22's milestone 1 timing criteria" the
sentence that begins "D143 decides criterion 5" becomes "D143 restates criterion 5 at 200 µs."
In the row "A prediction world holds at most 64" the last sentence becomes "The budget is
restated at 200 µs (D143), at which the bound of 64 is confirmed." The new row "The prediction
slot as a copy (D143)" is not added. The README sentences keep their figures and lose their
clauses on the copy.

```
**D143. Criterion 5 is restated: the prediction-world snapshot is at most 200 µs** (§10, §22)
- Why: the quiet window on the Apple M1 at `bef3455` put the prediction-world snapshot at
  140.8 µs against section 22's 100 µs (`docs/results/prediction-slot-apple-m1-2026-11-15.md`),
  on the spike's thrown scene with fifty claimed bodies among the 64 fixed colliders and
  proxies D139 admits, and its restore at 193.0 µs. The cost is the backend's serialisation of
  the claimed bodies' contact pairs as much as of the rows the bound counts
  (`docs/plans/criterion-5-proposal.md`, section 2).
  - *The budget is 200 µs*, the quiet figure with room for a backend bump and a denser scene.
  - *The bound of 64 stands*, confirmed at that budget by the measurement section 10 names.
  - *The restore is recorded, not budgeted.*
  - Rules out, for milestone 1, the lane that would hold the slot as a copy; the proposal
    keeps its probe's figures for the milestone that sizes the ring.
- Driven by: the quiet window's result files (`1d8042e`); the owner's answer of <date>.
- Replaced: section 22's "prediction-world snapshot at most 100 µs"; section 10's "about 92 µs
  at that count".
```

### Architecture amendments

For the recommended answer. They state the decision before the lane lands, as section 8.2's
NaN assertion was stated; the row "The prediction slot as a copy (D143)" below says what is not
built yet.

```
--- section 10, first paragraph: replace ---
proxy history and the physics world through `physics-api::snapshot`, which carries the backend's whole world with each collider's shape value and the structural-log entries and changed keys the physics sync has not yet consumed,
--- with ---
proxy history and the physics world through `physics-api::snapshot`, which carries the backend's whole world as a copy the backend makes, each collider with its shape, and the structural-log entries and changed keys the physics sync has not yet consumed,

--- section 10, third paragraph: replace ---
A slot's cost is dominated by the backend's serialisation of every body and collider it holds, bytes that copy-on-write in `predict` cannot share between slots, so copy-on-write snapshots are not built. The bound is on what the world holds instead: at most 64 fixed colliders and proxies together as rows, beside the fifty claimed bodies of section 22, which keeps a slot's take inside section 22's 100 µs budget with the pinned backend, and its restore, about 92 µs at that count, under it. The milestone 1 measurement of a slot, proxies included, confirms the count or changes it.
--- with ---
A slot's cost is the backend's world, every body, collider and contact pair it holds, which copy-on-write in `predict` cannot share between slots, so copy-on-write snapshots are not built. The slot holds that world as a copy the backend makes of its own state, each collider's shape shared with the live world, and never as bytes: as bytes, the fifty claimed bodies of section 22 among 64 fixed colliders and proxies took 140.8 µs to snapshot and 193.0 µs to restore on a quiet Apple M1, against section 22's 100 µs, and a third of the write was the claimed bodies' contact pairs, which no bound on rows holds down (D143). The backend's bytes remain what leaves the process or is compared: the journal checkpoint (section 15.5), the fresh solver's bytes (section 16) and the physics hash (section 8.3). The bound on what the world holds stands: at most 64 fixed colliders and proxies together as rows, beside the fifty claimed bodies. The measurement of the copied slot at that count, on a quiet machine, confirms the count or changes it.

--- section 22, the paragraph "Milestone 1's criteria are read as follows (D139).": after the sentence that ends "and a shipped binary only when it asks." insert ---
The prediction-world snapshot is the world snapshot and the physics snapshot of one ring slot, on the netcode spike's thrown scene among the fixed colliders and proxies section 10 admits; the slot's record of the claimed entities and its restore are recorded beside it (D143).

--- section 23, the row "COW": replace ---
Copy-on-write snapshots. Not built: a slot's cost is the physics backend's bytes, which copy-on-write cannot share between slots (section 10).
--- with ---
Copy-on-write snapshots. Not built: a slot's cost is the physics backend's world, which copy-on-write cannot share between slots (section 10).
```

Sentences checked and left as they are, since they read true of the byte form D143 keeps:
section 8.3's "Hashing `physics-api::snapshot` bytes is added once Rapier's serialisation is
shown to be order-stable"; section 15.5's "A checkpoint carries the physics snapshot of section
10 as bytes"; section 16's fresh-solver rule and its "Replacement is a backend swap behind
`physics-api` including `snapshot` and `restore`"; section 9.4's "at most 64 fixed colliders and
proxies together (section 10)". `CLAUDE.md`'s COW line cites D139 and D140 and stands.

### Plan amendments

`docs/plans/milestone-1.md`, for the recommended answer.

```
--- section 1, the row of criterion 5: replace its last two cells ---
| Not measured as a slot. Its parts: the world snapshot 5.4 µs with no statics (row 17); Rapier's write 32 µs beside no fixed collider and 92 µs beside 100 (row 87) | `measure`, confirmed by `spike-net`; decision 5.5 |
--- with ---
| Not measured as a slot. Its parts: the world snapshot 5.4 µs with no statics (row 17); Rapier's write 32 µs beside no fixed collider and 92 µs beside 100 (row 87). The quiet window measured the slot at 140.8 µs: not met (D143, `docs/plans/criterion-5-proposal.md`) | `measure`; `slot-copy`, then the second quiet window (section 3); decision 5.5 as D143 amends it |

--- section 3, the second lane table: add a row after `readme-sweep` ---
| `slot-copy` | The prediction slot holds the backend's world as a copy (D143). `lactose-physics-api`: the backend trait gains a copy and a restore from one, with the byte form as their default, and `PhysicsSnapshot` holds the copy and encodes it only when asked for its bytes or its hash. `lactose-physics-rapier`: the copy is a clone of every set and of the bookkeeping, shapes shared, and a restore from it builds a fresh pipeline. The interface amendment is called out in its merge (section 20). The three READMEs' slot sentences | `c9b-renames` merged, which renames test glue in the crates it edits, and `stream-impl` merged, which rewrites `crates/predict/README.md` | `physics-api`'s conformance run through the copy, and again through the bytes of a copy, on every pool size and layout. The spike's four recording tests record the hashes they recorded before the change, in the pull-request, layout and chaos modes. A copy that leaves out a part fails conformance. `cargo xtask measure prediction-slot rollback-seven-ticks` before and after, with load, as steering. The ring's memory per slot | 7 |

--- section 3, after the paragraph that begins "The whole is about 165 runs": add a paragraph ---
D143 adds `slot-copy`, seven runs, and two for its proposal's review and answer.

--- section 3, the queue table: replace ---
| 2, the client and then the save path | `client-fix`, `sour-evidence`, `spike-net`, `stream-impl`, `data-path`, `binds-design`, `binds-impl` | 49 |
--- with ---
| 2, the client and then the save path | `client-fix`, `sour-evidence`, `spike-net`, `stream-impl`, `slot-copy`, `data-path`, `binds-design`, `binds-impl` | 56 |

--- section 3: replace ---
Five rules bind whatever the coordinator reorders:
--- with ---
Six rules bind whatever the coordinator reorders:

--- section 3, after the rule that begins "A lane that merges after the measurement commit": add a rule ---
- Criterion 5 is proven at the first commit of main that holds `slot-copy`, not at the
  measurement commit. The coordinator runs `cargo xtask measure prediction-slot
  rollback-seven-ticks` there in a second quiet window, and those two files replace the
  measurement commit's for criteria 5 and 7. Every other file of the measurement commit stands,
  and session B is unchanged: its slot figure is recorded, of the byte form (D143).

--- section 7, step 1: replace ---
1. The measurement commit exists. The quiet window and session B have run on it, and their result
   files are on main. Session B's run on eight frame workers is criterion 1's proof.
--- with ---
1. The measurement commit exists. The quiet window and session B have run on it, and their result
   files are on main. Session B's run on eight frame workers is criterion 1's proof. The second
   quiet window has run at the first commit that holds `slot-copy`, and its two files are
   criterion 5's proof and criterion 7's rollback figure (D143).
```

### Verification rows

`docs/verification.md`. Each change is an exact replacement inside the row its opening words
name, or an append before the row's gate cell. Every quiet figure is from the Apple M1 at
`bef3455`, with the load averages its file records. The sentences that name D143 are written
for the recommended answer.

```
--- "| Prediction-world snapshot cost": replace ---
After the ECS fork's merge, under agent load: snapshot 151.5 µs (world 5.8, physics 145.7), restore 205.2 µs.
--- with ---
After the ECS fork's merge, under agent load: snapshot 151.5 µs (world 5.8, physics 145.7), restore 205.2 µs. Quiet window (`docs/results/prediction-slot-apple-m1-2026-11-15.md`, load averages 1.51 2.09 3.69): snapshot 140.8 µs (world 5.6, physics 135.2), record 54.4 µs, restore 193.0 µs (world 12.3, physics 180.6): criterion 5 is not met, and the restore is over section 10's figure. The planner's probe at `1d8042e` (`docs/plans/criterion-5-proposal.md`, section 2; the calmest of ten runs, load averages 6.83 15.67 16.88, physics snapshot 138.4 µs) splits the backend's write of 133.7 µs: the narrow phase 48.5 µs, which is the claimed bodies' 263 contact pairs, a manifold each; the collider set 36.0 µs, 27.2 µs of it the encode, decode and exchange of `shapes_out`; the bodies 24.6 µs; the broad phase 11.2 µs; the islands 6.6 µs; the bookkeeping 4.5 µs. Without a proxy the same fifty bodies' snapshot is 106 µs, so no count of rows brings the byte form inside the budget; the 59 proxies add 0.6 µs each, as D139 estimated. Decided (D143): a slot holds the backend's world as a copy the backend makes, shapes shared, and the bytes stay for the fresh solver, the journal checkpoint and the hash; the probe's copy took 28.2 µs and a restore from it 44.3 µs where the byte restore took 176.8 µs, and it re-simulated the spike's four scenes and its claim on approach to the byte restore's records over 1,800 ticks each. The budget stays 100 µs and the bound stays 64 until the copied slot is measured in a second quiet window, at the first commit that holds lane `slot-copy`.

--- "| Physics phase share": replace ---
Under agent load on the M1 at the lane's head after the ECS fork's merge: T₁ 15,815.3 µs, of which physics 3,564.4 µs; T₄ 7,354.0 µs, of which physics 2,602.1 µs.
--- with ---
Quiet window (`docs/results/composed-tick-apple-m1-2026-11-15.md`, load averages 1.43 2.09 3.71): T₁ 13,808.9 µs, of which physics 3,156.1 µs, 23%; T₄ 5,772.5 µs, of which physics 2,265.8 µs, 39%. It has no threshold (D139). The x86 desktop's file is session B's.

--- "| Physics costs on the milestone 1 ECS": replace ---
Under agent load on the M1 at the lane's head after the ECS fork's merge: sync 596.1, step 1,047.3, write-back 450.2, phase 2,078.9 µs, on 4 frame workers and 3 solver threads.
--- with ---
Quiet window (`docs/results/physics-phase-apple-m1-2026-11-15.md`, load averages 1.43 2.09 3.71): sync 554.3, step 985.4, write-back 419.0, phase 1,954.3 µs, on 4 frame workers and 3 solver threads: criterion 4 is met on the M1, and session B gives the x86 desktop's figure. The snapshot figures of this row, fifty bodies beside N fixed colliders, are of bodies that touch nothing: they hold no contact pair and do not bound a slot (the row "Prediction-world snapshot cost", D143).

--- "| A milestone 1 tick at 100k entities": replace ---
Under agent load on the M1 at the lane's head after the ECS fork's merge, each per-entity system's units forked over the workers (D137 part B): T₁ 10,849.4 µs, T₄ 4,445.0 µs, S 2,310.2 µs, projected to eight workers 3,377.6 µs; hash 0x11c1a653b85d7d3d.
--- with ---
Quiet window (`docs/results/tick-100k-apple-m1-2026-11-15.md`, load averages 1.47 2.11 3.72 before and 1.43 2.09 3.71 after), each per-entity system's units forked over the workers (D137 part B): T₁ 10,193.0 µs, T₄ 3,344.6 µs, S 1,061.8 µs, projected to eight workers 2,203.2 µs; 7·T₄ − T₁ is 13.2 ms against the 24 ms that steers; hash 0x11c1a653b85d7d3d. The projection steers; session B's run on eight performance cores is the proof.

--- "| An empty solver is a fresh solver (D124)": replace ---
Under agent load on the M1 at the lane's head after the ECS fork's merge: the reload tick's step 467.1 ms, its sync 144.9 ms, against 2.87 ms (sync 0.68 ms) for a normal tick.
--- with ---
Quiet window (`docs/results/reload-100k-apple-m1-2026-11-15.md`, load averages 1.43 2.09 3.71 before and 1.56 2.10 3.70 after): the reload tick's step 392.5 ms, its sync 127.0 ms, against 2.27 ms (sync 0.57 ms) for a normal tick. It has no threshold.

--- "| Sim-skeleton evaluation": replace ---
Result file `sim-skeleton`, its figure (b).
--- with ---
Result file `sim-skeleton`, its figure (b). Quiet window (`docs/results/sim-skeleton-apple-m1-2026-11-15.md`, load averages 1.51 2.09 3.69 before and 1.51 2.07 3.66 after): (a) median 160.5 µs; (b) the step with the skeleton 229.0 µs on one worker and 136.0 µs on four, 180.8 µs and 98.4 µs more than without it: criterion 6 is met on the M1, and session B gives the x86 desktop's figure.

--- "| Bodies' cost per tick": replace ---
(the app's solver pool, under agent load: 8,893.9 µs of bodies per tick on one worker, 9,108.5 on four)
--- with ---
(the app's solver pool; quiet window, `docs/results/bone-bodies-apple-m1-2026-11-15.md`, load averages 1.56 2.10 3.70 before and 1.51 2.09 3.69 after: 7,382.5 µs of bodies per tick on one worker, 6,098.2 on four)

--- "| The headless binary's 120-tick run": replace ---
the lane's head after the ECS fork's merge prints 0x92ff315464f84575 on the M1.
--- with ---
the quiet window's run at `bef3455` prints 0x92ff315464f84575 on the M1 (`docs/results/headless-120-apple-m1-2026-11-15.md`), which is the hash session B compares on x86-64.

--- "| `lld-link` on Windows": replace ---
Under heavy agent load on the M1 (load averages 23 to 45): slower median 3.90 s (sketch 2.58 s, app function 3.90 s); a second run, at load 23 falling to 9, gave 2.26 s (sketch 1.85 s). The `measure` workflow gives a windows-x64 runner's figure
--- with ---
Quiet window (`docs/results/edit-to-playing-apple-m1-2026-11-15.md`, load averages 3.79 7.12 5.87 before and 8.10 7.89 6.19 after, the task benchmark's and the measurement's own builds): slower median 1.56 s (sketch 1.51 s, app function 1.56 s) against 30 s, so criterion 9 is met on macos-arm64. The `measure` workflow's run 43659357193 at `bef3455` gave 3.21 s on a linux-x64 runner (sketch 0.93 s) and 5.83 s on a windows-x64 runner (sketch 1.89 s), each on one frame worker; a runner's file is checked in for an operating system the desktop does not boot (`docs/plans/milestone-1.md`, decision 5.9)

--- "| Section 22's milestone 1 timing criteria are proven by result files": append ---
The quiet window ran on the M1 at `bef3455` on 15 November 2026 (`docs/results/*-apple-m1-2026-11-15.*`, checked in at `1d8042e`): criteria 4, 6 and 9 are met there, criterion 1's projection is 2,203.2 µs, and criterion 5 is not met at 140.8 µs. D143 decides criterion 5: the slot holds a copy, and a second quiet window at the first commit that holds lane `slot-copy` writes `prediction-slot` and `rollback-seven-ticks` again (`docs/plans/criterion-5-proposal.md`). No file of session B is in `docs/results` yet.

--- "| Tick-start latency against the 20 µs P99 criterion": append ---
The quiet window (`docs/results/whisk-apple-m1-2026-11-15.md`, `bef3455`, load averages 1.51 2.07 3.66 before and 4.04 7.23 5.90 after, the benchmark's own): at 60 Hz P50 2.6 µs, P99 27.0 µs, max 69.6 µs, none of 5,000 over 1 ms. Not met, as on 9 and 10 October; D138's one attempt and its restatement at 35 µs stand.

--- "| Wake latency against the 20 µs P99 criterion": append ---
The quiet window (`docs/results/whisk-apple-m1-2026-11-15.md`): push from an idle pool P50 13.3 µs, P99 31.6 µs, under D113's 35 µs.

--- "| `lactose-predict` is implemented (D122 addendum)": append ---
Quiet window (`docs/results/rollback-seven-ticks-apple-m1-2026-11-15.md`, load averages 1.51 2.09 3.69): a seven-tick rollback on the thrown scene 3,185.9 µs, 455.1 µs a tick, recorded with no budget. The slot at the bound of 64 did not confirm it: 140.8 µs to snapshot and 193.0 µs to restore (the row "Prediction-world snapshot cost", D143).

--- "| A prediction world holds at most 64 fixed colliders and proxies together as rows (D139)": replace ---
with Rapier a slot's take reaches the 100 µs budget near 100 fixed colliders beside fifty bodies and its restore near 75, and a kinematic proxy is serialised as a fixed collider is.
--- with ---
the estimate that with Rapier a slot's take reaches the 100 µs budget near 100 fixed colliders beside fifty bodies, and its restore near 75, was of bodies that touch nothing; the quiet slot at 64, its fifty thrown bodies holding 263 contact pairs, took 140.8 µs to snapshot and 193.0 µs to restore. A fixed collider or a kinematic proxy costs the byte form about 0.6 µs to write, as estimated, and the claimed bodies' contact pairs cost the rest. The slot holds a copy instead (D143), and the bound stands at 64 until the copied slot's quiet measurement confirms it or changes it.

--- section 4, add rows ---
| The prediction slot as a copy (D143): `PhysicsBackend` gains a copy of its world and a restore from one, `PhysicsSnapshot` holds the copy, and the bytes stay for the fresh solver, the journal checkpoint and the hash. Not built until lane `slot-copy`, whose proof is `physics-api`'s conformance run through the copy and through the bytes of a copy, and the spike's recording tests recording the hashes they recorded before it. To measure with it: the copied slot in a second quiet window (`cargo xtask measure prediction-slot rollback-seven-ticks`), which proves criterion 5 and confirms or changes the bound of 64; the ring's memory per slot, which the planner's probe put at 537 kB for a copy where the bytes hold 262 kB, beside 102 kB of bookkeeping either way, on the criterion's scene; and SOUR's architecture mode over the spike's records after the lane, since the probe ran on aarch64 alone. If the copied snapshot is over 100 µs on the quiet M1 the budget returns to the owner | 1 |
| The slot's record (D143): `lactose-predict` copies every claimed entity's replicated persistent components into a map of maps after each tick, 54.4 µs for 51 entities on the quiet M1 (`docs/results/prediction-slot-apple-m1-2026-11-15.md`), which criterion 5 does not read and every predicted and re-simulated tick pays. The slot is private, so a flat buffer changes no signature. Decide it when `app` sizes the ring's depth from the measured cost of a tick (D122) | 4 |
| The netcode spike's baseline has no result file: the plan names `spike-net`, `cargo xtask measure` writes none, and the correction rates, absorbed maxima and divergence are in the rows "The reconciliation tolerance (D124)" and "`lactose-predict` is implemented (D122 addendum)" from the lane's runs under load. They are counts under seeded links and scripted clocks, which load does not move; architecture section 20 asks for a file all the same. Add the spike's measurement tests to `cargo xtask measure` before the second quiet window, or record at the exit why the rows stand | 1 |
```

### README sentences for `readme-sweep`

These are true from D143's commit until `slot-copy` merges, which rewrites the cost sentences
with its own figures. Line numbers are at `1d8042e`.

`crates/predict/README.md`:

```
--- lines 7-8: replace ---
Copy-on-write snapshots (COW) are not built unless
the milestone 1 measurement says so; nothing here would change if they were.
--- with ---
Copy-on-write snapshots (COW) are not built (D139, architecture section 10): a slot's cost is
the physics backend's world, which they could not share between slots.

--- line 58: replace ---
so a snapshot's representation (a full copy today, copy-on-write if measured necessary) is
--- with ---
so a snapshot's representation (a full copy of the world, and the physics world as
`lactose-physics-api` holds it) is

--- lines 242-256, from "The milestone 1 measurement of the snapshot cost per" to "changes nothing here.": replace with ---
  The budget is 100 µs for the world snapshot and the physics snapshot of one slot (section
  22), and a prediction world holds at most 64 fixed colliders and proxies together as rows
  (D139, architecture section 10), a content rule nothing here enforces. On a quiet Apple M1
  the slot of the spike's thrown scene at that count took 140.8 µs to snapshot, 135.2 µs of it
  the physics snapshot, and 193.0 µs to restore
  (`docs/results/prediction-slot-apple-m1-2026-11-15.md`): over the budget. The physics part
  of a slot therefore becomes a copy the backend makes (D143), which lane `slot-copy` builds
  and a second quiet window measures. `crates/predict/tests/cost.rs` times `World::snapshot`
  and `physics_api::snapshot` with the physics test backend, for fifty claimed dynamic bodies
  plus 0 to 10,000 statics, and says nothing of Rapier's cost. No signature holds a snapshot
  and the slot is private, so what a slot holds changes nothing here.

--- lines 1076-1077: replace ---
- Copy-on-write snapshots, or a cap on the statics a prediction world holds as rows:
  decided by the milestone 1 measurement; private to the slot.
--- with ---
- Copy-on-write snapshots: not built (D139). The cap on what a prediction world holds as rows,
  64 fixed colliders and proxies together, is a content rule nothing enforces (architecture
  section 10); milestone 3 designs what replaces it.
```

`crates/physics-api/README.md`:

```
--- lines 575-577, the first sentence: replace ---
- `PhysicsSnapshot::size_in_bytes` is the backend's part, what milestone 1 measures
  against the 100 µs prediction-world budget for 50 claimed bodies plus N statics; that
  measurement decides how many statics the prediction world may hold.
--- with ---
- `PhysicsSnapshot::size_in_bytes` is the backend's part. A prediction world holds at most 64
  fixed colliders and proxies together as rows (D139, architecture section 10). At that count,
  with the spike's fifty thrown bodies, the physics snapshot took 135.2 µs and its restore
  180.6 µs on a quiet Apple M1 (`docs/results/prediction-slot-apple-m1-2026-11-15.md`), over
  section 22's 100 µs before the world's part. A slot is therefore to hold the backend's world
  as a copy, and these bytes stay for the fresh solver, the journal checkpoint and the hash
  (D143; lane `slot-copy`).
```

`crates/physics-rapier/README.md`, which the sweep's list did not name:

```
--- "Known gaps", the item "Snapshot cost": after the sentence that ends "Rapier's own encoding of its sets is most of the bytes." add ---
  These fifty bodies touch nothing. Bodies thrown together cost more than the fixed colliders
  beside them: the spike's fifty among 64 fixed colliders and proxies take 135.2 µs to snapshot
  and 180.6 µs to restore on a quiet Apple M1, a third of the write their 263 contact pairs,
  of which 77 touch (`docs/results/prediction-slot-apple-m1-2026-11-15.md`, D143).
```

The sweep's other sentences (`crates/anim/README.md` on `on_load`, `crates/persist/README.md`
on the asset set) do not depend on D143.

## 7. What this document could not verify

- **No probe figure is quiet.** A reviewer and a lane were building throughout. The runs quoted
  are the calmest of three to ten, at one-minute loads of 5.5 to 9.0, with the measuring thread
  in the frame workers' scheduling class to hold a performance core, which the quiet window's
  thread did not need. The one check on them is the criterion's scene, where the probe's slot
  came within 8% of the quiet window's. Earlier runs at loads of 12 to 37 were 1.3 to 2.3
  times slower and are not quoted. The second quiet window is the measurement.
- **A copy's exactness is shown on aarch64, in release, in the default layout**, and in one
  other layout and worker count through the one spike test that compares them. The probe ran
  neither SOUR's layout and chaos modes nor x86-64. The lane owes the first two and the nightly
  the third.
- **The probe's copy kept the bytes beside it.** It restored from a copy only when the bytes it
  was handed matched a stashed snapshot, so it never exercised a slot with no byte form, a
  `PhysicsSnapshot` that encodes on demand, or the trait's default path. Those are the lane's.
- **The copy's memory is one sample per scene**, and it includes Rapier's workspace buffers,
  which a lane may leave out.
- **Session A, session B and the nightly were not read.** No file of either session is in
  `docs/results`, and criteria 2, 3 and 8 are reported as the documents state them. The
  runners' edit-to-playing figures are from the workflow's files in the coordinator's
  scratchpad, not from the repository.
- **The rollback's share.** A third of a re-simulated tick being the slot's snapshot and record
  sets the probe's slot on the thrown scene, 106 µs and 46 µs, against the quiet window's
  455.1 µs a tick. The rollback itself was not split.
