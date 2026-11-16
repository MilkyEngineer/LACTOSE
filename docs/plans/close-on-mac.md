# Milestone 1 closes on the Mac

Status: a decision of record, for the coordinator to apply. Written at main `d7a0088`,
architecture revision 3.3, and revised after each of its two reviews. The milestone 2 plan is
read on branch `milestone-2-plan` at `b81f5c8`. The owner decided on 16 November 2026. This
document writes that decision down with every sentence it changes. It changes no code, and it
applies nothing of section 7. It is committed as `docs/plans/close-on-mac.md` in the commit
that applies it. The coordinator applied section 7 on 16 November 2026: on main as D151, and its
pairs against the milestone 2 plan on that plan's branch.

The decision is `D<N>` below. The coordinator numbers it at application, and it is expected to
be D151. In the pairs against the milestone 2 plan it is written `D<M1>`, because that plan
already uses `D<N>` for its own first decision.

## 1. The owner's answer

The coordinator asked how milestone 1 closes, given that the M1 meets criterion 1 directly and
a labelled laptop misses on x86. The owner chose "Close on the Mac", the recommended option.
Its text, as the coordinator's brief gives it:

> Milestone 1 exits on the Mac's figures; criterion 1 is met by the 3.34 ms on 4 performance
> cores, a direct pass on fewer cores. Sessions A and B leave the exit and join milestone 2's
> one desktop sitting. The laptop's x86 misses are recorded as a finding to settle in milestone
> 2. The keeper lane runs now on the Mac, before the macOS 26 upgrade, without session A's
> figures. A planner writes the decision and the section 20 change; nothing is asked of you
> before the exit.

The owner answered a second question the same day, on criterion 9 at the exit: "Mac now, rest
at the sitting". This document decides nothing the two answers did not. Section 6 lists what
the first answer left open, with the owner's second answer on the first item and the
coordinator's ruling on each of the others.

## 2. The evidence

Every figure is read from a result file under `docs/results` at `d7a0088`. Nothing was measured
for this document. The M1's files are of quiet windows. The laptop's are labelled: an i7-12650H
with six performance and four efficiency cores on Windows 11, with other work running. Its
files give the processor 45.9 to 68.0% idle, and two do not say. Both machines ran
`cargo xtask measure --all` at `bef3455`. The M1's prediction slot is of the second quiet
window, at `c50b688`.

| Measurement | Read against | The M1 | The laptop |
|---|---|---|---|
| The 100k tick, criterion 1 | 4 ms on eight performance cores | T₁ 10,193.0 µs, T₄ 3,344.6 µs; projected to eight 2,203.2 µs | T₁ 13,856.0 µs, T₆ 4,689.9 µs; projected to eight 4,231.6 µs |
| The physics phase, criterion 4 | 3 ms | 1,954.3 µs | 4,028.4 µs |
| The prediction slot's snapshot, criterion 5 | 100 µs on the M1; recorded on x86 with no budget (plan section 4) | 90.0 µs, the copy (D143) | 235.5 µs, the byte form D143 replaced |
| The sim skeleton, criterion 6 | 0.5 ms | 229.0 µs on one worker | 186.7 µs on one worker |
| Edit-to-playing, criterion 9 | 30 s | 1.56 s | 3.62 s |
| The headless run's hash | Equal on x86-64 and the M1 | 0x92ff315464f84575 | The same |
| WHISK, the makespan ratio, milestone 0 | 1.25 at P50 and 1.5 at P99 | At most 1.143 and 1.235 | Up to 2.588 and 6.576; 4.023 at P99 on the wide-shallow shape |
| WHISK, the wake from an idle pool | 35 µs at P99 | 31.6 µs | 236.4 µs |
| WHISK, tick start at 60 Hz | 20 µs at P99, and 1 ms on Windows (D144) | 27.0 µs | 15,918.4 µs, under the wait lane `timer-windows` replaced |

The table shows three things the answer's summary does not.

