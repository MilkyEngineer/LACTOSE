# The scalar decision: 64 bits stays

Status: decision, written at main `0fad797`, architecture revision 3.3, and revised after its
first review. It is exit step 2 of `docs/plans/milestone-1.md`, section 7. Nothing of section
5 is applied until the owner answers section 4, and the coordinator applies it. This document
changes no code. The owner approved it on 16 November 2026; it is D146.

Decision 5.4 of the plan builds the 32-bit branch only if a probe passes the precision half
of architecture section 16's rule, and asks for this decision once the probe and the measured
64-bit rollback loop are on main. Both are. Section 1 is the evidence. Section 2 is the
decision, the coordinate it is read at and what would bring a 32-bit scalar back. Section 3
goes through every row and sentence that waited on it. Section 4 is the one question for the
owner. Section 5 is the text for the coordinator. Section 6 says what this document could not
verify.

In brief:

- **The simulation scalar is 64-bit, and the 32-bit branch is not built.** On upstream 32-bit
  Rapier the walk scene at x = 6 km differs from the same scene at the origin by 157.75 mm,
  against the rule's one millimetre. In 64 bits it differs by at most 0.032 mm.
- **The cost half is recorded and decides nothing.** In 64 bits the M1 meets every milestone 1
  budget that physics enters. No file times a 32-bit loop, and none is owed.
- **An unchanged scalar re-sets nothing.** The quantiser bounds, the tolerance defaults and
  the pinned hashes stand. The rows and sentences that waited on the scalar close or narrow
  (section 3).
- **Decision 5.4's cost condition is moot, and the scalar sets no ring-depth trigger.** The
  condition was written for a probe that passed. A shallow ring is `lactose-predict`'s
  question at its milestone 4 review, and one new verification row records what that review
  needs. A 32-bit scalar comes back only with a design that keeps simulation coordinates
  small, run against section 16's rule again.

A verification row is cited by its opening words. A number beside one is the plan's: the row's
line in `docs/verification.md` at `3264406`.

## 1. The evidence

**The precision half.** `docs/results/scalar-precision-apple-m1-2026-11-14.md`: Apple M1,
macOS 14.7.4, commit `017823e` (its code is `4c2cef5`'s), release, one thread. The load
averages were 7.30 7.99 13.79 before and 8.71 8.27 13.85 after. They move no figure: the run
is deterministic, and each precision's origin run, stepped a second time, was bit-equal to
the first. The probe is the walk scene of `crates/physics-rapier/tests/support` with its
square closed, on upstream `rapier3d` and `rapier3d-f64` 0.36.0, stepped 36,000 ticks at the
origin and translated. Each translated run is compared with the origin run of its precision,
every tick, differences taken in f64. The file carries the probe's source.

| Translation | 32-bit, largest difference (tick) | 32-bit, before the walker is within 2 cm of a block | 64-bit control, largest |
|---|---|---|---|
| (6000, 0, 6000) | 157.75 mm (1649) | 88.14 mm (449) | 0.0318 mm |
| (6000, 0, 0) | 94.38 mm (1440) | 34.07 mm (242) | 0.0319 mm |
| (12000, 0, 12000) | 120.84 mm (239) | 120.84 mm; never within 2 cm | 0.0319 mm |

- The control passes. Its 0.032 mm lies in y alone and is the same at 6 km and 12 km, whose
  spacings differ by two. It is the scene's sensitivity to the jump landings, and it bounds
  what the scene adds to a difference.
- 32 bits fails at 6 km by about a hundred times. No contact makes the error: the 6 km pairs
  reach 88.14 mm and 34.07 mm on the floor alone.
- The error is a speed bias of the 32-bit position sum, not the random walk of about 27 mm
  that decision 5.4 expected. The closed square undoes it each lap, so the table's figures
  are lower bounds.
- The straight walk shows the growth. It is the same scene walked along +x for 600 ticks, and
  the origin walker walks 29.48 m. At tick 600 the 32-bit walker is off by 0.64 mm at 64 m,
  33.68 mm at 1,024 m, 179.58 mm at 6 km and 403.43 mm at 12 km: 0.022, 1.14, 6.09 and
  13.69 mm per metre walked. In 64 bits every difference is 0.0000 mm.

**The 64-bit loop, as measured.** Every file is from the Apple M1, macOS 14.7.4, release,
written by `cargo xtask measure`, on four frame workers unless its row says otherwise. The
first three are the second quiet window's: the first two replace the measurement commit's
files for criteria 5 and 7, and the third is criterion 7's first `spike-net` file (D143's
second addendum).

| Figure | Value | File under `docs/results`, commit, load averages |
|---|---|---|
| A seven-tick rollback on the spike's thrown scene: the character and fifty claimed bodies in flight among the arena's five fixed colliders, with no proxy (`prediction_fixed_and_proxies` is 5 in the thrown replays) | 2,713.7 µs, the median of 20; 387.7 µs a re-simulated tick, the restore included | `rollback-seven-ticks-apple-m1-2026-11-15.md`, `c50b688`, 2.96 2.75 5.85 before, 3.44 2.86 5.87 after |
| One prediction slot on the thrown scene among 59 proxies, taken into a full ring of 60 | snapshot 90.0 µs (world 11.2, physics 78.9), record 61.5 µs, restore 76.5 µs | `prediction-slot-apple-m1-2026-11-15.md`, `c50b688`, 2.96 2.75 5.85 |
| Rollback cost per rolled-back tick and the deepest correction, eight ten-minute replays | the next table | `spike-net-apple-m1-2026-11-15.md`, `c50b688`, 3.44 2.86 5.87 before, 9.50 7.73 7.43 after |
| The physics phase: 100,000 rows, 2,000 awake bodies, three solver threads | 1,954.3 µs: sync 554.3, step 985.4, write-back 419.0 | `physics-phase-apple-m1-2026-11-15.md`, `bef3455`, 1.43 2.09 3.71 |
| The composed tick: the 100k scene with the 2,000 bodies beside it | on four workers 5,772.5 µs, physics 2,265.8 µs of it; on one 13,808.9 µs, physics 3,156.1 µs | `composed-tick-apple-m1-2026-11-15.md`, `bef3455`, 1.43 2.09 3.71 |

The replays of `spike-net`, each 36,000 ticks over a link of 100 ms round trip and 1% loss.
The last column is this document's multiplication of the two before it. It is not a
measurement: the file records no rollback's own depth and cost together.

