# The keeper's timed wait on Windows, and what tick start is held to there

Status: proposal, not applied. Written at main `88eecf1`, architecture revision 3.3. Nothing of
section 7 is applied until the owner answers section 6's question. This document changes no
code.

A Windows 11 laptop ran `cargo xtask measure --all` at `bef3455`. Its WHISK file
(`docs/results/whisk-laptop-windows-2026-11-15.md`) puts tick start at P50 4.3 ms and P99
15.9 ms at 60 Hz, against 20 µs, and at a 2 ms period 4,998 of 5,000 deadlines are over 1 ms
late. Section 1 gives the cause from the standard library's source. Section 2 says what
milestone 0's tick-start criterion, carried by D138, asks of Windows, and what the documents
leave open. Section 3 gives the options and section 4 the recommendation. Section 5 treats the
laptop's other two failures, the wake from a parked pool and the makespan, which are not this
cause, and carries the probe brief that would split them. Section 6 is the question for the
owner and section 7 the text for the coordinator: the decision entry D144, the amendments and
the verification rows.

Conventions:

- The laptop is the owner's test box: an i7-12650H, six performance and four efficiency cores,
  Windows 11 25H2 on the Balanced power plan, on AC. It proves no criterion
  (`docs/verification.md`, the row "Section 22's milestone 1 timing criteria"). Every Windows
  figure in this document is the laptop's, and every one steers.
- Probe 2 is the laptop's first timer probe: a scratch program outside the repository, run
  twice, once from a hidden process and once in a visible console, 1,000 samples a row, with
  10 to 46% of the processor busy with other work. It measured single waits, and the stepped
  wait to a deadline set from the previous wait's return. A range spans both runs.
- Probe 3 is its second: the stepped wait to a fixed schedule of deadlines, for four waits and
  six spin thresholds, 1,000 deadlines a row, every row run twice, from a hidden process at
  normal priority, with 14 to 31% of the processor busy. A pair "a / b" is the reported run and
  the run nine minutes before it. In the earlier run another process switched the system timer
  between 15.625 and 8 ms during the waitable timer's rows.
- The coordinator holds both probes' sources and outputs.
- Paths under `library/std/src` are the pinned toolchain's standard library, 1.99.0, as
  `rust-src` installs it. The laptop read the same calls in the rendered sources of 1.99.0 and
  1.100.0.
- `timer-windows` is this document's handle for a lane (section 4). It is not a crate name.
- A verification row is cited by its opening words.

## 1. The cause

`Sleep::park` (`crates/task/src/sleep.rs`, lines 86 to 106) waits for a deadline in steps.
While more than 50 µs are left it calls `park_timeout` for 0.6 of what is left, and inside the
last 50 µs it spins. The 0.6 answers macOS, which lengthens a timed wait in proportion to its
timeout (D113). The loop assumes that a wait asked for a fraction of a millisecond lasts about
that long. On Windows it does not.

**`park_timeout` ends in `WaitOnAddress`, with its timeout rounded up to whole milliseconds.**
`std::thread::park_timeout` (`thread/functions.rs`, line 596) calls the thread's parker
(`thread/thread.rs`, line 131), which on every Windows target but `win7` is the futex parker
(`sys/sync/thread_parking/mod.rs`, lines 1 to 17):

```rust
// sys/sync/thread_parking/futex.rs, lines 68 to 75
pub unsafe fn park_timeout(self: Pin<&Self>, timeout: Duration) {
    // Change NOTIFIED=>EMPTY or EMPTY=>PARKED, and directly return in the
    // first case.
    if self.state.fetch_sub(1, Acquire) == NOTIFIED {
        return;
    }
    // Wait for something to happen, assuming it's still set to PARKED.
    futex_wait(&self.state, PARKED, Some(timeout));

// sys/sync/futex/windows.rs, lines 66 and 67, inside wait_on_address
let timeout = timeout.map(dur2timeout).unwrap_or(c::INFINITE);
c::WaitOnAddress(addr, compare_addr, size, timeout) == c::TRUE

// sys/pal/windows/mod.rs, lines 240 to 254
pub fn dur2timeout(dur: Duration) -> u32 {
    // Note that a duration is a (u64, u32) (seconds, nanoseconds) pair, and the
    // timeouts in windows APIs are typically u32 milliseconds. To translate, we
    // have two pieces to take care of:
    //
    // * Nanosecond precision is rounded up
    // * Greater than u32::MAX milliseconds (50 days) is rounded up to INFINITE
    //   (never time out).
    dur.as_secs()
        .checked_mul(1000)
        .and_then(|ms| ms.checked_add((dur.subsec_nanos() as u64) / 1_000_000))
        .and_then(|ms| ms.checked_add(if dur.subsec_nanos() % 1_000_000 > 0 { 1 } else { 0 }))
```

So a step of 0.3 ms is asked as 1 ms, one of 1.2 ms as 2 ms, and the 10.0002 ms that is 0.6 of
a 60 Hz period as 11 ms. The unpark is `WakeByAddressSingle` (`sys/sync/futex/windows.rs`,
lines 71 to 76). Nothing in `library/std` raises the timer resolution: it holds no call of
`timeBeginPeriod` or `NtSetTimerResolution`.

**The wait then ends on the system's timer tick.** That is measured, not read. Under the
default timer, which `NtQueryTimerResolution` reported as 15.625 ms, probe 2's `park_timeout`
overshot by the tick less the request:

| Asked (ms) | 0.5 | 1 | 2 | 5 | 10 |
|---|---|---|---|---|---|
| Overshoot, P50 (ms) | 15.4 to 15.5 | 14.9 to 15.0 | 13.9 to 14.0 | 10.8 to 11.0 | 5.8 to 6.0 |

Every wait lasted 15.8 to 16.0 ms at the median, whatever was asked. On probe 3's fixed
schedule the shipped wait passed a 60 Hz deadline by P50 7.9 ms and P99 15.7 to 15.8 ms, 931 to
936 of 1,000 by more than 1 ms. At a 2 ms period it never reached 866 to 871 of 1,000
deadlines at all, since each wait outlasts seven of them. The benchmark's own 60 Hz median is
lower, 4.3 ms. Its schedule is not fixed, since it moves a late deadline half a period ahead
of the resume (`crates/task/benches/whisk.rs`, line 598), and its wait runs inside the
executor. The difference is not analysed.

**`std::thread::sleep` is another call, and no waker can end it.**

```rust
// sys/thread/windows.rs, lines 117 to 122
pub fn sleep(dur: Duration) {
    fn high_precision_sleep(dur: Duration) -> Result<(), ()> {
        let timer = WaitableTimer::high_resolution()?;
        timer.set(dur)?;
        timer.wait()
    }

// sys/pal/windows/time.rs, lines 83 to 88, 97 and 98, and 103
c::CreateWaitableTimerExW(
    null(),
    null(),
    c::CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
    c::TIMER_ALL_ACCESS,
)
let time = checked_dur2intervals(&duration).ok_or(())?.neg();
let result = unsafe { c::SetWaitableTimer(self.handle, &time, 0, None, null(), c::FALSE) };
let result = unsafe { c::WaitForSingleObject(self.handle, c::INFINITE) };
```

The duration is rounded down to units of 100 ns (`checked_dur2intervals`, lines 9 to 15). Each
call makes a timer and closes it. The handle never leaves the call, so nothing can end the
wait early. It falls back to `Sleep`, in whole milliseconds, only where the timer cannot be
made, which is before Windows 10 version 1803 (lines 127 to 131). Probe 2's `sleep` overshot by
P50 0.39 to 0.65 ms and P99 0.85 to 1.42 ms under the default timer, for every request from
0.5 to 10 ms.

**Every timed wait of the standard library that another thread can end is tick-bound on
Windows.** There are three, and each reaches `futex_wait` and so `WaitOnAddress` through
`dur2timeout`:

| Wait | Where it reaches `futex_wait` or `park_timeout` |
|---|---|
| `thread::park_timeout` | `sys/sync/thread_parking/futex.rs`, line 75 |
| `Condvar::wait_timeout` and its variants | `sys/sync/condvar/futex.rs`, line 49; `sys/sync/condvar/mod.rs` selects it for Windows |
| The channels' `recv_timeout`, `recv_deadline` and `send_timeout` | `sync/mpmc/context.rs`, line 133, `park_timeout` |

