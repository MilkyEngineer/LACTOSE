# Proposal: extract and the remote upload on the M1

Status: proposal, not applied. It answers what lane `measure-m2`'s first runs on the Apple M1, and
the three reviews of them, found against criteria 4 and 5 of milestone 2
(`docs/plans/milestone-2.md`, section 1). The owner rules on the remedy for each criterion; the two
questions are at the end. Every figure below is the lane's or a reviewer's, taken with other work
on the machine. None is a record (architecture section 20), and this proposal ran no measurement.
It changes no sentence of `docs/architecture.md`. Approved, it is applied as one decision entry,
the plan's and the verification rows' text below, and one implementation lane.

## What was measured

| | Criterion | Figure on the M1 | Where the time is |
|---|---|---|---|
| 4 | Extract's pass at 50,000 dirty rows, at most 1 ms | 1.72 ms on four workers, on the device and on null | The snapshot's fill: 1.08 ms at 10,000 presented rows. The ring alone is 0.65 ms |
| 5 | The remote upload of 5,000 entities at 144 Hz, at most 1 ms | 0.39 ms in one run and 1.83 ms in the next, the code unchanged | The core macOS starts the paced job on |

The two misses have nothing in common. The first is the engine's code. The second is where the
operating system runs a job that the test made light.

## Criterion 4: extract at most 1 ms for 50k dirty rows

**What the pass costs.**

- The ring's half forks over the chunks that hold changed rows (`crates/present-api/src/extract.rs`,
  `write_instances`). At 50,000 dirty rows it takes 0.65 ms on four workers and 27 ns a row on one,
  inside the 20 to 40 ns D21 assumed. Alone, it meets the criterion.
- The snapshot's half runs after it, on the pass's own thread (`extract.rs`, lines 268 to 279). At
  every pass that follows a tick, `Presented::fill` (lines 608 to 635) builds a list of every
  presented row for each copied column, `PresentSnapshot::fill` (`snapshot.rs`, line 481) sorts each
  list by key and copies the bytes, and `keys_of` sorts the keys once more. None of it depends on
  the dirty count.
- That half took 1.08 ms at the test's 10,000 presented rows, 0.06 ms at 1,000 and nothing at none.
  The pass on null was 1.75, 0.72 and 0.66 ms (`reviewer`'s probe).
- The sorts are the heaviest frames of it in `reviewer-gpu`'s sample of the pass: 136 samples in
  the columns' sort and 32 in `keys_of`, against 113 in `memmove`, part of which is the ring's, and
  98 in the ring's own rows.
- The same fill at the same 10,000 rows cost 0.31 to 0.33 ms in `present-impl`'s own test (row
  192, review round 2: on four workers 0.76 to 0.78 ms with the snapshot and 0.45 ms without).
  That scene spawns its rows in key order in one region (`crates/present-api/tests/cost.rs`,
  `scene`), so the sorts are handed sorted lists. `measure-m2`'s scene lays the presented rows over
  sixteen regions and four archetypes, as the plan's row asks, and the sorts do their work. By the
  two scenes' code, about 0.75 ms of the 1.08 is sorting, and the lists and the copy are about
  0.3 ms at 10,000 rows. That reading was not run here.
- The test adds 28 to 126 µs, a keeper thread inside the timed region. The lane's fix round removes
  it.
- 10,000 has no source. It came with `present-impl`'s cost test (the crate README's proof row
  "Cost", verification row 192). The plan asks for "a stated count" and states none.

**Is a serial rebuild of every presented row at every pass what D157 and D162 meant?** The copy is
D157's. The rebuild is not.

- D157 chose "presented rows, not instances", a whole state in exactly three buffers, and ruled out
  "a snapshot of differences". The buffer a pass takes is two publications old, so a pass that
  follows a tick must leave it whole. A copy of every presented row at such a pass is D157's
  design, and it was meant to be small: D157 kept instances out of the snapshot to rule out
  "copying every transform twice a tick".
- The lists and the sorts are `present-impl`'s first implementation. The crate README leaves the
  fill free: a snapshot is "always the whole state, never a difference, however the implementation
  fills it". Row 192 recorded the serial copy when the crate merged, at 0.3 ms: its scene's rows
  lay in key order, and the sorts had nothing to do.
- D162 meant neither. It counts on the result: its join margin is followed by "the passes, which
  criterion 4 holds to 1 ms at 50,000 dirty rows".
- Architecture section 5.3 says of the snapshot: "Its budget is measured in milestone 2, not
  assumed." This is that measurement.

**The remedies.**

| Remedy | What it does | What it rules out, or leaves |
|---|---|---|
| A. The fill keeps its order | Extract keeps the presented rows' order, each key with its chunk and row, from one pass to the next. A pass copies each row's components in that order and checks each place against the chunk's key as it copies. It sorts again, once, only when a place no longer holds its key or the count of presented rows changed. No sort and no list of rows where the presented rows are the previous pass's | Leaves a serial copy of every presented row at each pass after a tick: D157's cost, linear in the rows. Rules out nothing D157 decided |
| B. The buffers keep their bytes too | Each of the three buffers also keeps the mark it was last written at, and a pass rewrites only the rows stamped since | Cost follows the presented rows that changed. The worst case, every presented row changed, is A's copy, and a pass is budgeted by its worst case. Three marks and a rule for a stale buffer to prove |
| C. The fill is split across the pass's workers | One child of the pass's scope beside the ring's range makes the pass the longer of its halves, not their sum | The work stays, a worker leaves the ring's range, and the code runs on a second thread. At 10,000 rows the pass is still today's fill, 1.08 ms. Jobs cannot share one column's rows in safe Rust, so a fill forked by chunk is a larger change |
| D. A presented count with its reason | The scene states how many rows it presents, and why | Alone, it leaves a sort of every column at every pass, and a verdict that depends on the count chosen |
| E. The criterion restated | Criterion 4 read on the ring's part, the snapshot given a budget of its own | The plan's decision 5.2 rules out a criterion "read without the snapshot", and D162's margin counts on the whole pass |

**What a scene presents.**

- Milestone 2's scene presents no row. The client's seven rows hold a transform, a mesh and
  `sketch::orbit`, which is not extracted (`game/game-sim/src/sketch.rs`).
- Milestone 3 adds the lights of its tile-local lighting and skinned characters on terrain. Its
  500,000 instances are rows of a mesh and a transform, which go to the ring alone (D157). No
  document counts its lights; the one count a criterion gives is milestone 1's 200 characters
  (section 22).