| Scene, clock | Deepest correction, ticks | Per rolled-back tick, median / worst, µs | Rollbacks | Deepest times median |
|---|---|---|---|---|
| Dynamic capsule, clock script | 8 | 90.2 / 110.6 | 10 | 0.72 ms |
| Dynamic capsule, server clock | 8 | 93.4 / 93.4 | 1 | 0.75 ms |
| Kinematic character, clock script | 8 | 313.7 / 429.1 | 9 | 2.51 ms |
| Kinematic character, server clock | 0 | none | 0 | none |
| Fifty thrown bodies, clock script | 39 | 487.3 / 2,766.9 | 7,935 | 19.0 ms |
| Fifty thrown bodies, server clock | 39 | 458.5 / 1,381.9 | 3,477 | 17.9 ms |
| Claim in contact, clock script | 9 | 109.5 / 327.2 | 3,478 | 0.99 ms |
| Claim in contact, server clock | 8 | 107.8 / 196.7 | 3,388 | 0.86 ms |

The thrown scene alone has scripted link faults (`Faults::window` in
`crates/app/tests/spike.rs`): on one cycle in four the client's direction loses 18 ticks, and
on the next both directions lose 40. Its 39-tick correction follows the 40-tick outage (the
row "`lactose-predict` is implemented (D122 addendum)"). In the scenes whose link has no
scripted fault the deepest correction is 9 ticks.

**Row 41's reading** is in the probe's file and in the row "Rapier's 64-bit build uses SIMD
after its glam migration". The contact solver is SIMD in both precisions. In 64 bits its
kernel is `wide::f64x4`: two lanes per NEON instruction on aarch64, where 32 bits would run
four, and one `ymm` of four lanes on x86-64-v3. A 32-bit build runs four lanes there too,
since `rapier3d` refuses `simd8` beside `enhanced-determinism` (its `src/lib.rs`) and parry
then selects `WideF32x4` (`parry3d` 0.31.1, `src/lib.rs`). The glam-based code examined is
scalar f64. The substep integration was not isolated.

**An extra x86 data point.** The laptop's files at `bef3455` (an i7-12650H on Windows 11, with
other work on the machine) give 10,953.5 µs for the seven-tick rollback, with the slot still
in byte form, and 4,028.4 µs for the physics phase on six frame workers. They are labelled
data and prove no criterion (the row "Section 22's milestone 1 timing criteria"). Session B on
the desktop has not run.

**What the cost half says.**

- 64 bits fits each milestone 1 budget that physics enters, on the M1. Criterion 4 reads
  1,954.3 µs against 3 ms. Criterion 5 reads 90.0 µs against 100 µs. The x86 desktop's
  figure for criterion 4 is session B's.
- Criterion 7's rollback has no budget (plan section 4). At the spike's scale a re-simulated
  tick of fifty claimed bodies costs 387.7 µs in the seven-tick rollback, whose seven ticks
  take 16% of a 16.7 ms tick, and 458.5 to 487.3 µs at the median over the ten-minute
  replays.
- What 64 bits costs in kind is half the lanes of the contact kernels on NEON, and no lanes on
  x86-64-v3.