`thread::sleep` and `thread::sleep_until` are the only finer waits, and neither can be ended.
The shipped engine has one timed wait that a waker ends: `Sleep::park`. Linux and macOS are
not affected in this way: their timed parks take nanoseconds (`sys/sync/futex/unix.rs`, lines
28 to 30, an absolute `timespec`; `sys/sync/thread_parking/darwin.rs`, lines 90 and 91).

**A 1 ms timer period does not cure the rounding.** With `timeBeginPeriod(1)` held, probe 2's
`park_timeout` overshot by P50 0.28 to 1.07 ms and P99 1.26 to 2.03 ms, and returned up to
0.93 ms before the time asked. A last step under a millisecond is still asked as one
millisecond. Another process's 1 ms period did not shorten this process's waits, as far as
probe 2 can tell. In its visible run the system-wide resolution read 15.625 ms when the
default-timer rows began and 1.0 ms when they ended, and the rows taken last were still passed
by P50 13.7 and 15.1 ms. When the other request began is not known. Microsoft's page for
`timeBeginPeriod` says what the probe saw: from Windows 10 version 2004 a request holds for
the processes that made it.

## 2. What the criteria ask of Windows

**The texts.** Architecture section 22, milestone 0: "Tick-start latency, from a tick deadline
to the timer root resuming, under 20 µs at P99". The sentence names no machine. The makespan
sentence before it names "a developer M-series Mac and an x86 desktop", and section 20 says
bare-metal measurements run on those two. D113 set the 20 µs from the M1's spike alone. D138
carries it: "The x86 half closes in one session on the owner's desktop", whose files "pass or
fail makespan, wake, tick start and steal on x86". The plan (`docs/plans/milestone-1.md`,
section 4) runs session B "on each operating system it boots" and gives WHISK on the desktop
"the same" thresholds as on the M1.

**The reading.** The criterion binds on Windows. No text exempts an operating system, section
2 makes Windows the x86 client platform, and the timer root paces a client. The row "Windows
efficiency-core steering" already has the desktop measure a Windows setting.

**What the documents do not settle.**

- Which operating systems the desktop boots. The plan says they "come back with session A's
  files", and no file of session A is in `docs/results` at `88eecf1`.
- Whether a Windows failure is `keeper`'s. The plan sends a failure of session A "to
  `keeper`". D138 makes `keeper` one bounded attempt, proven on a quiet M1 for at most 1.5% of
  a core, and rules out a second. The M1 misses by under 10 µs for a cause nobody knows.
  Windows misses by 15 ms for a cause section 1 states. They are different work.
- What Windows is held to. The owner's restatement at 35 µs, should `keeper` fail, was decided
  for the M1's miss. Section 3 shows that on the laptop nothing short of a keeper that never
  waits reaches 20 µs or 35 µs.

**What a Windows `whisk` file proves at `bef3455`.** Its timer rows fail on any machine: the
tick is the operating system's, and Microsoft documents that since Windows 10 version 2004
another process's request does not shorten it. Its steal row is unaffected. Its wake row has
no timed wait on its path (section 5). Its makespan rows are taken after gaps of a tick, not
of the 1 ms the benchmark asks for, because the gap is a `sleep_until` through the keeper
(`crates/task/benches/whisk.rs`, line 470). So the file fails tick start whatever the desktop
is, and neither `keeper` nor the 35 µs restatement closes it. Session A at main's head reads
the same.

## 3. The options

**What the laptop measured.** Probe 3, the stepped wait of `Sleep::park` (0.6 of what is
left, a spin inside the threshold) to a fixed 60 Hz schedule, at the thresholds that matter.
The spin is its mean per deadline as a share of the period, which is the share of one core.

| Timed wait | Spin threshold (µs) | P50 (µs) | P99 (µs) | Over 35 µs, of 1,000 | Over 1 ms | Spin (% of one core) |
|---|---|---|---|---|---|---|
| `park_timeout`, default timer (shipped) | 50 | 7,875 / 7,889 | 15,740 / 15,824 | 995 / 998 | 936 / 931 | 0.00 |
| `park_timeout` under `timeBeginPeriod(1)` | 50 | 814 / 780 | 1,968 / 1,982 | 949 / 953 | 379 / 351 | 0.00 |
| | 500 | 418 / 355 | 1,522 / 1,563 | 730 / 690 | 194 / 192 | 0.41 / 0.40 |
| | 1,000 | 70 / 0.7 | 1,395 / 1,339 | 517 / 478 | 108 / 78 | 1.45 / 1.51 |
| | 2,000 | 0.2 / 0.3 | 950 / 1,097 | 186 / 196 | 4 / 11 | 6.17 / 5.75 |
| `std::thread::sleep` | 50 | 255 / 252 | 1,002 / 1,230 | 839 / 854 | 11 / 19 | 0.02 / 0.01 |
| | 250 | 68 / 75 | 1,028 / 983 | 544 / 554 | 12 / 10 | 0.29 / 0.27 |
| | 500 | 0.3 / 0.4 | 556 / 674 | 241 / 328 | 3 / 1 | 0.92 / 0.86 |
| | 1,000 | 0.4 / 0.3 | 591 / 722 | 244 / 255 | 1 / 8 | 1.17 / 1.30 |
| | 1,500 | 0.4 / 0.3 | 623 / 711 | 241 / 189 | 3 / 6 | 1.65 / 1.69 |
| | 2,000 | 0.2 / 0.3 | 241 / 361 | 85 / 100 | 2 / 0 | 7.72 / 7.77 |
| High-resolution waitable timer | 50 | 303 / 303 | 1,013 / 1,057 | 857 / 870 | 11 / 13 | 0.01 / 0.01 |
| | 250 | 53 / 88 | 842 / 3,196 | 528 / 585 | 3 / 18 | 0.30 / 0.26 |
| | 500 | 0.5 / 0.1 | 589 / 493 | 315 / 231 | 0 / 1 | 0.77 / 1.11 |
| | 1,000 | 0.3 / 0.0 | 556 / 607 | 173 / 84 | 1 / 9 | 1.52 / 1.98 |
| | 1,500 | 0.3 / 0.0 | 570 / 333 | 246 / 83 | 1 / 2 | 1.67 / 1.97 |
| | 2,000 | 0.3 / 0.1 | 319 / 143 | 103 / 56 | 2 / 0 | 7.68 / 6.86 |

What the table shows, each point the laptop's:

- No wait reaches 20 µs or 35 µs at P99 with a threshold in the sweep. The only rows under
  20 µs at P99 are a 2 ms threshold at a 2 ms period, which is a keeper that never waits: a
  whole core, and even there 3 to 9 deadlines of 1,000 were over 20 µs late.
- `sleep` and the waitable timer are alike at every threshold. They are the same timer.
- 500 µs is the knee. The median falls from 0.25 to 0.30 ms to under a microsecond, P99 from
  1.0 to 1.2 ms to 0.49 to 0.67 ms, and deadlines over 1 ms late from 11 to 19 to at most 3,
  for 0.77 to 1.11% of a core. A quarter to a third of deadlines are still over 35 µs late.
- 1,000 and 1,500 µs buy nothing both runs agree on, for 1.2 to 2.0% of a core. The same row's
  late count varies up to twofold between the runs.
- 2,000 µs costs 6.9 to 7.8% of a core and still leaves 56 to 103 deadlines of 1,000 over
  35 µs.
- `park_timeout` under a 1 ms period is worse than both at every threshold.
- Why deadlines are late, from probe 3's samples. Up to 1,500 µs, 91 to 100% of the late
  deadlines never entered the spin: the last wait itself returned after the deadline. At
  2,000 µs three quarters of them had spun and were late all the same. The laptop takes that
  for the spinning thread losing its core to the background load. That is a hypothesis.

