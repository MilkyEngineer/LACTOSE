# Proposal: tick start on macOS 27.2.1

Status: ruled on 17 November 2026. It answers the milestone 2 plan's section 9, step 5, and its
row on a re-run that crosses a milestone 1 criterion (`docs/plans/milestone-2.md`): the owner has
ruled that such a crossing is a finding, that milestone 1 stays closed, and that the planner brings
a restatement or a lane (D154). Written against main at `097f489`. On the question at the end the
owner chose the recommended option, "Remeasure, then restate". The text of "Applied now" was applied
at this proposal's merge; the rest is applied by the coordinator on the remeasurement's result,
never by this proposal. Nothing in milestone 2's lanes waits for it.

## The finding

The Mac moved to macOS 27.2.1 on 17 November 2026. The coordinator's quiet window ran
`cargo xtask measure --all` at `a539d60`, and every criterion's figure passes but one. The timer
wake at 60 Hz is tick start, held to under 35 µs at P99 on macOS and Linux (architecture section
22; D138's addendum).

| 60 Hz timer wake, 5,000 deadlines | Load before | P50 | P99 | Max | Over 1 ms |
|---|---|---|---|---|---|
| `0ec2d90`, macOS 14.7.4 (`whisk-apple-m1-2026-11-16`) | 5.71 | 2.9 µs | 24.4 µs | 50.8 µs | 0 |
| `a539d60`, 27.2.1, in the full set (`whisk-apple-m1-2026-11-17`) | 4.58 | 8.5 µs | 36.2 µs | 1,211.8 µs | 1 |
| `a539d60`, 27.2.1, the benchmark alone, run 2 | 4.61 | 8.7 µs | 36.6 µs | 87.4 µs | 0 |
| `0ec2d90`, 27.2.1, the benchmark alone | 12.40 | 6.5 µs | 34.6 µs | 109.8 µs | 0 |
| `a539d60`, 27.2.1, the benchmark alone, run 3 | 2.60 | 8.3 µs | 38.3 µs | 118.2 µs | 0 |

The three runs of the benchmark alone followed the full set back to back, in the order of the
table. Their files are not in the tree at `097f489`; "Applied now" checks them in. Loads are the
benchmark's own one-minute averages.

## The diagnosis

**It is not the tree.** Between `0ec2d90` and `a539d60` the benchmark's code did not change:
`git diff 0ec2d90 a539d60` over `crates/task`, `crates/core`, the lock file, the toolchain file and
the cargo configuration shows comments, a README and a clippy configuration. The run at `0ec2d90`
on macOS 27.2.1 sits with the three at `a539d60` and not with its own file of the day before.

**It is not load as such.** The four checked-in files of the shipped keeper on macOS 14.7.4
(`whisk-apple-m1-2026-10-09`, `-10-10`, `-11-15`, `-11-16`) were taken at loads of 1.51 to 5.71 and
give P50 2.6 to 3.7 µs and P99 24.4 to 29.9 µs. The four runs on 27.2.1 give P50 6.5 to 8.7 µs
and P99 34.6 to 38.3 µs. The two sets do not meet at either figure, and the fastest run on 27.2.1
began at the highest load, 12.40.

**It is not the machine being away.** The benchmark logs every sample over 1 ms with its time
(verification row 79). Four runs list one, 1,211.8 µs at 09:57:36 UTC, and it is the maximum, not
the P99. `pmset -g log` holds no sleep, wake or dark wake after 18:23:34 local that evening; the
second of the sample holds only the audio daemon releasing 22 idle-sleep assertions. The Mac was
on AC power from 18:23, and low power mode is off (read at the time of writing, not at the runs).
The stall log shows the miss is the body of the distribution and cannot say what moved it.

**It is not the timed wait growing longer, by the 2 ms period's control.** The keeper steps its
timed park by 0.6 of the time left and spins the last 50 µs (`crates/task/src/sleep.rs`,
`Sleep::park`). With every wait on time, its last waits at 60 Hz are asked for 256, 102 and 41 µs
and the last is asked to end 27 µs before the deadline; at a 2 ms period they are 192, 77 and
31 µs, ending 20 µs before it. A wait that macOS 27.2.1 lengthened by its timeout, as coalescing
does, or by a fixed amount, would pass the deadline at 2 ms first. The 2 ms row's median did not
move. So the spin window is not too short for the median deadline, and lengthening it is not
shown to help. A wait that returns late because the core idled deeper over the longer gap is the
cost below, not the timer's. No probe confirms any of this: `keeper`'s probe build, which counted
the park exits that passed the deadline (43 of 3,020 on macOS 14.7.4, D138's addendum), was never
committed.

**What moved is the cost of work that starts after an idle gap, and it grows with the gap.**

| Row of the benchmark | Idle gap | macOS 14.7.4, four files | macOS 27.2.1, four runs | CPU per event, against `0ec2d90`'s file |
|---|---|---|---|---|
| Timer wake, 2 ms period | 2 ms | P50 1.4 to 2.1 µs, P99 20.5 to 23.6 µs | P50 2.1 to 2.3 µs, P99 25.5 to 26.4 µs | 8 to 9 µs more |
| Push to a parked pool | 2 ms | P50 10.1 to 13.3 µs, P99 30.4 to 32.8 µs | P50 14.3 to 14.8 µs, P99 33.0 to 33.8 µs | 8 to 9 µs more |
| Timer wake, 60 Hz | 16.7 ms | P50 2.6 to 3.7 µs, P99 24.4 to 29.9 µs | P50 6.5 to 8.7 µs, P99 34.6 to 38.3 µs | 28 to 34 µs more |

CPU per event is the row's process CPU per second divided by its events per second. At a 2 ms gap
both rows moved by 2 to 3 µs at P99 against the file at `0ec2d90`. At 16.7 ms the median more than
doubled and the P99 rose by 10 to 14 µs. The full set says the same of the rest of the engine
(`docs/results/*-apple-m1-2026-11-17.*` against the files of 15 and 16 November). The simulation's
sustained work is within 2% or faster: `tick-100k` on four workers 3,264.4 µs against 3,344.6,
the prediction-world snapshot 74.4 µs against 90.0, `spike-net`'s heaviest scene 7% faster per
rollback tick. Work that is a short burst on parked workers is slower: `spike-net`'s rollback
medians on its three light scenes rose 26 to 42%, 24 to 36 µs on about 100 µs. This is where
D113 put the wake's floor, the cores leaving idle, and where `keeper`'s probes put the lost time
on macOS 14.7.4: after the park, on the keeper's first work past the deadline (D138's addendum).
On macOS 27.2.1 that cost is larger and reaches the median. The cause inside the system is not
found.

**The daemon is the less likely cause, and it is not excluded.** Every run on 27.2.1 had
`ecosystemd` busy, a system daemon of `Ecosystem.framework` that runs as root: about 40% of one
core through the evening by the coordinator's reading (`097f489`'s message). By its CPU time at
the time of writing it has averaged 15% of a core over the fourteen and a half hours since boot
and held 28% over two minutes, so it is not quiet yet. Its threads run at priority 4, the
kernel's background band, which macOS keeps on the efficiency cores of Apple silicon (not
re-checked on 27.2.1: `powermetrics` needs root). The keeper is a frame-critical worker on a
performance core, and the performance cluster has its own cache. A daemon that took time or cache
from it would slow sustained work, which is faster, and would not choose its victims by idle gap.
An indirect path, through what the system does with an idle performance cluster while the
efficiency cluster is busy, is not excluded. All four runs had the daemon, so they cannot
separate the two.

