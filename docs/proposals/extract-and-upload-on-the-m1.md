# Proposal: extract and the remote upload on the M1

Status: proposal, not applied, revised after its review. It answers what lane `measure-m2`'s first
runs on the Apple M1, and the three reviews of them, found against criteria 4 and 5 of milestone 2
(`docs/plans/milestone-2.md`, section 1). The owner rules on three things: the snapshot's fill, the
count of presented rows criterion 4 is judged at, and how criterion 5 is read. The three questions
are at the end. Every figure below is the lane's or a reviewer's, taken with other work on the
machine. None is a record (architecture section 20), and this proposal ran no measurement. It
amends one sentence of `docs/architecture.md` now, section 2's on where frame-critical workers run,
and one entry of section 21 at a lane's merge. Approved as recommended, it is applied as one
decision entry, that amendment, the plan's and the verification rows' text below, and two small
lanes.

The first version (commit `598b717`) was reviewed. Every finding is accepted, and "What the review
changed" lists the answers.

## What was measured

| | Criterion | Figure on the M1 | Where the time is |
|---|---|---|---|
| 4 | Extract's pass at 50,000 dirty rows, at most 1 ms | 1.72 ms on four workers, on the device and on null, at 10,000 presented rows | The snapshot's fill: 1.08 ms, 70 to 77% of it sorting. The ring alone is 0.65 ms |
| 5 | The remote upload of 5,000 entities at 144 Hz, at most 1 ms | 0.39 ms in one run and 1.83 ms in the next, the code unchanged | The class of core macOS starts the paced job on, and that core's clock |

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
- That half took 1.08 ms at the test's 10,000 presented rows, 0.06 ms at 1,000 and nothing at none
  (`reviewer`'s probe of the lane's runs).
- The proposal's review timed the half alone, on the lane's layout of the presented rows and on a
  layout whose keys lie in chunk order. It ran a counting null sink at load averages of 3.7 to 4.6,
  so the figures steer and prove nothing.

| The snapshot's half, in µs | 5,000 presented rows | 10,000 presented rows |
|---|---|---|
| Today's fill, the lane's layout | 522 to 557 | 1,009 to 1,151 |
| Today's fill, keys in chunk order | 148 to 209 | 277 to 392 |
| One sort a pass, nothing kept (remedy F) | 228 | 441 |
| One sort, its order kept (remedy A) | 89 to 136 | 179 to 216 |

- With the lane's 0.65 ms ring, today's pass is about 1.2 ms at 5,000 presented rows and 1.7 to
  1.8 ms at 10,000. It misses at either count.
- Sorting is 70 to 77% of today's half at 10,000 rows: 579 to 601 µs in the columns' sorts and 191
  to 202 µs in `keys_of`.
- The layout moves today's half by 2.6 times. `present-impl`'s own test read 0.31 to 0.33 ms for the
  same fill at the same 10,000 rows (verification row 192, review round 2). Its scene spawns the
  rows in key order in one region (`crates/present-api/tests/cost.rs`, `scene`), so its sorts are
  handed sorted lists. `measure-m2`'s scene interleaves the presented rows' keys across regions and
  archetypes, and the sorts do their work.
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
| A. One sort, its order kept | Extract sorts the presented rows' places once, by key, and keeps the sorted places, each key with its chunk and row, for the next pass. A pass copies each row's components in that order and checks each place against the chunk's key as it copies. It sorts again, once, only when a place no longer holds its key or the count of presented rows changed. A place says where a row is, not what it holds: the columns are read from the row's chunk at every pass | Leaves a serial copy of every presented row at each pass after a tick: D157's cost, linear in the rows. A pass after a presented row came, went or moved is F's pass and a check. Adds kept state and a rule for when it is stale. Rules out nothing D157 decided |
| F. One sort a pass, nothing kept | Extract sorts the places at every pass and copies in that order | No state and no rule: the same cost at every pass. At 5,000 rows the pass holds with about 0.1 ms left; at 10,000 it misses |
| B. The buffers keep their bytes too | Each of the three buffers also keeps the mark it was last written at, and a pass rewrites only the rows stamped since | Cost follows the presented rows that changed. The worst case, every presented row changed, is A's copy, and a pass is budgeted by its worst case. Three marks and a rule for a stale buffer to prove. Not measured |
| C. The fill is split across the pass's workers | One child of the pass's scope beside the ring's range makes the pass the longer of its halves, not their sum | The work stays, a worker leaves the ring's range, and the code runs on a second thread. With today's fill at 10,000 rows the pass is still that fill. Jobs cannot share one column's rows in safe Rust, so a fill forked by chunk is a larger change |
| D. A presented count with its reason | The scene states how many rows it presents, and why | Alone, it leaves today's fill: a pass of about 1.2 ms at 5,000 rows |
| E. The criterion restated | Criterion 4 read on the ring's part, the snapshot given a budget of its own | The plan's decision 5.2 rules out a criterion "read without the snapshot", and D162's margin counts on the whole pass |

On the review's figures the pass is:

| The pass, in ms, on the lane's 0.65 ms ring | 5,000 presented rows | 10,000 presented rows |
|---|---|---|
| Today's fill | about 1.2 | 1.7 to 1.8 |
| F, and A at a pass that sorts | about 0.9 | about 1.1 |
| A at a pass whose places hold | 0.74 to 0.79 | 0.83 to 0.87 |

**What a scene presents.**

- Milestone 2's scene presents no row. The client's seven rows hold a transform, a mesh and
  `sketch::orbit`, which is not extracted (`game/game-sim/src/sketch.rs`).
- Milestone 3 adds the lights of its tile-local lighting and skinned characters on terrain. Its
  500,000 instances are rows of a mesh and a transform, which go to the ring alone (D157). No
  document counts its lights or its characters; the one count a criterion gives is milestone 1's
  200 characters (section 22).
- Milestone 4's client presents criterion 5's 5,000 remote entities. Each is a presented row of the
  replicated world's snapshot, with `RemoteSlot` and `MeshInstance`. They are rows of another kind
  than the cost scene's: a client's pass copies no transform into the snapshot and writes no ring
  row for them (`extract.rs`, lines 113 to 115 and 554 to 560).

**The count.** No document forecasts how many rows a scene presents, so any count is an anchor and
not a forecast. 5,000 is the only count of presented rows a criterion of section 22 names. 10,000
is a test's constant. The count is chosen for its source, not for its figure: with remedy A the
pass holds at both, and the file prints the snapshot's part at both.

**Recommendation: A, judged at 5,000 presented rows, with 10,000 printed.**

- A removes what the implementation added and keeps what D157 decided. It needs no state in the
  buffers, so nothing about the triple buffer changes.
- A holds the criterion at either count, so the verdict does not turn on a count no document
  grounds. At 5,000 rows even its pass that sorts holds, at about 0.9 ms.
- F is simpler, and at 5,000 rows it is enough. It is not recommended, for three reasons. It misses
  at 10,000 rows, so it passes only if the count question goes one way. Its margin at 5,000 is
  about 0.1 ms on a loaded machine's probe, and a miss in the quiet window costs a second lane and
  a second window in which every track stops. And A contains it: A's pass that sorts is F's pass.
- What A costs over F is stated plainly. It keeps places between passes, it needs a rule for when
  they are stale, its tests run in two kinds of layout (below), and it is about one agent run
  more. Where presented rows come and go at most passes, A buys nothing over F.
- 5,000 is judged because it has a source, weak as it is, and 10,000 has none. 10,000 is printed so
  that the harder case is on record and the choice hides nothing.
- Half the presented rows are moved because half the scene's rows are. With A the snapshot's part
  does not depend on it.
- B is not built now. Its gain is the typical pass, not the worst one, and the counts above do not
  need it. It is the next step if A's part is over its target.

**The change.**

- *Crates.* `lactose-present-api` alone: `Presented::fill`, `Presented::changed`, `keys_of` and
  `ExtractState`'s private `presented` in `src/extract.rs`; `PresentSnapshot::fill`, which is
  `pub(crate)`, in `src/snapshot.rs`. The count and the layout change in the two cost tests,
  `crates/present-api/tests/cost.rs` and `crates/app-client/tests/extract.rs`. `lactose-ecs` is
  untouched: the fill reads `World::chunks` and `ChunkRef::keys`, as it does today.
- *Contracts.* No public signature changes. The crate README's "Extract" gains what the snapshot's
  half costs, and its proof row "Cost" states 5,000 rows whose keys no chunk order follows. D157
  gains an addendum at the lane's merge.
- *A kept place is a hint, never a key.* Architecture section 8.2 says chunk and row indices "are
  never semantic keys". A place is checked against its row's `StableKey` at every pass, and no byte
  of the snapshot depends on it: the buffer equals the one F writes. The review's prototype
  matched today's fill in 9,600 frames of 9,600, over eight layouts.