The timer's single waits, in probe 2, overshot by P50 0.29 to 0.71 ms and P99 0.88 to 1.69 ms
for every request from 0.5 to 10 ms. The overshoot does not grow with the request. It fits a
timer that expires on the system's finest clock period, which `NtQueryTimerResolution`
reported as 0.5 ms, followed by a wake from idle; that reading is a hypothesis. The 0.6 step
was made for a leeway that grows with the timeout. Here each step is another draw from a
fixed overshoot, and the last step leaves 0.4 of what was left to absorb it. A last wait
aimed at the deadline less a fixed margin is untested (section 5's brief, part B).

**(a) No FFI: the coarse wait through `std::thread::sleep`, in bounded slices.**

- Reaches: what the waitable timer reaches, row for row; neither 20 µs nor 35 µs.
- Wake path: none inside a slice. A claimed bit, or a new earlier deadline, is seen when the
  slice ends. A push that claims the keeper waits out the slice and its overshoot, and on a
  one-worker frame pool (D111) the keeper is every push's only sleeper. Keeping the keeper out
  of `notify_one` while another sleeper exists changes the modelled protocol.
- Cost: a slice of 1 ms wakes the keeper several hundred times a second for as long as a
  timer is pending, which at 60 Hz is always. D113 declined a thread that wakes to keep cores
  shallow. Each slice also makes and closes a timer.
- Code: `lactose-task`, safe code, no manifest change.
- Runs: 8 to 10. The concurrency reviewer reads it, and the loom models change if the keeper
  is kept out of `notify_one`.
- A game process pays wake latency on Windows: up to a slice for a push or a timer that needs
  the keeper.

**(b) A high-resolution waitable timer beside a wake event.**

- Reaches: P50 0.30 ms and P99 1.0 to 1.1 ms at 60 Hz with the shipped step and spin; with
  (d), the rows above. Neither 20 µs nor 35 µs.
- Wake path: each frame-critical worker owns the timer and an auto-reset event. The step is a
  `WaitForMultipleObjects` on both. A waker that claims a sleeper's bit sets that worker's
  event beside the unpark it makes today. A signal that arrives late costs one more trip round
  `Sleep::park`'s loop, as a late park token does (`crates/task/src/sleep.rs`, lines 19 to 21):
  the cleared bit is the wake-up.
- Cost: no spin of its own. Two handles for each frame-critical worker. One `SetEvent` for
  each wake of a claimed sleeper, on Windows only; its cost is not measured (section 5's
  brief, row W7, and the lane's benchmark).
- Code: `lactose-task`, which architecture section 21 already lists and which already calls
  Windows through `windows-sys` for the core topology (`crates/task/src/platform.rs`). Six FFI
  calls. `crates/task/Cargo.toml` adds the features `Win32_System_Threading` and
  `Win32_Security` to its `windows-sys` line. No package is added: a scratch crate with those
  features was checked for `x86_64-pc-windows-msvc` and locks `windows-sys` 0.61.2 and
  `windows-link` 0.2.1 alone, both already in `Cargo.lock`; without `Win32_Security` neither
  `CreateWaitableTimerExW` nor `CreateEventW` resolves. `lactose-core` is the other section 21
  crate every binary links, and it is inside the boundary, where a wait on a clock does not
  belong.
- Manifest: the change moves the CI cache key, so its merge is pushed alone (D125).
- Runs: 8. Seven for the lane, and the concurrency reviewer beside its first review.
- A game process pays nothing it can see. Nothing is process-wide.

**(c) `timeBeginPeriod(1)` for the process's life.**

- Reaches: P50 0.78 to 0.81 ms and P99 2.0 ms at 60 Hz with the shipped step and spin, and
  with (d) still P99 0.95 to 1.10 ms at a 2 ms threshold: worse than (b) at every threshold,
  because the rounding to a millisecond stays. Neither 20 µs nor 35 µs.
- Wake path: unchanged, the unpark.
- Cost: a 1 ms clock interrupt for as long as the process runs. Microsoft's page: it "can also
  reduce overall system performance, because the thread scheduler switches tasks more often",
  and high resolutions "can also prevent the CPU power management system from entering
  power-saving modes". The same page: "Starting with Windows 11, if a window-owning process
  becomes fully occluded, minimized, or otherwise invisible or inaudible to the end user,
  Windows does not guarantee a higher resolution than the default system resolution." The
  laptop's hidden process kept its 1 ms; a minimised game window is the case the page names,
  and it was not measured.
- Code: two FFI calls and the feature `Win32_Media`, which imports `Winmm.dll`. The setting is
  the process's, so a library sets it for its host: the executor's builder in `lactose-task`,
  paired at shutdown.
- Runs: 7.
- A game process pays a 1 ms timer on the whole process, and loses it when its window is
  minimised.

**(d) A spin threshold per platform, on top of any of them.**

- Reaches, on (a) or (b): at 500 µs, a median under a microsecond and P99 0.49 to 0.67 ms at
  60 Hz. At 2,000 µs, P99 0.14 to 0.36 ms. Neither 20 µs nor 35 µs at any threshold short of
  the period.
- Wake path: immediate during the spin, which re-reads the bit and the clock on every turn.
- Cost: one performance core at full power for the spin, every deadline, whenever the frame
  workers are otherwise idle: a capped or idle client, a headless run. At 500 µs that is 130
  to 185 µs a deadline, 0.77 to 1.11% of a core at 60 Hz, and in proportion more where the
  timer root also paces frames. At 2,000 µs it is 6.9 to 7.8%. A busy pool pays nothing,
  since a busy worker fires timers when it looks for work. The power cost is not measured.
- Code: a constant in `crates/task/src/sleep.rs`. Safe code.
- Runs: none beside another option's lane; 5 alone.
- D113 declined a warm sleeper at 1.3 to 1.9% of a core, its power cost unmeasured. D138
  reopened it for the keeper at no more than 1.5%. 500 µs is inside that. 2,000 µs is five
  times it and does not buy the criterion.

## 4. Recommendation

**(b) with (d) at 500 µs, in milestone 1: on Windows the keeper's step is a wait on a
high-resolution waitable timer and a wake event, it spins the last 500 µs, and tick start on
Windows is held to 1 ms at P99.**

- (b) is the one option that keeps the wake path. The executor's wake criterion and its
  re-arming for an earlier deadline both rest on a waker ending the keeper's wait. (a) gives
  that up for a millisecond at a time, to avoid FFI in a crate that already has it.
- (c) is worse than (b) on every figure and is process-wide.
- 500 µs is the one threshold that pays. For about 1% of a core while the frame workers are
  idle, two ticks in three start on time and ninety-nine in a hundred within 0.7 ms. Without
  it the median tick starts 0.3 ms late. The cost is inside the 1.5% D138 allowed the keeper,
  and near what the keeper costs on the M1: 0.0108 s of process CPU a second at 60 Hz
  (`docs/results/whisk-apple-m1-2026-11-15.md`).
- The 20 µs is not pursued on Windows. Nothing the laptop ran reaches it short of a whole
  core, and a 2 ms threshold spends five times D138's allowance and misses. The engine
  tolerates a tick that starts under a millisecond late. Deadlines are absolute, so lateness
  does not accumulate (`crates/task/src/time.rs`, `TickPacer::poll`). A busy pool fires a
  timer when a worker next looks for work, so under load tick start already waits out a job.
- 1 ms at P99 is set at what the operating system gives, as D113 set the push criterion: the
  laptop's four rows at 500 µs are 0.49 to 0.67 ms, with a seventh to a third of its processor
  busy. It is a criterion of the 60 Hz row. macOS and Linux keep 20 µs.
- The step stays 0.6 on every platform, so what ships on Windows is the loop probe 3 ran.
- It is a lane of its own, `timer-windows`. `keeper` stays D138's one attempt at the M1's two
  states, and it waits for session A. This lane waits for nothing.
- It is milestone 1's because D138 makes tick start an exit condition and section 4 of the
  plan runs session B on Windows if the desktop boots it. Without the lane that file's timer
  rows fail by three orders of magnitude and its makespan rows are of the wrong gap.

**What would change it.**

- The lane's own measurement. Probe 3 emulates the loop on one thread at normal priority; the
  keeper is a frame-critical worker inside the executor. The lane runs the benchmark on the
  laptop with the threshold at 50 and at 500 µs. If the 500 µs build's process CPU at 60 Hz
  is more than 0.015 s a second above the 50 µs build's, which is 1.5% of a core, or its P99
  is not under 1 ms, the threshold stays 50 µs and Windows is held to 2 ms, recorded as an
  addendum to D144. The owner is not asked again.
- The desktop booting no Windows. Then no session of milestone 1 reads a Windows timer row,
  the lane moves to the head of milestone 2, where the first Windows client needs it, and the
  row is gated there (section 6, answer 4).
- The desktop's own file missing 1 ms on a quiet machine. Then the figure returns to the owner
  with that file.
- Part B of section 5's brief showing a wait asked once that holds P99 under 35 µs inside
  1.5% of a core. Then Windows keeps the M1's criterion. The deadlines that spun and were
  still late at 2,000 µs make this unlikely.

**The lane, `timer-windows`.**

- Scope: `lactose-task` alone. On Windows, outside loom and Miri, each frame-critical worker
  makes a high-resolution waitable timer and an auto-reset event when it starts and closes
  them when it exits. The `sync` shim's timed park becomes a wait on both. `Pool::wake`,
  `Pool::wake_all` and the wake of `Pool::announce_entry` set the claimed worker's event
  beside the unpark. The spin threshold is 500 µs on Windows and 50 µs elsewhere. A worker
  whose timer cannot be made, before Windows 10 version 1803, keeps
  `std::thread::park_timeout` and logs it once, as a thread class that cannot be applied is
  logged. Nothing changes on another platform, under loom or under Miri.
- With it: `crates/task/Cargo.toml`'s two features; `SAFETY.md`'s section for the calls; the
  README sentences of section 7; the benchmark's timer threshold on Windows, 1 ms; and the
  benchmark's process CPU column on Windows, which is `null` today because
  `process_cpu_seconds` is Unix-only, so the file shows what the keeper costs.
- Proof it owes, by tests that run on CI's windows-x64 job:
  - The defect: the median lateness of fifty 2 ms deadlines through `sleep_until`, on an
    otherwise idle pool, is under half a default tick, 7.8 ms. Today's code gives about 14 ms
    and the fixed code under 1 ms, so the bound is far from both.
  - The wake path, by bounded waits and not by timing: on a one-worker frame pool, a keeper
    waiting on a deadline ten seconds away is ended by an earlier timer, by pushed work and by
    shutdown. Each test fails when the event is not set.
  - The loom models pass unchanged, and so does Miri.
- Gates beside the standard ones: the Windows cross-target clippy for `lactose-task`;
  `cargo xtask features`, since the features of `windows-sys` reach the boundary's closure
  through `lactose-core`; the merge pushed alone (D125).
- Measurement, on the laptop as labelled data: `cargo bench -p lactose-task --bench whisk` at
  the lane's base, and at its head with the threshold at 50 and at 500 µs. The timer rows
  show the fix and decide the threshold. The makespan rows show how much of their failure was
  the tick-long gap. The wake row shows what the `SetEvent` costs.
- Runs: 8.
- After it merges, a Windows session writes its `whisk` file at the first commit of main that
  holds the lane. Every other file of session B stands at the measurement commit: the lane
  changes no code on another platform and none that the other measurements wait in.

## 5. The wake from a parked pool, and the makespan

**The wake is not this cause.** In the benchmark's wake phase no timer is registered. The
pusher sleeps between pushes with `std::thread::sleep` on its own thread and waits for the run
with `block_on`, which parks untimed (`crates/task/benches/whisk.rs`, lines 561 to 563;
`crates/task/src/executor.rs`, line 153). So no worker is the keeper, every worker is in
`Sleep::park` with no deadline, which is `std::thread::park`, and that is `WaitOnAddress` with
no timeout (`sys/sync/thread_parking/futex.rs`, line 55). The push ends it with
`WakeByAddressSingle`. A timer tick cannot lengthen a wait that has no timeout. The laptop's
P50 145.9 µs and P99 236.4 µs, against the M1's 13.3 and 31.6 µs, come from something else.
Their mean, 123.9 µs, is under their median, so some wakes are much faster than most.

**The makespan shares this cause in one respect.** Its runs are meant to start a millisecond
apart, "as a frame does". The gap is a `sleep_until` through the keeper, so on Windows each
lasted a tick, and each run began from a pool idle about sixteen times longer than intended.
How much of the failure that is, nobody knows. `timer-windows` shows it, since the laptop runs
the benchmark before and after. The rest fits the wake: the makespan fails with no background
load (P99 4.02 wide-shallow, 6.58 fan-in-fan-out) and mostly passes with two zstd tasks
keeping cores awake.

**Why it matters beyond milestone 0.** The laptop's 100k tick has S, the time that does not
shrink with workers, at 2.86 ms on six frame workers, against the M1's 1.06 ms on four
(`docs/results/tick-100k-laptop-windows-2026-11-15.md`). Criterion 1 is T₈ at most 4 ms on the
desktop. If the desktop boots Windows and wakes as the laptop does, criterion 1 is exposed to
the same cost.

**Candidates.** Each is a hypothesis.

- The cores' idle states. The M1's floor was the cores leaving deep idle, 24 to 29 µs (D113).
  An x86 laptop's deeper states take longer to leave, and the Balanced plan uses them.
- The scheduler's choice of core. Frame workers are not pinned. Windows may wake one on an
  efficiency core or on a core it has parked, or queue several wakes on a few cores. In probe
  3's reported run a thread that spun for up to 2 ms before a deadline was still over 20 µs
  late for about one deadline in twelve.
- Windows' power throttling of a process with no foreground window. The benchmark ran from a
  hidden process. Probe 2's hidden run saw the processor at 38% of its nominal performance and
  its visible run at 75%, with more other work in the second.
- The system timer's rate, which changes how deeply cores idle. This is the one way the wake
  could share section 1's cause, and it bears on option (c).

**Nothing is decided here.** The laptop proves nothing, and the desktop has not run. The row
"Wake latency against the 20 µs P99 criterion" records the cause as not the timed wait and
the probe as briefed. A decision comes due if the desktop's Windows file fails the wake or
the makespan: the plan's exit, step 6, puts each miss to the owner, and the probe below is
what that ruling will need.

**The brief the coordinator can send the laptop.** It follows briefs 2 and 3. Part A splits
the wake. Part B is the timed wait the laptop suggested after probe 3; answer 3 of section 6
needs it, and the lane reads it.

```text
Brief 4 for the Windows laptop: what a wake from a parked thread costs and what moves it, and
the timed wait asked once.

Rules as in briefs 2 and 3: a scratch crate outside the repository, the stable toolchain,
windows-sys as the one dependency, unsafe only at FFI calls, clippy clean. Nothing in the
repository is touched, committed or pushed. Change no power or system setting. What the probe
may set is the state of its own process and threads, all of which ends with the process:
thread priority, thread affinity, timeBeginPeriod(1) paired with timeEndPeriod(1), and the
power-throttling state of the process and its threads. The wake lock as before. Two runs, one
from your hidden tool process and one in a visible console window in the foreground, each at
most fifteen minutes. If a run would take longer, cut the sample counts in proportion and say
so.

PART A, THE WAKE

What is asked. WHISK's wake row on this laptop is P50 145.9 us and P99 236.4 us, from a push
to a parked pool, against 13.3 and 31.6 us on the M1. No timed wait is on that path: the
workers are in std::thread::park. The probe finds where the time goes.

The wake. A sleeper thread at THREAD_PRIORITY_ABOVE_NORMAL, which is what the engine gives a
frame worker, parks with std::thread::park. A waker thread at the same priority waits a gap,
reads the clock (t0), calls unpark, and reads the clock again (t1). The sleeper reads the
clock when park returns (t2), reads GetCurrentProcessorNumber, stores both and parks again.
The latency is t2 - t0 and the call is t1 - t0. The waker starts the next gap only once the
sleeper has stored its sample.

Conditions. Each is 2,000 wakes after 20 discarded, with a gap of 2 ms slept through with
std::thread::sleep, unless its line says otherwise.

- W0, the baseline: as above, nothing pinned. It should reproduce WHISK's row.
- G, the gap: 50 us, 200 us and 1 ms with the waker spinning through the gap; then 1 ms,
  5 ms, 16 ms (1,000 wakes) and 100 ms (300 wakes) with the waker asleep through it.
- W1, pinned: the sleeper on one hardware thread of a performance core, the waker on a
  hardware thread of another performance core.
- W2, warm sibling: W1, with a third thread spinning on the other hardware thread of the
  sleeper's core.
- W3, warm elsewhere: W1, with a third thread spinning on a third performance core.
- W4, efficiency core: W1 with the sleeper on an efficiency core.
- W5, timeBeginPeriod(1) held; otherwise W0.
- W6, throttling off: SetProcessInformation with ProcessPowerThrottling, control mask
  PROCESS_POWER_THROTTLING_EXECUTION_SPEED and state mask 0, and the same for both threads
  with SetThreadInformation and ThreadPowerThrottling; otherwise W0.
- W7, an event: the sleeper waits on an auto-reset event with WaitForSingleObject and the
  waker calls SetEvent; otherwise W0.
- F, the fan-out: six sleepers, nothing pinned, all parked. The waker unparks the six in
  order with nothing between. 1,000 rounds, 2 ms apart. For each round, each sleeper's
  latency from t0, read before the first unpark, and its processor.

For each row: the latency's minimum, P50, P99, P99.9, maximum and mean in us; how many wakes
fell under 10, 10 to 20, 20 to 35, 35 to 75, 75 to 150, 150 to 300, 300 to 1,000 and over
1,000 us; the call's P50 and P99; how many wakes ran on each logical processor; and how many
ran on a different processor from the wake before. For F: the P50 and P99 of the first,
second and so on to the sixth sleeper to start, and the mean number of distinct processors in
a round.

Print once: each logical processor's core and efficiency class, from
GetLogicalProcessorInformationEx, and which processors the pinned rows used.

Log beside the run, each second, with the UTC time at which each row starts and ends so the
two line up: Processor Information(*) % Processor Performance, Parking Status, % C1 Time,
% C2 Time and % C3 Time, and Processor(_Total) % Processor Time. Report them per row: the
mean performance, how many processors were parked, and the idle-state shares of the
processors the row ran on.

Read, and change nothing: the active power scheme and its processor settings (powercfg
/getactivescheme; powercfg /qh SCHEME_CURRENT SUB_PROCESSOR): the core parking minimum and
maximum, the processor idle disable setting, the performance boost mode, the energy
performance preference, and the heterogeneous policy and thread scheduling policies if they
are listed; the Windows power mode; AC or battery.

Only if the coordinator's message says the owner agreed, since it changes a setting: W0, G
and F once more with the processor idle states disabled on AC (powercfg /setacvalueindex
SCHEME_CURRENT SUB_PROCESSOR IDLEDISABLE 1, then powercfg /setactive SCHEME_CURRENT), and the
setting put back to the value it was read as, and re-activated, straight after. Report the
setting before, during and after. If the alias is not listed, say so and skip this part.

PART B, THE TIMED WAIT ASKED ONCE

Your point 6 of brief 3. The high-resolution waitable timer, one handle, on brief 3's fixed
schedule, 1,000 deadlines a row, brief 3's columns, and one more: of the deadlines over 20 us
late, how many had entered the spin.

- Shape 1, asked once: one wait for the time left less D, then a spin to the deadline. If
  the wait returns with more than D left, wait again the same way.
- Shape 2, brief 3's: 0.6 of what is left, a spin inside M.
- D for shape 1: 500, 750, 1,000, 1,500, 2,000 and 2,500 us. M for shape 2: 500 us only.
- Period: 16.667 ms, and 6.944 ms for D of 500, 1,000 and 2,000 and for shape 2.
- Priority: every row once at normal priority and once at THREAD_PRIORITY_ABOVE_NORMAL.

Report as before: the source between BEGIN and END marker lines with its manifest line, each
run's output verbatim, the counters per row, the settings read, and what the results show,
with each point labelled measured or hypothesis.
```

**What each result of part A would mean.**

| Result | Reading | Where it leads |
|---|---|---|
| The latency falls as the gap shortens, and falls to tens of microseconds in W2 or W3 | The cores' idle states, as on the M1 | The operating system's floor: a restated push criterion for Windows, or a warm sleeper, by the owner |
| W2 moves it and W3 does not | The sleeper's own core, not the package | The same, and a pre-warm before a known deadline becomes possible |
| W6, or the visible run, moves it | Windows throttles the process | A power-throttling opt-out for frame-critical threads in `lactose-core`'s thread class, with the row "Windows efficiency-core steering"; Windows measurements run in a foreground console |
| W1 moves it, wakes land on efficiency cores, or F's sleepers start one after another on few processors | The scheduler's choice of core | Affinity or CPU sets for frame workers, with the same row |
| W5 moves it | The timer's rate after all | Option (c) is weighed again, for the wake |
| W7 differs from W0 | The wake primitive | `timer-windows` takes the cheaper one for its event |
| Nothing moves it | Unknown | A system trace on the laptop |

## 6. The question for the owner

The coordinator asks it in these words; the recommended option is first. It also asks which
operating systems the desktop boots, a fact no document records: if it boots no Windows,
answer 4 becomes the recommendation.

> On Windows the timer root starts ticks up to a whole tick late. Rust's timed park there
> waits in whole milliseconds on the 15.6 ms system timer: a laptop measured tick start at P99
> 15.9 ms at 60 Hz, against the 20 µs criterion. Windows has a finer timer, but it overshoots
> by about half a millisecond, and on the laptop only a keeper that spun a whole core met the
> criterion. What is Windows held to?
>
> 1. **Fix the wait, spin the last 500 µs, hold Windows to 1 ms at P99 (Recommended):** One
>    lane, timer-windows, of eight runs in milestone 1. The keeper waits on a high-resolution
>    timer and a wake event, in lactose-task, with FFI and two more windows-sys features. The
>    spin costs about 1% of one core while the frame workers are idle, inside the 1.5% D138
>    allowed. On the laptop two ticks in three then started on time and P99 was 0.49 to
>    0.67 ms. If the lane's own measurement does not bear that out, it falls back to answer 2
>    without asking again. macOS and Linux keep 20 µs.
> 2. **Fix the wait, add no spin, hold Windows to 2 ms at P99:** The same lane. Nothing spins
>    longer than today. On the laptop the median tick then started 0.3 ms late and P99 was 1.0
>    to 1.2 ms.
> 3. **Fix the wait now, decide the figure after one more probe:** The same lane as answer 1.
>    Windows keeps 20 µs on paper until the laptop measures a wait asked once with a longer
>    spin. A 2 ms spin already cost 7 to 8% of a core there and left 6 to 10% of deadlines
>    over 35 µs late.
> 4. **Leave it for milestone 2:** No lane now. The row is re-gated to milestone 2, where the
>    first Windows client needs it, and a Windows session's timer rows prove nothing in
>    milestone 1.

## 7. Text for the coordinator (exact)

D144 is the next free number. The first block is for answer 1, and the coordinator writes the
date of the owner's answer where it says `<date>`. Each amendment is anchored on text checked
at `88eecf1`: each anchor occurs once, matched regardless of whitespace where it crosses a
line break.

### Decision entry

```
**D144. On Windows the keeper waits on a high-resolution timer beside a wake event and spins the last 500 µs, and tick start there is held to 1 ms at P99** (§5.1, §21, §22, `lactose-task`, `docs/plans/milestone-1.md`)
- Why: on Windows the timer root started ticks up to a whole tick late. An i7-12650H laptop on
  Windows 11, the owner's test box, which proves no criterion, ran the benchmark at `bef3455`
  (`docs/results/whisk-laptop-windows-2026-11-15.md`): tick start at 60 Hz was P50 4.3 ms and
  P99 15.9 ms against 20 µs, and at a 2 ms period 4,998 of 5,000 deadlines were over 1 ms late.
  The cause is in the pinned standard library (1.99.0, `library/std/src`).
  `std::thread::park_timeout` on Windows is `WaitOnAddress` (`sys/sync/futex/windows.rs`) with
  its timeout rounded up to whole milliseconds (`dur2timeout`, `sys/pal/windows/mod.rs`), and
  that wait ends on the system's timer tick, 15.6 ms by default. `Condvar::wait_timeout` and
  the channels' `recv_timeout` end in the same call, so the standard library has no timed wait
  on Windows that another thread can end and that is finer than the tick. `std::thread::sleep`
  is finer, a high-resolution waitable timer, and nothing can end it. Two probes on the laptop
  (`docs/plans/windows-timer-proposal.md`; 1,000 samples a row, each row twice, 10 to 46% of
  the processor busy with other work) measured each: a `park_timeout` of 0.5 to 10 ms lasted
  about 16 ms whatever was asked, and the high-resolution timer overshot by about half a
  millisecond at the median whatever was asked. Under the stepped wait of `Sleep::park` to a
  fixed 60 Hz schedule, with the shipped 50 µs spin, a deadline was passed by P50 7.9 ms and
  P99 15.7 to 15.8 ms as shipped, by P50 0.78 to 0.81 ms and P99 2.0 ms with
  `timeBeginPeriod(1)` held, and by P50 0.30 ms and P99 1.0 to 1.1 ms with the timer in place
  of the park. No spin threshold up to 2 ms brought any of them under 35 µs at P99; only a
  keeper that never waited was under 20 µs.
  - *On Windows the keeper's step is a wait on a high-resolution waitable timer and on a wake
    event.* Each frame-critical worker owns both. A waker that claims a sleeper's bit sets
    that worker's event beside the unpark, and a late signal costs one more trip round
    `Sleep::park`'s loop, as a late park token does: the cleared bit stays the wake-up. The
    code is in `lactose-task`, which adds the `Win32_System_Threading` and `Win32_Security`
    features of `windows-sys` and no package. Rules out the coarse wait through
    `std::thread::sleep` in slices, which no waker can end, so a push or an earlier deadline
    that needs the keeper waits out a slice; and `timeBeginPeriod(1)`, which is worse on every
    figure, holds a 1 ms clock interrupt for the process's life, and which Windows 11 does not
    guarantee to a process whose window is minimised or occluded.
  - *The Windows keeper spins the last 500 µs; the step stays 0.6 everywhere.* On the laptop
    that threshold took the median from 0.30 ms to under a microsecond and P99 to 0.49 to
    0.67 ms, with at most 3 deadlines of 1,000 over 1 ms late, for 0.77 to 1.11% of one core
    at 60 Hz while the frame workers are otherwise idle. That is inside the 1.5% D138 allowed
    the keeper. Thresholds of 1,000 and 1,500 µs bought nothing more, and 2,000 µs cost 6.9 to
    7.8% of a core and still left 56 to 103 deadlines of 1,000 over 35 µs. Rules out a Windows
    keeper that spins to the criterion.
  - *Tick start on Windows is held to 1 ms at P99, at 60 Hz.* It is set at what the operating
    system gives, as D113 set the push criterion. The engine tolerates it: deadlines are
    absolute, so lateness does not accumulate, and a busy pool fires timers at job boundaries.
    macOS and Linux keep 20 µs, and D138's attempt and its restatement at 35 µs stand for
    them. Rules out a criterion on Windows that only a whole core can meet.
  - *The lane confirms the threshold on the real executor.* The probes emulate the loop on one
    thread at normal priority. The lane runs the benchmark on the laptop with the threshold at
    50 and at 500 µs. If the 500 µs build's process CPU at 60 Hz is more than 0.015 s a second
    above the 50 µs build's, which is 1.5% of a core, or its P99 is not under 1 ms, the
    threshold stays 50 µs and Windows is held to 2 ms at P99, recorded as an addendum here.
    The desktop's file is the proof either way.
  - *It is a lane of its own in milestone 1, `timer-windows`, not D138's one attempt.* The
    M1's miss is under 10 µs for a cause nobody knows; Windows' is 15 ms for a known one.
    Rules out spending `keeper`'s attempt on it, and a Windows session whose timer rows fail
    whatever the machine is.
  - *A Windows session writes its `whisk` file at the first commit of main that holds the
    lane.* Every other file of session B stands at the measurement commit.
  - *The wake from a parked pool on Windows is another cause and is not decided.* The laptop's
    P50 145.9 µs has no timed wait on its path. Its makespan rows were taken after gaps of a
    tick, not of 1 ms, which the lane's before and after on the laptop shows. A probe is
    briefed, and a decision comes due if the desktop's Windows file fails either.
- Driven by: the Windows laptop's result files at `bef3455` and its two timer probes; the
  owner's answer of <date> (fix the wait in milestone 1, spin the last 500 µs and hold Windows
  to 1 ms at P99).
- Replaced: nothing of D113 for macOS. Its premise, that a keeper which steps its timed park
  and spins the last 50 µs can meet 20 µs, holds where the timed park takes less than a
  millisecond; on Windows it does not. D138's "pass or fail ... tick start ... on x86" reads
  the Windows timer rows against 1 ms.
```

### If the owner answers 2: no added spin, 2 ms

The title's "and spins the last 500 µs, and tick start there is held to 1 ms at P99" becomes
"and tick start there is held to 2 ms at P99". The fourth bullet is struck. The second and
third bullets are replaced by these, "Driven by" names answer 2, and "Replaced" ends "against
2 ms".

```
  - *The step and the spin do not change.* 0.6 of the time left and the last 50 µs, on every
    platform. A threshold of 500 µs on Windows would take the median from 0.30 ms to under a
    microsecond for about 1% of one core while the frame workers are otherwise idle; the owner
    declined the spin. Rules out any warm keeper on Windows.
  - *Tick start on Windows is held to 2 ms at P99, at 60 Hz.* It is set at what the operating
    system gives, as D113 set the push criterion: the laptop's rows are 1.0 to 1.2 ms on a
    fixed schedule and at most 1.69 ms in the first probe. The engine tolerates it: deadlines
    are absolute, so lateness does not accumulate, and a busy pool fires timers at job
    boundaries. macOS and Linux keep 20 µs, and D138's attempt and its restatement at 35 µs
    stand for them. Rules out a criterion on Windows that only a whole core can meet.
```

### If the owner answers 3: the figure after one more probe

The title's "and tick start there is held to 1 ms at P99" becomes "and what tick start is
held to there is decided after one more probe". The fourth bullet is struck. The third bullet
is replaced by this, and "Driven by" names answer 3. Section 22 is not amended, the
benchmark's threshold on Windows does not change, and the planner returns with the figure.

```
  - *What tick start is held to on Windows is not decided.* The 20 µs stands on paper. The
    laptop measures the keeper's wait asked once, for the time left less a fixed margin, with
    margins up to 2.5 ms, at a frame worker's priority
    (`docs/plans/windows-timer-proposal.md`, section 5, part B). The planner returns with its
    figures and an addendum here: the figure, and the threshold that buys it. Rules out
    restating the criterion before the form the laptop suggested is measured.
```

### If the owner answers 4: leave it for milestone 2

```
**D144. The keeper's timed wait on Windows ends on the 15.6 ms timer tick; it is fixed at milestone 2** (§5.1, §22, `lactose-task`, `docs/plans/milestone-1.md`)
- Why: as the first paragraph of the entry above.
  - *The fix is milestone 2's first lane on Windows*, where the first Windows client paces
    frames: a high-resolution waitable timer beside a wake event in `lactose-task`, with a
    spin of the last 500 µs (`docs/plans/windows-timer-proposal.md`, options (b) and (d)).
    What Windows is then held to is decided with that lane.
  - *In milestone 1 a Windows session's timer rows prove nothing*, and its makespan rows are
    taken after gaps of a tick. Tick start's x86 half closes on another operating system of
    the desktop, or is recorded as not shown on x86. Rules out reading a Windows timer row as
    the executor's.
- Driven by: the Windows laptop's result files at `bef3455` and its two timer probes; the
  owner's answer of <date>.
- Replaced: nothing.
```

Under answer 4 no architecture amendment is applied, the plan gets only the one sentence
given at the end of "Plan amendments", and the verification rows are applied with the new
row's gate `| 2 |` and its opening "The keeper's timed wait on Windows (D144): not built, and
milestone 2's first lane on Windows."

### Architecture amendments

For answer 1. They state the decision before the lane lands, as D143's did; the row "The
keeper's timed wait on Windows (D144)" says what is not built yet.

```
--- section 5.1, "Scheduling mechanics": after the sentence that ends ---
because macOS lengthens a timed wait by a leeway proportional to the timeout (D113).
--- insert ---
On Windows each step is a wait on a high-resolution waitable timer and on an event that a waker sets, because every timed wait of the standard library that another thread can end is rounded up to whole milliseconds there and ends on the system's timer tick, 15.6 ms by default; that timer overshoots by about half a millisecond whatever is asked, so the keeper spins the last 500 microseconds there (D144).

--- section 21, the table: replace ---
| `task` | deques, scope borrows, panic guard |
--- with ---
| `task` | deques, scope borrows, panic guard, core-topology queries, the keeper's timed wait on Windows |

--- section 22, the row of milestone 0: replace ---
Tick-start latency, from a tick deadline to the timer root resuming, under 20 µs at P99;
--- with ---
Tick-start latency, from a tick deadline to the timer root resuming, under 20 µs at P99, and under 1 ms on Windows, where the finest timed wait a waker can end overshoots by about half a millisecond (D144);
```

Section 21's row gains the core-topology queries as well: `crates/task/src/platform.rs` has
called `sysctlbyname` and `GetLogicalProcessorInformationEx` since milestone 0, and
`crates/task/SAFETY.md` describes them under a reason the row does not list.

For answer 2, section 5.1's inserted sentence ends at "15.6 ms by default (D144)." and
section 22 reads "under 2 ms on Windows". For answer 3, section 22 is left as it is.

### Plan amendments

`docs/plans/milestone-1.md`, for answers 1, 2 and 3.

```
--- section 3, the second lane table, the row `keeper`: replace ---
| Session A's numbers | Result file `whisk` from a quiet M1, or the cause stated | 4 to 8, counted as 6 |
--- with ---
| Session A's numbers; `timer-windows` merged, since both edit `crates/task/src/sleep.rs`. The Windows timed wait is not this lane's (D144) | Result file `whisk` from a quiet M1, or the cause stated | 4 to 8, counted as 6 |

--- section 3, the second lane table: add a row after `slot-copy` ---
| `timer-windows` | The keeper's timed wait on Windows (D144), in `lactose-task` alone. On Windows, outside loom and Miri, each frame-critical worker makes a high-resolution waitable timer and an auto-reset event when it starts and closes them when it exits; the `sync` shim's timed park is a wait on both; `Pool::wake`, `Pool::wake_all` and the wake of `Pool::announce_entry` set the claimed worker's event beside the unpark. The spin threshold is 500 µs on Windows and 50 µs elsewhere; the step stays 0.6. A worker whose timer cannot be made keeps `std::thread::park_timeout` and logs it once. `crates/task/Cargo.toml` adds `Win32_System_Threading` and `Win32_Security` to its `windows-sys` features. `SAFETY.md`'s section for the calls; the README's sentences; the benchmark's timer threshold on Windows, 1 ms, and its process CPU column on Windows | None | On CI's windows-x64 job: the median lateness of fifty 2 ms deadlines through `sleep_until` on an otherwise idle pool is under 7.8 ms, half a default tick, where today's code gives about 14 ms; on a one-worker frame pool a keeper waiting on a deadline ten seconds away is ended by an earlier timer, by pushed work and by shutdown, each inside the test's bound, and each test fails when the event is not set. The loom models and Miri pass unchanged. The Windows cross-target clippy and `cargo xtask features` pass. The merge is pushed alone (D125). The laptop runs the benchmark at the lane's base, and at its head with the threshold at 50 and at 500 µs, as labelled data: the timer rows with their process CPU, and the makespan and wake rows beside them. If the 500 µs build's process CPU at 60 Hz is more than 0.015 s a second above the 50 µs build's there, or its P99 is not under 1 ms, the lane ships 50 µs and a benchmark threshold of 2 ms, and says so for D144's addendum | 8: seven, and the concurrency reviewer beside its first review |

--- section 3, after the paragraph that begins "D143 adds `slot-copy`": add a paragraph ---
D144 adds `timer-windows`, eight runs, and two for its proposal's review and answer.

--- section 3, the queue table: replace ---
| 1, the ECS and then render | `scalar-probe`, `ecs-a`, `ecs-kernel`, `ecs-fork`, `spike-rhi`, `rhi-api`, `rhi-null`, `keeper`, `readme-sweep` | 51 |
--- with ---
| 1, the ECS and then render | `scalar-probe`, `ecs-a`, `ecs-kernel`, `ecs-fork`, `spike-rhi`, `rhi-api`, `rhi-null`, `timer-windows`, `keeper`, `readme-sweep` | 59 |

--- section 3: replace ---
Six rules bind whatever the coordinator reorders:
--- with ---
Seven rules bind whatever the coordinator reorders:

--- section 3, after the rule that begins "Criterion 5 is proven at the first commit of main that holds `slot-copy`": add a rule ---
- On Windows, milestone 0's `whisk` file is written at the first commit of main that holds
  `timer-windows`, by `cargo xtask measure whisk`, not at the measurement commit. Before the
  lane its timer rows fail on any machine and its makespan rows are taken after gaps of a
  tick (D144). Every other file of session B stands at the measurement commit.

--- section 4, the paragraph "Session A": replace ---
if it fails there, that failure is the session's result and goes to `keeper`.
--- with ---
if it fails there, that failure is the session's result and goes to `keeper`, except the timer rows on Windows, whose cause is known and which `timer-windows` fixes (D144).

--- section 4, the table of what each timing criterion runs: replace ---
| Milestone 0, WHISK | `cargo bench -p lactose-task --bench whisk` | Its own thresholds (`crates/task/README.md`) | The same |
--- with ---
| Milestone 0, WHISK | `cargo bench -p lactose-task --bench whisk` | Its own thresholds (`crates/task/README.md`) | The same; on Windows the 60 Hz timer row against 1 ms at P99, in a file written at the first commit that holds `timer-windows` (D144) |

--- section 7, step 1: replace ---
criterion 5's proof and criterion 7's rollback figure (D143).
--- with ---
criterion 5's proof and criterion 7's rollback figure (D143). If the desktop boots Windows, its `whisk` file there has been written at the first commit that holds `timer-windows` (D144).
```

For answer 2: the lane row's "The spin threshold is 500 µs on Windows and 50 µs elsewhere; the
step stays 0.6." becomes "The step and the spin stay 0.6 and 50 µs."; its "1 ms" becomes
"2 ms"; its proof reads "and at its head" for "and at its head with the threshold at 50 and
at 500 µs" and loses its last sentence; and the WHISK row reads "against 2 ms at P99". For
answer 3: the lane row loses "the benchmark's timer threshold on Windows, 1 ms, and" and its
last sentence, and the WHISK row's addition reads "The same; on Windows in a file written at
the first commit that holds `timer-windows`, its timer row read against the figure D144's
addendum sets".

For answer 4 only this is applied:

```
--- section 4, the paragraph "Session A": replace ---
if it fails there, that failure is the session's result and goes to `keeper`.
--- with ---
if it fails there, that failure is the session's result and goes to `keeper`, except the timer rows on Windows, whose cause is known and which milestone 2 fixes (D144).
```

### Verification rows

`docs/verification.md`. Each change is an exact replacement inside the row its opening words
name, an append before the row's gate cell, or a new row at the end of section 4. Written for
answer 1; for answer 2 the new row and the first replacement read 2 ms for 1 ms and lose the
threshold's sentences, and for answer 3 they read "the figure is not decided" for "held to
1 ms at P99".

```
--- "| Tick-start latency against the 20 µs P99 criterion.": replace ---
That is the size of Windows' default timer tick, about 15.6 ms. The stepped `park_timeout` of `crates/task/src/sleep.rs` asks for a fraction of the time left, and a timed wait on Windows ends on a tick unless the process asks for finer timing; not yet confirmed by a probe, and the fix is open.
--- with ---
That is Windows' default timer tick, 15.6 ms, and the cause is confirmed (D144, `docs/plans/windows-timer-proposal.md`). In std 1.99.0, `park_timeout` on Windows is `WaitOnAddress` with its timeout rounded up to whole milliseconds (`library/std/src/sys/sync/futex/windows.rs`; `dur2timeout` in `sys/pal/windows/mod.rs`), which ends on the tick, and so do `Condvar::wait_timeout` and the channels' `recv_timeout`; `std::thread::sleep` is a high-resolution waitable timer that no waker can end. The laptop's two probes (1,000 samples a row, each row twice, 10 to 46% of the processor busy with other work): a `park_timeout` of 0.5 to 10 ms lasts about 16 ms whatever is asked, and the high-resolution timer overshoots by about half a millisecond at the median whatever is asked. Under the stepped wait of `Sleep::park` to a fixed 60 Hz schedule with the 50 µs spin, a deadline is passed by P50 7.9 ms and P99 15.7 to 15.8 ms as shipped, by P50 0.78 to 0.81 ms and P99 2.0 ms with `timeBeginPeriod(1)` held, and by P50 0.30 ms and P99 1.0 to 1.1 ms with the timer in place of the park. With the timer and a spin threshold of 500 µs: P50 under a microsecond, P99 0.49 to 0.67 ms, 0.77 to 1.11% of one core. With 2,000 µs: 6.9 to 7.8% of a core, and 56 to 103 deadlines of 1,000 still over 35 µs late. Only a keeper that never waited was under 20 µs at P99. Decided (D144): on Windows the keeper's step is a wait on a high-resolution waitable timer and a wake event, it spins the last 500 µs, and tick start there is held to 1 ms at P99; lane `timer-windows` builds it (the row "The keeper's timed wait on Windows (D144)").

--- "| Wake latency against the 20 µs P99 criterion.": replace ---
and does better with two zstd tasks running, which fits workers waking slowly from idle; not diagnosed.
--- with ---
and does better with two zstd tasks running, which fits workers waking slowly from idle. It is not the timed wait (D144): the pool of the wake phase parks with no deadline, std's `park`, which is `WaitOnAddress` with no timeout, and is woken by `WakeByAddressSingle`. The makespan phase's 1 ms gap between runs is a `sleep_until` through the keeper and lasted a tick on Windows, so its runs began from a pool idle about sixteen times longer than a frame's gap; lane `timer-windows` runs the benchmark on the laptop before and after, which shows that part. The rest is not diagnosed. The candidates are the cores' idle states, the scheduler's choice of core on a hybrid part, Windows' power throttling of a process without a foreground window, and the system timer's rate; the laptop probe briefed in `docs/plans/windows-timer-proposal.md`, section 5, part A, splits them. A decision comes due if the desktop's Windows file fails the wake or the makespan.

--- "| Section 22's milestone 1 timing criteria are proven by result files": append before its gate cell ---
 On Windows, milestone 0's `whisk` file is written at the first commit of main that holds lane `timer-windows`, not at the measurement commit (D144).

--- section 4: add a row ---
| The keeper's timed wait on Windows (D144): not built; lane `timer-windows`. Each frame-critical worker gets a high-resolution waitable timer and an auto-reset event on Windows, `Sleep::park`'s step waits on both, a waker that claims a sleeper's bit sets the event beside the unpark, and the keeper spins the last 500 µs there. Its tests, on CI's windows-x64 job: the median lateness of fifty 2 ms deadlines through `sleep_until` on an otherwise idle pool is under half a default tick, which today's code fails at about 14 ms; on a one-worker frame pool a keeper waiting on a deadline ten seconds away is ended by an earlier timer, by pushed work and by shutdown, and each test fails with the event unset. To measure, on the laptop as labelled data and on the desktop if it boots Windows: `whisk`'s 60 Hz timer row against 1 ms at P99, with the threshold at 50 and at 500 µs on the laptop, and the process CPU column, which the benchmark does not fill on Windows today; its wake row before and after, since each wake of a claimed sleeper gains one `SetEvent`; its makespan rows before and after, since the 1 ms gap between runs goes through the keeper. If the 500 µs build's process CPU at 60 Hz is more than 0.015 s a second above the 50 µs build's on the laptop, which is 1.5% of a core, or its P99 is not under 1 ms, the threshold stays 50 µs and the figure is 2 ms, by an addendum to D144. Open until the desktop's file: whether the figure holds on a quiet machine. Not measured: the keeper at a frame worker's priority; a wait asked once for the time left less a fixed margin, which the brief of `docs/plans/windows-timer-proposal.md`, section 5, part B, measures; the spin's power cost; a 1 ms period under a minimised window, which Microsoft's page for `timeBeginPeriod` says Windows 11 does not guarantee | 1 |
```

### README sentences, for the lane

`crates/task/README.md`, for answer 1. The lane writes them with its code; they are given so
the brief can name them.

```
--- "Behaviour the signatures cannot express", "Workers": after the sentence that ends ---
the keeper, parks only until the earliest deadline.
--- insert ---
On Windows its timed wait is a high-resolution waitable timer beside an event that a waker sets, and it spins the last 500 µs before a deadline, because `std::thread::park_timeout` there waits in whole milliseconds on the system's 15.6 ms timer tick and the finer timer overshoots by about half a millisecond (D144); a worker whose timer cannot be made keeps `park_timeout`.

--- "Proof for milestone 0", the table: replace ---
| Timer wake (a tick deadline until the frame-critical task that slept until it resumes, at 60 Hz) | under 20 µs at P99 |
--- with ---
| Timer wake (a tick deadline until the frame-critical task that slept until it resumes, at 60 Hz) | under 20 µs at P99; on Windows under 1 ms (D144) |
```

## 8. What this document could not verify

- **Nothing here was run on Windows by the planner.** Every Windows figure is the laptop's,
  from one benchmark run and two probes on a machine doing other work, on the Balanced plan.
  No figure is from a quiet machine, and none is the desktop's.
- **The probes emulate the keeper.** One thread, at normal priority, with no affinity, in a
  hidden process, runs `Sleep::park`'s loop with no executor round it. The keeper is a
  frame-critical worker at a raised priority, and after its wait it still fires the timer and
  runs the task. The lane's benchmark on the laptop is the first measurement of the real
  thing, which is why D144 lets it move the threshold.
- **Run to run, a row's late count varies up to twofold.** The two runs of probe 3 agree on
  the knee at 500 µs and on the cost of 2,000 µs, and on little between. In the earlier run
  another process switched the system timer during the waitable timer's rows, and its 250 µs
  row there (P99 3.2 ms) is disturbed.
- **Why spinning deadlines were late is a hypothesis.** At a 2 ms threshold three quarters of
  the late deadlines had spun. The laptop reads it as descheduling. The planner did not see
  the samples file, only the laptop's counts from it, which are of the reported run.
- **The spin's cost is a share of a core, not a power figure.** Nothing measured power, and
  the benchmark reports no process CPU on Windows today.
- **A wait asked once is untested.** The 0.6 step was measured because it ships. Whether one
  wait for the time left less a fixed margin serves Windows better is part B's question.
- **The half-millisecond granule is a hypothesis.** The overshoot is measured;
  `NtQueryTimerResolution`'s finest period of 0.5 ms is measured; that the one causes the
  other is not shown.
- **That `WaitOnAddress` ends on the tick is measured, not read.** The standard library's
  source shows the rounding to milliseconds. What the kernel does with the timeout is the
  probes' evidence. The waits lasted 15.8 to 16.0 ms where the reported tick is 15.625 ms; the
  difference is not explained. Nor is the benchmark's lower median at 60 Hz, 4.3 ms against
  probe 3's 7.9 ms.
- **Microsoft's pages were read once, on 15 November 2026**: `timeBeginPeriod` (the Windows 10
  version 2004 and Windows 11 remarks, the power remark, `Winmm.dll`) and
  `CreateWaitableTimerExW` (the high-resolution flag, from Windows 10 version 1803). They are
  the vendor's statements. The minimised-window case was not measured; the laptop's hidden
  process kept its 1 ms period, and a few of its default-timer waits in probe 2's visible run
  returned at once, unexplained.
- **The design of option (b) was compiled, not run.** A scratch crate, removed, with the timer,
  the event, the wait on both and `SetEvent` checks for `x86_64-pc-windows-msvc` against
  `windows-sys` 0.61.2 under the features named. Whether an event wakes a thread as fast as
  `WakeByAddressSingle` is not known; the brief's row W7 and the lane's benchmark answer it.
- **That only three call sites wake a claimed sleeper is a reading of
  `crates/task/src/pool.rs`** (`announce_entry`, `wake`, `wake_all`). The lane and the
  concurrency reviewer hold it; a wake that misses the event ends late, at the timer, and
  never hangs, since a keeper always has a deadline.
- **Linux is unmeasured.** Its timed park takes nanoseconds, so it has no rounding of this
  kind. No Linux `whisk` file exists; the desktop's, if it boots Linux, is the first.
- **Which operating systems the desktop boots** is in no document. Session A has returned no
  file.
- **The wake and the makespan are undiagnosed.** Section 5 shows only that no timed wait is on
  the wake's path and that the makespan's gap was a tick. Its candidates are hypotheses, and
  probe 2's lower processor performance in its hidden run came with less background work, so
  it singles out none.