- The laptop's slot figure is not a miss of anything x86 is held to. The plan records the slot
  on x86 against no budget, since the client of criterion 7 is the aarch64 machine. The figure
  is also of the byte form, which missed on the M1 too, at 140.8 µs (the row "Section 22's
  milestone 1 timing criteria").
- The laptop's makespan miss is larger than the 4.02 the question quoted. That is one shape's
  P99, and the worst shape's is 6.576.
- The laptop's timer row has a known cause, which lane `timer-windows` fixed after `bef3455`
  (D144). At the lane's commit `f0b965c` the laptop's tick start is 533.0 and 502.9 µs at P99,
  under 1 ms, and its wake and makespan rows still fail (the row "The keeper's timed wait on
  Windows (D144)").

The laptop proves no timing criterion. It does prove one thing load cannot change: its headless
run and its 100k scene print the M1's hashes.

**How criterion 1 is read.** The criterion asks for at most 4 ms on eight performance cores.
The M1 has four, and its tick is 3,344.6 µs on four frame workers. The answer reads that as a
pass: a tick inside the budget on fewer performance cores than the criterion grants meets a
budget stated for eight. The reading assumes one thing, that more frame workers never lengthen
the tick. On the M1 the tick is 10,193.0 µs on one worker and 3,344.6 µs on four, so four
workers do not lengthen it against one. Those are two points, and they say nothing of two or
three workers. Nothing measures the assumption from four to eight on that machine, which has no
more performance cores. Nothing shows it on x86. The laptop's tick is shorter on six workers
than on one too, 4,689.9 µs against 13,856.0 µs, and it ends over the budget; what eight
desktop cores do is unmeasured. So the reading closes the criterion on the M1 and says nothing
of x86, which is the finding of section 3. The projection is no part of the reading. It steers,
as D139 decided.

## 3. The decisions

**3.1 Criterion 1 is met on the M1 by a direct run.** T₄, the median tick on four frame workers
at `bef3455`, is 3,344.6 µs against 4 ms. Decision 5.1 of the milestone 1 plan made session B's
T₈ the proof, said the M1 could not prove the criterion, and kept the projection as a question
should session A show fewer than eight performance cores. D<N> replaces those sentences and the
heading's second half, and leaves the rest of 5.1: the tick is still the scene without physics
and without the world hash, and eight frame workers still means what it meant for the x86
figure. Rules out the projection as proof on any machine, which D139 ruled out and this does
not bring back. What would change it: a run on the M1 where the tick is longer on more workers,
which would take the reading's one assumption away.

**3.2 Sessions A and B leave milestone 1's exit and run in milestone 2's one desktop sitting.**
Each runs at the commit the milestone 1 plan gives it, which is what the milestone 2 plan
already says. Session A passes or fails the x86 half of milestone 0's criteria and prints the
desktop's topology. Session B gives the x86 figures of criteria 1, 4, 6 and 9 and compares the
headless run's hash. Every row that waited on a session is closed on the M1's file, or its
desktop part is gated at milestone 2. Criterion 9 is met at the exit on the M1's file, and its
Windows figure moves with session B, with its Linux figure if the desktop boots Linux (the
owner's answer, item 1 of section 6). D138's sentence on the x86 half is amended by an addendum
and not rewritten. Rules out an exit that waits on the desktop. What would change it: the owner
sitting at the desktop before milestone 1's exit is recorded, in which case the files are read
at the exit as the plan first had it.

**3.3 The laptop's misses are a finding, in a new verification row gated at milestone 2.** The
row carries the figures and the commit. It is read as what the desktop sitting must settle: a
pass there, or a remedy the planner brings to the owner. A miss on the desktop that the laptop
did not show joins it. Rules out reading a labelled, loaded laptop as a failed criterion, and
reading it as nothing. What would change it: nothing before the sitting.

**3.4 `keeper` runs now.** It runs on the M1 under macOS 14.7, before the Mac is upgraded,
without session A's numbers. Its other condition, that `timer-windows` has merged, is met at
`9c1e005`. D138's rule otherwise stands: one bounded attempt, or tick start restated at 35 µs.
A failure of milestone 0's criteria found on x86 in the sitting goes to the finding and opens
no second attempt. Rules out waiting for the x86 numbers. What would change it: nothing.

**3.5 Architecture section 20 gains one sentence, and section 22 one.** Section 20 keeps both
machines and says whose sitting is whose. Section 22's paragraph on how milestone 1's criteria
are read says today that the 100k tick runs on eight frame workers. It gains the reading of
section 2 with its assumption, so the document that binds agrees with the decision and rests on
no more than the decision does. The criterion's own words in the milestone table do not change.
The section 22 sentence goes beyond the answer's "the section 20 change", and it stays by the
ruling of item 5 in section 6.

No lane is added and none is removed. `keeper` was already counted at six runs. This decision
costs about five: its writing, a review, an answer, that answer's review and its application.

## 4. Where each change lands

| File | Place | What changes |
|---|---|---|
| `docs/decisions-log.md` | A new entry, D<N> | The decision |
| `docs/decisions-log.md` | D138, after its "Replaced" | An addendum: the x86 half is settled in milestone 2's sitting, and the attempt runs before the x86 numbers |
| `docs/architecture.md` | Section 20, the results sentence | One sentence: milestone 1's measurements are the Mac's, the desktop's first sitting is milestone 2's. The bullet is rewrapped to its end |
| `docs/architecture.md` | Section 22, how criterion 1 is read | One sentence: a tick inside the budget on fewer performance cores meets it, on the reading that more workers do not lengthen it |
| `docs/plans/milestone-1.md` | Section 1, criterion 1 | Met by the quiet window's T₄; proven by it |
| `docs/plans/milestone-1.md` | Section 3, `keeper`'s Needs | Session A's numbers go; the lane runs now |
| `docs/plans/milestone-1.md` | Section 4, after the three sittings | A paragraph: both sessions run in milestone 2's sitting, how sections 1 and 2 are read, and what else follows |
| `docs/plans/milestone-1.md` | Section 4, criterion 1's row | The M1's column is the proof, the desktop's the x86 figure |
| `docs/plans/milestone-1.md` | Decision 5.1 | Appended, as D143 and D146 were appended to 5.5 and 5.4: the four things D<N> replaces in it |
| `docs/plans/milestone-1.md` | Section 5, the withdrawn question | Appended: the exception for a desktop with fewer than eight performance cores is withdrawn |
| `docs/plans/milestone-1.md` | Section 7, exit step 1 | Session B and the desktop's `whisk` file leave the step |
| `docs/verification.md` | Rows 64, 77, 80 and 164 | The M1's part done where there is one; the desktop's part gated at 2 |
| `docs/verification.md` | Row 81 | `keeper` runs now; the x86 half gated at 2 |
| `docs/verification.md` | Rows 94, 123, 133 and 149 | Done on the M1's files; the desktop's figure is the new row's |
| `docs/verification.md` | A new row, after row 168 | The x86 half of milestones 0 and 1, and the finding |
| `docs/plans/milestone-2.md` on its branch | Sections 2, 3, 4, 6, 7 and 9 | Section 5 below |

Rows 20 and 90 are Done and say that session B gives the desktop's figure. That stays true and
they are not edited. Row 79, the wall-clock log, waits on `keeper` and is not edited. The
plan's other mentions of the two sessions are left as approved: its sections 1 and 2 still name
session B as what closes criteria 6 and 9 and four rows, and the paragraph added to section 4
says how those are read.

## 5. The milestone 2 plan

The plan on branch `milestone-2-plan` already has session C's sitting carry "whatever of
milestone 1's sessions A and B is still owed". D<N> makes that definite. The pairs of section 7
do six things to it.

- Section 4: the sitting runs both sessions whole, and the desktop's row says what the sitting
  returns.
- Section 2: the rows gated at 2 by D<N> join its table: 64, 77, 80, 81 and the new row. Row
  164 gains its first half. The new row closes when every criterion the two sessions read
  passes on the desktop, not the laptop's misses alone.
- Section 6: row 164's gate is 2 before the plan touches it.
- Section 7: exit step 1 names the two sessions' files, and exit step 6 has the reviewers read
  the new row against those files. Without that no reviewer would read milestone 0's and
  milestone 1's x86 half at any exit, and nothing would reach step 7 for the owner to rule on.
- Section 9: milestone 1's exit no longer waits on the desktop. Five rows of its table are
  settled by D<N> and say so, and one records the two questions the re-base puts to the owner.
- Section 3: the run count.

**The run counts do not change.** The 22 lanes stay 170 runs and the document work about 22.
The sitting is the owner's time, and its rows close in the coordinator's pass. One cost is
contingent: if the desktop misses, the planner's remedy is about four runs for a proposal with
its review, its answer and its application, and seven more if the remedy is a lane. The plan's
section 3 gains that sentence.

**One consequence the plan's section 9 now states.** Session C sits at milestone 2's
measurement commit. An x86 miss of a milestone 1 criterion is therefore known near milestone
2's end, not before its first lane, and an ECS lane it calls for can no longer run ahead of
`present-impl`.

The plan is a draft, to be re-based when milestone 1 exits. Its line numbers will move, and
these pairs are checked at `b81f5c8` alone.

## 6. What the answer left open, as ruled

The first answer left six things open. The owner answered the first on 16 November 2026,
through the coordinator's question, and the coordinator ruled on the other five the same day.
This document records each where it lands.

1. **Criterion 9 on Windows and Linux at the exit.** The criterion says "per platform". At the
   exit macOS has the M1's file and Linux a runner's file (D149). Windows has no file that
   proves it: the runner's 5.83 s is a figure in row 64, and the laptop's 3.62 s is labelled.
   The owner's answer, "Mac now, rest at the sitting", the recommended option: criterion 9 is
   met at the exit on the M1's file, 1.56 s, and is recorded as met as restated by D<N>; its
   Windows and Linux figures come from session B in milestone 2's desktop sitting. On Linux
   that is if the desktop boots Linux; if it does not, the runner's file stands, as D149
   decided. No runner's file is checked in for Windows. The entry's second decision and row 64
   say so.
2. **One commit for the sitting.** Session C's `cargo xtask measure --all` at milestone 2's
   measurement commit writes milestone 1's files and the WHISK file on the desktop anyway.
   Sessions A and B at their own commits add `bef3455`, and `9c1e005` for the Windows `whisk`
   file, to the owner's sitting. Reading milestone 1's x86 figures from session C's files would
   make it one checkout, and would change D144's commit for the desktop's Windows file and the
   plan's "at one commit". Ruled: raised with the owner when the milestone 2 plan is re-based.
   That plan's section 9 records it.
3. **When the owner sits.** Sessions A and B need no milestone 2 code. Tied to session C they
   report at milestone 2's measurement commit. Ruled: raised with the owner at the same
   re-base, and recorded in the same place.
4. **Whether this document is committed.** Ruled: it is, as `docs/plans/close-on-mac.md`, and
   the entry names it.
5. **The section 22 sentence.** The answer names "the section 20 change" alone. Without a
   sentence in section 22 the architecture would say the tick runs on eight frame workers while
   D<N> closes it on four. Ruled: the sentence stays, and states the assumption it rests on.
6. **The exit's verdicts.** Exit step 5's reviewer has three verdicts. Criterion 9 is "met as
   restated, naming D<N>" by the owner's answer of item 1. Ruled: criterion 1 is the same, not
   "met". The entry's second decision says both.

## 7. Text for the coordinator (exact)

Every OLD below occurs exactly once in its file at the commit named. An OLD or a NEW may span
lines, and is matched with its line breaks. `planner-close-on-mac-check.py`, kept in the
coordinator's scratch area and not part of the tree, checks that and what its header lists. Its
output closes this section. Main may have moved since `d7a0088`: run the script with
`--worktree` on main before applying.

### Decision entry

```
**D<N>. Milestone 1 closes on the Mac: criterion 1 is met by the M1's run on four performance cores, and the x86 half of milestones 0 and 1 is settled in milestone 2's desktop sitting** (§20, §22, `lactose-task`, `docs/plans/milestone-1.md`, `docs/plans/close-on-mac.md`, `docs/results`)
- Why: milestone 1's exit waited on two sittings at the owner's x86 desktop, session A for the
  x86 half of milestone 0's criteria (D138) and session B for criterion 1's proof and the x86
  figures of the other timing criteria (D139). Neither has run. The M1's files are in. At
  `bef3455` its 100k tick takes 10,193.0 µs on one frame worker and 3,344.6 µs on four, against
  4 ms (`docs/results/tick-100k-apple-m1-2026-11-15.md`); criteria 4, 6 and 9 are met there,
  and criterion 5 at `c50b688`. A labelled laptop, an i7-12650H with six performance cores on
  Windows 11, ran `cargo xtask measure --all` at `bef3455` with other work on it. It proves no
  timing criterion; its hashes equal the M1's. It misses where the M1 passes. The owner chose
  to close milestone 1 on the Mac. `docs/plans/close-on-mac.md` holds the evidence; these are
  the decisions.
  - *Criterion 1 is met on the M1 by a direct run: T₄, the median tick on four frame workers,
    is at most 4 ms.* A tick inside the budget on fewer performance cores than the criterion
    grants meets a budget stated for eight. The reading assumes that more frame workers never
    lengthen the tick. The M1's T₁ and T₄ show four workers do not lengthen it against one;
    nothing shows it from four to eight, or on x86. The projection steers as before and is no
    part of the proof. Rules out holding milestone 1 for a machine with eight performance
    cores; and the projection standing as proof on any machine, which D139 ruled out and this
    does not bring back.
  - *Sessions A and B leave milestone 1's exit and run in milestone 2's one desktop sitting*,
    each at the commit milestone 1's plan gives it. Session A passes or fails milestone 0's
    makespan, wake, tick start and steal on x86 and prints the desktop's topology. Session B
    gives the x86 figures of criteria 1, 4, 6 and 9 and compares the headless run's hash.
    Milestone 1's timing criteria are proven by the M1's files, and the desktop's part of each
    row that waited on a session is gated at milestone 2. Criterion 9 is met at the exit on the
    M1's file, 1.56 s against 30; its other platforms' figures are session B's, and Linux has a
    runner's file unless session B runs there (D149). The exit's reviewer reads criteria 1 and
    9 as met as restated, by this decision. Rules out a milestone 1 exit that waits on the
    desktop, and an x86 half that no milestone owns.
  - *The laptop's misses are a finding, gated at milestone 2.* At `bef3455`: the 100k tick
    4,689.9 µs on six frame workers, projected to eight 4,231.6 µs, against 4 ms; the physics
    phase 4,028.4 µs against 3 ms; the wake from an idle pool 236.4 µs at P99 against 35; the
    makespan ratio up to 6.576 at P99 against 1.5. Its prediction slot, 235.5 µs, is of the
    byte form D143 replaced, which the plan records on x86 against no budget. The desktop
    sitting settles each miss: a pass there, or a remedy the planner brings to the owner. Rules
    out reading a labelled, loaded laptop as a failed criterion, and reading it as nothing.
  - *`keeper` runs now, on the M1 under macOS 14.7, before the Mac is upgraded and without
    session A's numbers.* D138's rule otherwise stands: one bounded attempt, or tick start
    restated at 35 µs. A failure of milestone 0's criteria found on x86 in the sitting goes to
    the finding, not to `keeper`. Rules out a second attempt after the x86 numbers.
  - *Section 20 keeps both machines and says whose sitting is whose, and section 22 states the
    reading of criterion 1 with its assumption.* The criterion's words in the milestone table
    do not change.
- Driven by: the owner's answer of 16 November 2026 to the coordinator's question on how
  milestone 1 closes, "Close on the Mac", and the owner's answer of the same day on criterion 9
  at the exit, "Mac now, rest at the sitting"; the M1's and the labelled laptop's result files
  at `bef3455`; the coordinator's rulings of that day on what else the first answer left open;
  the document's first review, which found the plan sentences the first draft left standing and
  the assumption the draft's section 22 sentence left out.
- Replaced: D139's "Criterion 1 is proven by a direct run on eight performance cores". In
  `docs/plans/milestone-1.md`: the heading and sentences of decision 5.1 that 5.1 now names;
  section 4's "Its run of the 100k tick on eight frame workers is criterion 1's proof" and, of
  a failure in session A, its "goes to `keeper`"; section 5's exception for a desktop with
  fewer than eight performance cores; and exit step 1. For the x86 half, D138's "Each is an
  exit condition of milestone 1", and its "after the x86 numbers", both by the addendum there.
  In section 20 and section 22 nothing: each gains a sentence. D144's rule for the desktop's
  Windows `whisk` file stands, and is met at milestone 2.
```

### Addendum to D138

**D138, the addendum.** Line 4970 at `d7a0088`. OLD:

~~~
- Replaced: nothing. It records what was done without a record.
~~~

NEW:

~~~
- Replaced: nothing. It records what was done without a record.
- Addendum (D<N>, 2026-11-16): *the x86 half leaves milestone 1's exit, and the keeper's attempt
  no longer waits for it.* The owner closed milestone 1 on the Mac. The session on the owner's
  desktop runs in milestone 2's one desktop sitting, beside milestone 1's own x86 session, and
  the x86 half of each criterion is settled there: a pass, or a remedy the planner brings to the
  owner. Its rows are gated at milestone 2. The M1's half of each stays an exit condition of
  milestone 1. The lane of the bounded attempt runs before the x86 numbers, on the M1 under
  macOS 14.7 and before that machine is upgraded. An x86 failure found in the sitting joins
  D<N>'s finding and opens no second attempt. The rule is otherwise as decided: one attempt, or
  tick start restated at 35 µs.
~~~

### Architecture amendments

The first pair takes the bullet to its end and the second one line, so that neither leaves a
short line behind. Both are wrapped at 95 columns, as the two paragraphs are.

**Section 20, bare-metal measurements.** Line 1308 at `d7a0088`. OLD:

~~~
  results, since CI has no hybrid hardware. A measurement that proves a milestone criterion
  is a file under `docs/results` that carries its machine, date, commit, detected topology,
  workers and load; one command, `cargo xtask measure`, writes those of milestone 1 and
  later. A number taken while other work loads the machine steers and proves nothing.
~~~

NEW:

~~~
  results, since CI has no hybrid hardware. Milestone 1's are the Mac's; the desktop's first
  sitting is milestone 2's, and it measures milestones 0 and 1 there too (D<N>). A measurement
  that proves a milestone criterion is a file under `docs/results` that carries its machine,
  date, commit, detected topology, workers and load; one command, `cargo xtask measure`, writes
  those of milestone 1 and later. A number taken while other work loads the machine steers and
  proves nothing.
~~~

**Section 22, how criterion 1 is read.** Line 1395 at `d7a0088`. OLD:

~~~
counted once; on a part that reports no core types, eight of its physical cores. It excludes
~~~

NEW:

~~~
counted once; on a part that reports no core types, eight of its physical cores. A tick inside
the budget on fewer performance cores meets the criterion, on the reading that more frame
workers do not lengthen it, and milestone 1's is the M1's on four (D<N>). The tick excludes
~~~

### Plan amendments

`docs/plans/milestone-1.md`.

**Plan section 1, criterion 1.** Line 37 at `d7a0088`. OLD:

~~~
Every system runs as one job, so the worker count changes nothing | `ecs-a`, `ecs-kernel`, `ecs-fork`; proven by session B's run on eight performance cores (section 4) |
~~~

NEW:

~~~
Every system runs as one job, so the worker count changes nothing. After `ecs-fork` the quiet window at `bef3455` measured T₄ at 3,344.6 µs on four frame workers: met, by a direct run on fewer performance cores than the criterion grants (D<N>) | `ecs-a`, `ecs-kernel`, `ecs-fork`; proven by the quiet window's T₄ on the M1 (section 4, and decision 5.1 as D<N> amends it) |
~~~

**Plan section 3, what `keeper` needs.** Line 152 at `d7a0088`. OLD:

~~~
| Session A's numbers; `timer-windows` merged, since both edit `crates/task/src/sleep.rs`. The Windows timed wait is not this lane's (D144) |
~~~

NEW:

~~~
| `timer-windows` merged, since both edit `crates/task/src/sleep.rs`; it merged at `9c1e005`. The lane runs now, on the M1 under macOS 14.7 and before the Mac is upgraded, without session A's numbers (D<N>). The Windows timed wait is not this lane's (D144) |
~~~

**Plan section 4, the sittings.** Line 281 at `d7a0088`. OLD:

~~~
**What each timing criterion runs, and against what.**
~~~

NEW:

~~~
D<N> takes sessions A and B out of milestone 1's exit: the owner closed the milestone on the Mac on
16 November 2026. Both run in milestone 2's one desktop sitting, each at the commit given above.
What they close there is the x86 half of milestone 0's criteria and the x86 figures of criteria 1,
4, 6 and 9. Milestone 1's criteria rest on the quiet windows. Where sections 1 and 2 name session B
as what closes a criterion or a row, the quiet window closes it for milestone 1, row 125's x86-64
hash with the labelled laptop's run (the verification row "The headless binary's 120-tick run"),
and session B gives the x86 figure in that sitting. Session B's T₈ is no longer criterion 1's proof
(decision 5.1). A failure in session A no longer goes to `keeper`, which runs first: it joins the
finding that the verification row "The x86 half of milestones 0 and 1 (D<N>)" records, beside the
labelled laptop's misses.

**What each timing criterion runs, and against what.**
~~~

**Plan section 4, criterion 1's row.** Line 285 at `d7a0088`. OLD:

~~~
| Steers, and proves nothing: 7·T₄ − T₁ at most 24 ms | The proof: T₈ at most 4 ms on eight frame workers, one for each of eight performance cores |
~~~

NEW:

~~~
| The proof (D<N>): T₄ at most 4 ms on four frame workers, a direct run on fewer performance cores than the criterion grants. The projection steers, and proves nothing: 7·T₄ − T₁ at most 24 ms | The x86 figure, in milestone 2's desktop sitting: T₈ at most 4 ms on eight frame workers, one for each of eight performance cores |
~~~

**Plan decision 5.1.** Line 335 at `d7a0088`. OLD:

~~~
the kernel path's target four times harder.
~~~

NEW:

~~~
the kernel path's target four times harder. The owner closed milestone 1 on the Mac on 16 November
2026, and D<N> replaces four things in this decision: its heading's "and the desktop's eight
performance cores prove it"; its proof, from "The owner's desktop has eight or more performance
cores" to "is at most 4 ms"; its sentence "The M1 has four performance cores and cannot prove the
criterion, so there a projection steers"; and its sentence "If session A shows fewer than eight
performance cores, the projection comes back to the owner as a question". The criterion is met on
the M1 by a direct run: T₄, the median of 60 ticks on four frame workers, is 3,344.6 µs at
`bef3455` (`docs/results/tick-100k-apple-m1-2026-11-15.md`). A tick inside the budget on fewer
performance cores than the criterion grants meets it, on the reading that more frame workers never
lengthen the tick. T₁ and T₄ show that four workers do not lengthen it against one on the M1, and
nothing shows it from four to eight, or on x86. The projection still steers, and it proves nothing
on any machine. Session B's T₈ is the x86 figure, taken in milestone 2's desktop sitting, and a
miss there is a finding for the planner.
~~~

**Plan section 5, the withdrawn question.** Line 553 at `d7a0088`. OLD:

~~~
back as a question only if session A shows fewer than eight performance cores.
~~~

NEW:

~~~
back as a question only if session A shows fewer than eight performance cores. D<N> withdraws the
exception: the projection proves nothing on any machine.
~~~

**Plan section 7, exit step 1.** Line 768 at `d7a0088`. OLD:

~~~
1. The measurement commit exists. The quiet window and session B have run on it, and their result
   files are on main. Session B's run on eight frame workers is criterion 1's proof. The second
   quiet window has run at the first commit that holds `slot-copy`, and its three files are
   criterion 5's proof, criterion 7's rollback figure and criterion 7's first `spike-net` file
   (D143). If the desktop boots Windows, its `whisk` file there has been written at the first commit
   that holds `timer-windows` and, if one is made, the follow-up that sets its Windows threshold
   back (D144).
~~~

NEW:

~~~
1. The measurement commit exists. The quiet window has run on it, and its result files are on
   main. Its T₄ on four frame workers is criterion 1's proof (decision 5.1, D<N>). The second
   quiet window has run at the first commit that holds `slot-copy`, and its three files are
   criterion 5's proof, criterion 7's rollback figure and criterion 7's first `spike-net` file
   (D143). Sessions A and B are no part of the exit: they run in milestone 2's desktop sitting,
   and the desktop's Windows `whisk` file is written there (D144, D<N>).
~~~

### Verification rows

`docs/verification.md`. Rows 94 and 149 take more than one pair. The last pair adds a row after
the file's last line.

**Row 64, lld-link and edit-to-playing.** Line 64 at `d7a0088`. OLD:

~~~
it is criterion 9's on Linux unless session B runs there, and then session B's file is (D149) | 1 |
~~~

NEW:

~~~
it is criterion 9's on Linux unless session B runs there, and then session B's file is (D149). Milestone 1's part is **Done** (D<N>): criterion 9 is met at the exit on the M1's file. Its Windows figure, and its Linux figure if the desktop boots Linux (D149), are session B's, in milestone 2's desktop sitting (the row "The x86 half of milestones 0 and 1 (D<N>)"). Until then Windows has the runner's 5.83 s above and the labelled laptop's 3.62 s (`docs/results/edit-to-playing-laptop-windows-2026-11-15.md`), which prove no criterion | 2 |
~~~

**Row 77, wake latency.** Line 77 at `d7a0088`. OLD:

~~~
A decision comes due if the desktop's Windows file fails the wake or the makespan. | 1 (carried, D138) |
~~~

NEW:

~~~
A decision comes due if the desktop's Windows file fails the wake or the makespan. The M1's half is **Done** (D<N>): the quiet window's P99 is 31.6 µs, under 35. The x86 half is session A's, in milestone 2's desktop sitting, and the laptop's miss is part of the finding it settles (the row "The x86 half of milestones 0 and 1 (D<N>)") | 2 (carried, D138 and D<N>) |
~~~

**Row 80, the topology parsers.** Line 80 at `d7a0088`. OLD:

~~~
they first run for real on CI and the x86 desktop | 1 (carried, D138) |
~~~

NEW:

~~~
they first run for real on CI and the x86 desktop. They have run on a linux-x64 runner (`docs/results/edit-to-playing-github-runner-linux-x64-2026-11-15.md`: 1 performance core and no efficiency core) and on a labelled hybrid laptop under Windows 11 (`docs/results/whisk-laptop-windows-2026-11-15.md`: 6 performance and 4 efficiency cores, an i7-12650H's). The desktop's run is session A's, in milestone 2's desktop sitting (D<N>) | 2 (carried, D138 and D<N>) |
~~~

**Row 81, tick start.** Line 81 at `d7a0088`. OLD:

~~~
lane `timer-windows` builds it (the row "The keeper's timed wait on Windows (D144)"). | 1 (carried, D138) |
~~~

NEW:

~~~
lane `timer-windows` builds it (the row "The keeper's timed wait on Windows (D144)"). The quiet window's P99 at 60 Hz is 27.0 µs on the M1 (`docs/results/whisk-apple-m1-2026-11-15.md`). `keeper`'s one attempt runs now on the M1, before the Mac is upgraded and without the x86 numbers (D<N>). The x86 half is session A's, in milestone 2's desktop sitting (the row "The x86 half of milestones 0 and 1 (D<N>)") | 1 (the M1's half, by `keeper` or D138's restatement) and 2 (the x86 half, D<N>) |
~~~

**Row 94, the 100k tick, where it is measured.** Line 94 at `d7a0088`. OLD:

~~~
Measured on eight performance cores in session B; the M1's projection of decision 5.1 steers (`docs/plans/milestone-1.md`).
~~~

NEW:

~~~
Proven on the M1 by T₄ on four frame workers (D<N>); the projection of decision 5.1 steers, and the run on eight performance cores is session B's, in milestone 2's desktop sitting (`docs/plans/milestone-1.md`).
~~~

**Row 94, the 100k tick, its proof.** Line 94 at `d7a0088`. OLD:

~~~
The projection steers; session B's run on eight performance cores is the proof. | 1 |
~~~

NEW:

~~~
The projection steers. **Done** (D<N>): criterion 1 is met on the M1 by this T₄, a direct run inside the 4 ms on four performance cores, fewer than the criterion grants. The reading is that more frame workers do not lengthen the tick: T₁ and T₄ show that four do not lengthen it against one on the M1, and nothing shows it from four to eight, or on x86. Session B's run on eight performance cores is the x86 figure, taken in milestone 2's desktop sitting (the row "The x86 half of milestones 0 and 1 (D<N>)") | 1 |
~~~

**Row 123, the sim skeleton.** Line 123 at `d7a0088`. OLD:

~~~
criterion 6 is met on the M1, and session B gives the x86 desktop's figure. | 1 |
~~~

NEW:

~~~
criterion 6 is met on the M1, and session B gives the x86 desktop's figure. **Done** on the M1's file (D<N>): session B runs in milestone 2's desktop sitting (the row "The x86 half of milestones 0 and 1 (D<N>)") | 1 |
~~~

**Row 133, the headless run's hash.** Line 133 at `d7a0088`. OLD:

~~~
which is the hash session B compares on x86-64. | 1 |
~~~

NEW:

~~~
which is the hash session B compares on x86-64. **Done** for milestone 1 (D<N>): a labelled x86-64 run at the same commit printed that hash (`docs/results/headless-120-laptop-windows-2026-11-15.md`), and the desktop's comparison is session B's, in milestone 2's desktop sitting (the row "The x86 half of milestones 0 and 1 (D<N>)") | 1 |
~~~

**Row 149, the timing criteria, whose files.** Line 149 at `d7a0088`. OLD:

~~~
on a quiet Apple M1 and on the owner's x86 desktop at one commit (`docs/plans/milestone-1.md`, section 4). Criterion 1 is proven by the desktop's run on eight performance cores; on the M1 a projection from a one-worker and a four-worker run steers (D139).
~~~

NEW:

~~~
on a quiet Apple M1 (`docs/plans/milestone-1.md`, section 4); the owner's x86 desktop writes its files in milestone 2's desktop sitting (D<N>). Criterion 1 is met on the M1 by a direct run, T₄ on four frame workers at most 4 ms, on fewer performance cores than the criterion grants (D<N>); the projection from a one-worker and a four-worker run steers and proves nothing (D139).
~~~

**Row 149, the timing criteria, session B's files.** Line 149 at `d7a0088`. OLD:

~~~
No file of session B is in `docs/results` yet.
~~~

NEW:

~~~
No file of the desktop is in `docs/results`: sessions A and B run in milestone 2's desktop sitting (D<N>).
~~~

**Row 149, the timing criteria, closed.** Line 149 at `d7a0088`. OLD:

~~~
not at the measurement commit (D144). | 1 |
~~~

NEW:

~~~
not at the measurement commit (D144). **Done** on the M1's files (D<N>): criteria 1, 4, 5, 6 and 9 are met there, criterion 1 by T₄ at 3,344.6 µs, and what the desktop owes is the row "The x86 half of milestones 0 and 1 (D<N>)" | 1 |
~~~

**Row 164, the keeper's timed wait on Windows.** Line 164 at `d7a0088`. OLD:

~~~
where the first Windows client's frame pacing is decided (D144) | 1 (the desktop's file) and 2 (the list) |
~~~

NEW:

~~~
where the first Windows client's frame pacing is decided (D144). The desktop's file is session A's and is written in milestone 2's desktop sitting, at those commits (D<N>) | 2 |
~~~

**The new row, the x86 half and the finding.** Line 168 at `d7a0088`. OLD:

~~~
and its cap is that review's (D124's spike-net addendum) | 4 |
~~~

NEW:

~~~
and its cap is that review's (D124's spike-net addendum) | 4 |
| The x86 half of milestones 0 and 1 (D<N>): milestone 1 closed on the Apple M1's files, and the owner's x86 desktop has measured nothing. Sessions A and B of `docs/plans/milestone-1.md`, section 4, run in milestone 2's one desktop sitting, each at the commit that plan gives it. Session A is the WHISK benchmark and `lactose-task`'s tests, which pass or fail milestone 0's makespan, wake, tick start and steal on x86, tick start on Windows against 1 ms (D144). Session B is `cargo xtask measure --all` and the spike's recording tests under each system the desktop boots, read against criterion 1's 4 ms on eight frame workers, criterion 4's 3 ms, criterion 6's 0.5 ms and criterion 9's 30 s, with the headless run's hash against the M1's. The finding the sitting settles: a labelled laptop, an i7-12650H with six performance and four efficiency cores on Windows 11, ran `cargo xtask measure --all` at `bef3455` with other work on it (`docs/results/*-laptop-windows-2026-11-15.*`; the processor 45.9 to 68.0% idle where the file says). It proves no timing criterion; its hashes equal the M1's. It misses where the M1 passes. The 100k tick: T₆ 4,689.9 µs on six frame workers, projected to eight 4,231.6 µs, against 4 ms; the M1's T₄ is 3,344.6 µs. The physics phase: 4,028.4 µs against 3 ms; the M1's is 1,954.3 µs. The wake from an idle pool: P99 236.4 µs against 35; the M1's is 31.6 µs. The makespan ratio: up to 2.588 at P50 and 6.576 at P99 against 1.25 and 1.5; the M1's is at most 1.143 and 1.235. Its prediction slot's snapshot, 235.5 µs, is of the byte form D143 replaced, which the plan records on x86 against no budget. Its timer rows are of the wait lane `timer-windows` replaced (the row "The keeper's timed wait on Windows (D144)"). Its sim skeleton at 186.7 µs and its edit-to-playing at 3.62 s pass. Each miss closes by a pass on the desktop, or by a remedy the planner brings to the owner; a miss on the desktop that the laptop did not show joins the finding | 2 |
~~~

### The milestone 2 plan

`docs/plans/milestone-2.md` on branch `milestone-2-plan`. Here the decision is `D<M1>`, and
`D<N>` is that plan's own.

**Milestone 2 plan section 2, the rows sessions A and B close.** Line 80 at `b81f5c8`. OLD:

~~~
| 63 | Windows efficiency-core steering | `eco-qos` | The benchmark's rows with two background threads, before and after, on the laptop, which is a hybrid part |
~~~

NEW:

~~~
| 63 | Windows efficiency-core steering | `eco-qos` | The benchmark's rows with two background threads, before and after, on the laptop, which is a hybrid part |
| 64 | `lld-link` on Windows | Session B, in session C's sitting | The desktop's edit-to-playing file under each system it boots, at most 30 s (D<M1>) |
| 77 | Wake latency against the 20 µs P99 criterion | Session A, in session C's sitting | The desktop's `whisk` file: the wake from an idle pool under 35 µs at P99, with the makespan and the steal beside it. The laptop missed the first two (D<M1>) |
| 80 | The Linux and Windows core-topology parsers | Session A, in session C's sitting | The topology the desktop's files print (D<M1>) |
| 81 | Tick-start latency against the 20 µs P99 criterion | Session A, in session C's sitting, for the x86 half | The desktop's `whisk` file: under 1 ms at P99 on Windows (D144), and on its other system under 20 µs, or under 35 µs if D138 has restated it |
~~~

**Milestone 2 plan section 2, row 164 and the finding's row.** Line 92 at `b81f5c8`. OLD:

~~~
| 164 | The keeper's timed wait on Windows | `frame-design` | Its second half asks that the unmeasured list be read where frame pacing on Windows is designed. `frame-design` reads it. An item its design uses becomes a measurement it names for `measure-m2`, and the rest is re-gated with its reason |
~~~

NEW:

~~~
| 164 | The keeper's timed wait on Windows | `frame-design`; session A, in session C's sitting | Its second half asks that the unmeasured list be read where frame pacing on Windows is designed. `frame-design` reads it. An item its design uses becomes a measurement it names for `measure-m2`, and the rest is re-gated with its reason. Its first half, the desktop's Windows `whisk` file, is session A's (D<M1>) |
| added by D<M1> | The x86 half of milestones 0 and 1 | Sessions A and B, in session C's sitting; the planner on a miss | Every criterion sessions A and B read passes on the desktop, the labelled laptop's misses first, or the planner brings a remedy and the owner rules on it at this plan's exit step 7 |
~~~

**Milestone 2 plan section 3, the run count.** Line 195 at `b81f5c8`. OLD:

~~~
Question 6's split, if it is asked, adds five.
~~~

NEW:

~~~
Question 6's split, if it is asked, adds five. D<M1> adds none: the sitting is the owner's, and
its rows close in the coordinator's pass. A miss on the desktop adds the planner's remedy, about
four runs for a proposal with its review, its answer and its application, and seven more if the
remedy is a lane.
~~~

**Milestone 2 plan section 4, the desktop.** Line 246 at `b81f5c8`. OLD:

~~~
| The desktop: the owner's, eight or more performance cores, booting Windows and another system (D144). Its CPU model, its GPU and its other system are unknown until milestone 1's session A returns | The x86 desktop figures section 20 asks for. A second discrete GPU | Anything without the owner at it |
~~~

NEW:

~~~
| The desktop: the owner's, eight or more performance cores, booting Windows and another system (D144). Its CPU model, its GPU and its other system are unknown until its sitting, where milestone 1's session A returns them (D<M1>) | The x86 desktop figures section 20 asks for, and the x86 half of milestones 0 and 1 (D<M1>). A second discrete GPU | Anything without the owner at it |
~~~

**Milestone 2 plan section 4, session C's sitting.** Line 265 at `b81f5c8`. OLD:

~~~
  Whatever of milestone 1's sessions A and B is still owed runs in the same sitting, at their own
  commits. Under a system that is not a client platform, the measurements that need a window
~~~

NEW:

~~~
  Milestone 1's sessions A and B run in the same sitting, at their own commits (D<M1>): session
  A's WHISK benchmark and `lactose-task` tests, for the x86 half of milestone 0's criteria, and
  session B's `cargo xtask measure --all`, for the x86 figures of milestone 1's. The row "The
  x86 half of milestones 0 and 1" holds what they settle, the labelled laptop's misses first.
  Under a system that is not a client platform, the measurements that need a window
~~~

**Milestone 2 plan section 6, row 164's gate.** Line 992 at `b81f5c8`. OLD:

~~~
| 164, The keeper's timed wait on Windows | 1 and 2 | The list is read by `frame-design`: an item its design uses becomes a measurement it names, and the rest is re-gated with its reason |
~~~

NEW:

~~~
| 164, The keeper's timed wait on Windows | 2 | The list is read by `frame-design`: an item its design uses becomes a measurement it names, and the rest is re-gated with its reason. D<M1> has already moved the desktop's file to gate 2 |
~~~

**Milestone 2 plan section 7, exit step 1.** Line 1034 at `b81f5c8`. OLD:

~~~
1. The measurement commit exists. The quiet window, the laptop's session and session C have run
   on it, and their result files are on main.
~~~

NEW:

~~~
1. The measurement commit exists. The quiet window, the laptop's session and session C have run
   on it, and their result files are on main. Milestone 1's sessions A and B have run in session
   C's sitting, and their files are on main too (D<M1>).
~~~

**Milestone 2 plan section 7, exit step 6.** Line 1045 at `b81f5c8`. OLD:

~~~
6. A `reviewer` and a `reviewer-gpu` read each of the seven criteria against its test and its
   result file. They report one verdict for each: met; met as restated, naming the decision; or
   not met. They re-run what they can on the M1 and state what they could not.
~~~

NEW:

~~~
6. A `reviewer` and a `reviewer-gpu` read each of the seven criteria against its test and its
   result file. They report one verdict for each: met; met as restated, naming the decision; or
   not met. They re-run what they can on the M1 and state what they could not. They read the
   row "The x86 half of milestones 0 and 1" against sessions A and B's files the same way
   (D<M1>).
~~~

**Milestone 2 plan section 9, what it assumes.** Line 1078 at `b81f5c8`. OLD:

~~~
This plan assumes milestone 1's criteria are met as written, with one exception a file on main
already shows: tick start on the M1 is 27.0 µs at P99 against 20
(`docs/results/whisk-apple-m1-2026-11-15.md`), which `keeper` fixes or D138 restates at 35 µs.
Nothing in milestone 2 reads the difference.
~~~

NEW:

~~~
This plan assumes milestone 1's criteria are met on the M1, as D<M1> closed the milestone there:
criterion 1 by the M1's direct run on four performance cores, and the x86 half of milestones 0
and 1 left to session C's sitting. One exception a file on main already shows: tick start on the
M1 is 27.0 µs at P99 against 20 (`docs/results/whisk-apple-m1-2026-11-15.md`), which `keeper`
fixes or D138 restates at 35 µs. Nothing in milestone 2 reads the difference.
~~~

**Milestone 2 plan section 9, the sequence's first step.** Line 1086 at `b81f5c8`. OLD:

~~~
1. Milestone 1's exit is recorded, on macOS 14.7, the system its figures were taken on. `keeper`
   has run or tick start is restated by then, since its question is of that system.
~~~

NEW:

~~~
1. Milestone 1's exit is recorded, on macOS 14.7, the system its figures were taken on. It does
   not wait for sessions A and B (D<M1>). `keeper` has run or tick start is restated by then,
   since its question is of that system.
~~~

**Milestone 2 plan section 9, criterion 1.** Line 1113 at `b81f5c8`. OLD:

~~~
| Criterion 1, the 100k tick on eight performance cores, is not proven: session B has not run. If it misses, the owner rules for another ECS lane or a restatement | An ECS lane edits `lactose-ecs` first; `present-impl` waits behind it. Section 4's arithmetic for extract assumes the ECS at the speed `tick-100k` measured on the M1 under macOS 14.7 |
~~~

NEW:

~~~
| Criterion 1 is met on the M1 by its run on four performance cores (D<M1>). The run on eight is session B's, in session C's sitting, and the laptop's 4,689.9 µs on six is part of the finding. If the desktop misses, the planner brings a remedy and the owner rules: another ECS lane, or a restatement | The miss is known at the measurement commit, not before the first lane. An ECS lane then edits `lactose-ecs` under the one-lane rule. Section 4's arithmetic for extract assumes the ECS at the speed `tick-100k` measured on the M1 under macOS 14.7 |
~~~

**Milestone 2 plan section 9, tick start.** Line 1114 at `b81f5c8`. OLD:

~~~
| Tick start: `keeper` runs after session A's files, or the criterion is restated | `keeper` edits `lactose-task`. `eco-qos` waits behind it, and `frame-design` reads its outcome for frame pacing |
~~~

NEW:

~~~
| Tick start: `keeper` runs before the exit, without session A's files (D<M1>), or the criterion is restated | `keeper` edits `lactose-task`. `eco-qos` waits behind it, and `frame-design` reads its outcome for frame pacing |
~~~

**Milestone 2 plan section 9, criterion 9.** Line 1115 at `b81f5c8`. OLD:

~~~
| Criterion 9, edit-to-playing on Windows and Linux, is session B's. Over thirty seconds brings a build-time proposal | That proposal can change the workspace's build layout, and decision 5.15's figure is read again with it |
~~~

NEW:

~~~
| Criterion 9, edit-to-playing on Windows and Linux, is session B's, in session C's sitting (D<M1>). Over thirty seconds brings a build-time proposal | That proposal can change the workspace's build layout, and decision 5.15's figure is read again with it |
~~~

**Milestone 2 plan section 9, the desktop's facts.** Line 1116 at `b81f5c8`. OLD:

~~~
| Session A returns the desktop's CPU, GPU and second system | Section 4's desktop row, and what session C can run under the second system |
~~~

NEW:

~~~
| Session A runs in session C's sitting (D<M1>), so the desktop's CPU, GPU and second system are not known before it unless the owner sends them ahead. At this plan's re-base the owner is asked two things D<M1> left open: whether the sitting is one commit, with milestone 1's x86 figures read from session C's files, and whether to sit before the measurement commit | Section 4's desktop row, and what session C can run under the second system. One commit for the sitting changes D144's commit for the desktop's Windows `whisk` file and session B's. An earlier sitting brings an x86 miss of a milestone 1 criterion forward |
~~~

**Milestone 2 plan section 9, the rows.** Line 1117 at `b81f5c8`. OLD:

~~~
| Rows that wait on the desktop or on `keeper`: 64, 94, 123, 133, 149, and 77, 79, 80, 81 and 164's first half. The owner may re-gate any of them at exit step 6 | A row re-gated to 2 joins section 2's table. Session C can carry sessions A and B's commands |
~~~

NEW:

~~~
| D<M1> closes rows 94, 123, 133 and 149 on the M1 and gates at 2 the desktop's part of rows 64, 77, 80, 81 and 164, with a row of its own for the x86 half. Rows 79 and 81's M1 half wait on `keeper` | The rows gated at 2 are in section 2's table. Session C's sitting carries sessions A and B's commands |
~~~

### The check

Run from the root of a checkout that holds `d7a0088` and `b81f5c8`, with this document at its
committed path:

```
python3 -I planner-close-on-mac-check.py --doc docs/plans/close-on-mac.md
```

~~~
docs/architecture.md at d7a0088: 1455 lines
  section 20, bare-metal measurements: line 1308, text, 4 -> 6 lines: ok
  section 22, how criterion 1 is read: line 1395, text, 1 -> 3 lines: ok
docs/decisions-log.md at d7a0088: 6434 lines
  D138, the addendum: line 4970, append, 1 -> 10 lines: ok
docs/plans/milestone-1.md at d7a0088: 815 lines
  plan section 1, criterion 1: line 37, table, 1 -> 1 lines: ok
  plan section 3, what `keeper` needs: line 152, table, 1 -> 1 lines: ok
  plan section 4, the sittings: line 281, text, 1 -> 12 lines: ok
  plan section 4, criterion 1's row: line 285, table, 1 -> 1 lines: ok
  plan decision 5.1: line 335, append, 1 -> 13 lines: ok
  plan section 5, the withdrawn question: line 553, append, 1 -> 2 lines: ok
  plan section 7, exit step 1: line 768, text, 7 -> 6 lines: ok
docs/verification.md at d7a0088: 168 lines
  row 64, lld-link and edit-to-playing: line 64, row, 1 -> 1 lines, gate '1' -> '2': ok
  row 77, wake latency: line 77, row, 1 -> 1 lines, gate '1 (carried, D138)' -> '2 (carried, D138 and D<N>)': ok
  row 80, the topology parsers: line 80, row, 1 -> 1 lines, gate '1 (carried, D138)' -> '2 (carried, D138 and D<N>)': ok
  row 81, tick start: line 81, row, 1 -> 1 lines, gate '1 (carried, D138)' -> "1 (the M1's half, by `keeper` or D138's restatement) and 2 (the x86 half, D<N>)": ok
  row 94, the 100k tick, where it is measured: line 94, row, 1 -> 1 lines, gate '1' -> '1': ok
  row 94, the 100k tick, its proof: line 94, row, 1 -> 1 lines, gate '1' -> '1': ok
  row 123, the sim skeleton: line 123, row, 1 -> 1 lines, gate '1' -> '1': ok
  row 133, the headless run's hash: line 133, row, 1 -> 1 lines, gate '1' -> '1': ok
  row 149, the timing criteria, whose files: line 149, row, 1 -> 1 lines, gate '1' -> '1': ok
  row 149, the timing criteria, session B's files: line 149, row, 1 -> 1 lines, gate '1' -> '1': ok
  row 149, the timing criteria, closed: line 149, row, 1 -> 1 lines, gate '1' -> '1': ok
  row 164, the keeper's timed wait on Windows: line 164, row, 1 -> 1 lines, gate "1 (the desktop's file) and 2 (the list)" -> '2': ok
  the new row, the x86 half and the finding: line 168, addrow, 1 -> 2 lines, adds a row of 1981 chars at gate '2': ok
