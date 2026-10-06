# Wake-latency spike, Apple M1, 2026-10-06

Question: can a parked worker of `lactose-task` start a job within 20 µs at P99 of the push
without a permanently spinning core? Machine: Apple M1 (4 performance, 4 efficiency cores),
macOS 14.7.4, load average 1.5 to 4.5 from other processes throughout, with spikes to 12 and
36 noted where they fell. 5,000 samples per case, pushes 2 ms apart, as the benchmark does.
The harness used raw libSystem calls and measured the same quantity as the benchmark's
`measure_wake_latency`. All latencies are in µs.

## Answer

- **At a known deadline (the timer root): yes.** A stepped timed park plus a spin of at most
  50 µs reaches P99 6 to 10 µs at 60 and 144 Hz for about 0.6% of a core.
- **From an unpredictable push: no.** No wake primitive, QoS class or real-time policy gets
  under 20 µs. The floor is the cores leaving a deep idle state, not the scheduler.
- **A bug in the shipped timer keeper.** A single `park_timeout(deadline - now)` is
  lengthened by macOS timer coalescing in proportion to the timeout (about 26% at priority 47,
  50% at priority 31). In the benchmark the timer root resumed 5.55 ms late at P50 and 8.2 ms
  at P99 at 60 Hz. The same leeway applies to `mach_wait_until`,
  `os_sync_wait_on_address_with_deadline` and kevent timers without `NOTE_CRITICAL`.

## Task role

A process started from Terminal has task role `TASK_UNSPECIFIED`, under which user-interactive
and user-initiated QoS are clamped to priority 31. `task_policy_set` with
`TASK_FOREGROUND_APPLICATION` gives the real priority 47 and approximates a frontmost game;
cases marked "fg" ran that way, "unspec" cases in the default role. The checked-in benchmark
at `1d0b4b1` ran under the clamp.

## 1. Push from an idle pool

| Primitive | unspec P50 / P99 / max | fg P50 / P99 / max |
|---|---|---|
| std park (a dispatch semaphore on this target) | 15.9 / 28.0 / 434 | 15.4 / 27.4 / 70 |
| std park, user-interactive QoS | 16.0 / 28.4 / 825 | 14.5 / 26.5 / 51 |
| Mach semaphore | 15.4 / 27.8 / 1083 | 14.0 / 26.0 / 54 |
| `os_sync_wait_on_address` | 17.2 / 29.7 / 2110 | 16.4 / 28.7 / 47 |
| `pthread_cond` | 20.4 / 35.2 / 770 | 16.8 / 29.8 / 69 |
| pipe | 21.8 / 35.8 / 601 | 20.5 / 33.3 / 58 |
| kqueue `EVFILT_USER` | 19.7 / 34.6 / 387 | 17.7 / 31.0 / 41 |
| Time-constraint policy, std park | 15.7 / 26.3 / 41 | 15.1 / 25.5 / 44 |
| Time-constraint policy, Mach semaphore | | 14.1 / 24.2 / 54 |
| Lingering spinner (reference) | | 0.1 / 0.2 / 3.4 at 99.9% of a core |

Waiter CPU was 0.9 to 1.1% of a core in every parked case. Priority 47 removes the tail beyond
P99.9 and nothing else. The time-constraint policy is honoured for a non-audio process (priority
97, held over 60 s), and a thread spinning at 100% under it is demoted to priority 0.

## 2. Idle state

All fg, std park, user-interactive.

| Case | P50 / P99 / max | Cost |
|---|---|---|
| Control | 15.6 / 27.7 / 45 | |
| One busy thread, any QoS | 1.9 / 4.3 / 10 | a core |
| Push gap 100 µs / 500 µs / 2 ms / 10 ms | P99 6.6 / 19.1 / 27 / 35 | |
| Heartbeat thread waking every 100 µs | 3.3 / 7.2 / 26 | 1.9% of a core |
| Heartbeat 150 µs | 3.3 / 8.5 / 19 | 1.3% |
| Heartbeat 200 µs | 12.9 / 24.7 / 55 | 4.2% |
| Heartbeat 250 µs and longer | P99 25 to 28 | |

Latency grows with how long the cores have been idle, and any thread waking at least every
150 to 180 µs removes the floor. The power cost of never reaching deep idle was not measured
(`powermetrics` needs sudo).

## 3. Known deadline

Deadline-to-start latency of the waiting thread, fg unless marked.

| Method | 500 Hz P50 / P99 / max | 60 Hz P50 / P99 / max | 144 Hz P50 / P99 / max | Cost |
|---|---|---|---|---|
| `park_timeout`, today's keeper | 410 / 473 / 511 | 2030 / 2078 / 2174 | 1402 / 1715 / 1780 | 0.3 to 1.3% |
| `park_timeout`, unspec | 673 / 915 / 2200 | 4959 / 8155 / 11921 | | |
| `park_timeout`, lead 500 µs then spin | 0.1 / 0.1 / 51 | | | 7% |
| kevent `NOTE_CRITICAL`, lead 100 µs | 0.1 / 0.1 / 1029 | 0.1 / 0.2 / 175 | 0.1 / 0.1 / 882 | 0.6% at 60 Hz, 1.5% at 144 Hz |
| Stepped park (0.6 of time left, spin the last 50 µs) | 0.1 / 6.6 / 102 | 0.1 / 10.3 / 196 | 0.1 / 6.1 / 237 | 0.6% at 60 Hz, 1.1% at 144 Hz |
| Stepped park, unspec | 0.1 / 36.5 / 3832 | 0.1 / 45.1 / 4868 | 0.9 / 53.5 / 4796 | |
| Time-constraint policy, `mach_wait_until`, lead 50 µs | 0.1 / 0.2 / 60 | | | 3.1% |

The stepped park needs no lead parameter and no kqueue; it is the fix chosen for the keeper.
At priority 31 neither variant reaches 20 µs, so the engine must run in the foreground task
role or give the keeper the time-constraint policy.

## 4. Prototype in the benchmark

A prototype with the stepped park and a "warm sleeper" (frame worker 0 parking in 100 µs
steps while idle) passed the push criterion in `cargo bench -p lactose-task --bench whisk`
(P50 2.0, P99 16.2 µs; timer wake at 60 Hz P99 26.8 µs; makespans within noise) but every run
with it had a phase stall for 16 to 18 minutes with the frame workers parked and nothing
firing. The cause was not found and isolated reproductions were normal. The stepped park lands
only after that is explained; the warm sleeper is not adopted.

## Not tested

Power and energy; core placement (macOS has no affinity API); thermal throttling; loom and
Miri on the prototype; dispatch-source timers with `DISPATCH_TIMER_STRICT`; a real
LaunchServices foreground app rather than `task_policy_set`; any x86 machine.