- *A place does not say what a row holds.* A row can keep its chunk and its row while its archetype
  changes, and `ChunkRef` has no identity that outlives a pass (`crates/ecs/src/change.rs`). A fill
  that kept each column's list beside the order wrote a wrong buffer in 217 frames of 9,600 in the
  review's probe: it dropped `Teleported` from a row that gained it in place. The columns are read
  from the chunk at every pass.
- *Whose.* A lane's, `snapshot-order`, not the architect's: every signature and every rule of the
  facade holds.
- *Size.* One small lane, about 300 lines with its tests. `reviewer` alone reviews it, since the
  fill stays on the pass's thread. About six agent runs.
- *What it must show.* A test that compares the buffer each pass wrote with `present-impl`'s fill of
  the same world, over random sequences of moves, spawns, despawns, extracted components added and
  removed, region loads and unloads, restores and `set_tick`, under the reader cadences of the
  triple buffer's test. It runs at the production layout, at a chunk capacity alone, and under
  every layout mode SOUR runs. The recorded test's hashes unchanged.
- *The count of passes that sorted.* SOUR's layout modes other than production permute rows at
  every sync point (`crates/sim/README.md`, `layout_mode`), so under them A sorts at every pass:
  30 of 30 in the review's probe, against none of 30 where the layout keeps rows in place. The
  count is asserted to be zero in a steady scene at the production layout and at a chunk capacity
  alone. Under SOUR's modes it is recorded and not asserted. A unit test inside the crate reads it;
  no public accessor is added.
- *The figures.* In `extract-50k`, the snapshot's part at the plan's scene, with a target of 0.2 ms
  on the M1, and the pass at most 1 ms on the device, on the M1 in the quiet window and on the
  desktop. On null, two arms that are printed and not judged: the part at 10,000 presented rows,
  and the part of a pass that sorts, with one presented row despawned and another spawned every
  tick.

**What would change it.**

- The owner ruling for 5,000 rows and against kept state. Then F is the lane, a run shorter, and
  the arm that sorts is dropped.
- Milestone 3's plan naming a scene that presents more than 5,000 rows. That plan then restates the
  row.
- Milestone 4's replication showing that presented rows come and go at most passes. Then the kept
  places buy nothing, and the lane that finds it removes them.

## Criterion 5: remote transform upload at most 1 ms for 5k entities at 144 Hz

**What was measured, and under what.**

- The frame's job is one `sample_batch`, the conversion to render space and the ring's write. With
  frames run back to back it takes 0.27 to 0.29 ms, and 96% or more of the jobs begin on
  performance cores.
- Paced at 144 Hz the job is 0.3 ms of work every 6.9 ms. Seven runs back to back gave medians from
  0.32 to 1.84 ms. The load average does not say which a run gives.
- The job's time follows two things, the class of core and that core's clock, and the runs show
  four cases of them.

| Where the job ran | The job's median | Source |
|---|---|---|
| A performance core at full clock | 0.32 to 0.34 ms | `reviewer-concurrency`'s four fast runs |
| A performance core at reduced clock | 0.55 to 0.65 ms | Its three slow runs; 0.57 and 0.58 ms in `reviewer`'s probe |
| An efficiency core at full clock | 0.43 ms | Two of `reviewer-concurrency`'s runs. One run of `reviewer`'s, at a load average of 13.7, read 0.90 ms on efficiency cores whose clock is less sure |
| An efficiency core at reduced clock | 1.4 to 1.9 ms | The slow runs of both, where 804 to 1,106 of 1,440 jobs began there |

- The clock moves the figure more than the class does. A performance core at reduced clock is the
  case the first version left out: it is slower than full clock by 1.6 to 2.1 times, and still
  under 1 ms.
- The conversion, a loop over the job's own vectors, slowed 6.8 to 8.6 times, so the cause is the
  core and its clock and not the writer. P99 and row 102's retry rate follow the same split.
- The frame-critical class is already macOS's highest QoS class
  (`crates/core/src/thread_class.rs`, lines 203 to 211), and D17 records that "macOS offers only
  QoS, not pinning".
- Every run of the lane and of its reviews was started from a terminal. The process then has the
  `TASK_UNSPECIFIED` role, in which user-interactive QoS is clamped to priority 31 (verification
  row 78; `docs/results/spike-wake-apple-m1-2026-10-06.md`, "Task role"). Only `lactose-task`'s
  benchmark sets `TASK_FOREGROUND_APPLICATION`. And the test's frame holds nothing but the job.
- The proposal's review ran a stand-in of the two conditions: plain threads at user-interactive
  QoS, a job paced at 144 Hz, in each role, alone and in a busy frame, where three other threads
  work for 3 ms and meet the job before it runs. It is not the engine's executor. It found the
  machine in two states at load averages of 3.3 to 6.5, its performance cores' calibration at 16.6
  to 22.5 µs in one and at 9.7 µs in the other.

| Jobs that began on efficiency cores, of 1,440 | The slower state (2 runs a case) | The faster state (4 runs a case) |
|---|---|---|
| Unspecified role, alone | 1,148 and 1,152 | 271 to 315 |
| Unspecified role, busy frame | 871 and 1,121 | 271 to 315 |
| Foreground role, alone | 1,060 and 1,161 | 0, 0, 0 and 1 |
| Foreground role, busy frame | 0 and 0 | 0 in all four |

- In the stand-in only the foreground role with a busy frame held the performance cores in both
  states. In the slower state those cores ran at about half their clock.
- The client has no 144 Hz frame at milestone 2. Its frame is paced by the timer root at the tick's
  60 Hz (D162, row 228). The pacing at 144 Hz is the test's.

**What the criterion means.** Architecture section 5.3 asks for the upload's "cost at 144 Hz for
several thousand entities". That is a share of a frame: of the 6.94 ms a 144 Hz frame has, what the
upload takes on the cores a frame's work runs on. To time it, the test makes a frame of nothing
else. macOS reads 0.3 ms of work every 6.9 ms as a light load and runs it where it is cheapest. The
frame that results still ends 5 ms early, so 1.83 ms is not what the upload takes from a frame that
needs its time.

The criterion is read on the frames whose job ran on a performance core, at whatever clock that
core had. The class is the one a frame with work in it ran on in the stand-in. The clock is not the
test's to choose: a performance core at reduced clock is what such a frame gets, and it counts as
it ran.

It is not a promise that macOS gives a frame the performance cores. Whether the engine's frame is
kept there is the engine's question. The stand-in suggests an answer and no run through the
executor gives one. It is not criterion 5's.

**Why the class of core and not the chain's time.** The lane's fix round times a fixed chain of
safe code at each job's start and bands the frames by it. The chain cannot say the class. By the
reviews' chains a performance core at reduced clock reads 1.7 to 2.3 times the fastest chain, an
efficiency core at its fastest 1.3 to 2.2 times, and an efficiency core at reduced clock 2 to 4.2
times. The ranges overlap, so a bound on the chain's time either drops the performance cores'
slower frames or admits the efficiency cores'. The class is read from the system instead, and the
lane's bands stay beside it as the record of the clock.

**The remedies.**

| Remedy | What it does | What it rules out, or leaves |
|---|---|---|
| A. Read on performance cores, in the foreground role | `lactose-task` gives measurement two safe calls: the foreground task role, which its benchmark already takes, and the class of the core the calling thread is on. The cost tests take the role and print it. Criterion 5 is the median of the frames whose job began and ended on a performance core, with their share of the run. The figures over all frames, over the rest and by clock band are recorded beside it | A lane of about six agent runs, and section 21's entry for `task` reworded. No threshold is chosen after the data. Leaves the placement of the engine's own frames to the placement rounds |
| B. The foreground role, all frames | The cost tests take the role. The criterion stays as written, over all frames, with the lane's clock bands beside it | About four agent runs. In the stand-in a job alone in the foreground role still began on efficiency cores in 1,060 and 1,161 of 1,440 frames in the slower state, so the verdict can follow the machine's state |
| C. Read in a clock band, no new code | Criterion 5 is the median of the frames whose chain read under 1.25 times the run's fastest, the bound already in the lane's test, with their share | Drops a performance core at reduced clock with the efficiency cores. A quiet M1 may leave under a tenth of its frames in the band, and then the one run judges nothing. The role stays clamped |
| D. Measure the placement first | Criterion 5 stays open on the M1 until the placement rounds are run | About five agent runs before milestone 2 can exit. Their result does not move what a performance core costs |
| E. Hold the performance cores | A means macOS gives: the table below. The first is a parallel workgroup, by a spike and then a lane | Twelve agent runs or more, FFI in `lactose-task` or `lactose-core`, and no measurement of its effect or its power. In the stand-in the foreground role and a busy frame already held the cores; what was left was the clock, which a workgroup does not set |
| F. Restate for the M1 | 2 ms on the M1 over all frames, 1 ms elsewhere | Writes one state of the scheduler into the engine's budget. A fast run then passes with a margin that means nothing, and the figure stops being comparable with the desktop's |

**The means macOS gives.**