- Milestone 4's client presents criterion 5's 5,000 remote entities. Each is a presented row of the
  replicated world's snapshot, with `RemoteSlot` and `MeshInstance` and no transform (the crate
  README, "Remote entities are not in it").

**Recommendation: A and D, at 5,000 presented rows, half of them moved.**

- 5,000 is criterion 5's count, the most presented rows any criterion of section 22 names. It is an
  upper anchor from the architecture's own figures, not a forecast. 1,000 was a probe's knob and
  10,000 a test's constant; neither has a reason.
- Half are moved because half the scene's rows are. With A the snapshot's part does not depend on
  it.
- A removes what the implementation added and keeps what D157 decided. It needs no state in the
  buffers, so nothing about the triple buffer changes.
- B is not built now. Its gain is the typical pass, not the worst one, and the counts above do not
  need it. It is the next step if A's part is over its target.
- Estimates, not measurements. Today's fill at 5,000 rows lies between the probe's 0.06 ms at
  1,000 and 1.08 ms at 10,000, about 0.3 to 0.55 ms, which puts the pass between 0.85 and 1.2 ms:
  on the line. A's copy is about row 192's 0.31 to 0.33 ms at 10,000 rows that lie together, so
  0.15 to 0.25 ms at 5,000 that do not, which puts the pass between 0.65 and 0.9 ms. Both are
  verification items below.

**The change.**

- *Crates.* `lactose-present-api` alone: `Presented::fill`, `Presented::changed`, `keys_of` and
  `ExtractState`'s private `presented` in `src/extract.rs`; `PresentSnapshot::fill`, which is
  `pub(crate)`, in `src/snapshot.rs`. The count changes in the two cost tests,
  `crates/present-api/tests/cost.rs` and `crates/app-client/tests/extract.rs`. `lactose-ecs` is
  untouched: the fill reads `World::chunks` and `ChunkRef::keys`, as it does today.
- *Contracts.* No public signature changes. The crate README's "Extract" gains what the snapshot's
  half costs, and its proof row "Cost" states 5,000. D157 gains an addendum at the lane's merge.
- *Whose.* A lane's, `snapshot-order`, not the architect's: every signature and every rule of the
  facade holds.
- *Size.* One small lane, about 300 lines with its test. `reviewer` alone reviews it, since the fill
  stays on the pass's thread. Five to seven agent runs.
- *What it must show.* A test that compares the buffer each pass wrote with `present-impl`'s fill of
  the same world, over random sequences of moves, spawns, despawns, extracted components added and
  removed, region loads and unloads, restores and `set_tick`, under the reader cadences of the
  triple buffer's test and under every layout capacity SOUR runs. A count, asserted, of the passes
  that sorted. The recorded test's hashes unchanged. In `extract-50k`, the snapshot's part at the
  plan's scene, with a target of 0.2 ms on the M1, and the pass at most 1 ms on the device, on the
  M1 in the quiet window and on the desktop.

**What would change it.**

- A quiet run of today's fill at 5,000 rows with the pass under 0.8 ms. Then D alone closes the
  criterion and the lane waits for the milestone that presents more. The run is `cargo xtask measure
  extract-50k` on `measure-m2`'s branch with the test's `PRESENTED` at 5,000, in a quiet window. It
  is the second option of the owner's question.
- Milestone 3's plan naming a scene that presents more than 5,000 rows. That plan then restates the
  row.

## Criterion 5: remote transform upload at most 1 ms for 5k entities at 144 Hz

**What was measured, and under what.**

- The frame's job is one `sample_batch`, the conversion to render space and the ring's write. With
  frames run back to back it takes 0.27 to 0.29 ms, and 96% or more of the jobs begin on
  performance cores.
- Paced at 144 Hz the job is 0.3 ms of work every 6.9 ms. Seven runs back to back gave medians from
  0.32 to 1.84 ms. In a slow run the job began on an efficiency core in 804 to 1,106 of 1,440
  frames, with that cluster at half to two thirds of its clock, and took about 1.4 to 1.9 ms there
  against 0.32 to 0.65 ms on a performance core.
- The load average does not say which mode a run falls into. The conversion, a loop over the job's
  own vectors, slowed 6.8 to 8.6 times, so the cause is the core and its clock and not the writer.
  P99 and row 102's retry rate follow the same split.
- The frame-critical class is already macOS's highest QoS class
  (`crates/core/src/thread_class.rs`, lines 203 to 211), and D17 records that "macOS offers only
  QoS, not pinning".
- With a core spinning beside it, or unpaced, `presentation`'s review read the fast mode in 7 runs
  of 7 (row 19). In this review one spinning thread did not hold it: 886 of 1,440 jobs still began
  on efficiency cores.
- No report names two conditions of every run. Each was started from a terminal, so the process had
  the `TASK_UNSPECIFIED` role, in which user-interactive QoS is clamped to priority 31 (verification
  row 78; `docs/results/spike-wake-apple-m1-2026-10-06.md`, "Task role"). Only `lactose-task`'s
  benchmark sets `TASK_FOREGROUND_APPLICATION`: nothing in `lactose-app`, `lactose-platform`,
  `cargo xtask measure` or a test does. And the test's frame holds nothing but the job. No run
  measured the job in the foreground role, or in a frame whose other workers were busy.
- The client has no 144 Hz frame at milestone 2. Its frame is paced by the timer root at the tick's
  60 Hz (D162, row 228). The pacing at 144 Hz is the test's.

**What the criterion means.** Architecture section 5.3 asks for the upload's "cost at 144 Hz for
several thousand entities". That is a share of a frame: of the 6.94 ms a 144 Hz frame has, what the
upload takes on the cores the frame's work runs on. To time it, the test makes a frame of nothing
else. macOS reads 0.3 ms of work every 6.9 ms as a light load and runs it where it is cheapest. The
frame that results still ends 5 ms early, so 1.83 ms is not what the upload takes from a frame that
needs its time. The criterion is read where the job ran at full clock.

It is not a promise that macOS gives a frame the performance cores. Whether a frame with real work
is kept there is the engine's question. No run so far answers it, and it is not criterion 5's.

**The remedies.**