docs/plans/milestone-2.md at b81f5c8: 1220 lines
  milestone 2 plan section 2, the rows sessions A and B close: line 80, rows, 1 -> 5 lines, 5 rows of 5 cell marks: ok
  milestone 2 plan section 2, row 164 and the finding's row: line 92, rows, 1 -> 2 lines, 2 rows of 5 cell marks: ok
  milestone 2 plan section 3, the run count: line 195, append, 1 -> 4 lines: ok
  milestone 2 plan section 4, the desktop: line 246, table, 1 -> 1 lines: ok
  milestone 2 plan section 4, session C's sitting: line 265, text, 2 -> 5 lines: ok
  milestone 2 plan section 6, row 164's gate: line 992, table, 1 -> 1 lines: ok
  milestone 2 plan section 7, exit step 1: line 1034, append, 2 -> 3 lines: ok
  milestone 2 plan section 7, exit step 6: line 1045, append, 3 -> 5 lines: ok
  milestone 2 plan section 9, what it assumes: line 1078, text, 4 -> 5 lines: ok
  milestone 2 plan section 9, the sequence's first step: line 1086, text, 2 -> 3 lines: ok
  milestone 2 plan section 9, criterion 1: line 1113, table, 1 -> 1 lines: ok
  milestone 2 plan section 9, tick start: line 1114, table, 1 -> 1 lines: ok
  milestone 2 plan section 9, criterion 9: line 1115, table, 1 -> 1 lines: ok
  milestone 2 plan section 9, the desktop's facts: line 1116, table, 1 -> 1 lines: ok
  milestone 2 plan section 9, the rows: line 1117, table, 1 -> 1 lines: ok