| Means | `unsafe` | Evidence | Power | The rest of the frame |
|---|---|---|---|---|
| A higher QoS class | None | None exists: frame-critical is user-interactive already | None | None |
| The foreground task role | One FFI call, today in `lactose-task`'s benchmark alone (architecture section 21) | Lifts the clamp from priority 31 to 47; in the wake spike it removed the tail beyond P99.9. In the stand-in it held the performance cores with a busy frame in 6 runs of 6, and alone only in the faster state | None of its own | Every class of the process gets its real priority, the network and present threads included |
| A parallel workgroup (`os_workgroup_parallel_create`, `os_workgroup_join`) | FFI in `lactose-task` or `lactose-core`; section 21's table amended | Public since macOS 11. It names the frame's workers as one workload and carries no deadline. Not measured | Unknown | Each worker joins once at start; nothing per frame |
| A work interval | Not available | The SDK's `os/workgroup_interval.h` declares start, update and finish and no creation call: "the only supported interval workgroups are for audio workloads" | None | None |
| The time-constraint policy | An FFI call | The wake spike: honoured, and "a thread spinning at 100% under it is demoted to priority 0" | Unknown | It fits one thread that waits for a deadline. The frame's workers run jobs for milliseconds and rescan for about 50 µs before they park |
| A spin that holds the clock | None | Fast in `presentation`'s review (row 19); not held in the lane's review, 886 of 1,440 jobs on efficiency cores | A core at all times; the heat and the battery are not measured (`powermetrics` needs sudo) | One performance core less for the frame. D113 rejected the warm sleeper, and Goal 1 says utilisation is never a target |
| The system's Game Mode | None: packaging | Not verified here. It cannot apply to a test binary | The system's | The system's |

**Recommendation: A, and the placement rounds owed before milestone 3.**

- *The band is fixed before the data.* The criterion's band is a class of core, read from the
  system. It has no threshold. The clock bands recorded beside it keep the bounds the lane's test
  already holds: full under 1.25 times the run's fastest chain, reduced under 2 times, half at 2
  times or over. The plan's row states them.