| Remedy | What it does | What it rules out, or leaves |
|---|---|---|
| A. Read the figure at a stated placement | Criterion 5 is the median of the frames whose job began at full clock, by a calibration the test times at each job's start, with that band's share of the run. The figures over all frames and an unpaced control are recorded beside it. `measure-m2`'s fix round already adds the bands and the control | Rules out reading the scheduler's choice as the engine's cost. Leaves the placement unexplained until it is measured |
| B. Measure the placement first | Criterion 5 stays open until the job is measured in the foreground role and in a busy frame | A small lane and its run before milestone 2 can exit. Its result does not move the full-clock figure |
| C. Hold the performance cores | A means macOS gives: the table below | No means below is both measured and free. Each is engine code written before the cause is known |
| D. Restate for the M1 | 2 ms on the M1 over all frames, 1 ms elsewhere | Writes one mode of the scheduler into the engine's budget. A fast run then passes with a margin that means nothing, and the figure stops being comparable with the desktop's |

**The means macOS gives.**

| Means | `unsafe` | Evidence | Power | The rest of the frame |
|---|---|---|---|---|
| A higher QoS class | None | None exists: frame-critical is user-interactive already | None | None |
| The foreground task role | An FFI call, today in `lactose-task`'s benchmark alone (architecture section 21) | Lifts the clamp from priority 31 to 47; in the wake spike it removed the tail beyond P99.9. Its effect on placement is not measured | None of its own | Every class of the process gets its real priority, the network and present threads included |
| A parallel workgroup (`os_workgroup_parallel_create`, `os_workgroup_join`) | FFI in `lactose-task` or `lactose-core`; section 21's table amended | Public since macOS 11. It names the frame's workers as one workload and carries no deadline. Not measured | Unknown | Each worker joins once at start; nothing per frame |
| A work interval | Not available | The SDK's `os/workgroup_interval.h` declares start, update and finish and no creation call: "the only supported interval workgroups are for audio workloads" | None | None |
| The time-constraint policy | An FFI call | The wake spike: honoured, and "a thread spinning at 100% under it is demoted to priority 0" | Unknown | It fits one thread that waits for a deadline. The frame's workers run jobs for milliseconds and rescan for about 50 µs before they park |
| A spin that holds the clock | None | Fast in `presentation`'s review (row 19); not held in this one, 886 of 1,440 jobs on efficiency cores | A core at all times; the heat and the battery are not measured (`powermetrics` needs sudo) | One performance core less for the frame. D113 rejected the warm sleeper, and Goal 1 says utilisation is never a target |
| The system's Game Mode | None: packaging | Not verified here. It cannot apply to a test binary | The system's | The system's |

**Recommendation: A, and the measurement of B owed before milestone 3.**

- The figure the criterion wants exists in the file the lane's fix round writes. On the reviews'
  figures it is 0.32 to 0.65 ms.
- The placement is a finding about the engine on macOS, and it gets its own verification row and a
  measurement that turns it into a decision. A phase of `lactose-task`'s benchmark, which may set
  the role, runs a job of fixed safe work at 144 Hz from the timer root in each role, alone and
  after a stand-in that keeps the job's other workers busy.
- Its three outcomes are three decisions. Fast in the foreground role: the clamp was the cause, the
  measurement tests take the role, and criterion 5 is read over all frames again. Slow alone and
  fast in the busy frame: macOS spends a light frame's slack, and nothing changes. Slow in the busy
  frame: the planner proposes a placement means before milestone 3 fixes a frame budget on the M1,
  the parallel workgroup first.
- No means is built now. The only evidence for any of them is a spin that worked once.
- P99 is read the same way. The full-clock band's P99 over 2 ms is a finding about the frame's
  tail; the P99 over all frames is recorded with the placement.

**What the desktop and the Windows laptop must show.**

- *The desktop*, session C: the same file. Criterion 5 there is the full-clock band's median at
  most 1 ms, with the band's share. On a part with no efficiency cores the band should hold nearly
  every frame. A share under nine tenths there is that machine's clock ramp from idle, and goes
  into the placement row with its figures.