docs/architecture.md: 1459 lines after the pairs, 1455 before
docs/decisions-log.md: 6443 lines after the pairs, 6434 before
docs/plans/milestone-1.md: 838 lines after the pairs, 815 before
docs/verification.md: 169 lines after the pairs, 168 before
docs/plans/milestone-2.md: 1236 lines after the pairs, 1220 before
the row "The headless binary's 120-tick run", cited 1 times, begins 1 row
the row "The keeper's timed wait on Windows (D144)", cited 2 times, begins 1 row
the row "The x86 half of milestones 0 and 1", cited 2 times, begins 1 row
the row "The x86 half of milestones 0 and 1 (D<N>)", cited 8 times, begins 1 row
figures: 38 looked for in their files at d7a0088, 0 missing
figures written in the NEWs: 0 unchecked
quoted phrases: 11 looked for, 0 missing
the document writes 31 of the figures, 0 missing
the decision entry: one block of 57 lines, 5 decisions, its three parts present, 5 of 5 replaced phrases quoted
figures written in the entry and the prose: 0 unchecked
placeholders: the decision is named 55 times as main's pairs name it and 22 times as the plan branch's do; others none
the document holds every OLD and NEW as a fenced block
38 pairs, 0 failing
~~~

## 8. What this document could not verify

- The owner's two answers and the coordinator's rulings are quoted from the coordinator's
  briefs. This document did not see either question or its record.
- No figure was measured again. Each is read from its file at `d7a0088`. The script checks that
  each stands there, and that no other decimal figure is written in a NEW, in the entry or in
  this document's prose. It cannot tell a right figure put in the wrong sentence.
- That more frame workers never lengthen the tick rests on two points on the M1, T₁ and T₄. No
  file holds a run on two or three workers, or on its efficiency cores.
- The laptop's load is unknown for two of its files, the physics phase's and the rollback's,
  which give no idle share.
- The desktop's CPU, its GPU and its second system are in no document. Whether it has eight
  performance cores is the owner's word of 14 November 2026.
- The milestone 2 plan is a draft on a branch, not approved, and will be re-based. Its pairs
  are checked at `b81f5c8` and nowhere else.
- The laptop's tick start after `timer-windows`, 533.0 and 502.9 µs, and the runner's 5.83 s
  are read from verification rows, not from checked-in result files.
- Whether the decision is D151 depends on what is applied before it.
- Every pair is applied in memory alone. No pair was applied to a file, on main or on the
  branch.