**What tells them apart is a run with the daemon quiet, and the coordinator can take it in
minutes.** No new code, no lane and no root are needed. The daemon's CPU time is readable by any
user, so each run carries the daemon's share as a figure. If the daemon never goes quiet, the
question answers itself another way: a daemon the system runs for days is the system this machine
has.

## The remeasurement

It is the coordinator's, about 21 minutes of quiet window, and costs no agent run.

1. *Watch.* At any quiet point, read the daemon's CPU time twice, a minute apart:
   `ps -o time= -p "$(pgrep -x ecosystemd)"`. The daemon is quiet when it adds under one second of
   CPU in a minute, which is under 2% of a core.
2. *The window.* As soon as the daemon has been quiet for five minutes, or on 19 November 2026 as
   it is: every track stops, the Mac holds `caffeinate -i -s -d` on AC power with low power mode
   off (`pmset -g`), and three full runs follow back to back, each
   `cargo xtask measure whisk --out target/tick-start-<n>`. A full run takes about seven minutes.
   The daemon's CPU time and `date -u +%s` are read before and after each run. The runs are taken
   at main's head while no code of `lactose-task` or `lactose-core` has changed since `a539d60`,
   and otherwise in a worktree at `a539d60` (the plan's section 9, step 5).
3. *The reading.* For each run: the 60 Hz row's P50 and P99, the wake row's P99, and the daemon's
   share of a core over the run.

| Result | What it means | What follows |
|---|---|---|
| Every run under 30 µs at P99 | The M1 is back where it was on macOS 14.7.4, whose worst file gave 29.9 µs. The cause was the daemon or the hours after the upgrade | The finding closes on those files. Nothing is restated and no lane runs |
| A run at 30 µs or over, the daemon quiet | The system. A pass between 30 and 35 µs is the same state as a miss: four runs span 34.6 to 38.3 µs | The remedy the owner ruled |
| A run at 30 µs or over, the daemon never quiet by 19 November | Not separated. The daemon is then part of macOS 27.2.1 as this machine runs it | The remedy the owner ruled, and the row says the cause was not separated |
| A run over 42 µs, whatever the daemon does | The restated figure below no longer stands above the worst run by the spread | Back to the planner with the files, before any remedy |

The wake from an idle pool passes in all four runs with 1.2 to 2.0 µs to spare. It is not part of
this finding. If a run of the remeasurement puts it at 35 µs or over, that is a second crossing
under the same ruling, and it comes to the planner with these files; no figure is proposed for it
now.

The owner can separate the cause without waiting, by stopping the daemon for one run as root or by
restarting the Mac before the window. Neither is needed for the remedy, and no agent runs `sudo`.

## The options

**Remeasure, then restate if it still misses. Recommended.** The three runs above. If they do not
close the finding, tick start on macOS is restated at 50 µs at P99. It rules out choosing a remedy
on four runs that share one removable confound. It costs one quiet window and at most two days.

**Restate now.** Tick start on macOS becomes 50 µs on the four runs in hand. It rules out learning
whether the miss is transient before the architecture changes. If the daemon was the cause, the
criterion is then looser than the machine needs until a later file shows it. Architecture section 20
says a number taken while other work loads the machine steers and proves nothing, and whether a
busy system daemon is such work is the open point.

**A bounded lane in `lactose-task`.** One more attempt at the keeper, described below. It rules out
a restatement only if it succeeds, and it is chosen blind: on macOS 14.7.4 the one variant that
helped gained 2 to 6 µs at P99 (D138's addendum), which would put the M1 at 29 to 36 µs, on the
criterion again. It reopens D138, which ruled out a second attempt, and D113, which adopts no warm
sleeper until power can be measured. No agent can measure power on this machine.

**The recommendation is the first.** A restatement changes section 22, and it should rest on runs
that show the miss is the system's. If it is, the restatement is right for the reason D113 and
D138 gave: the figure is what the operating system gives a core that has idled, not what the
executor adds. Two facts would change it. If a milestone 2 design names a use that needs a tick or
a frame to start within 35 µs of its deadline on macOS, the lane is the remedy. The plan names no
such use, and `frame-design`, which reads the timer's unmeasured list (row 164), is where one
would appear. If a remeasured run is over 42 µs, 50 µs is the wrong figure and the planner is
asked again.

## The restatement

**The figure is 50 µs at P99 at 60 Hz, on macOS alone.** The worst full run on macOS 27.2.1 is
38.3 µs. The shipped keeper's runs on macOS 14.7.4 spanned 22.2 to 29.9 µs (verification row 81),
a spread of 7.7 µs under one system. 38.3 plus 7.7 is 46.0, rounded up to the next 5 µs. D138's
35 µs stood 5.1 µs above the worst run then on record, and one system upgrade used that up. Linux
keeps 35 µs and Windows 1 ms (D144): neither has a figure that says otherwise, and the desktop's
first sitting gives them (D151).

**No consumer needs 35 µs.** Lateness does not accumulate (`TickPacer::poll`, D144), 50 µs is
0.3% of a tick, and Windows is held to twenty times as much.

**It rules out** reading the criterion on macOS as a regression gate finer than about 12 µs. A
quiet window's file is read against the last one, as the plan's section 9 has it, and that reading
is what catches a smaller move; it caught this one. It rules out another attempt at the keeper for
this cause in milestone 2. It does not rule out the lead-up warm spin D113 left as a builder
option, for when a consumer needs it and power can be measured.

**It is read again at milestone 2's measurement commit.** If the M1's file there is under 30 µs,
the planner proposes 35 µs again. The files of macOS 14.7.4 stay milestone 1's proof (D154).

## The lane, if the owner prefers it

| Lane | What it does | Needs | Proof it owes | Runs |
|---|---|---|---|---|
| `keeper-warm` | One bounded attempt on macOS 27.2.1, in `lactose-task` alone; what ships is in `crates/task/src/sleep.rs`. It writes `keeper`'s throwaway probes again, since none was committed, and places the lost time: the park's exit against the first work after it. It then tries D113's lead-up warm spin as `keeper` measured it: on macOS the keeper spins the last 200 µs and reads one line of a 256 KiB buffer on each turn. Windows and Linux are untouched | The remeasurement's files, which are its base. No lane editing `crates/task` in flight; none of milestone 2's does | In quiet windows, three interleaved A/B pairs of full runs. It ships only if every changed run is under 30 µs at P99 at 60 Hz, the 60 Hz row's process CPU rises by at most 0.5% of a core, and the 2 ms row's rise is stated. `keeper` saw 0.25% and 4.4% on macOS 14.7.4. The loom models and Miri pass. If it fails, nothing ships and tick start is restated at 50 µs with the attempt recorded | 6: a small lane's 5, and 1 for `reviewer-concurrency` beside `reviewer` |

Its brief is the size of `keeper`'s. If it ships, architecture section 5.1's "spins the last 50
microseconds" and D113's "no warm sleeper" change with it, with its power cost stated as
unmeasured. Each A/B block stops the tracks as a quiet window does. The desktop's sittings are
not affected: the change is macOS's alone.

## Text for the coordinator (exact)

Every OLD below occurs exactly once in its file at `097f489`. An OLD or a NEW may span lines.

### Applied now, whatever the owner rules

**The re-run files.** The three pairs in the coordinator's scratch directory `qw-m2-1-reruns` are
copied byte for byte into `docs/results` under these names and added by name. They hold no machine
path. Nothing in `tools/xtask` reads a result file's name.

| From | To, in `docs/results` |
|---|---|
| `whisk-a539d60-run2.md`, `.json` | `whisk-apple-m1-2026-11-17-run2.md`, `.json` |
| `whisk-0ec2d90-on-27.2.1.md`, `.json` | `whisk-apple-m1-2026-11-17-at-0ec2d90.md`, `.json` |
| `whisk-a539d60-run3.md`, `.json` | `whisk-apple-m1-2026-11-17-run3.md`, `.json` |

The remeasurement's files are checked in the same way: the first full file of a UTC date under the
tool's name, a later one of that date with the next `-run<n>`. A short run is never checked in.

**The naming rule.** `docs/results/README.md`, line 12 at `097f489`. OLD:

~~~
## The milestone measurements
~~~

NEW:

~~~
A second file of one measurement on one machine and UTC date keeps the first file's name and adds
a suffix after the date that says what it is: `-run2`, `-run3` and so on for a repeat at the same
commit, `-at-<commit>` for a run at another commit. The tool writes every run under the plain
name, so a repeat is run with `--out` into a directory of its own and renamed when it is checked
in; its `Commit` and `Command` rows say what it ran. The row of `docs/verification.md` that cites
such a file says why it was taken.

## The milestone measurements
~~~

**The upgrade's row, and the finding's row after it.** `docs/verification.md`, line 172 at
`097f489`. The new row is one line. OLD:

~~~
From the upgrade on, a fix that changes measured code is measured on macOS 27.2.1, and its file says so | 2 |
~~~

NEW:

~~~
From the upgrade on, a fix that changes measured code is measured on macOS 27.2.1, and its file says so. **Done**, the re-run: the coordinator's quiet window of 17 November 2026 at `a539d60` wrote the 24 files `docs/results/*-apple-m1-2026-11-17.*` (`097f489`). Every criterion's figure passes but tick start's: `tick-100k` T₄ 3,264.4 µs against 4 ms (3,344.6 before); the physics phase 1,953.5 µs against 3 ms (1,954.3); the prediction-world snapshot 74.4 µs against 100 (90.0); `sim-skeleton` (b) 85.0 µs on four workers and 182.2 µs on one, against 500 (98.4 and 180.8); edit-to-playing 2.27 s against 30 (1.56); the makespan and the steal; the wake from an idle pool at P99 33.8 µs against 35 (31.0). `headless-120` prints the hash it printed. The simulation's sustained work is within 2% or faster. A short burst of work on parked workers is slower: `spike-net`'s rollback medians per tick rose 26 to 42% on its three light scenes under the clock script (108.6 to 137.0 µs, 87.0 to 123.2 µs and 86.8 to 111.2 µs) and fell 7% on fifty thrown bodies, recorded with no threshold. Tick start crosses its criterion, 36.2 µs against 35, and that finding is the row "Tick start on macOS 27.2.1 (D154)" | 2 |
| Tick start on macOS 27.2.1 (D154). The M1's re-run after the upgrade fails tick start's 35 µs (D138): `docs/results/whisk-apple-m1-2026-11-17.md` (`a539d60`, load averages 4.58 3.85 3.38 before and 2.97 3.97 3.67 after, the benchmark's own) gives at 60 Hz P50 8.5 µs, P99 36.2 µs, max 1,211.8 µs and one of 5,000 deadlines over 1 ms, against P50 2.9 µs and P99 24.4 µs at `0ec2d90` on macOS 14.7.4. Three full runs of the benchmark alone followed in the same window: `a539d60` twice, P50 8.7 and 8.3 µs, P99 36.6 and 38.3 µs (`whisk-apple-m1-2026-11-17-run2`, `-run3`), and `0ec2d90`, the old file's commit, P50 6.5 µs, P99 34.6 µs (`whisk-apple-m1-2026-11-17-at-0ec2d90`, load 12.40 before), none over 1 ms. It is not the tree: between the two commits no code of `lactose-task` or `lactose-core` changed, nor the lock file, the toolchain or the cargo configuration. It is not load as such: the four checked-in files of macOS 14.7.4, 9 October to 16 November at loads 1.51 to 5.71, give P50 2.6 to 3.7 µs and P99 24.4 to 29.9 µs, and the fastest of the four runs on 27.2.1 began at the highest load. The machine was not away: the one sample over 1 ms (09:57:36 UTC) falls on no sleep, wake or dark wake of `pmset -g log`. It is not the keeper's timed wait growing longer, by a control and not by a probe: the 2 ms period ends on waits of the same lengths with less time in hand before the deadline, and its median is 2.1 to 2.3 µs against 1.4 to 2.1, its P99 25.5 to 26.4 µs against 20.5 to 23.6. What moved is the cost of work that starts after an idle gap, and it grows with the gap: against the file at `0ec2d90` the process spends 8 to 9 µs more CPU per event in both rows at a 2 ms gap and 28 to 34 µs more at 60 Hz, and the wake from an idle pool is P99 33.0 to 33.8 µs against 30.4 to 32.8, still under 35. The cause inside the system is not found, and one thing is not separated from it: every run on 27.2.1 had a system daemon, `ecosystemd`, busy at background priority, about 40% of one core through that evening. Milestone 1 is not reopened (D154). The remedy is the owner's, on the question of `docs/proposals/tick-start-macos-27.md`. Its remeasurement is three full runs in a quiet window once the daemon is quiet, or on 19 November 2026 as it is, each with the daemon's share of a core over it: the finding closes with no change if every one is under 30 µs at P99 at 60 Hz, and otherwise tick start is restated on macOS or one bounded lane in `lactose-task` attempts it, as the owner rules | 2 |
~~~

**The tick-start row.** `docs/verification.md`, line 81 at `097f489`. OLD:

~~~
(P99 22.21 to 24.71 µs) are under it too.
~~~

NEW:

~~~
(P99 22.21 to 24.71 µs) are under it too. On macOS 27.2.1 the M1's file at `a539d60` gives 36.2 µs and fails it (`docs/results/whisk-apple-m1-2026-11-17.md`); the files of macOS 14.7.4 stay milestone 1's proof, and the finding is the row "Tick start on macOS 27.2.1 (D154)".
~~~

### After the remeasurement

One of these is appended to the text of the row "Tick start on macOS 27.2.1 (D154)", before its
` | 2 |`, with the figures in angle brackets filled from the files. Each begins with its space.

~~~
--- every run under 30 µs ---
 **Done**: the remeasurement of <date> at `<commit>` (`<files>`), the daemon at <share> of a core, gives at 60 Hz P50 <a> to <b> µs and P99 <c> to <d> µs, under 35 with the margin macOS 14.7.4 gave. Nothing is restated. The miss of 17 November was the daemon or the hours after the upgrade

--- a run at 30 µs or over, and the owner ruled a restatement ---
 **Done** by restatement (D<n>): the remeasurement of <date> at `<commit>` (`<files>`), the daemon at <share> of a core, gives at 60 Hz P50 <a> to <b> µs and P99 <c> to <d> µs. Tick start is held to 50 µs at P99 on macOS; Linux keeps 35 µs and Windows 1 ms. It is read again at milestone 2's measurement commit, and under 30 µs there brings 35 µs back as a proposal

--- a run at 30 µs or over, and the owner ruled the lane ---
 The remeasurement of <date> at `<commit>` (`<files>`), the daemon at <share> of a core, gives at 60 Hz P50 <a> to <b> µs and P99 <c> to <d> µs. Lane `keeper-warm` makes one bounded attempt (the proposal's row); if it fails, tick start is restated at 50 µs on macOS
~~~

### If tick start is restated

The number is the next free one at the merge, D156 at `097f489`. The architecture's revision marker
does not move: D138's restatement was a merge's amendment at `0ec2d90`, inside revision 3.3.

**Section 22, milestone 0's criteria.** `docs/architecture.md`, line 1433 at `097f489`. OLD:

~~~
under 35 µs at P99 (D138), and under 1 ms on Windows
~~~

NEW:

~~~
under 35 µs at P99 (D138), under 50 µs on macOS, where work that starts on a core that has idled costs more since macOS 27 (D156), and under 1 ms on Windows
~~~

**The executor's criteria table.** `crates/task/README.md`, line 235 at `097f489`. OLD:

~~~
under 35 µs at P99 (D138); on Windows under 1 ms (D144)
~~~

NEW:

~~~
under 35 µs at P99 (D138); on macOS under 50 µs (D156); on Windows under 1 ms (D144)
~~~

**The benchmark's threshold.** `crates/task/benches/whisk.rs`: `TIMER_P99_LIMIT_NANOS` becomes
50,000 on macOS, and its doc comment and the module's "Timer wake" line say so, as `b7cedcf`
changed the same three places for D138. It is one fix round to a lane, one run. It changes no
measured code, so no file is taken again; the next quiet window's file carries the new verdict.

**The decision entry.** Appended to `docs/decisions-log.md`. With "Restate now" the second sentence
of "Why" is left out.

~~~
**D156. On macOS tick start is held to 50 µs at P99** (§22, `lactose-task`, `docs/results`)
- Why: the development Mac moved to macOS 27.2.1 on 17 November 2026 (D154), and the M1's re-run
  there fails D138's 35 µs with the executor's code unchanged: at 60 Hz, P99 34.6 to 38.3 µs and
  P50 6.5 to 8.7 µs in four full runs (`docs/results/whisk-apple-m1-2026-11-17.md` and its three
  suffixed files), against 24.4 to 29.9 µs and 2.6 to 3.7 µs in the four files of macOS 14.7.4.
  The remeasurement of <date> at `<commit>` (`<files>`), with the system daemon of the first
  evening at <share> of a core, gave P99 <c> to <d> µs and P50 <a> to <b> µs. What moved is the
  cost of work that starts after an idle gap, and it grows with the gap: the 2 ms period, whose
  timed waits end as the 60 Hz period's do, moved by 2 to 3 µs at P99 and not at the median. The
  cause inside the system is not found.
  - *The figure is what the operating system gives*, as D113 set the push criterion and D138 this
    one. 50 µs is the worst full run on macOS 27.2.1, 38.3 µs, plus the 7.7 µs between the best
    and worst runs of the shipped keeper on macOS 14.7.4 (22.2 to 29.9 µs), rounded up to the
    next 5 µs.
  - *No consumer needs 35 µs.* Lateness does not accumulate (`TickPacer::poll`), 50 µs is 0.3% of
    a tick, and Windows is held to 1 ms (D144).
  - *Linux keeps 35 µs and Windows 1 ms.* Neither has a figure that says otherwise; the desktop's
    first sitting gives them (D151).
  - *The files of macOS 14.7.4 stay milestone 1's proof*, and milestone 1 is not reopened (D154).
  - *It is read again at milestone 2's measurement commit.* If the M1's file there is under
    30 µs, the planner proposes 35 µs again.
  - Rules out: another attempt at the keeper for this cause in milestone 2, the lead-up warm spin
    staying D113's builder option; reading the criterion on macOS as a regression gate finer
    than about 12 µs, since a quiet window's file is read against the last one
    (`docs/plans/milestone-2.md`, section 9).
- Driven by: the quiet window of 17 November 2026 (`097f489`);
  `docs/proposals/tick-start-macos-27.md`; the owner's ruling of <date>.
- Replaced: on macOS, the 35 µs of D138's addendum for tick start.
~~~

### If the lane is chosen

The row of "The lane, if the owner prefers it" is the lane's brief. It runs beside milestone 2's
queues and fills a free slot. Its own addendum to D138 records the attempt, and it brings the
text for section 5.1 and D113 if it ships.

## The owner's question

The coordinator asks the first. The second is asked only if the owner answers "Remeasure, then ask
me" and the remeasurement does not close the finding; its figures in angle brackets are filled
from the files.

```json
{"questions":[
{"header":"Tick start","question":"After the upgrade to macOS 27.2.1 the M1 misses one milestone 0 criterion. Tick start, the 60 Hz timer wake, is 34.6 to 38.3 µs at P99 in four runs against 35 µs; it was 24.4 µs on macOS 14.7.4 with the same executor code. Every other figure passes. The likely cause is the system: work that starts on an idle core costs more than it did. Every run also had a post-upgrade system daemon busy, so that is not yet separated. Milestone 1 stays closed either way. What is the remedy?","multiSelect":false,"options":[
{"label":"Remeasure, then restate (Recommended)","description":"The coordinator takes three full runs in a quiet window once the daemon is quiet, or on 19 November as it is: about 21 minutes with the tracks stopped, no agent runs. If every run is under 30 µs the finding closes and nothing changes. If not, tick start on macOS is restated at 50 µs at P99: the worst run plus the spread seen on macOS 14.7.4. Linux keeps 35 µs and Windows 1 ms. Nothing needs 35 µs: lateness does not accumulate, and 50 µs is 0.3% of a tick. It is read again at milestone 2's measurement commit."},
{"label":"Remeasure, then a lane","description":"The same three runs. If they do not close it, one bounded lane in `lactose-task`, about 6 agent runs beside milestone 2's lanes: the keeper spins longer before a deadline and keeps its core warm. It ships only if every run is then under 30 µs. It burns CPU while idle (0.25% of a core at 60 Hz when last tried) and a power cost no agent can measure, and it reopens D138, which ruled out a second attempt. If it fails, 50 µs."},
{"label":"Restate now at 50 µs","description":"No remeasurement. Tick start on macOS becomes 50 µs at P99 on today's four runs. If the daemon was the cause and goes quiet, the criterion stays looser than the machine needs until milestone 2's measurement commit reads it again."},
{"label":"Remeasure, then ask me","description":"The same three runs. If they do not close the finding, the question comes back to you with their figures."}]}
]}
```

```json
{"questions":[
{"header":"Tick start","question":"The remeasurement did not close tick start on macOS 27.2.1: three full runs give <c> to <d> µs at P99 against 35 µs, with the system daemon at <share> of a core. Milestone 1 stays closed. What is the remedy?","multiSelect":false,"options":[
{"label":"Restate at 50 µs (Recommended)","description":"Tick start on macOS is held to 50 µs at P99: the worst run plus the spread seen on macOS 14.7.4. Linux keeps 35 µs and Windows 1 ms. Nothing needs 35 µs: lateness does not accumulate, and 50 µs is 0.3% of a tick. It is read again at milestone 2's measurement commit."},
{"label":"A bounded lane","description":"One lane in `lactose-task`, about 6 agent runs beside milestone 2's lanes: the keeper spins longer before a deadline and keeps its core warm. It ships only if every run is then under 30 µs. It burns CPU while idle and a power cost no agent can measure, and it reopens D138. If it fails, 50 µs."}]}
]}
```

## What this proposal did not verify

- Nothing was measured for it: three lanes were building. Every figure is from a file in the tree
  at `097f489`, from the three re-run files, or from the quiet window's comparison of the two sets.
  The daemon's CPU time, the power log and the power settings were read on the Mac while writing.
- The cause inside macOS 27.2.1. No probe ran. "The cost of work after an idle gap" is read from
  the benchmark's three rows, the process's CPU time and the full set, not from the cores'
  idle states or frequencies, which need root to read.
- That the daemon's threads stay on the efficiency cores on macOS 27.2.1, what the daemon does,
  and whether it ever goes quiet.
- That the timed wait is not later at the P99. The 2 ms row is a control for the median, and a
  weaker one for the tail.
- That a warm spin would help on macOS 27.2.1. The lane's expected gain is `keeper`'s on macOS
  14.7.4.