- What 32 bits would save is not known. No file times a 32-bit loop, and this document does
  not estimate one. The loop is not solver arithmetic alone: every re-simulated tick also
  pays the slot's record, 61.5 µs on the slot test's scene (the row "The slot's record
  (D143)"). The files do not split a tick's cost.
- None of it can change the outcome. Section 16's rule adopts 32 bits only if the precision
  half passes, whatever the cost.

## 2. The decision

**The simulation scalar is 64-bit, and the 32-bit branch is not built.** Section 16 adopts 32
bits only if positional error at the map edge stays under one millimetre over a ten-minute
standard scene and every SOUR mode passes; otherwise 64-bit stays. The precision half fails
by about a hundred times, so 64-bit stays and the SOUR half is moot (decision 5.4).
Milestone 1 ships `lactose-math`'s `Scalar` as it is and `rapier3d-f64` as vendored. Nothing
is built for 32 bits: no 32-bit `Scalar`, no second vendored Rapier, no second golden table,
no 32-bit quantiser bounds, no SOUR run. The layouts that hold at either width stay as they
are (D119, D128); they cost nothing and no test of them is removed.

**The decision is read at x = 6 km, and it sets no map extent.** No document states the
map's extent. D59's "±6 km" is the nearest figure, and decision 5.4 names it. The distance is
a coordinate's: (6000, 0, 6000) is the corner of a map twelve kilometres across, 8.5 km from
the origin in a straight line. The plan's second distance, 8 km, was not read: the probe's
first run included it and its control failed, and the second run took 12 km in its place.
8 km lies in 6 km's binade, [4,096, 8,192) m, where a 32-bit position has the same 0.49 mm
spacing, and 12 km lies past the boundary. So the evidence reaches this far:

- 32 bits exceeds the millimetre at every distance measured from 1,024 m up: at 1,024 m on
  the straight walk, within its first two seconds, and at 6 km and 12 km on both walks. At
  64 m the straight walk stays under it for its ten seconds, at 0.64 mm. The reference game
  is an open world (architecture section 2), so no extent it could have is measured to pass.
- 64 bits is measured to x = 12 km, at or under 0.032 mm.
- A map with a coordinate past 12 km is outside the measurement, not outside the scalar. A
  64-bit position at 12 km is spaced 1.8e-12 m, by arithmetic and not by test.

The owner is not asked for an extent. The decision is the same at every extent from 1,024 m
to 12 km, which is what the evidence covers, so the choice is not open here. Nothing else in
milestone 1 reads a map extent, and the plan's one open question on it is moot (section 3).

**Decision 5.4's cost condition is moot, and the scalar sets no ring-depth trigger.**
Decision 5.4 ends on a condition. If the probe passes, the owner is asked whether to
commission the branch, "which is worth its cost only if the 64-bit rollback loop leaves the
ring shallower than the deepest correction the spike records". The condition was written for
a probe that passed. The probe failed, so the condition does not arise. Under section 16 a
32-bit scalar cannot be adopted while the precision half fails, whatever the ring's depth.

Read to the letter, the condition would hold today, and the owner is told so (section 4). The
spike's deepest correction is 39 ticks. A ring that covers it needs most of a tick at the
quiet seven-tick figure, and more than a tick at the replays' medians:

| Ring depth | What it covers | At 387.7 µs a tick | At 458.5 to 487.3 µs a tick | Share of a 16.7 ms tick |
|---|---|---|---|---|
| 9 ticks | The deepest correction in the scenes whose link has no scripted fault | 3.5 ms | 4.1 to 4.4 ms | 21% to 26% |
| 39 ticks | The correction after the thrown scene's 40-tick outage | 15.1 ms | 17.9 to 19.0 ms | 91% to 114% |
| 60 ticks | The placeholder depth | 23.3 ms | 27.5 to 29.2 ms | 140% to 175% |

These are multiplications, not measurements, and there is no ring to read them against.
Section 9.4 derives the ring's depth from the frame re-simulation budget and the measured
cost of a tick. No document gives that budget a figure (sections 5.1, 9.4 and 11, D33,
D122), and `lactose-app` builds its predictor at `DEFAULT_DEPTH`, sixty, which
`crates/predict/README.md` calls a placeholder. The statistic matters as much as the budget:
at the replays' worst per-tick figures, 1,381.9 and 2,766.9 µs, nine ticks need 12.4 to
24.9 ms.

A shallow ring is `lactose-predict`'s question at its milestone 4 review, not the scalar's.
The rows "Solver history departure in the prediction world", "A claimed body's sleep state"
and "The slot's record (D143)" are gated there already, and D124's spike-net addendum gives
that review the cap on a correction's depth or cost. The width is not the nearest lever in
the loop: the slot's record and the claim policy are. The depths above are recorded for that
review in one new verification row (section 5). The row carries no consequence for the
scalar and returns nothing to the planner.

**What brings a 32-bit scalar back is a design, not a measurement.** A design that keeps
simulation coordinates small, origin rebasing per region for example, is proposed and then
run against section 16's rule again. Nothing else brings it back: not a second run at the
same coordinates, not an estimate of what 32 bits would save, not a shallow ring, and not
session B.

**Session B can change nothing here.** The precision half is arithmetic and does not run in
`measure`. The cost that section 16 names is NEON's, and the M1 is the NEON machine. On
x86-64-v3 the 64-bit contact kernel already runs the four lanes a 32-bit one would, so the
lane argument for 32 bits is absent there. If the desktop's physics phase is over 3 ms, that
is criterion 4 not met on x86, and it goes to the owner at exit step 6 as criterion 4's.
What 32 bits would change there besides lanes is not measured. Session B's rollback and slot
figures are of the byte form at `bef3455` (plan section 3), so they do not compare with the
M1's at `c50b688`; the M1's own rollback there was 455.1 µs a tick (the row "`lactose-predict`
is implemented (D122 addendum)").

**Cost.** No lane. The plan counts three runs for this decision (section 3): this document,
its review and the answer. No decision must be taken before any lane can start.

## 3. Consequences, row by row

Each line names what waited on the scalar, what the decision does to it, and the block of
section 5 that carries the text.

| What waited | What the decision does | Block |
|---|---|---|
| Plan criterion 8, "Scalar decided by the section 16 rule" | Met: the rule's precision half ran and decided, and the entry exists | Plan, row 8 |
| Plan row 14 and the row "Scalar precision" | Closed. The row gains the decision, the coordinate it is read at and what would bring a 32-bit scalar back | Plan, row 14; verification |
| Plan row 41 and the row "Rapier's 64-bit build uses SIMD after its glam migration" | Closed as the cost half: the reading, with the measured 64-bit loop beside it and the statement that no 32-bit loop is owed | Plan, row 41; verification |
| Plan row 93, "The quantiser bounds follow the scalar" | The scalar does not move, so the bounds do not | Plan, row 93 |
| The row "`lactose-net-api` is implemented", "the 32-bit position bound of 2^24 units, re-checked when the scalar decision lands" | Re-checked and closed. Under the 64-bit scalar a position round-trips exactly for every quantised value (`crates/net-host/README.md`, "Quantisation"), so the bound does not apply. The rotation quantiser's non-idempotence was measured under the 64-bit scalar and stands: one trip moves 1.53% of uniform rotations by one unit, and nothing relies on a value being stable | Verification |
| Plan row 105 and the row "The reconciliation tolerance (D124)", "The defaults are re-set from the scalar decision at milestone 1" | Not re-set, and nothing is re-measured. The row's gate becomes 4 (below) | Plan, row 105; verification |
| D124, "the scalar decision re-measures the defaults" and "the defaults re-set at milestone 1 (scalar)" | Replaced by the entry, and D124 gains an addendum | Decisions log |
| D59, "decided in milestone 1" | Decided; D59 gains an addendum | Decisions log |
| Architecture section 2, the row "World coordinates" | States the width and where it was decided | Architecture |
| Architecture section 16, "The simulation scalar is decided in milestone 1 by a precision test" | The paragraph states the decision, the coordinate, the evidence, the measured loop, whose question a shallow ring is, and what would bring a 32-bit scalar back | Architecture |
| Architecture section 22, milestone 1's "Scalar decided by the section 16 rule" | The criterion row stands as written: it is the criterion, and the table carries no status. The paragraph that says how the criteria are read gains one sentence. Section 22.1's row "Simulation scalar precision, 1" stands, as the allocator's does at 0 | Architecture |
| The row "`lactose-input` is implemented", "a switch to a 32-bit `Scalar` re-runs them" | No switch, so no re-run | Verification |
| `crates/math/README.md`, `crates/physics-api/README.md`, `crates/predict/README.md` | Three sentences that call the width pending | READMEs |
| Six doc comments: two in `lactose-math` and one each in `lactose-anim`, `lactose-physics-api`, `lactose-predict` and `lactose-sim` | The same sentences in source, as a group of their own for one gated commit | Doc comments |
| Plan decision 5.4, its last sentence: the cost condition | Moot. It was written for a probe that passed. The entry's "Replaced" clause names it, and 5.4 is marked with its outcome, as 5.5 is | Decisions log; plan |
| The plan's open question 5, "The reference map's extent, only if the probe's 6 km and 8 km runs fall on different sides of one millimetre" | Moot. The runs read, at 6 km and 12 km, both exceed one millimetre, and no extent is set | Plan |
| The row "`lactose-predict` is implemented (D122 addendum)", "the ring depth `app` derives", and plan row 94, "Ring depth" | Milestone 1 records no derived depth: `app` builds at the placeholder. The depth moves to the new row | Verification; plan, row 94 |
| New: the row "The frame re-simulation budget has no figure" | What `lactose-predict`'s milestone 4 review needs to size the ring, with the recorded depths. No consequence for the scalar | Verification |
| Plan exit step 2 | Names this document | Plan |

**The tolerance defaults.** D124 deferred two re-sets: "the defaults re-set at milestone 1
(scalar) and 4 (spike)". The first had one reason, in
`docs/proposals/physics-history-reconciliation.md`: "a 32-bit scalar's spacing at the map edge
is of their order". The width does not change. Every figure the defaults rest on, D124's
probes and the spike's eight replays, was taken under the 64-bit scalar. So the scalar
decision re-measures nothing and re-sets nothing. The second is the row's own finding: "The
spike gives no case for doubling; the decision waits on a sleep-state rule for claimed bodies
and a re-measure", and the rows it waits on are gated at milestone 4. The two do not collide.
This decision removes the milestone 1 re-set and leaves the wait as it was. The row's
milestone 1 part is then done, and its gate becomes 4.

**READMEs checked and left alone.** `crates/anim/README.md`, `crates/net-host/README.md` and
`crates/persist/README.md` state what holds at either width ("for both scalar widths", "under
a 32-bit scalar only for integers within 2^24 units"). Those sentences are true and call
nothing pending. `crates/physics-rapier/README.md` has no sentence on the width.
`crates/physics-api/README.md`'s table row "The scalar decision: nothing here depends on the
width" stands. The first review searched crates, tools, docs and doc comments again and found
one sentence this document had missed, in `crates/math/src/raw.rs`; it is the sixth doc
comment.

## 4. The question for the owner

One question. The coordinator asks it in these words; the recommended option is first.

> **Header:** Scalar
>
> Milestone 1's exit step 2 is the scalar decision. On upstream 32-bit Rapier the walk scene
> at x = 6 km drifts 157.75 mm from the same scene at the origin within ten minutes. The
> architecture's rule allows one millimetre, and 64 bits stays under 0.032 mm. In 64 bits the
> M1 meets its physics budgets: 1.95 ms of 3 ms at 2,000 bodies, and 90 µs of 100 µs for the
> prediction snapshot. Do you approve the decision?
>
> 1. **Approve: 64-bit stays, no 32-bit branch (Recommended):** The architecture's rule
>    decides it. No second vendored Rapier and no 32-bit scalar are built, and criterion 8
>    closes. Decision 5.4's cost condition, a ring shallower than the spike's deepest
>    correction, is set aside as moot, because the precision half failed: the condition was
>    written for a probe that passed. Read to the letter it would hold today, for the 39-tick
>    correction after the spike's scripted outage. A shallow ring goes to lactose-predict's
>    milestone 4 review, with one new verification row. No lane.
> 2. **Send it back:** Nothing is applied and criterion 8 stays open. Say what the decision
>    lacks, for example a second scene or a stated map extent.

No timing probe is offered. A probe outside the workspace would time upstream 32-bit Rapier's
step alone on the thrown scene; it would not time the loop, with its sync, write-back and
slot. The rule does not weigh cost, so the figure would feed no decision now. A design that
brings 32 bits back measures its own loop, on the Rapier pinned then.

## 5. Text for the coordinator (exact)

The text is for answer 1. `D<N>` is the number the coordinator allocates at application.
`<date>` is the date of the owner's answer. Each block names one file, and each OLD is the
file's own text at `0fad797`, byte for byte, line breaks included. A script checked that
every OLD occurs exactly once in its file there, and still once when the earlier blocks of
the same file have been applied. No two blocks of a file overlap.

The owner approved answer 1 on 16 November 2026, and the coordinator applied it as D146, with
the second review's wording for the cost condition in the plan 5.4 block and in the entry's
Replaced clause.

The blocks are in two groups. The document group is the decisions log, the architecture, the
plan, the verification rows and three READMEs: 26 blocks, and no code. The source group is
six doc comments in `.rs` files, under its own heading at the end, for one gated commit apart
from the documents.

### Decision entry

It goes at the end of section 9 of `docs/decisions-log.md`, after D145.

```
=== docs/decisions-log.md ===
--- OLD ---
## 10. Review history
--- NEW ---
**D<N>. The simulation scalar is 64-bit: 32 bits fails the precision half of section 16's rule at x = 6 km, and the 32-bit branch is not built** (§2 World coordinates, §16, §22, `lactose-math`, `lactose-anim`, `lactose-physics-api`, `lactose-predict`, `lactose-sim`, `docs/plans/milestone-1.md`)
- Why: section 16 adopted 32 bits only if positional error at the map edge stayed under one
  millimetre over a ten-minute standard scene and every SOUR mode passed, and D139 built the
  32-bit branch only if a probe on upstream 32-bit Rapier passed the precision half. The
  probe's file and the measured 64-bit rollback loop, the two inputs D139's plan named, are on
  main. `docs/plans/scalar-decision.md` holds the evidence and the consequences row by row;
  these are the decisions.
  - *64 bits stays, and milestone 1's criterion "Scalar decided by the section 16 rule" is
    met.* On upstream `rapier3d` 0.36.0 the walk scene, stepped 36,000 ticks, differs from the
    same scene at the origin by 157.75 mm at (6000, 0, 6000), 94.38 mm at (6000, 0, 0) and
    120.84 mm at (12000, 0, 12000), against one millimetre
    (`docs/results/scalar-precision-apple-m1-2026-11-14.md`, Apple M1, `017823e`, one thread;
    the run is deterministic, so load moves no figure). The 6 km pairs differ by 88.14 mm and
    34.07 mm before the walker first comes within Rapier's 2 cm prediction distance of a
    block, so no contact makes the error. The same pairs on `rapier3d-f64` stay at or under
    0.032 mm, in y alone and alike at 6 km and 12 km: the scene's own sensitivity, which
    bounds what it adds. The error is a speed bias of the 32-bit position sum, not the random
    walk of about 27 mm the plan expected: a straight walk drifts 1.14 mm per metre walked at
    1,024 m, 6.09 mm at 6 km and 13.69 mm at 12 km, so the closed square's figures are lower
    bounds. Rules out running the rule's SOUR half, which is moot, and a second run of the
    precision half.
  - *The rule is read at x = 6 km, and this entry sets no map extent.* No document states
    one; D59's ±6 km is the nearest figure, a coordinate's distance, so that (6000, 0, 6000)
    is the corner of a map twelve kilometres across. The plan's 8 km was not read: the
    probe's first run included it and its control failed, and the second took 12 km in its
    place. 8 km lies in 6 km's binade, [4,096, 8,192) m, where a 32-bit position has the same
    0.49 mm spacing. 32 bits exceeds the millimetre at every distance measured from 1,024 m
    to 12 km, at 1,024 m on the straight walk alone, and 64 bits is measured to 12 km. Rules
    out reading the entry as a bound on the map: a coordinate past 12 km lies outside the
    measurement, not outside the scalar, which is spaced 1.8e-12 m at 12 km by arithmetic.
  - *The 32-bit branch is not built.* No 32-bit `Scalar` in `lactose-math`, no second vendored
    Rapier, no second golden table, no 32-bit quantiser bounds and no SOUR run in 32 bits. The
    layouts that hold at either width (D119, D128) stay as they are. Rules out a 32-bit
    configuration kept building beside the 64-bit one.
  - *The cost half is recorded and decides nothing.* The contact solver is SIMD in both
    precisions: two 64-bit lanes per NEON instruction where 32 bits would run four (the
    probe's file, "Row 41"), and four per AVX instruction on x86-64-v3, as many as 32 bits
    would run there, since `rapier3d` 0.36.0 refuses `simd8` beside `enhanced-determinism`
    and `parry3d` 0.31.1 then selects `WideF32x4` (each crate's `src/lib.rs`). In 64 bits, on
    the quiet Apple M1: at `c50b688` a seven-tick rollback of the spike's thrown scene, fifty
    claimed bodies, takes 2,713.7 µs, 387.7 µs a re-simulated tick
    (`docs/results/rollback-seven-ticks-apple-m1-2026-11-15.md`), and the prediction-world
    snapshot 90.0 µs against section 22's 100 µs
    (`docs/results/prediction-slot-apple-m1-2026-11-15.md`); at `bef3455` the physics phase at
    2,000 awake bodies takes 1,954.3 µs against 3 ms
    (`docs/results/physics-phase-apple-m1-2026-11-15.md`). Every milestone 1 budget that
    physics enters is met in 64 bits on the M1. The x86 desktop's figures are session B's and
    change nothing here: the precision half is arithmetic, and on x86-64-v3 the lane argument
    for 32 bits is absent. No 32-bit loop was timed, so what 32 bits would save is not known,
    and none is owed: the rule does not weigh cost. Rules out a cost argument reopening the
    rule.
  - *An unchanged scalar re-sets nothing.* `lactose-net-host`'s position quantiser round-trips
    exactly for every value under the 64-bit scalar, so a 32-bit scalar's bound of 2^24 units
    does not apply, and the rotation quantiser's measured non-idempotence stands, with nothing
    relying on a stable value. The reconciliation tolerance's defaults (D124) were set and
    measured under the 64-bit scalar: the scalar decision re-measures none, and their re-set
    from the spike's figures is milestone 4's, after the sleep-state rule for claimed bodies.
    The golden bits and the hashes pinned under the 64-bit scalar stand.
  - *Decision 5.4's cost condition is moot, and the scalar sets no ring-depth trigger.* The
    plan made the branch worth its cost only if the 64-bit rollback loop left the ring
    shallower than the deepest correction the spike records. That was written for a probe that
    passed, and under section 16 a 32-bit scalar cannot be adopted while the precision half
    fails, whatever the ring's depth. Read to the letter the condition would hold: the spike's
    deepest correction is 39 ticks, after its scripted 40-tick outage, which need 15.1 ms at
    387.7 µs a tick and 17.9 to 19.0 ms at the replays' medians, against a 16.7 ms tick. No
    ring is sized yet: no document gives the frame re-simulation budget a figure, and
    `lactose-app` builds its predictor at the placeholder depth of sixty. A shallow ring is
    `lactose-predict`'s question at its milestone 4 review, where the slot's record, the claim
    policy and a cap on a correction's depth come before the width; the verification row "The
    frame re-simulation budget has no figure" records the depths for it. Rules out a ring
    depth, an estimate of what 32 bits would save, or session B reopening the scalar.
  - *What brings a 32-bit scalar back is a design.* One that keeps simulation coordinates
    small, origin rebasing per region for example, is proposed and then run against section
    16's rule again. Rules out a second run of the rule at the same coordinates.
- Driven by: lane `scalar-probe`'s result file and its review, whose own straight walk the
  file's agrees with; the measurement commit's `physics-phase` file at `bef3455`; the second
  quiet window's files at `c50b688` (D143's second addendum); the review of
  `docs/plans/scalar-decision.md`, on which the cost condition was set aside and the ring's
  depth left to `lactose-predict`; the owner's approval of <date>.
- Replaced: D59's "decided in milestone 1" and its "the safer default is kept until the test
  decides". Section 2's "64-bit by default, decided finally by a milestone 1 precision and
  performance test at full map extent" and section 16's paragraph on the scalar to be decided.
  D124's "the scalar decision re-measures the defaults" and, among its deferred items, "the
  defaults re-set at milestone 1 (scalar)". In `docs/plans/milestone-1.md`: decision 5.4's
  sentence "The owner is then asked whether to commission the branch", its cost condition
  against the deepest correction the spike records, which is moot because the precision half
  failed; its second distance, 8 km, and its expectation of "about 27 mm as a random walk";
  and its open question 5, the reference map's extent. The plan now marks each.

## 10. Review history
```

### Decisions log addenda

```
=== docs/decisions-log.md ===
--- OLD ---
- Replaced: revision 2's fixed 64-bit positions and 64-bit Rapier build; revision 1's f32 math;
  revision 3.1's type alias (R3-10).
--- NEW ---
- Replaced: revision 2's fixed 64-bit positions and 64-bit Rapier build; revision 1's f32 math;
  revision 3.1's type alias (R3-10).
- Addendum (D<N>, the scalar decision, <date>): decided. 64-bit stays: on upstream 32-bit
  Rapier the walk scene at x = 6 km differs from the same scene at the origin by 157.75 mm,
  against the rule's one millimetre, and the 32-bit branch is not built.
```

```
=== docs/decisions-log.md ===
--- OLD ---
  frame; the worst single sample was 22.5 ms per tick at load 19 to 25. A correction-depth or
  cost cap belongs to predict's milestone 4 review.
--- NEW ---
  frame; the worst single sample was 22.5 ms per tick at load 19 to 25. A correction-depth or
  cost cap belongs to predict's milestone 4 review.
- Addendum (D<N>, the scalar decision, <date>): the scalar stays 64-bit, the width the
  defaults were set under and the spike measured them at, so the scalar decision re-measures
  and re-sets none of them. Of "the defaults re-set at milestone 1 (scalar) and 4 (spike)"
  only milestone 4's is left, after the sleep-state rule for claimed bodies.
```

### Architecture amendments

`docs/architecture.md`. The first is in section 2's table, the second is section 16's
paragraph, and the third ends the paragraph of section 22 that says how milestone 1's
criteria are read. Section 22's criterion row and section 22.1's row are not changed.

```
=== docs/architecture.md ===
--- OLD ---
64-bit by default, decided finally by a milestone 1 precision and performance test at full map extent. Switching is a Rapier crate switch, not a one-line change.
--- NEW ---
64-bit: milestone 1's precision test at x = 6 km decided it, with the 64-bit rollback loop measured beside it (section 16). Switching would be a Rapier crate switch, not a one-line change.
```

```
=== docs/architecture.md ===
--- OLD ---
The simulation scalar is decided in milestone 1 by a precision test at full map extent: 32-bit
is adopted if positional error at the map edge stays under one millimetre over a ten-minute
standard scene and every SOUR mode passes; otherwise 64-bit stays. Switching means
`rapier3d` versus `rapier3d-f64` and their parry counterparts, a crate change behind
`physics-api`. The 64-bit cost that matters is the client's rollback re-simulation on NEON,
which sets ring depth, so the test measures that loop, not only server throughput.
--- NEW ---
The simulation scalar is 64-bit (D<N>). Milestone 1 decided it by a precision test: 32-bit
would have been adopted had positional error at the map edge stayed under one millimetre over
a ten-minute standard scene and every SOUR mode passed. The test is read at x = 6 km, since
no document states the map's extent. On upstream 32-bit Rapier the walk scene at
(6000, 0, 6000) differs from the same scene at the origin by 157.75 mm, and at
(12000, 0, 12000) by 120.84 mm; in 64 bits it differs by at most 0.032 mm at either. The
precision half fails, so the SOUR half was not run and no 32-bit branch is built. A switch
would mean `rapier3d` for `rapier3d-f64` and their parry counterparts, a crate change behind
`physics-api`. The 64-bit cost that matters is the client's rollback re-simulation on NEON,
which sets ring depth: the contact solver runs two 64-bit lanes per NEON instruction where
32 bits would run four, and a re-simulated tick of the netcode spike's fifty thrown bodies
costs 387.7 µs on a quiet Apple M1. The rule does not weigh that cost, and a shallow ring is
`predict`'s question when milestone 4 sizes it (section 9.4), not the scalar's. A 32-bit
scalar comes back only with a design that keeps simulation coordinates small, origin rebasing
per region for example, proposed and then run against this rule again; the rule is not run
again at the same coordinates.
```

```
=== docs/architecture.md ===
--- OLD ---
macos-arm64 mean an x86 server and an aarch64 client exchange those bytes tick for tick. A
difference between platforms is a finding and calls for a run across two machines.
--- NEW ---
macos-arm64 mean an x86 server and an aarch64 client exchange those bytes tick for tick. A
difference between platforms is a finding and calls for a run across two machines. The
scalar's rule is read at x = 6 km, D59's figure, since no document states the map's extent,
and its SOUR half is run only if its precision half passes, which it did not (section 16,
D<N>).
```

### Plan amendments

`docs/plans/milestone-1.md`. Six table rows, the end of decision 5.4, its open question 5
and exit step 2.

```
=== docs/plans/milestone-1.md ===
--- OLD ---
| 8 | Scalar decided by the section 16 rule | A precision probe's result file `scalar-precision` and a decision entry | Nothing exists. No document states the full map extent the rule tests at | `scalar-probe`, then the planner's decision (decision 5.4) |
--- NEW ---
| 8 | Scalar decided by the section 16 rule | A precision probe's result file `scalar-precision` and a decision entry | Met: 64-bit stays (D<N>). On upstream 32-bit Rapier the walk scene translated to x = 6 km differs from the origin run by 157.75 mm, and by 120.84 mm at 12 km, against one millimetre; the 64-bit control stays at or under 0.032 mm (`docs/results/scalar-precision-apple-m1-2026-11-14.md`). The rule is read at x = 6 km, D59's figure, since no document states the map's extent. The SOUR half is moot and the 32-bit branch is not built | `scalar-probe`, then the planner's decision (decision 5.4, `docs/plans/scalar-decision.md`) |
```

```
=== docs/plans/milestone-1.md ===
--- OLD ---
| 14 | Scalar precision | `scalar-probe`, then the planner | The probe's result file and the scalar decision entry |
--- NEW ---
| 14 | Scalar precision | `scalar-probe`, then the planner | Closed: the probe's result file and the scalar decision entry, D<N>. 64-bit stays |
```

```
=== docs/plans/milestone-1.md ===
--- OLD ---
| 41 | Rapier's 64-bit build uses SIMD | `scalar-probe` | A reading of the vendored source and one disassembly; it is the cost half of the scalar decision |
--- NEW ---
| 41 | Rapier's 64-bit build uses SIMD | `scalar-probe` | Closed: a reading of the vendored source and one disassembly. The contact solver runs two 64-bit lanes per NEON instruction against 32 bits' four, and four per AVX instruction on x86-64-v3. It is the cost half of the scalar decision, which D<N> records beside the measured 64-bit rollback loop; no 32-bit loop is owed |
```

```
=== docs/plans/milestone-1.md ===
--- OLD ---
| 93 | `lactose-net-api` is implemented | `sour-evidence`, `spike-net`, the scalar decision | The `ProtocolHash` pinned over `Game::types()` (C3). The spike measures the gather path. The quantiser bounds follow the scalar |
--- NEW ---
| 93 | `lactose-net-api` is implemented | `sour-evidence`, `spike-net`, the scalar decision | The `ProtocolHash` pinned over `Game::types()` (C3). The spike measures the gather path. The quantiser bounds follow the scalar, which stays 64-bit (D<N>): a position round-trips exactly for every quantised value, a 32-bit scalar's bound of 2^24 units does not apply, and the rotation quantiser's figures stand as measured |
```

```
=== docs/plans/milestone-1.md ===
--- OLD ---
| 94 | `lactose-predict` is implemented | `spike-net`, `measure` | Ring depth, the slot with Rapier and the absorbed figures. How a re-spawned body presents is milestone 2's (row text in section 6) |
--- NEW ---
| 94 | `lactose-predict` is implemented | `spike-net`, `measure` | The slot with Rapier and the absorbed figures. Milestone 1 records no derived ring depth: `app` builds at `DEFAULT_DEPTH` (D<N>), and the depth moves to the verification row "The frame re-simulation budget has no figure", gated at milestone 4. How a re-spawned body presents is milestone 2's (row text in section 6) |
```

```
=== docs/plans/milestone-1.md ===
--- OLD ---
| 105 | The reconciliation tolerance (D124) | `spike-net`, the scalar decision | The spike's figures under the defaults; the defaults are re-set only if the scalar changes |
--- NEW ---
| 105 | The reconciliation tolerance (D124) | `spike-net`, the scalar decision | The spike's figures under the defaults; the defaults are re-set only if the scalar changes, and it does not (D<N>). Milestone 1's part is closed. The re-set from the spike's figures is milestone 4's, after the sleep-state rule for claimed bodies |
```

```
=== docs/plans/milestone-1.md ===
--- OLD ---
Rapier, the quantiser bounds, every SOUR mode), which is worth its cost only if the 64-bit rollback
loop leaves the ring shallower than the deepest correction the spike records.
--- NEW ---
Rapier, the quantiser bounds, every SOUR mode), which is worth its cost only if the 64-bit rollback
loop leaves the ring shallower than the deepest correction the spike records. The probe did not
pass: on upstream 32-bit Rapier the walk scene at x = 6 km differs from the origin run by
157.75 mm (`docs/results/scalar-precision-apple-m1-2026-11-14.md`). Its second run took 12 km in
place of 8 km, which lies in 6 km's binade, and it found a speed bias where this paragraph
expected a random walk. 64 bits stay and the branch is not built. The cost condition above does
not arise, since it was written for a probe that passed. A shallow ring is `lactose-predict`'s
question at its milestone 4 review (D<N>, `docs/plans/scalar-decision.md`).
```

```
=== docs/plans/milestone-1.md ===
--- OLD ---
Still open:

5. The reference map's extent, only if the probe's 6 km and 8 km runs fall on different sides of
   one millimetre.
--- NEW ---
No longer open (D<N>):

5. The reference map's extent, only if the probe's 6 km and 8 km runs fall on different sides of
   one millimetre. Moot: the runs read, at 6 km and 12 km, both exceed one millimetre, and no
   extent is set.
```

```
=== docs/plans/milestone-1.md ===
--- OLD ---
2. The planner has written the scalar decision from the probe and the measured rollback loop, and
   the owner has approved it.
--- NEW ---
2. The planner has written the scalar decision from the probe and the measured rollback loop, and
   the owner has approved it: `docs/plans/scalar-decision.md`, D<N>.
```

### Verification rows

`docs/verification.md`. Each OLD is a fragment inside the row its opening words name. The
last block adds a row at the end of section 4.

The row "Scalar precision":

```
=== docs/verification.md ===
--- OLD ---
By architecture section 16 the 32-bit branch is not built and the rule's SOUR half is moot |
--- NEW ---
By architecture section 16 the 32-bit branch is not built and the rule's SOUR half is moot. Decided (D<N>): the scalar is 64-bit. The rule is read at x = 6 km, D59's figure, since no document states the map's extent; 8 km lies in 6 km's binade and was not read; 64 bits is measured to 12 km. The plan's cost condition is moot, since it was written for a probe that passed, and a shallow ring is `lactose-predict`'s question at milestone 4 (the row "The frame re-simulation budget has no figure"). A 32-bit scalar comes back only with a design that keeps simulation coordinates small, run against the rule again |
```

The row "Rapier's 64-bit build uses SIMD after its glam migration":

```
=== docs/verification.md ===
--- OLD ---
The f64 contact solver runs two lanes per NEON instruction against f32's four; the glam-based code examined is scalar f64 |
--- NEW ---
The f64 contact solver runs two lanes per NEON instruction against f32's four; the glam-based code examined is scalar f64. The cost half of the scalar decision (D<N>), with the 64-bit loop as measured on the quiet M1 beside it: a re-simulated tick of the spike's thrown scene costs 387.7 µs at `c50b688` (`docs/results/rollback-seven-ticks-apple-m1-2026-11-15.md`), and the physics phase at 2,000 awake bodies 1,954.3 µs against 3 ms at `bef3455` (`docs/results/physics-phase-apple-m1-2026-11-15.md`). No 32-bit loop was timed and none is owed: section 16's rule does not weigh cost |
```

The row "`lactose-input` is implemented":

```
=== docs/verification.md ===
--- OLD ---
since Rosetta cannot run the x86-64-v3 build; a switch to a 32-bit `Scalar` re-runs them)
--- NEW ---
since Rosetta cannot run the x86-64-v3 build; the scalar stays 64-bit (D<N>), so nothing re-runs them)
```

The row "`lactose-net-api` is implemented":

```
=== docs/verification.md ===
--- OLD ---
the rotation quantiser's one-percent non-idempotence and the 32-bit position bound of 2^24 units, re-checked when the scalar decision lands;
--- NEW ---
the rotation quantiser's one-percent non-idempotence and the 32-bit position bound of 2^24 units, re-checked when the scalar decision lands, and closed by it (D<N>): the scalar stays 64-bit, so the position bound does not apply, and the rotation figures below, taken under the 64-bit scalar, stand, with nothing relying on a value being stable;
```

The row "`lactose-predict` is implemented (D122 addendum)":

```
=== docs/verification.md ===
--- OLD ---
and the deepest correction as `.deepest_correction`. | 1 and 2 |
--- NEW ---
and the deepest correction as `.deepest_correction`. Milestone 1 records no derived depth: `app` builds at `DEFAULT_DEPTH` (D<N>); the depth moves to the row "The frame re-simulation budget has no figure". | 1 and 2 |
```

The row "The reconciliation tolerance (D124)", twice. The second block changes its gate.

```
=== docs/verification.md ===
--- OLD ---
The defaults are re-set from the scalar decision at milestone 1 and the spike's figures at milestone 4.
--- NEW ---
The scalar decision re-sets none of the defaults (D<N>): the scalar stays 64-bit, the width the probes set them under and the spike measured them at. They are re-set from the spike's figures at milestone 4.
```

```
=== docs/verification.md ===
--- OLD ---
the approach and doubled-tolerance figures above are not in it. | 1 and 4 |
--- NEW ---
the approach and doubled-tolerance figures above are not in it. Milestone 1's part is **Done**: the spike's figures under the defaults are recorded, and neither the scalar decision (D<N>) nor the spike re-sets a default. The doubling waits on the sleep-state rule for claimed bodies (the row "A claimed body's sleep state") and its re-measure. | 4 |
```

A new row, after the row "The physics hash beside the world hash (D145)":

```
=== docs/verification.md ===
--- OLD ---
so no shipped tick hashes the bytes | 1 |
--- NEW ---
so no shipped tick hashes the bytes | 1 |
| The frame re-simulation budget has no figure (architecture sections 5.1, 9.4 and 11; D33, D122), so no ring depth is derived: `lactose-app` builds its predictor at `DEFAULT_DEPTH`, sixty, a placeholder. Found by the scalar decision (D<N>), which draws no consequence for the scalar from it; the owner is `lactose-predict`'s milestone 4 review. Recorded for it, on the quiet M1 at `c50b688`: a re-simulated tick of the spike's thrown scene costs 387.7 µs in the seven-tick rollback (`docs/results/rollback-seven-ticks-apple-m1-2026-11-15.md`), and over the ten-minute replays 458.5 to 487.3 µs at the median and 1,381.9 to 2,766.9 µs at the worst (`docs/results/spike-net-apple-m1-2026-11-15.md`). By multiplication, not by measurement: nine ticks, the deepest correction in the scenes whose link has no scripted fault, need 3.5 ms at the first figure, 4.1 to 4.4 ms at the medians and 12.4 to 24.9 ms at the worst; the 39 ticks after the thrown scene's 40-tick outage need 15.1 ms and 17.9 to 19.0 ms; the placeholder's sixty need 23.3 ms and 27.5 to 29.2 ms, against a 16.7 ms tick. At `lactose-predict`'s milestone 4 review, when `app` sizes the ring (the row "The slot's record (D143)"): state the budget and the per-tick statistic the depth is derived from, record on the quiet M1 the smallest depth the client derives over a ten-minute thrown-scene replay, and record the thrown scene's deepest correction outside its two fault windows (on one cycle in four the client's direction loses 18 ticks, and on the next both directions lose 40). A correction deeper than the ring is a rebuild from replicated state (section 9.4), and its cap is that review's (D124's spike-net addendum) | 4 |
```

### README sentences

Three READMEs, the last blocks of the document group.

```
=== crates/math/README.md ===
--- OLD ---
`Scalar` is 64-bit until the milestone 1 precision test (architecture section 16). The
--- NEW ---
`Scalar` is 64-bit: milestone 1's precision test decided it, and no 32-bit branch is built
(architecture section 16, D<N>). The
```

```
=== crates/physics-api/README.md ===
--- OLD ---
  so no padding changes if milestone 1 picks a four-byte scalar.
- What changes then: `lactose-math`'s `Scalar` and the `Scalar::NAME` tag (`Scalar64` to
--- NEW ---
  so no padding would change under a four-byte scalar. Milestone 1 decided the width, 64-bit,
  and builds no 32-bit branch (architecture section 16, D<N>).
- What a switch would change: `lactose-math`'s `Scalar` and the `Scalar::NAME` tag (`Scalar64` to
```

```
=== crates/predict/README.md ===
--- OLD ---
- The tolerance's defaults re-set from the spike's `absorbed` figures and from the scalar
  decision, and `net`'s release of a thrown object at rest: milestone 1 and 4.
--- NEW ---
- The tolerance's defaults re-set from the spike's `absorbed` figures, and `net`'s release of
  a thrown object at rest: milestone 4. The scalar decision re-sets none: the scalar stays
  64-bit, the width the defaults were set under (D<N>).
```

### Source group: six doc comments, for one gated commit

These six blocks are apart from the document group. They change no code, but they are in
`.rs` files, so the commit that carries them runs the gates of `docs/agents.md`, section 4.
The coordinator applies them in one commit of their own, with `D<N>` already replaced: left
as written, `<N>` reads to rustdoc as an HTML tag. Every NEW line is under 100 columns. No
block adds an intra-doc link; the two [`Scalar`] links are the ones those lines carry today.

```
=== crates/math/src/scalar.rs ===
--- OLD ---
/// The simulation float: 64-bit until the milestone 1 precision test decides finally.
--- NEW ---
/// The simulation float: 64-bit, decided by the milestone 1 precision test (D<N>).
```

```
=== crates/math/src/raw.rs ===
--- OLD ---
//! Architecture section 16 decides the width in milestone 1. Switching it means editing this
//! file and nothing else in the crate: the alias, the matching unsigned integer, the constants
//! module and the `libm` bindings (the `f`-suffixed functions for 32 bits). It is deliberately
//! not a Cargo feature, because features unify across a build and an outside crate could then
//! change boundary numerics.
--- NEW ---
//! Architecture section 16 decided the width in milestone 1: 64-bit (D<N>). Switching it means
//! editing this file and nothing else in the crate: the alias, the matching unsigned integer,
//! the constants module and the `libm` bindings (the `f`-suffixed functions for 32 bits). It
//! is deliberately not a Cargo feature, because features unify across a build and an outside
//! crate could then change boundary numerics.
```

```
=== crates/anim/src/components.rs ===
--- OLD ---
//! bytes, so the layout holds whichever width the milestone 1 precision test picks for
//! [`Scalar`] without a padding change; `lib.rs` asserts the sizes for both widths. Every
--- NEW ---
//! bytes, so the layout holds at either width of [`Scalar`] without a padding change;
//! `lib.rs` asserts the sizes for both widths, and milestone 1 kept 64 bits (D<N>). Every
```

```
=== crates/physics-api/src/components.rs ===
--- OLD ---
//! of eight bytes, so the layout holds whichever width the milestone 1 precision test picks
//! for [`Scalar`] without a padding change; `lib.rs` asserts the sizes for both widths.
--- NEW ---
//! of eight bytes, so the layout holds at either width of [`Scalar`] without a padding
//! change; `lib.rs` asserts the sizes for both widths, and milestone 1 kept 64 bits (D<N>).
```

```
=== crates/predict/src/compare.rs ===
--- OLD ---
    /// scalar against the residuals D124 measured. Milestone 1 re-measures them with the
    /// scalar decision and the spike re-sets them from `PredictionStats::absorbed`.
--- NEW ---
    /// scalar against the residuals D124 measured. The scalar decision kept 64 bits and
    /// re-set none (D<N>); milestone 4 re-sets them from `PredictionStats::absorbed`.
```

```
=== crates/sim/tests/hash.rs ===
--- OLD ---
/// the 64-bit simulation scalar; the scalar decision of milestone 1 re-pins them.
--- NEW ---
/// the 64-bit simulation scalar, which the scalar decision of milestone 1 kept (D<N>).
```

## 6. What this document could not verify

- **The probe was not run again.** Its figures are the file's. The file does not carry the
  probe's lock file, and this document ran no probe.
- **The 32-bit run exists on aarch64 alone.** Whether upstream 32-bit Rapier gives the same
  bits on x86-64 was not run. A difference of bits does not move a failure of a hundred times.
- **8 km was never read.** The binade argument is about spacing. The file says the bias also
  depends on the step's fraction of the spacing, so 8 km's own figure is not known.
- **No 32-bit loop was timed.** What 32 bits would save in a rollback or a physics phase is not
  known, on either architecture.
- **No ring depth exists to measure.** No document gives the frame re-simulation budget a
  figure, and `lactose-app` does not size the ring. The budgets in section 2 are
  multiplications of per-tick figures, not measurements of a nine-tick or a 39-tick rollback.
- **The thrown scene's deepest correction outside its two fault windows is not recorded.**
  `spike-net` carries each replay's deepest alone. The nine ticks are from the scenes whose
  link has no scripted fault.
- **The load rise during `spike-net` is not attributed.** Its load averages go from 3.44
  before the eight replays to 9.50 after. D143's second addendum says every track was
  stopped, which fits the replays' own threads; nothing here measured it. Section 2 gives
  both per-tick figures for that reason.
- **A tick's cost is not split.** The slot test's scene holds 59 proxies that the rollback's
  scene does not, so the slot's figures were not subtracted from the 387.7 µs.
- **The small-coordinates design is named, not examined.** Origin rebasing per region is an
  example of what would be proposed. This document did not check it against one physics world
  per server (section 16) or against reconciliation at full precision (section 9.4).
- **Session B has not run.** Criterion 4 has no x86 figure from a quiet machine. The laptop's
  files are labelled data.
- **The six doc-comment blocks were not built.** Their anchors and their line lengths are
  checked; `cargo fmt --check`, clippy, rustdoc and the doc tests were not run over the new
  text.
- **Row 41's x86-64 reading is of a cross-compiled object**, as its file says. Nothing here
  ran on x86-64: Rosetta cannot run the x86-64-v3 baseline (the row "Local x86-64 runs on
  Apple Silicon").