- *One run, never made again.* The plan's quiet window runs `cargo xtask measure --all` once
  (the plan's section 4), and that run is criterion 5's. It is not repeated for where its frames
  ran. With under a tenth of its frames on performance cores, 144 of 1,440, it judges nothing: the
  M1's criterion 5 then waits for the placement rounds, and the desktop's stands on its own run. No
  run cited here fell under it: the fewest on performance cores was 279 of 1,440.
- *The role is taken, and priced on its own.* It is one FFI call that the benchmark already makes
  (`crates/task/benches/whisk.rs`, `foreground_task_role`), moved behind a safe function.
  Verification row 78 already holds the executor's benchmark to that role, for the same reason:
  under the clamp the frame's workers run at priority 31.
- *The placement is a finding about the engine on macOS.* It gets its own verification row and a
  measurement that turns it into a decision: a phase of `lactose-task`'s benchmark runs a job of
  fixed safe work at 144 Hz from the timer root in each role, alone and after a stand-in that keeps
  the job's other workers busy, in five interleaved rounds or more.
- *Its rule is read on the busy case, core and clock apart, and an outcome needs every round.* In
  the foreground role, under a tenth of the jobs on efficiency cores: no placement means is built.
  If the unspecified role then puts over a tenth there, the role is what holds the cores, and how a
  client started outside an application bundle takes it is proposed with milestone 3's plan. Over
  a tenth in the foreground role: the planner proposes a placement means before milestone 3 fixes
  a frame budget on the M1, the parallel workgroup first. Rounds that disagree decide nothing and
  are made once more on another day. The cases alone decide nothing for the engine.
- No means is built now. E is not among the owner's options: its price is a spike and a lane
  against evidence that the cores were already held, and the owner can still name it.
- F is not among the options either. It makes the M1 pass by writing the scheduler's slow state
  into the budget.
- P99 is read the same way. The performance-core frames' P99 over 2 ms is a finding about the
  frame's tail; the P99 over all frames is recorded with the placement.

**The change.**

- *Crates.* `lactose-task`: the role call moves out of `benches/whisk.rs` into the library, behind
  a safe function that returns the role the system reports. A second safe function gives the class
  of the core the calling thread is on. On macOS it is `pthread_cpu_number_np` read against the
  per-level core counts `src/platform.rs` already reads; on Windows and Linux, the current
  processor read against the efficiency classes and capacities it already reads.
  `lactose-app-client`'s tests: the three cost tests of the `integration` target take the role in
  their shared support, and `tests/upload.rs` records each job's class of core.
- *Contracts.* Two public functions are added to `lactose-task`, for measurement. Its README says
  that nothing in a frame calls them. Section 21's entry for `task` is reworded at the lane's
  merge, as below.
- *The numbering.* macOS documents no map from a CPU number to its class. The reviews' probes took
  the efficiency cores to be the lowest-numbered, which holds on the M1. The lane shows it on the
  machine: the file gives the fastest chain on each class, and the efficiency class's is the
  slower. Where the call cannot tell, the file says so and the criterion is read over all frames.
- *`frame-pace` is not covered.* It runs the client binary, whose role is the placement row's.
- *Whose.* A lane's, `measure-role`. The functions are small and for tests; no rule of the crate's
  facade changes.
- *Size.* About six agent runs, with `reviewer` alone: the new `unsafe` is FFI calls. It runs in
  parallel with `snapshot-order`, after `measure-m2` merges.

**What the desktop and the Windows laptop must show.**

- *The desktop*, session C: the same file. Criterion 5 there is the performance-core frames' median
  at most 1 ms, with their share. On a part with one class of core every frame counts. Its clock
  bands show that machine's ramp from idle, which is recorded.
- *The laptop*, labelled, a hybrid part under Windows 11: Windows' share of frames by class of
  core, with frame-critical at above-normal priority and no opt-out of power throttling (D17's
  addendum). It proves nothing for the criterion. It is the first evidence for row 233's second
  question.
- For criterion 4 both show the pass at the plan's scene. The snapshot's part is host memory, so
  what lane `snapshot-order` saves carries over. The ring's part does not: on a discrete GPU it is
  write-combined memory (the plan's decision 5.2), and it is the desktop's to show.

**What would change it.**

- The quiet window's performance-core median over 1 ms. Then the miss is the engine's, and the
  batch call is where a lane looks first.
- The class-of-core call failing its check on the M1. Then C is what is left, with its limits, and
  the role is still taken.
- The owner wanting no new FFI in milestone 2. Then B or C, by whether the role is wanted.

## The amendments (exact text)

Each OLD is one whole line of `docs/architecture.md` at `12154bb`, occurring once in it.
`measure-m2`'s merge text touches neither line.

- *Section 2, now.* "Frame-critical workers on performance cores" is false of the runs this
  proposal cites: 804 to 1,106 of 1,440 frame-critical jobs began on efficiency cores. The
  architecture is binding, and milestone 3's plan would read the sentence as a guarantee. The new
  text says what the engine does and what it does not control. It is applied with the decision.
- *Section 21, at `measure-role`'s merge*, and only if the owner takes remedy A or B for
  criterion 5. Under B the entry names the task-role call alone.
- Section 5.1's "Frame-critical workers equal the performance-core count on every platform" and
  "They run at user-interactive QoS" are true and do not change. Section 5.3's snapshot budget goes
  into revision 3.5 with the quiet window's figure (the plan's section 8).

~~~text
=== docs/architecture.md line 62: section 2, the Concurrency row
OLD:
| Concurrency | Systems complete by continuation counters. Fork-join only inside a system, same-scope help only. `'static` stackless futures for IO, GPU and asset work on a custom executor. No stackful fibers. No tokio. Frame-critical workers on performance cores, background class on efficiency cores. |
NEW:
| Concurrency | Systems complete by continuation counters. Fork-join only inside a system, same-scope help only. `'static` stackless futures for IO, GPU and asset work on a custom executor. No stackful fibers. No tokio. Frame-critical workers equal the performance cores in number and ask for them by class; background class on efficiency cores. The class is a request: on macOS, which offers only QoS and no pinning (D17), where a frame's job starts, and at what clock, is measured, not assumed (D16x). |

=== docs/architecture.md line 1462: section 21, the entry for `task` (at `measure-role`'s merge)
OLD:
| `task` | deques, scope borrows, panic guard, task futures, core-topology queries, the keeper's timed wait on Windows, the benchmark's and tests' process-CPU and task-role calls |
NEW:
| `task` | deques, scope borrows, panic guard, task futures, core-topology queries, the keeper's timed wait on Windows, the benchmark's and tests' process-CPU calls, the task-role and current-core calls it gives measurement as safe functions |
~~~

## The rows (exact text)

Each OLD is one whole line of the named file at `12154bb`, occurring once in it. The format is the
one `measure-m2`'s merge text uses. A NEW of several lines replaces the one. The rows are written
for the recommended answers. `D16x` stands for the decision's number, which the coordinator
allocates; the rows name each other by `D16x` and never by a row number.

**The dependency.** `measure-m2`'s merge rewrites verification lines 16, 19 and 102 and adds two
rows after line 245. These rows are applied after that merge, and its text is not edited here.

- *Rows 16, 19 and 102.* Each change below adds text after one span. Each span occurs once in the
  file at `12154bb`, and once in the lane's NEW for that row as its merge text stood after round
  three. After the lane's merge the same text is added after the same span of the merged line. The
  spans are whole sentences or cells, since their last words alone recur:
  - row 16: "Per-dirty-row extract cost; the workload and machines of D153, result file
    `extract-50k`";
  - row 19: "Cost per frame for 5k interpolated entities at 144 Hz; the workload of D153, result
    file `remote-upload-5k`";
  - row 102: "The reader's retry rate under a 30 Hz writer and the footprint are in result file
    `remote-upload-5k` (D153).".
- *The two new verification rows* have no OLD line. They are appended at the end of
  `docs/verification.md`, whose last table is the file's last content at `12154bb` and after the
  lane's merge. They land after the lane's two rows whichever is applied first.
- *The plan's lines* 59, 60, 183, 337, 338, 397 and 408 are not in the lane's merge text and apply
  as written. Line 183 is the lane table's last row, and its block adds the two lanes after it.

**What each ruling touches.** A ruling other than the recommended one goes back to the planner for
these rows, one agent run.

| The owner's question | The rows that follow its answer |
|---|---|
| The fill | The plan's line 397, the lane row `snapshot-order`, the fill's verification row, D16x's first two paragraphs |
| The count | The plan's lines 59, 337 and 397, verification row 16, the lane row `snapshot-order`, D16x's third paragraph |
| Criterion 5 | The plan's lines 60, 338 and 408, verification rows 19 and 102, the lane row `measure-role`, section 21's entry, D16x's fifth and sixth paragraphs |

~~~text
=== docs/plans/milestone-2.md line 59: criterion 4's row: the scene's presented rows, the snapshot, the lane
OLD:
| 4 | Extract at most 1 ms for 50k dirty rows | A cost test: 100,000 rows with a `Transform`, 50,000 of them moved every tick, extract's pass alone, the median of 60 passes, writing the device's ring (decision 5.2) | A quiet M1; the desktop | `extract-50k` | `present-impl`, `renderer-impl`, `measure-m2` |
NEW:
| 4 | Extract at most 1 ms for 50k dirty rows | A cost test: 100,000 rows with a `Transform`, 5,000 of them presented rows, 50,000 moved every tick and half the presented rows among them, extract's pass alone, the median of 60 passes, writing the device's ring and the snapshot (decision 5.2, D16x) | A quiet M1; the desktop | `extract-50k` | `present-impl`, `renderer-impl`, `measure-m2`, `snapshot-order` |

=== docs/plans/milestone-2.md line 60: criterion 5's row: read on performance cores, in the foreground role
OLD:
| 5 | Remote transform upload at most 1 ms for 5k entities at 144 Hz | A cost test: 5,000 slots of the interpolation buffer written at 30 Hz, sampled in one batch and written to the staging ring every frame, paced at 144 Hz for ten seconds, the median frame (decision 5.3) | A quiet M1; the desktop | `remote-upload-5k` | `presentation`, `measure-m2` |
NEW:
| 5 | Remote transform upload at most 1 ms for 5k entities at 144 Hz | A cost test: 5,000 slots of the interpolation buffer written at 30 Hz, sampled in one batch and written to the staging ring every frame, paced at 144 Hz for ten seconds in the foreground task role, the median of the frames whose job ran on a performance core, with their share of the run (decision 5.3, D16x) | A quiet M1; the desktop | `remote-upload-5k` | `presentation`, `measure-m2`, `measure-role` |

=== docs/plans/milestone-2.md line 183: the lane table: two lanes after its last row
OLD:
| `eco-qos` | Windows efficiency-core steering (row 63): `ThreadClass::Background` sets the power-throttling thread setting, in `crates/core/src/thread_class.rs`. Whether `ThreadClass::Io` follows is reported and not decided | Nothing: `keeper` merged at `0ec2d90` | On the laptop, the WHISK benchmark's rows with two background zstd tasks, before and after. Miri for `lactose-core` | 5 |
NEW:
| `eco-qos` | Windows efficiency-core steering (row 63): `ThreadClass::Background` sets the power-throttling thread setting, in `crates/core/src/thread_class.rs`. Whether `ThreadClass::Io` follows is reported and not decided | Nothing: `keeper` merged at `0ec2d90` | On the laptop, the WHISK benchmark's rows with two background zstd tasks, before and after. Miri for `lactose-core` | 5 |
| `snapshot-order` | The snapshot's fill in `lactose-present-api` (D16x): one sort of the presented rows' places by key, where `present-impl`'s fill sorts every column and the keys apart, and the sorted places kept for the next pass while each still holds its key. The columns a row is copied from are read from its chunk at every pass. No public signature changes. The two cost tests present 5,000 rows whose keys no chunk order follows, `crates/present-api/tests/cost.rs` and `extract-50k`, which gains its two arms on null | `measure-m2` merged | The verification row's: the equality test against `present-impl`'s fill, no pass that sorts in a steady scene where the layout keeps rows in place, the recorded test's hashes unchanged, and the snapshot's parts in `extract-50k` | 6 |
| `measure-role` | Two safe calls for measurement in `lactose-task` (D16x): the foreground task role on macOS, moved out of the benchmark, which returns the role the system reports, and the class of the core the calling thread is on, on macOS, Windows and Linux. The three cost tests of `lactose-app-client`'s `integration` target take the role and print it, in their shared support. `remote-upload-5k` records for each frame the class of core its job began and ended on and gives the figures of section 4's row | `measure-m2` merged | The role read back in each file. In `remote-upload-5k`'s file, the fastest chain on each class of core, the efficiency class's the slower: the numbering the macOS call rests on, shown on the machine. The loom models and the cross-target clippy runs for `lactose-task`. Section 21's entry for `task`, reworded at the merge | 6 |

=== docs/plans/milestone-2.md line 337: row `extract-50k`: 5,000 presented rows, their layout, the two arms
OLD:
| `extract-50k` | 100,000 rows with a `Transform` and a mesh component, in four archetypes and sixteen regions, as milestone 1's cost scene is laid out, and a stated count of presented rows besides, each with an `extracted` component other than the mesh and the transform, so the snapshot's part measures a snapshot that holds something: rows that are only a mesh and a transform go to the ring alone (D157). A kernel moves 50,000 of them every tick. One tick, then `Simulation::run_passes` with extract alone registered, sixty times, on N frame workers, N the performance cores up to eight. The ring is the device's mapped buffer, and the run is repeated on null. The file gives the pass's median, the ring's part and the snapshot's part, the cost per dirty row on one worker, and the ring's memory type and flags. Each pass ends with an empty submit that signals its timeline value, so the ring's regions come free; a wait for a free region is left out of the median and reported | Criterion 4: the median on the device at most 1 ms, on the M1 and on the desktop |
NEW:
| `extract-50k` | 100,000 rows with a `Transform` and a mesh component, in four archetypes and sixteen regions, as milestone 1's cost scene is laid out, 5,000 of them presented rows, one in twenty of the rows a kernel moves and one in twenty of the rest, their keys interleaved across the chunks of both presented archetypes and of several regions, so that no chunk order is key order, each with an `extracted` component other than the mesh and the transform, so the snapshot's part measures a snapshot that holds something: rows that are only a mesh and a transform go to the ring alone (D157). 5,000 is criterion 5's count, the most presented rows a criterion of section 22 names, though those are a client's remote rows, which copy no transform (D16x). A kernel moves 50,000 rows every tick, 2,500 of the presented rows among them. One tick, then `Simulation::run_passes` with extract alone registered, sixty times, on N frame workers, N the performance cores up to eight. The ring is the device's mapped buffer, and the run is repeated on null. The file gives the pass's median, the ring's part and the snapshot's part, the cost per dirty row on one worker, and the ring's memory type and flags; and, on null, the snapshot's part at 10,000 presented rows and its part of a pass that sorts, an arm in which one presented row is despawned and another spawned every tick. Neither arm is judged. Each pass ends with an empty submit that signals its timeline value, so the ring's regions come free; a wait for a free region is left out of the median and reported | Criterion 4: the median on the device at most 1 ms, on the M1 and on the desktop |

=== docs/plans/milestone-2.md line 338: row `remote-upload-5k`: the role, the class of core, the criterion's frames
OLD:
| `remote-upload-5k` | 5,000 slots of `lactose-net-host`'s interpolation buffer. One writer thread pushes a state to every slot at 30 Hz. The presentation schedule's remote upload runs every frame, paced at 144 Hz by a timer for ten seconds: one `sample_batch`, the conversion to render space, the write to the device's ring. Each frame ends with an empty submit that signals its timeline value, so the ring's regions come free as they do in a frame. The file gives the median, P99 and worst frame, the reader's retries, the buffer's bytes and the ring's memory type | Criterion 5: the median at most 1 ms, on the M1 and on the desktop. P99 is recorded; over 2 ms it is a finding |
NEW:
| `remote-upload-5k` | 5,000 slots of `lactose-net-host`'s interpolation buffer. One writer thread pushes a state to every slot at 30 Hz. The presentation schedule's remote upload runs every frame, paced at 144 Hz by a timer for ten seconds: one `sample_batch`, the conversion to render space, the write to the device's ring. Each frame ends with an empty submit that signals its timeline value, so the ring's regions come free as they do in a frame. The test takes the foreground task role on macOS, and the file gives the role it ran in. Each frame's job records the class of the core it began on and ended on, which `lactose-task` reads, and the time of a fixed chain of safe code at its start. The file gives the median, P99 and worst frame over all frames, over the frames whose job began and ended on a performance core and over the rest, each with its share of the frames and its split by clock band, a band being the chain's time against the run's fastest, full under 1.25 times, reduced under 2 times and half at 2 times or over; the same figures of an unpaced control run; the reader's retries, the buffer's bytes and the ring's memory type | Criterion 5: the median of the performance-core frames at most 1 ms, on the M1 and on the desktop, with their share beside it; a core at reduced clock counts as it ran, and on a machine with one class of core every frame counts. The run is the quiet window's one run and is never made again for where its frames ran; with under a tenth of them on performance cores it judges nothing, and the placement rounds come first (D16x). P99 is recorded for each; the performance-core frames' over 2 ms is a finding. Where the frames ran is a verification row's, not the criterion's (D16x) |

=== docs/plans/milestone-2.md line 397: decision 5.2's last line: what the snapshot holds
OLD:
the file says what the snapshot holds.
NEW:
the file says what the snapshot holds. It holds 5,000 of the scene's rows, half of them moved, their
keys interleaved across chunks, and its part of the pass is one copy of them in key order, an order
kept from the pass before while it holds (D16x).

=== docs/plans/milestone-2.md line 408: decision 5.3's last line: the finding went to the planner
OLD:
goes to the planner as a finding about the frame's tail.
NEW:
goes to the planner as a finding about the frame's tail. It went, and the median with it (D16x): on
the M1 both follow the class of core macOS starts the frame's job on, and that core's clock, so the
criterion reads the frames whose job ran on a performance core, at whatever clock, with their share
of the run.

=== docs/verification.md line 16: verification row 16: the scene, and when criterion 4 is read
OLD:
| Extract cost | Per-dirty-row extract cost; the workload and machines of D153, result file `extract-50k` | 2 |
NEW:
| Extract cost | Per-dirty-row extract cost; the workload and machines of D153, result file `extract-50k`. The scene presents 5,000 of its rows, half of them moved, their keys interleaved across chunks: criterion 5's count, the most presented rows a criterion of section 22 names, though of another kind (D16x). Criterion 4 is read at that scene once lane `snapshot-order` has merged; a figure taken on the fill that sorts every column at every pass is steering, and the file's figure at 10,000 presented rows is recorded, not judged | 2 |

=== docs/verification.md line 19: verification row 19: how criterion 5 is read
OLD:
| Remote transform upload | Cost per frame for 5k interpolated entities at 144 Hz; the workload of D153, result file `remote-upload-5k` | 2. **Steering only** (branch `presentation`, `crates/presentation/tests/cost.rs`, ignored, run in release on the Apple M1; not the result file, which is `measure-m2`'s): 5,000 remote rows of a client's replicated world, the schedule's frame (one `sample_batch` call, the merge with the meshes and the conversion) and the upload from the frame's job to a closure that counts the rows, paced at 144 Hz for 10 s with a plain thread pushing a state into every slot at 30 Hz. The sample's median varied between runs, from 0.26 to 1.8 ms, and the cause is not measured: paced at 144 Hz with no other core busy it was 1.8 ms (6 of 6 runs, `reviewer-concurrency`); with a core spinning beside it, or unpaced, 0.26 to 0.38 ms (7 of 7); in the branch's last runs, below, 0.39 to 0.45 ms. The batch call's figure is a second `sample_batch` call on the same slots made after the frame, not a share of the frame's time. The branch's last runs, with the frame handed the acquired pair: two runs as the test stands, at 1-minute load averages 3.32 then 3.26 (other agents' work on the machine): the frame's median 452.9 and 452.8 µs, 99th percentile 592.1 and 897.2 µs; of it the sample's median 449.8 and 449.7 µs and the upload's 3.4 and 3.8 µs; the second batch call's median 307.3 and 307.1 µs. Two runs with one core spinning beside it, at 3.21 then 4.11 (4.38 after): the frame's median 400.6 and 390.5 µs, 99th percentile 2,526.4 and 832.9 µs; the sample's median 394.3 and 385.9 µs; the second batch call's median 268.2 and 266.3 µs. The ring's copy is not in it while `RemoteWriter::write` is the facade's. `measure-m2` makes the recorded figure: it runs the writer under `ThreadClass::Network`, times the batch call inside the frame, and reports the reader's retry rate (row 102) |
NEW:
| Remote transform upload | Cost per frame for 5k interpolated entities at 144 Hz; the workload of D153, result file `remote-upload-5k`. Criterion 5 reads the frames whose job began and ended on a performance core, at whatever clock, in the foreground task role, with their share of the run; the figures over all frames and by clock band are recorded beside them, and where macOS starts the job is D16x's placement row, not the criterion's | 2. **Steering only** (branch `presentation`, `crates/presentation/tests/cost.rs`, ignored, run in release on the Apple M1; not the result file, which is `measure-m2`'s): 5,000 remote rows of a client's replicated world, the schedule's frame (one `sample_batch` call, the merge with the meshes and the conversion) and the upload from the frame's job to a closure that counts the rows, paced at 144 Hz for 10 s with a plain thread pushing a state into every slot at 30 Hz. The sample's median varied between runs, from 0.26 to 1.8 ms, and the cause is not measured: paced at 144 Hz with no other core busy it was 1.8 ms (6 of 6 runs, `reviewer-concurrency`); with a core spinning beside it, or unpaced, 0.26 to 0.38 ms (7 of 7); in the branch's last runs, below, 0.39 to 0.45 ms. The batch call's figure is a second `sample_batch` call on the same slots made after the frame, not a share of the frame's time. The branch's last runs, with the frame handed the acquired pair: two runs as the test stands, at 1-minute load averages 3.32 then 3.26 (other agents' work on the machine): the frame's median 452.9 and 452.8 µs, 99th percentile 592.1 and 897.2 µs; of it the sample's median 449.8 and 449.7 µs and the upload's 3.4 and 3.8 µs; the second batch call's median 307.3 and 307.1 µs. Two runs with one core spinning beside it, at 3.21 then 4.11 (4.38 after): the frame's median 400.6 and 390.5 µs, 99th percentile 2,526.4 and 832.9 µs; the sample's median 394.3 and 385.9 µs; the second batch call's median 268.2 and 266.3 µs. The ring's copy is not in it while `RemoteWriter::write` is the facade's. `measure-m2` makes the recorded figure: it runs the writer under `ThreadClass::Network`, times the batch call inside the frame, and reports the reader's retry rate (row 102) |

=== docs/verification.md line 102: verification row 102: the retry rate follows the placement
OLD:
| Interpolation slot footprint (about 530 bytes; 5k entities about 2.6 MB) against the remote-upload budget, the reader's retry rate under one writer at 30 Hz with the stated orderings, and a loom model of the sequence lock. The reader's retry rate under a 30 Hz writer and the footprint are in result file `remote-upload-5k` (D153). The loom model: **Done** (lane `interp-loom`, head 09449ad): the slot's lock is `crates/net-host/src/seqlock.rs`, generic over a `Memory` trait and the ring's shape; the buffer runs it over the standard atomics at eight states of eight words, and `src/models.rs` (feature `loom`) runs the same `reset`, `push` and `read` over loom's atomics at two states of two words, no two pushed states sharing a word. Three models, each a writer on the main thread against one spawned reader reading twice, at preemption bound 2 (bounded, not exhaustive), every read equal to the slot's contents after one write and none older than the read before it: `the_ring_wraps_under_a_reader` (a full ring pushed twice past its oldest; 242,926 executions), `the_generation_changes_under_a_reader` (a release, a reallocation and a push; 139,452) and `the_ring_grows_at_its_front_under_a_reader` (a push at the ring's front, then one past it; 129,052), each made to show one kind of mixed read: state words, `generation`, and `count` against the states. At bound 3 all three pass too (1,246,450, 735,412 and 606,050 executions). The lock's four orderings stronger than `Relaxed`, each weakened in turn, make all three models find a torn read: the writer's `fence(Release)` (`*_without_the_writers_release_fence`), the even sequence's `Release` (`*_without_the_even_sequences_release`), the reader's first `Acquire` (`*_without_the_readers_first_acquire`) and its `fence(Acquire)` (`*_without_the_readers_acquire_fence`); none is stronger than it needs to be, and `seqlock.rs` states which release pairs with which acquire. `count` stored after the even sequence store (mutant P10 of the concurrency review) passes the first two models, the hardware stress test and Miri's, and fails the third at once. A unit test without loom (`each_ordering_stronger_than_relaxed_has_one_site`) holds each ordering to one site, which is how the weakened models relax one at a time: it counts the calls one thread makes and the sites in the code, so a second site on a reader's retry path fails it too. Residuals: loom 0.7.2 backtracks only against an atomic's last access, so the writer is the main thread: with the reader on the main thread the ring-wrap model runs one execution and passes with each ordering weakened, and the models' pass is evidence through the weakened models that fail them; the count's clamp is unreachable, since every stored count is at most the ring's length; the 32-bit sequence admits a torn read only if a multiple of 2^31 writes fall between a reader's two loads; loom does not model load buffering (readers store nothing, so no load-store cycle exists) and keeps seven stores per atomic: the ring and front-growth models' readers may read five sequence values, a margin of two, the generation model's seven, no margin, so a further write there needs a model of its own; one reader, since a second only loads and a model of two exceeded loom's branch limit; the shipped `Native` orderings stay under Miri's 150-push stress test, which in the branch's review read torn samples with each of the four weakened. The nightly loom job runs it: `cargo test -p lactose-net-host --features loom --release --locked`, 21 s on the M1 | 2 |
NEW:
| Interpolation slot footprint (about 530 bytes; 5k entities about 2.6 MB) against the remote-upload budget, the reader's retry rate under one writer at 30 Hz with the stated orderings, and a loom model of the sequence lock. The reader's retry rate under a 30 Hz writer and the footprint are in result file `remote-upload-5k` (D153). The rate follows the cores the reader and the writer run on, from 8.6e-5 unpaced to 1.9e-2 paced on slow cores in the probes of `measure-m2`'s review, so the file gives it with the share of frames on each class of core beside it, and no figure of it is a criterion (D16x). The loom model: **Done** (lane `interp-loom`, head 09449ad): the slot's lock is `crates/net-host/src/seqlock.rs`, generic over a `Memory` trait and the ring's shape; the buffer runs it over the standard atomics at eight states of eight words, and `src/models.rs` (feature `loom`) runs the same `reset`, `push` and `read` over loom's atomics at two states of two words, no two pushed states sharing a word. Three models, each a writer on the main thread against one spawned reader reading twice, at preemption bound 2 (bounded, not exhaustive), every read equal to the slot's contents after one write and none older than the read before it: `the_ring_wraps_under_a_reader` (a full ring pushed twice past its oldest; 242,926 executions), `the_generation_changes_under_a_reader` (a release, a reallocation and a push; 139,452) and `the_ring_grows_at_its_front_under_a_reader` (a push at the ring's front, then one past it; 129,052), each made to show one kind of mixed read: state words, `generation`, and `count` against the states. At bound 3 all three pass too (1,246,450, 735,412 and 606,050 executions). The lock's four orderings stronger than `Relaxed`, each weakened in turn, make all three models find a torn read: the writer's `fence(Release)` (`*_without_the_writers_release_fence`), the even sequence's `Release` (`*_without_the_even_sequences_release`), the reader's first `Acquire` (`*_without_the_readers_first_acquire`) and its `fence(Acquire)` (`*_without_the_readers_acquire_fence`); none is stronger than it needs to be, and `seqlock.rs` states which release pairs with which acquire. `count` stored after the even sequence store (mutant P10 of the concurrency review) passes the first two models, the hardware stress test and Miri's, and fails the third at once. A unit test without loom (`each_ordering_stronger_than_relaxed_has_one_site`) holds each ordering to one site, which is how the weakened models relax one at a time: it counts the calls one thread makes and the sites in the code, so a second site on a reader's retry path fails it too. Residuals: loom 0.7.2 backtracks only against an atomic's last access, so the writer is the main thread: with the reader on the main thread the ring-wrap model runs one execution and passes with each ordering weakened, and the models' pass is evidence through the weakened models that fail them; the count's clamp is unreachable, since every stored count is at most the ring's length; the 32-bit sequence admits a torn read only if a multiple of 2^31 writes fall between a reader's two loads; loom does not model load buffering (readers store nothing, so no load-store cycle exists) and keeps seven stores per atomic: the ring and front-growth models' readers may read five sequence values, a margin of two, the generation model's seven, no margin, so a further write there needs a model of its own; one reader, since a second only loads and a model of two exceeded loom's branch limit; the shipped `Native` orderings stay under Miri's 150-push stress test, which in the branch's review read torn samples with each of the four weakened. The nightly loom job runs it: `cargo test -p lactose-net-host --features loom --release --locked`, 21 s on the M1 | 2 |

=== docs/verification.md: two rows appended at the end of the file, after the table's last row
NEW:
| Where macOS starts a frame-critical worker's paced job, and at what clock (D16x's placement row). `remote-upload-5k`'s job is 0.3 ms of work every 6.9 ms. In the runs of lane `measure-m2` and its first review on the M1 under macOS 27.2.1 it took 0.32 to 0.34 ms on a performance core at full clock, 0.55 to 0.65 ms on one at reduced clock and 1.4 to 1.9 ms on an efficiency core at reduced clock; in a slow run 804 to 1,106 of 1,440 jobs began on efficiency cores, seven runs back to back gave medians from 0.32 to 1.84 ms, and the load average did not say which. Every one of those runs was started from a terminal, so in the `TASK_UNSPECIFIED` role, where user-interactive QoS is clamped to priority 31 (row 78), and its frame held nothing else. A stand-in of the proposal's review, plain threads at user-interactive QoS and not the executor, ran the four cases, with the machine in two states, its performance cores' chain at 16.6 to 22.5 µs or at 9.7 µs. In the foreground role a job after 3 ms of work on three other threads began on a performance core in every frame of 6 runs of 6, at the slower clock in the slower state; alone it began on efficiency cores in 1,060 and 1,161 of 1,440 frames in the slower state and in at most 1 in the faster. In the unspecified role 871 to 1,152 of 1,440 began there in the slower state and 271 to 315 in the faster, busy or alone. Not measured: any of this through the executor and the timer root; the role a client started as an application bundle is given (`docs/results/spike-wake-apple-m1-2026-10-06.md`, "Not tested"). Owed: a phase of `lactose-task`'s benchmark. A 144 Hz timer root runs a job of fixed safe work, 0.3 ms at full clock, through `run_job`, in the four cases, each role alone and after a stand-in that keeps the job's other workers busy for 3 ms, 1,440 frames a case, in five interleaved rounds or more, a case in its own process where the role cannot be given back. For each case and round the file gives the share of jobs that began on each class of core, each class's calibration and the job's median. The rule is read on the busy case, the class of core and the clock apart, and an outcome needs every round. Under a tenth of the jobs on efficiency cores in the foreground role, in every round: no placement means is built; and if the unspecified role puts over a tenth there in every round, the role is what holds the cores, and the planner proposes with milestone 3's plan how a client started outside an application bundle takes it. Over a tenth in the foreground role, in every round: the planner proposes a placement means before milestone 3 fixes a frame budget on the M1, a parallel workgroup first (`os_workgroup_parallel_create`; the SDK's `os/workgroup_interval.h` gives a frame no work interval). Rounds that disagree decide nothing: they are made once more on another day, and the planner reads both sets. The performance cores' calibration in the busy case is recorded as the clock a frame gets, which no placement means sets. The cases alone are recorded and decide nothing for the engine. The laptop's `remote-upload-5k` gives Windows' share of frames by class of core beside it, which is evidence for row 233 | 3 |
| The snapshot's fill (D16x, lane `snapshot-order`). `present-impl`'s fill builds a list of every presented row for each copied column, sorts each list by key, copies the bytes and sorts the keys once more, at every pass after a tick: 1.08 ms of a 1.72 ms pass at 10,000 presented rows in `measure-m2`'s runs, whatever the dirty count. In the probe of the proposal's review (a counting null sink, loads 3.7 to 4.6, steering) that half is 1.01 to 1.15 ms at 10,000 rows laid out as `measure-m2`'s scene lays them, 70 to 77% of it the sorts, and 0.28 to 0.39 ms where the keys lie in chunk order, as in `present-impl`'s cost scene (row 192); at 5,000 rows it is 0.52 to 0.56 ms, a pass of about 1.2 ms on the lane's 0.65 ms ring. The lane sorts the presented rows' places once, by key, and keeps the sorted places, each key with its chunk and row, for the next pass: a pass checks each place against the chunk's key as it copies, and sorts again only when a place no longer holds its key or the count of presented rows changed. A place says where a row is, not what it holds: the columns a row is copied from are read from its chunk at every pass. It is a hint and not a key (architecture section 8.2): no byte of the snapshot depends on it, and every buffer still holds the whole state (D157). The probe's prototype read 0.09 to 0.14 ms at 5,000 rows and 0.18 to 0.22 ms at 10,000, on every layout, and 0.23 and 0.44 ms with a sort at every pass. Owed by the lane: a test that compares the buffer each pass wrote with `present-impl`'s fill of the same world, over random sequences of moves, spawns, despawns, extracted components added and removed, region loads and unloads, restores and `set_tick`, under the reader cadences of the triple buffer's test, at the production layout, at a chunk capacity alone and under every layout mode SOUR runs; the count of passes that sorted, asserted to be none in a steady scene where the layout keeps rows in place, and recorded, not asserted, under SOUR's modes other than production, which permute rows at every sync point and so sort at every pass; the recorded test's hashes unchanged; and in `extract-50k` the snapshot's part at the plan's scene, at 10,000 presented rows and for a pass that sorts. The lane's target for the part at the plan's scene is 0.2 ms on the M1; the criterion is the pass's. If the part is over the target, the next step is to rewrite in each buffer only the rows stamped since that buffer was last written, which the planner sizes then | 2 |
~~~

## Decision entry (exact text; number at merge)

**D16x. Criteria 4 and 5 of milestone 2 on the M1: the snapshot's fill sorts once and keeps its
order, the cost scene presents 5,000 rows, and the remote upload is read on performance cores**
(§2, §5.1, §5.3, §21, §22, `lactose-present-api`, `lactose-task`, `docs/plans/milestone-2.md`)
- Why: lane `measure-m2`'s first runs on the M1 put both criteria over 1 ms, and their reviews
  found one cause in the engine and one in the operating system. Extract's pass took a median of
  1.72 ms on four workers, on the device and on null: 1.08 ms of it the snapshot's fill at 10,000
  presented rows and 0.65 ms the ring at 50,000 dirty rows, 27 ns a row on one worker, inside
  D21's 20 to 40. The remote upload's median was 0.39 ms in one run and 1.83 ms in the next with
  the code unchanged.
  - *A copy of the presented rows at every pass after a tick is D157's; the sorts are not.* D157
    chose a whole state in three buffers and ruled out a snapshot of differences, so a pass that
    follows a tick leaves its buffer whole. `present-impl`'s fill also builds a list of rows for
    each column and sorts it, and sorts the keys again: 70 to 77% of its time at 10,000 rows in
    the proposal's review, and little where the rows lie in key order, as in the scene of
    verification row 192. The fill sorts the presented rows' places once, by key, and keeps the
    sorted places for the next pass. A pass checks each place against its chunk's key as it copies
    and sorts again only when a place no longer holds its key or the count of presented rows
    changed. No public signature changes. The lane is `snapshot-order`.
  - *A kept place is a hint, never a key.* It says where a row is, not what it holds: the columns
    a row is copied from are read from its chunk at every pass, since a row can keep its chunk
    and its row while its archetype changes. No byte of the snapshot depends on a place. The
    buffer equals the one a fill that keeps nothing writes, and the lane's test holds that under
    every layout mode SOUR runs. Section 8.2's rule on chunk and row indices stands.
  - *The cost scene presents 5,000 rows, half of them moved, their keys interleaved across
    chunks.* The count follows the diagnosis and not the figure: the fill that keeps its order
    holds the criterion at 10,000 rows too, and the file prints its part there. 5,000 is criterion
    5's count, the only count of presented rows a criterion of section 22 names, and of another
    kind: a client's remote rows copy no transform. No document counts milestone 3's lights or
    its characters, and its plan restates the count if its scene presents more. The layout is
    stated because `present-impl`'s fill cost 2.6 times less where the keys lay in chunk order.
  - *Criterion 4 keeps its 1 ms and its snapshot.* D162's join margin counts on the passes within
    it, and the plan's decision 5.2 rules out a criterion read without the snapshot.
  - *Criterion 5 is a cost, read on performance cores.* It is the share of a 144 Hz frame the
    upload takes on the cores a frame's work runs on. The test's frame holds nothing else, 0.3 ms
    of work every 6.9 ms, and macOS started it on an efficiency core at reduced clock in more than
    half of a slow run's frames. The job took 0.32 to 0.34 ms on a performance core at full clock,
    0.55 to 0.65 ms on one at reduced clock and 1.4 to 1.9 ms on an efficiency core at reduced
    clock. The criterion reads the median of the frames whose job began and ended on a performance
    core, at whatever clock, with their share of the run. The class is read from the system, so no
    bound is chosen after the data; the time of a calibration chain cannot stand in for it, since
    a performance core at reduced clock and an efficiency core read alike. The quiet window's run
    is the one run. With under a tenth of its frames on performance cores it judges nothing.
  - *The cost tests run in the foreground task role.* From a terminal a process has
    `TASK_UNSPECIFIED`, where user-interactive QoS is clamped to priority 31 (verification row
    78). `lactose-task` gives measurement two safe calls, the role and the class of the calling
    thread's core, and section 21's entry says so. The lane is `measure-role`.
  - *Section 2 says what the engine does, not what macOS grants.* "Frame-critical workers on
    performance cores" was false of 804 to 1,106 of 1,440 jobs in the slow runs. The workers ask
    by class, and where a frame's job starts is measured.
  - *Where a frame runs is measured before the engine acts.* In a stand-in of the proposal's
    review, plain threads and not the executor, a job in the foreground role after 3 ms of work
    on three other threads began on a performance core in every frame of six runs; alone, or in
    the unspecified role, it did not. A phase of `lactose-task`'s benchmark repeats the four cases
    through the executor, in five interleaved rounds or more, before milestone 3 fixes a frame
    budget on the M1 (verification).
  - Rules out: criterion 4 read on the ring alone; a snapshot of differences, which D157 ruled out
    and this keeps out; a row's columns kept beside its place; a run made again for where its
    frames ran; a band whose bound is set after its data; a spin that holds the clock, which costs
    a core at all times and did not hold the cores in the lane's review; the time-constraint
    policy on the frame's workers, under which a thread that spins is demoted
    (`docs/results/spike-wake-apple-m1-2026-10-06.md`); a work interval for the frame, which the
    SDK gives to audio alone; a parallel workgroup before the placement rounds; a criterion
    restated for the M1.
- Driven by: lane `measure-m2`'s runs, and the three reports of their first review, `reviewer`'s,
  `reviewer-concurrency`'s and `reviewer-gpu`'s; `docs/proposals/extract-and-upload-on-the-m1.md`
  and the probes of its review; the owner's ruling.
- Replaced: the 10,000 presented rows of `present-impl`'s cost test and that scene's key order
  (verification row 192, the crate README's proof row "Cost"); in the plan's decision 5.3, the
  median over all frames as the figure criterion 5 reads; section 2's "Frame-critical workers on
  performance cores".

## The owner's questions

As `AskUserQuestion` input, the recommended option first. The figures in them are the review's
probes', taken on a loaded machine with a null sink, and the questions say so.

~~~json
{
  "questions": [
    {
      "question": "Criterion 4 (extract at most 1 ms for 50k dirty rows) misses on the M1 because the snapshot's fill sorts every column at every pass. On a loaded-machine probe that half is 0.52 to 0.56 ms at 5,000 presented rows and 1.01 to 1.15 ms at 10,000, beside a 0.65 ms ring: a pass of about 1.2 ms or 1.7 ms. The count is the next question. Which fill?",
      "header": "Criterion 4",
      "multiSelect": false,
      "options": [
        {
          "label": "Sort once, keep the order (Recommended)",
          "description": "Lane `snapshot-order`, about six agent runs: one sort of the presented rows' places by key, kept for the next pass and checked against each row's key as it copies. Half 0.09 to 0.14 ms at 5,000 rows and 0.18 to 0.22 ms at 10,000: a pass of about 0.74 to 0.79 ms or 0.83 to 0.87 ms, so it holds at either count. A pass after a presented row comes, goes or moves sorts once and costs what the next option costs. Adds kept state and a rule for when it is stale."
        },
        {
          "label": "One sort every pass",
          "description": "The same lane with nothing kept between passes, about five agent runs: the places are sorted at every pass. Half 0.23 ms at 5,000 rows and 0.44 ms at 10,000: a pass of about 0.88 ms or 1.09 ms. It holds at 5,000 with about 0.1 ms left and misses at 10,000. No state, no stale rule, the same cost at every pass."
        },
        {
          "label": "Rewrite only changed rows",
          "description": "The lane also keeps each buffer's bytes and rewrites only the rows stamped since that buffer was last written, about eight agent runs. The cost follows change, but the worst case is the first option's copy, and each of the three buffers needs its own mark: more to prove. Not measured."
        },
        {
          "label": "No code: restate the criterion",
          "description": "Today's fill stays, a pass of about 1.2 ms at 5,000 presented rows and 1.7 ms at 10,000. Criterion 4 is read on the ring's part alone, 0.65 ms, and the snapshot gets a budget of its own. Contradicts the plan's decision 5.2, and D162's join margin counts on the whole pass."
        }
      ]
    },
    {
      "question": "No document says how many rows a scene presents. 10,000 came with `present-impl`'s cost test, with no reason. 5,000 is criterion 5's count, the only count of presented rows a criterion names, though those are a client's remote rows, which copy no transform. The file prints the snapshot's part at both counts whichever is judged. At how many presented rows is criterion 4 judged?",
      "header": "Count",
      "multiSelect": false,
      "options": [
        {
          "label": "5,000 judged, 10,000 printed (Recommended)",
          "description": "The count with a source in the architecture, weak as it is. The harder case stays on record beside it. With the kept order even a pass that sorts holds at 5,000, at about 0.9 ms. Milestone 3's plan restates the count if its scene presents more."
        },
        {
          "label": "10,000 judged, 5,000 printed",
          "description": "The count the two cost tests hold today, and the harder case. With the kept order the pass is about 0.83 to 0.87 ms, and about 1.1 ms at a pass that sorts, which is printed and not judged. With one sort a pass it is about 1.09 ms: a miss."
        },
        {
          "label": "Both judged",
          "description": "Criterion 4 must hold at 5,000 and at 10,000 presented rows. On the probe's figures only the kept order does, with 0.13 to 0.17 ms left at 10,000. Two figures to hold in the quiet window instead of one."
        }
      ]
    },
    {
      "question": "Criterion 5 (remote upload at most 1 ms for 5k entities at 144 Hz): on the M1 the job took 0.32 to 0.34 ms on a performance core at full clock, 0.55 to 0.65 ms on one at reduced clock and 1.4 to 1.9 ms on an efficiency core at reduced clock. macOS chose among them run by run, so the medians ran from 0.32 to 1.84 ms. Every run was started from a terminal, where the top QoS class is clamped. How is the criterion read?",
      "header": "Criterion 5",
      "multiSelect": false,
      "options": [
        {
          "label": "Performance cores, role taken (Recommended)",
          "description": "Lane `measure-role`, about six agent runs: `lactose-task` gives the tests two safe calls, the foreground task role (one FFI call its benchmark already makes) and the class of the current core (new FFI; section 21's entry reworded). Criterion 5 is the median of the frames whose job began and ended on a performance core, at full or reduced clock, with their share of the run. The class is read from the system, so there is no threshold to set after the data. One run, never repeated: with under a tenth of its frames on performance cores it judges nothing. Where macOS runs the engine's frames is measured before milestone 3, in five rounds or more."
        },
        {
          "label": "Foreground role, all frames",
          "description": "The tests take the role, the one FFI call, about four agent runs. The criterion stays as written, over all frames of the one run, with clock bands recorded. In the review's stand-in a job alone in the foreground role still began on efficiency cores in 1,060 and 1,161 of 1,440 frames in one state of the machine and in at most 1 in the other, so the verdict can follow that state."
        },
        {
          "label": "Clock band, no new code",
          "description": "Criterion 5 is the median of the frames whose calibration read under 1.25 times the run's fastest, the bound already in the lane's test, with their share. It cannot tell a performance core at reduced clock from an efficiency core, so both fall outside. One run, never repeated: a quiet M1 may leave under a tenth of its frames in the band, and then it judges nothing. The role stays clamped."
        },
        {
          "label": "Placement rounds first",
          "description": "Criterion 5 stays open on the M1. A phase of the executor's benchmark runs the four cases, two roles alone and in a busy frame, in five interleaved rounds or more, about five agent runs, before milestone 2 can exit. The reading is ruled on after it."
        }
      ]
    }
  ]
}
~~~

Two remedies for criterion 5 are not options, and the owner can still name either. A spike of a
parallel workgroup is twelve agent runs or more against a stand-in in which the cores were already
held. A criterion of 2 ms for the M1 writes the scheduler's slow state into the budget.

## Cost and order

| Step | Who | Agent runs |
|---|---|---|
| `measure-m2`'s fix rounds and merge: the clock bands, the control, the count stated | The lane, as briefed | Already counted |
| This proposal's review, its answer and the answer's review | `reviewer`, `planner`, `reviewer-diff` | 3 |
| The decision, section 2's amendment, the plan's lines and the rows, applied after `measure-m2`'s merge | The coordinator | None |
| `snapshot-order`: the fill, its tests, the count and the layout in the two cost tests | A lane from main, `reviewer`, `reviewer-diff`, `merger` | 6 |
| `measure-role`: the two calls, the role in the cost tests, the class in `remote-upload-5k` | A lane from main, `reviewer`, `reviewer-diff`, `merger` | 6 |
| The measurement commit's quiet window, the laptop's session and session C | As the plan has them | Already counted |
| The placement phase of `lactose-task`'s benchmark, five rounds or more | A small lane, before milestone 3's plan | 5 |

The two lanes run in parallel. They touch different files of `lactose-app-client`'s tests but for
the shared support, and the second to merge takes the first's. The quiet window comes after both
merge. Nothing here adds a quiet window.

## What the review changed

`reviewer` attacked the first version and found no blocking defect, five serious, five minor and
three nits. All are accepted.

| Finding | Answer |
|---|---|
| S1. The estimate of today's fill at 5,000 rows was wrong | Accepted. The estimate is gone. The probe's figures replace it: a half of 0.52 to 0.56 ms and a pass of about 1.2 ms. The plan's row for `extract-50k` states the layout |
| S2. The first question tied the count to the lane and left out the fill that keeps nothing | Accepted. The count is its own question. One sort a pass is remedy F and the first question's second option, with why A is recommended over it |
| S3. "Full clock" was not defined, its figures were a performance-core band, and re-runs had no limit | Accepted. The criterion's band is the class of core, read from the system, with no threshold. The clock bands keep the lane's bounds, stated in the plan's row. The run is the quiet window's one run. A performance core at reduced clock is named and counts |
| S4. The placement rule could not decide on one run a case | Accepted. Five interleaved rounds or more, the share on each class of core and each class's calibration a case and round, an outcome only where every round agrees, the rule read on the busy case by role, core and clock apart |
| S5. The second question's options were not fair | Accepted. The foreground role is priced on its own and offered twice, with the class of core and without. The workgroup spike is priced apart and is not an option. Each option states its bound and its one run. The placement option says rounds |
| M1. The place check proves the order and the set, not the columns | Accepted. D16x, remedy A and the fill's row say that the columns are read from the chunk at every pass |
| M2. SOUR's layout modes make A sort at every pass | Accepted. The count of sorting passes is asserted where the layout keeps rows in place and recorded under SOUR's modes. The equality test runs both kinds |
| M3. The reason for 5,000 described other rows, and D16x ruled out how it was chosen | Accepted. The count is said to be of another kind and to follow the diagnosis. Both counts are printed. "Chosen after its figure" is gone |
| M4. Section 2's sentence contradicts the runs | Accepted. Section 2 is amended now, as OLD and NEW |
| M5. The block at verification line 245 put the new rows before the lane's | Accepted. The two rows are appended at the end of the file, with no OLD line |
| N1. Row 19's span occurs twice in the file | Accepted. The spans are given whole, and each whole span occurs once in the file |
| N2. The estimate for A was pessimistic | Accepted. The measured 0.09 to 0.14 ms is quoted, a pass of 0.74 to 0.79 ms |
| N3. D16x should say why a kept place is not a key | Accepted. D16x's second paragraph and the fill's row say it |

## What this proposal could not verify

- Any figure. It ran no measurement: a lane was measuring on the machine. Every figure is from the
  lane's reports, the three reports of its first review and the probes of this proposal's review,
  and none was taken on a quiet machine.
- The fill's figures on the device. The review's probe wrote a counting null sink, and the pass
  figures add the lane's 0.65 ms ring to its halves.
- The engine's executor and timer root under the two conditions. The stand-in is plain threads, a
  condition variable and pthread QoS. What it shows of the role and of a busy frame is the
  placement row's to measure.
- That macOS numbers the efficiency cores first on machines other than the M1. No document says
  so. Lane `measure-role` shows it on each machine it runs on, or reports that it cannot tell.
- That a process can give the foreground role back. If it cannot, a case of the placement rounds
  is a process.
- The role a client started as an application bundle is given. The wake spike lists it as not
  tested.
- Game Mode, and the effect of a parallel workgroup on placement. The workgroup calls were read in
  the macOS 27.2 SDK's headers on the development Mac; nothing was linked or run.
- The lane's second fix round. The spans of rows 16, 19 and 102 were checked against its merge
  text as it stood after round three, which had not changed when this revision was written. The
  clock bands are described as its test holds them on its branch. If the round moves a span or a
  bound, the rows' text follows it.
- The desktop. Its CPU is unknown until its first sitting, so "one class of core" is the plan's
  expectation of it.