- *The laptop*, labelled, a hybrid part under Windows 11: Windows' share of frames at full clock,
  with frame-critical at above-normal priority and no opt-out of power throttling (D17's addendum).
  It proves nothing for the criterion. It is the first evidence for row 233's second question.
- For criterion 4 both show the pass at the plan's scene. The snapshot's part is host memory, so
  what lane `snapshot-order` saves carries over. The ring's part does not: on a discrete GPU it is
  write-combined memory (the plan's decision 5.2), and it is the desktop's to show.

**What would change it.**

- The quiet window's full-clock band over 1 ms. Then the miss is the engine's, and the batch call
  is where a lane looks first.
- A quiet window whose runs hold under a tenth of their frames in the full-clock band. Then no run
  judges, and B's measurement comes before the criterion is read.

## The rows (exact text)

Each OLD is one whole line of the named file at `12154bb`, occurring once in it. The format is the
one `measure-m2`'s merge text uses. A NEW of several lines replaces the one.

**The dependency.** `measure-m2`'s merge rewrites verification lines 16, 19 and 102 and adds two
rows after line 245. These rows are applied after that merge, and its text is not edited here.

- Rows 16, 19 and 102: each change below adds text after one span. The span occurs once in the
  file at `12154bb`, and once in the lane's NEW for that row as its merge text stood after round
  three. After the lane's merge the same text is added after the same span of the merged line. The
  spans are: "result file `extract-50k`" in row 16's second cell; "result file `remote-upload-5k`"
  in row 19's second cell; and row 102's sentence that ends "are in result file `remote-upload-5k`
  (D153).".
- The two new rows go after the table's last row, which after the lane's merge is its row on GPU
  timestamps.
- The plan's lines 59, 60, 337, 338, 397 and 408 are not in the lane's merge text and apply as
  written.
- `D16x` stands for the decision's number, which the coordinator allocates. Row 19 names the first
  new row as "D16x's placement row".

~~~text
=== docs/plans/milestone-2.md line 59: criterion 4's row: the scene's presented rows, the snapshot, the lane
OLD:
| 4 | Extract at most 1 ms for 50k dirty rows | A cost test: 100,000 rows with a `Transform`, 50,000 of them moved every tick, extract's pass alone, the median of 60 passes, writing the device's ring (decision 5.2) | A quiet M1; the desktop | `extract-50k` | `present-impl`, `renderer-impl`, `measure-m2` |
NEW:
| 4 | Extract at most 1 ms for 50k dirty rows | A cost test: 100,000 rows with a `Transform`, 5,000 of them presented rows, 50,000 moved every tick and half the presented rows among them, extract's pass alone, the median of 60 passes, writing the device's ring and the snapshot (decision 5.2, D16x) | A quiet M1; the desktop | `extract-50k` | `present-impl`, `renderer-impl`, `measure-m2`, `snapshot-order` |

=== docs/plans/milestone-2.md line 60: criterion 5's row: read at full clock, with the share
OLD:
| 5 | Remote transform upload at most 1 ms for 5k entities at 144 Hz | A cost test: 5,000 slots of the interpolation buffer written at 30 Hz, sampled in one batch and written to the staging ring every frame, paced at 144 Hz for ten seconds, the median frame (decision 5.3) | A quiet M1; the desktop | `remote-upload-5k` | `presentation`, `measure-m2` |
NEW:
| 5 | Remote transform upload at most 1 ms for 5k entities at 144 Hz | A cost test: 5,000 slots of the interpolation buffer written at 30 Hz, sampled in one batch and written to the staging ring every frame, paced at 144 Hz for ten seconds, the median of the frames whose job began at full clock, with their share of the run (decision 5.3, D16x) | A quiet M1; the desktop | `remote-upload-5k` | `presentation`, `measure-m2` |

=== docs/plans/milestone-2.md line 337: row `extract-50k`: 5,000 presented rows, half of them moved
OLD:
| `extract-50k` | 100,000 rows with a `Transform` and a mesh component, in four archetypes and sixteen regions, as milestone 1's cost scene is laid out, and a stated count of presented rows besides, each with an `extracted` component other than the mesh and the transform, so the snapshot's part measures a snapshot that holds something: rows that are only a mesh and a transform go to the ring alone (D157). A kernel moves 50,000 of them every tick. One tick, then `Simulation::run_passes` with extract alone registered, sixty times, on N frame workers, N the performance cores up to eight. The ring is the device's mapped buffer, and the run is repeated on null. The file gives the pass's median, the ring's part and the snapshot's part, the cost per dirty row on one worker, and the ring's memory type and flags. Each pass ends with an empty submit that signals its timeline value, so the ring's regions come free; a wait for a free region is left out of the median and reported | Criterion 4: the median on the device at most 1 ms, on the M1 and on the desktop |
NEW:
| `extract-50k` | 100,000 rows with a `Transform` and a mesh component, in four archetypes and sixteen regions, as milestone 1's cost scene is laid out, 5,000 of them presented rows, each with an `extracted` component other than the mesh and the transform, so the snapshot's part measures a snapshot that holds something: rows that are only a mesh and a transform go to the ring alone (D157). 5,000 is criterion 5's count, the most presented rows a criterion of section 22 names (D16x). A kernel moves 50,000 rows every tick, 2,500 of the presented rows among them. One tick, then `Simulation::run_passes` with extract alone registered, sixty times, on N frame workers, N the performance cores up to eight. The ring is the device's mapped buffer, and the run is repeated on null. The file gives the pass's median, the ring's part and the snapshot's part, the cost per dirty row on one worker, and the ring's memory type and flags. Each pass ends with an empty submit that signals its timeline value, so the ring's regions come free; a wait for a free region is left out of the median and reported | Criterion 4: the median on the device at most 1 ms, on the M1 and on the desktop |

=== docs/plans/milestone-2.md line 338: row `remote-upload-5k`: the clock bands, the control, the criterion's band
OLD:
| `remote-upload-5k` | 5,000 slots of `lactose-net-host`'s interpolation buffer. One writer thread pushes a state to every slot at 30 Hz. The presentation schedule's remote upload runs every frame, paced at 144 Hz by a timer for ten seconds: one `sample_batch`, the conversion to render space, the write to the device's ring. Each frame ends with an empty submit that signals its timeline value, so the ring's regions come free as they do in a frame. The file gives the median, P99 and worst frame, the reader's retries, the buffer's bytes and the ring's memory type | Criterion 5: the median at most 1 ms, on the M1 and on the desktop. P99 is recorded; over 2 ms it is a finding |
NEW:
| `remote-upload-5k` | 5,000 slots of `lactose-net-host`'s interpolation buffer. One writer thread pushes a state to every slot at 30 Hz. The presentation schedule's remote upload runs every frame, paced at 144 Hz by a timer for ten seconds: one `sample_batch`, the conversion to render space, the write to the device's ring. Each frame ends with an empty submit that signals its timeline value, so the ring's regions come free as they do in a frame. The file gives the median, P99 and worst frame over all frames and for each clock band, each band's share of the frames, the same figures of an unpaced control run, the reader's retries, the buffer's bytes and the ring's memory type. A band is the frames whose calibration, a fixed chain of safe code timed at the job's start, read on one side of a threshold the file states with its reason | Criterion 5: the full-clock band's median at most 1 ms, on the M1 and on the desktop, with the band's share beside it; a run with under a tenth of its frames in that band judges nothing and is made again. P99 is recorded for each band and for all frames; the full-clock band's over 2 ms is a finding. Where the frames ran is a verification row's, not the criterion's (D16x) |

=== docs/plans/milestone-2.md line 397: decision 5.2's last line: what the snapshot holds
OLD:
the file says what the snapshot holds.
NEW:
the file says what the snapshot holds. It holds 5,000 of the scene's rows, half of them moved, and
its part of the pass is one copy of them, in an order kept from the pass before (D16x).

=== docs/plans/milestone-2.md line 408: decision 5.3's last line: the finding went to the planner
OLD:
goes to the planner as a finding about the frame's tail.
NEW:
goes to the planner as a finding about the frame's tail. It went, and the median with it (D16x): on
the M1 both follow the core macOS starts the frame's job on, so the criterion reads the frames whose
job began at full clock, with their share of the run.

=== docs/verification.md line 16: verification row 16: the scene, and when criterion 4 is read
OLD:
| Extract cost | Per-dirty-row extract cost; the workload and machines of D153, result file `extract-50k` | 2 |
NEW:
| Extract cost | Per-dirty-row extract cost; the workload and machines of D153, result file `extract-50k`. The scene presents 5,000 of its rows, half of them moved: criterion 5's count, the most presented rows a criterion of section 22 names (D16x). Criterion 4 is read at that scene once lane `snapshot-order` has merged, whose fill keeps the presented rows' order between passes and sorts nothing while they are the same rows; a figure taken at 10,000 presented rows, or on the fill that sorts every column at every pass, is steering | 2 |

=== docs/verification.md line 19: verification row 19: how criterion 5 is read
OLD:
| Remote transform upload | Cost per frame for 5k interpolated entities at 144 Hz; the workload of D153, result file `remote-upload-5k` | 2. **Steering only** (branch `presentation`, `crates/presentation/tests/cost.rs`, ignored, run in release on the Apple M1; not the result file, which is `measure-m2`'s): 5,000 remote rows of a client's replicated world, the schedule's frame (one `sample_batch` call, the merge with the meshes and the conversion) and the upload from the frame's job to a closure that counts the rows, paced at 144 Hz for 10 s with a plain thread pushing a state into every slot at 30 Hz. The sample's median varied between runs, from 0.26 to 1.8 ms, and the cause is not measured: paced at 144 Hz with no other core busy it was 1.8 ms (6 of 6 runs, `reviewer-concurrency`); with a core spinning beside it, or unpaced, 0.26 to 0.38 ms (7 of 7); in the branch's last runs, below, 0.39 to 0.45 ms. The batch call's figure is a second `sample_batch` call on the same slots made after the frame, not a share of the frame's time. The branch's last runs, with the frame handed the acquired pair: two runs as the test stands, at 1-minute load averages 3.32 then 3.26 (other agents' work on the machine): the frame's median 452.9 and 452.8 µs, 99th percentile 592.1 and 897.2 µs; of it the sample's median 449.8 and 449.7 µs and the upload's 3.4 and 3.8 µs; the second batch call's median 307.3 and 307.1 µs. Two runs with one core spinning beside it, at 3.21 then 4.11 (4.38 after): the frame's median 400.6 and 390.5 µs, 99th percentile 2,526.4 and 832.9 µs; the sample's median 394.3 and 385.9 µs; the second batch call's median 268.2 and 266.3 µs. The ring's copy is not in it while `RemoteWriter::write` is the facade's. `measure-m2` makes the recorded figure: it runs the writer under `ThreadClass::Network`, times the batch call inside the frame, and reports the reader's retry rate (row 102) |
NEW:
| Remote transform upload | Cost per frame for 5k interpolated entities at 144 Hz; the workload of D153, result file `remote-upload-5k`. Criterion 5 reads the frames whose job began at full clock, by the file's calibration, with their share of the run; the figures over all frames are recorded beside them, and where macOS starts the job is D16x's placement row, not the criterion's | 2. **Steering only** (branch `presentation`, `crates/presentation/tests/cost.rs`, ignored, run in release on the Apple M1; not the result file, which is `measure-m2`'s): 5,000 remote rows of a client's replicated world, the schedule's frame (one `sample_batch` call, the merge with the meshes and the conversion) and the upload from the frame's job to a closure that counts the rows, paced at 144 Hz for 10 s with a plain thread pushing a state into every slot at 30 Hz. The sample's median varied between runs, from 0.26 to 1.8 ms, and the cause is not measured: paced at 144 Hz with no other core busy it was 1.8 ms (6 of 6 runs, `reviewer-concurrency`); with a core spinning beside it, or unpaced, 0.26 to 0.38 ms (7 of 7); in the branch's last runs, below, 0.39 to 0.45 ms. The batch call's figure is a second `sample_batch` call on the same slots made after the frame, not a share of the frame's time. The branch's last runs, with the frame handed the acquired pair: two runs as the test stands, at 1-minute load averages 3.32 then 3.26 (other agents' work on the machine): the frame's median 452.9 and 452.8 µs, 99th percentile 592.1 and 897.2 µs; of it the sample's median 449.8 and 449.7 µs and the upload's 3.4 and 3.8 µs; the second batch call's median 307.3 and 307.1 µs. Two runs with one core spinning beside it, at 3.21 then 4.11 (4.38 after): the frame's median 400.6 and 390.5 µs, 99th percentile 2,526.4 and 832.9 µs; the sample's median 394.3 and 385.9 µs; the second batch call's median 268.2 and 266.3 µs. The ring's copy is not in it while `RemoteWriter::write` is the facade's. `measure-m2` makes the recorded figure: it runs the writer under `ThreadClass::Network`, times the batch call inside the frame, and reports the reader's retry rate (row 102) |

=== docs/verification.md line 102: verification row 102: the retry rate follows the placement
OLD:
| Interpolation slot footprint (about 530 bytes; 5k entities about 2.6 MB) against the remote-upload budget, the reader's retry rate under one writer at 30 Hz with the stated orderings, and a loom model of the sequence lock. The reader's retry rate under a 30 Hz writer and the footprint are in result file `remote-upload-5k` (D153). The loom model: **Done** (lane `interp-loom`, head 09449ad): the slot's lock is `crates/net-host/src/seqlock.rs`, generic over a `Memory` trait and the ring's shape; the buffer runs it over the standard atomics at eight states of eight words, and `src/models.rs` (feature `loom`) runs the same `reset`, `push` and `read` over loom's atomics at two states of two words, no two pushed states sharing a word. Three models, each a writer on the main thread against one spawned reader reading twice, at preemption bound 2 (bounded, not exhaustive), every read equal to the slot's contents after one write and none older than the read before it: `the_ring_wraps_under_a_reader` (a full ring pushed twice past its oldest; 242,926 executions), `the_generation_changes_under_a_reader` (a release, a reallocation and a push; 139,452) and `the_ring_grows_at_its_front_under_a_reader` (a push at the ring's front, then one past it; 129,052), each made to show one kind of mixed read: state words, `generation`, and `count` against the states. At bound 3 all three pass too (1,246,450, 735,412 and 606,050 executions). The lock's four orderings stronger than `Relaxed`, each weakened in turn, make all three models find a torn read: the writer's `fence(Release)` (`*_without_the_writers_release_fence`), the even sequence's `Release` (`*_without_the_even_sequences_release`), the reader's first `Acquire` (`*_without_the_readers_first_acquire`) and its `fence(Acquire)` (`*_without_the_readers_acquire_fence`); none is stronger than it needs to be, and `seqlock.rs` states which release pairs with which acquire. `count` stored after the even sequence store (mutant P10 of the concurrency review) passes the first two models, the hardware stress test and Miri's, and fails the third at once. A unit test without loom (`each_ordering_stronger_than_relaxed_has_one_site`) holds each ordering to one site, which is how the weakened models relax one at a time: it counts the calls one thread makes and the sites in the code, so a second site on a reader's retry path fails it too. Residuals: loom 0.7.2 backtracks only against an atomic's last access, so the writer is the main thread: with the reader on the main thread the ring-wrap model runs one execution and passes with each ordering weakened, and the models' pass is evidence through the weakened models that fail them; the count's clamp is unreachable, since every stored count is at most the ring's length; the 32-bit sequence admits a torn read only if a multiple of 2^31 writes fall between a reader's two loads; loom does not model load buffering (readers store nothing, so no load-store cycle exists) and keeps seven stores per atomic: the ring and front-growth models' readers may read five sequence values, a margin of two, the generation model's seven, no margin, so a further write there needs a model of its own; one reader, since a second only loads and a model of two exceeded loom's branch limit; the shipped `Native` orderings stay under Miri's 150-push stress test, which in the branch's review read torn samples with each of the four weakened. The nightly loom job runs it: `cargo test -p lactose-net-host --features loom --release --locked`, 21 s on the M1 | 2 |
NEW:
| Interpolation slot footprint (about 530 bytes; 5k entities about 2.6 MB) against the remote-upload budget, the reader's retry rate under one writer at 30 Hz with the stated orderings, and a loom model of the sequence lock. The reader's retry rate under a 30 Hz writer and the footprint are in result file `remote-upload-5k` (D153). The rate follows the cores the reader and the writer run on, from 8.6e-5 unpaced to 1.9e-2 paced on slow cores in the probes of `measure-m2`'s review, so the file gives it with each clock band's share of the frames beside it, and no figure of it is a criterion (D16x). The loom model: **Done** (lane `interp-loom`, head 09449ad): the slot's lock is `crates/net-host/src/seqlock.rs`, generic over a `Memory` trait and the ring's shape; the buffer runs it over the standard atomics at eight states of eight words, and `src/models.rs` (feature `loom`) runs the same `reset`, `push` and `read` over loom's atomics at two states of two words, no two pushed states sharing a word. Three models, each a writer on the main thread against one spawned reader reading twice, at preemption bound 2 (bounded, not exhaustive), every read equal to the slot's contents after one write and none older than the read before it: `the_ring_wraps_under_a_reader` (a full ring pushed twice past its oldest; 242,926 executions), `the_generation_changes_under_a_reader` (a release, a reallocation and a push; 139,452) and `the_ring_grows_at_its_front_under_a_reader` (a push at the ring's front, then one past it; 129,052), each made to show one kind of mixed read: state words, `generation`, and `count` against the states. At bound 3 all three pass too (1,246,450, 735,412 and 606,050 executions). The lock's four orderings stronger than `Relaxed`, each weakened in turn, make all three models find a torn read: the writer's `fence(Release)` (`*_without_the_writers_release_fence`), the even sequence's `Release` (`*_without_the_even_sequences_release`), the reader's first `Acquire` (`*_without_the_readers_first_acquire`) and its `fence(Acquire)` (`*_without_the_readers_acquire_fence`); none is stronger than it needs to be, and `seqlock.rs` states which release pairs with which acquire. `count` stored after the even sequence store (mutant P10 of the concurrency review) passes the first two models, the hardware stress test and Miri's, and fails the third at once. A unit test without loom (`each_ordering_stronger_than_relaxed_has_one_site`) holds each ordering to one site, which is how the weakened models relax one at a time: it counts the calls one thread makes and the sites in the code, so a second site on a reader's retry path fails it too. Residuals: loom 0.7.2 backtracks only against an atomic's last access, so the writer is the main thread: with the reader on the main thread the ring-wrap model runs one execution and passes with each ordering weakened, and the models' pass is evidence through the weakened models that fail them; the count's clamp is unreachable, since every stored count is at most the ring's length; the 32-bit sequence admits a torn read only if a multiple of 2^31 writes fall between a reader's two loads; loom does not model load buffering (readers store nothing, so no load-store cycle exists) and keeps seven stores per atomic: the ring and front-growth models' readers may read five sequence values, a margin of two, the generation model's seven, no margin, so a further write there needs a model of its own; one reader, since a second only loads and a model of two exceeded loom's branch limit; the shipped `Native` orderings stay under Miri's 150-push stress test, which in the branch's review read torn samples with each of the four weakened. The nightly loom job runs it: `cargo test -p lactose-net-host --features loom --release --locked`, 21 s on the M1 | 2 |

=== docs/verification.md line 245: two rows after the table's last row
OLD:
| What lane `frame-impl`'s runs on the M1 under synchronisation validation do not show (D162, row 230). The held frames, the dropped ones and the calls that made no submit came from a probe build of a scratch copy, whose present calls each slept 40 ms: the binary as it ships, paced for 20 s, held none of 1,201 frames under KosmicKrisp and queued 1,196 of 1,201 under MoltenVK. The probe's torn frames are torn after the renderer's prepare and before the record, so none wrote its target; a torn frame whose submit wrote the target, and one torn at `submit`, were not in a run on a device. Lavapipe's one queue did not run here: KosmicKrisp's shared queue stood in for it. And, as row 230 says, the layer does not report a re-take whose barrier names too narrow a source stage, so these runs are evidence of the write after a write alone | 2 |
NEW:
| What lane `frame-impl`'s runs on the M1 under synchronisation validation do not show (D162, row 230). The held frames, the dropped ones and the calls that made no submit came from a probe build of a scratch copy, whose present calls each slept 40 ms: the binary as it ships, paced for 20 s, held none of 1,201 frames under KosmicKrisp and queued 1,196 of 1,201 under MoltenVK. The probe's torn frames are torn after the renderer's prepare and before the record, so none wrote its target; a torn frame whose submit wrote the target, and one torn at `submit`, were not in a run on a device. Lavapipe's one queue did not run here: KosmicKrisp's shared queue stood in for it. And, as row 230 says, the layer does not report a re-take whose barrier names too narrow a source stage, so these runs are evidence of the write after a write alone | 2 |
| Where macOS starts a frame-critical worker's paced job (D16x's placement row). `remote-upload-5k`'s job is 0.3 ms of work every 6.9 ms. In the runs of lane `measure-m2` and its reviews on the M1 under macOS 27.2.1 it began on an efficiency core in more than half of a slow run's frames, 804 to 1,106 of 1,440, at half to two thirds of that cluster's clock, and took about 1.4 to 1.9 ms there against 0.32 to 0.65 ms on a performance core; seven runs back to back gave medians from 0.32 to 1.84 ms, and the load average did not say which; unpaced, 96% or more of the jobs began on performance cores, at 0.27 to 0.29 ms. With a core spinning beside it, or unpaced, `presentation`'s review read the fast mode in 7 runs of 7 (row 19); in this review one spinning thread did not hold it (886 of 1,440 jobs on efficiency cores). Every run was started from a terminal, so in the `TASK_UNSPECIFIED` role, where user-interactive QoS is clamped to priority 31 (row 78), and its frame held nothing else. Not measured: the job in the `TASK_FOREGROUND_APPLICATION` role; the job in a frame whose other workers are busy; the role a client started as an application bundle is given (`docs/results/spike-wake-apple-m1-2026-10-06.md`, "Not tested"). Owed: a phase of `lactose-task`'s benchmark, which sets the role already (architecture section 21): a 144 Hz timer root runs a job of fixed safe work, 0.3 ms at full clock, through `run_job`, in each role, alone and after a stand-in that keeps the job's other workers busy for 3 ms, 1,440 frames each, and gives the share of jobs that began at full clock and the job's median. It decides. Under a tenth of the jobs slow in the foreground role, alone: the clamp was the cause, the tests of `cargo xtask measure` take the role through a safe call of a section 21 crate, which amends that section's table, and criterion 5 is read over all frames again. Slow alone and not after the stand-in: macOS spends a light frame's slack, and nothing changes. Slow after the stand-in: the planner proposes a placement means before milestone 3 fixes a frame budget on the M1, a parallel workgroup first (`os_workgroup_parallel_create`; the SDK's `os/workgroup_interval.h` gives a frame no work interval). The laptop's `remote-upload-5k` gives Windows' share of frames at full clock beside it, which is evidence for row 233 | 3 |
| The snapshot's fill (D16x, lane `snapshot-order`). `present-impl`'s fill builds a list of every presented row for each copied column, sorts each list by key, copies the bytes and sorts the keys once more, at every pass after a tick: 1.08 ms of a 1.72 ms pass at 10,000 presented rows in `measure-m2`'s runs, whatever the dirty count. The sorts are its heaviest frames in `reviewer-gpu`'s sample (168 samples against 113 of `memmove`, part of which is the ring's), and where the presented rows lie in key order, as `present-impl`'s cost scene spawns them, the same fill at the same count cost 0.31 to 0.33 ms (row 192, review round 2). The lane keeps the presented rows' order, each key with its chunk and row, from one pass to the next, checks each place against the chunk's key as it copies, and sorts again only when a place no longer holds its key or the count of presented rows changed; a pass whose presented rows are the previous pass's then sorts nothing, builds no list of rows and copies each row's components once. Every buffer still holds the whole state (D157). Owed by the lane: a test that compares the buffer each pass wrote with `present-impl`'s fill of the same world, over random sequences of moves, spawns, despawns, extracted components added and removed, region loads and unloads, restores and `set_tick`, under the reader cadences of the triple buffer's test and every layout capacity SOUR runs; a count, asserted, of the passes that sorted; the recorded test's hashes unchanged; and in `extract-50k` the snapshot's part at the plan's scene. The lane's target for that part is 0.2 ms on the M1, what the ring's part leaves of 1 ms with a margin; the criterion is the pass's. Estimates, not measurements: today's fill at 5,000 presented rows costs between the 60.6 µs the review's probe read at 1,000 and the 1,087.7 µs it read at 10,000, about 0.3 to 0.55 ms, which puts the pass between 0.85 and 1.2 ms; the copy without the sorts costs about row 192's 0.31 to 0.33 ms at 10,000 rows that lie together, so 0.15 to 0.25 ms at 5,000 that do not. If the part is over the target, the next step is to rewrite in each buffer only the rows stamped since that buffer was last written, which the planner sizes then | 2 |
~~~

## Decision entry (exact text; number at merge)

**D16x. Criteria 4 and 5 of milestone 2 on the M1: the snapshot's fill keeps its order, the cost
scene presents 5,000 rows, and the remote upload is read at full clock** (§5.1, §5.3, §22,
`lactose-present-api`, `docs/plans/milestone-2.md`)
- Why: lane `measure-m2`'s first runs on the M1 put both criteria over 1 ms, and its three reviews
  found one cause in the engine and one in the operating system. Extract's pass took a median of
  1.72 ms on four workers, on the device and on null: 1.08 ms of it the snapshot's fill at 10,000
  presented rows, a count no document gives a reason for, and 0.65 ms the ring at 50,000 dirty
  rows, 27 ns a row on one worker, inside D21's 20 to 40. The remote upload's median was 0.39 ms in
  one run and 1.83 ms in the next with the code unchanged: it follows the core macOS starts the
  paced job on.
  - *A copy of the presented rows at every pass after a tick is D157's; the sorts are not.* D157
    chose a whole state in three buffers and ruled out a snapshot of differences, so a pass that
    follows a tick leaves its buffer whole. `present-impl`'s fill also builds a list of rows for
    each column and sorts it, and sorts the keys again; where the rows lie in key order the same
    fill at the same count costs 0.3 ms (verification row 192). The fill keeps the rows' order
    from one pass to the next, checks each place against its chunk's key as it copies, and sorts
    again only when a place no longer holds its key or the count of presented rows changed. No
    public signature changes. The lane is `snapshot-order`.
  - *The cost scene presents 5,000 rows, half of them moved.* It is criterion 5's count, the most
    presented rows a criterion of section 22 names: a client's remote entities are presented rows
    of its replicated world. Milestone 2's scene presents none, and the only other count a
    criterion gives is milestone 1's 200 characters.
  - *Criterion 4 keeps its 1 ms and its snapshot.* D162's join margin counts on the passes within
    it, and the plan's decision 5.2 rules out a criterion read without the snapshot.
  - *Criterion 5 is a cost, read at full clock.* It is the share of a 144 Hz frame the upload takes
    on the cores the frame's work runs on. The test's frame holds nothing else, 0.3 ms of work
    every 6.9 ms, and macOS started it on an efficiency core at a reduced clock in more than half
    of a slow run's frames; such a frame still ends 5 ms early. The criterion reads the median of
    the frames whose job began at full clock, by a calibration the test times at each job's start,
    with that band's share of the run. The figures over all frames are recorded.
  - *Where a frame runs is measured before the engine acts.* Every run was made in the
    `TASK_UNSPECIFIED` role, where user-interactive QoS is clamped to priority 31, and in a frame
    with nothing else in it. A phase of `lactose-task`'s benchmark measures the job in each role,
    alone and in a busy frame, before milestone 3 fixes a frame budget on the M1 (verification).
  - Rules out: criterion 4 read on the ring alone, or at a presented count chosen after its figure;
    a snapshot of differences, which D157 ruled out and this keeps out; a spin that holds the
    clock, which costs a core at all times and did not hold the mode in this review's probe; the
    time-constraint policy on the frame's workers, under which a thread that spins is demoted
    (`docs/results/spike-wake-apple-m1-2026-10-06.md`); a work interval for the frame, which the
    SDK gives to audio alone; a criterion restated for the M1 before the placement is measured in
    the foreground role.
- Driven by: lane `measure-m2`'s runs, and the three reports of their first review, `reviewer`'s,
  `reviewer-concurrency`'s and `reviewer-gpu`'s; `docs/proposals/extract-and-upload-on-the-m1.md`;
  the owner's ruling.
- Replaced: the 10,000 presented rows of `present-impl`'s cost test (verification row 192, the
  crate README's proof row "Cost"); in the plan's decision 5.3, the median over all frames as the
  figure criterion 5 reads.

No sentence of `docs/architecture.md` changes. Section 5.1's "They run at user-interactive QoS" and
section 2's "Frame-critical workers on performance cores" stand until the placement row's
measurement. What they say for macOS is that row's proposal's, or revision 3.5's, which also takes
the snapshot's measured budget into section 5.3 (the plan's section 8).

## The owner's questions

As `AskUserQuestion` input, the recommended option first.

~~~json
{
  "questions": [
    {
      "question": "Criterion 4 (extract at most 1 ms for 50k dirty rows): the pass is 1.72 ms on the M1, and 1.08 ms of it is the snapshot's fill at 10,000 presented rows, a count nothing justifies. Which remedy?",
      "header": "Criterion 4",
      "multiSelect": false,
      "options": [
        {
          "label": "Keep the order, 5,000 rows (Recommended)",
          "description": "Lane `snapshot-order` in `lactose-present-api`: the fill stops sorting every column at every pass. No public signature changes; five to seven agent runs. The cost scene presents 5,000 rows, criterion 5's count, half of them moved. Criterion 4 is read in the quiet window after the lane merges; the pass is estimated at 0.65 to 0.9 ms."
        },
        {
          "label": "5,000 rows, measure first",
          "description": "No code now. The scene's count becomes 5,000 and the quiet window reads today's fill, estimated at 0.85 to 1.2 ms for the pass. A miss then costs the lane and a second quiet window."
        },
        {
          "label": "Rewrite only changed rows",
          "description": "The lane also keeps each buffer's bytes and rewrites only the rows stamped since that buffer was last written. The cost follows change, but the worst case is the same copy, and each of the three buffers needs its own mark: more to prove, about two more agent runs."
        },
        {
          "label": "Restate the criterion",
          "description": "Read criterion 4 on the ring's part alone (0.65 ms) and give the snapshot a budget of its own. Contradicts the plan's decision 5.2, and D162's join margin counts on the whole pass."
        }
      ]
    },
    {
      "question": "Criterion 5 (remote upload at most 1 ms for 5k entities at 144 Hz): the median is 0.39 ms or 1.83 ms according to whether macOS starts the paced job on a performance core or on an efficiency core at a reduced clock. Which remedy?",
      "header": "Criterion 5",
      "multiSelect": false,
      "options": [
        {
          "label": "Read at full clock (Recommended)",
          "description": "Criterion 5 is the median of the frames whose job began at full clock, with their share of the run; the figures over all frames are recorded. No engine change in milestone 2. Where macOS runs a frame becomes a verification row, measured by a phase of the executor's benchmark before milestone 3 (about five agent runs then)."
        },
        {
          "label": "Measure placement first",
          "description": "Criterion 5 stays open. The benchmark phase runs now, in the foreground task role and in a busy frame (about five agent runs), and the owner rules on its result. Milestone 2's exit waits for it."
        },
        {
          "label": "Hold the cores now",
          "description": "Spike a parallel workgroup and the foreground role in `lactose-task`, then read criterion 5 over all frames. `unsafe` FFI and an amendment to architecture section 21; a spike and a lane, twelve or more agent runs; the effect and the power cost are not measured."
        },
        {
          "label": "Restate for the M1",
          "description": "Criterion 5 becomes 2 ms on the M1 over all frames and stays 1 ms elsewhere. Writes one mode of the scheduler into the engine's budget, and the M1's figure stops being comparable with the desktop's."
        }
      ]
    }
  ]
}
~~~

## Cost and order

| Step | Who | Agent runs |
|---|---|---|
| `measure-m2`'s fix round and merge: the clock bands, the control, the count stated | The lane, as briefed | Already counted |
| This proposal's review and answer | `reviewer`, `planner` | 2 |
| The decision, the plan's lines and the rows, applied after `measure-m2`'s merge | The coordinator | None |
| `snapshot-order`: the fill, its tests, the count in the two cost tests | A lane from main, `reviewer`, `reviewer-diff`, `merger` | 5 to 7 |
| The measurement commit's quiet window, the laptop's session and session C | As the plan has them | Already counted |
| The placement phase of `lactose-task`'s benchmark | A small lane, before milestone 3's plan | 5 |

The quiet window comes after `snapshot-order` merges. Nothing here adds a quiet window.

## What this proposal could not verify

- Any figure. It ran no measurement: a lane was measuring on the machine. Every figure is from the
  lane's reports and the three review reports, and none was taken on a quiet machine.
- Today's fill at 5,000 presented rows. It is interpolated between two probe readings.
- The cost of the copy without the sorts. It is inferred from row 192's figures for a scene whose
  rows lie in key order, and from sample counts taken at a load average of 35. That the two scenes'
  orders explain 0.3 ms against 1.08 ms is read from their code, not run.
- That the foreground role changes where macOS starts the job, and that a busy frame holds the
  performance cores. Neither was run. They are the placement row's measurement.
- The role a client started as an application bundle is given. The wake spike lists it as not
  tested.
- Game Mode, and the effect of a parallel workgroup on placement. The workgroup calls were read in
  the macOS 27.2 SDK's headers on the development Mac; nothing was linked or run.
- The lane's fix round. The spans of rows 16, 19 and 102 were checked against its merge text as it
  stood after round three, and the clock bands are described as its fix brief asks for them. If the
  round moves a span or names the bands otherwise, the rows' text follows it.
- The desktop. Its CPU is unknown until its first sitting, so "no efficiency cores" is the plan's
  expectation of it.
