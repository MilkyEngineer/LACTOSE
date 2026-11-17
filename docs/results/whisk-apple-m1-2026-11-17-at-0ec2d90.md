| | |
|---|---|
| Command | `cargo xtask measure whisk --out target/qw-rerun-0ec2d90`, which ran `WHISK_BENCH_MACHINE=apple-m1 cargo bench --locked -p lactose-task --bench whisk`, its files written under the target directory and moved here |
| Profile | bench |
| OS | macos 27.2.1 (aarch64) |
| Load average | 12.40 6.63 4.67 before, 2.60 4.63 4.45 after |

# WHISK benchmark, apple-m1, 2026-11-17

Written by `cargo bench -p lactose-task --bench whisk`. Thresholds are those of
`crates/task/README.md`.

| | |
|---|---|
| CPU | Apple M1 (macos, aarch64) |
| Detected topology | 4 performance, 4 efficiency cores |
| Workers | 4 frame-critical, 4 background |
| Commit | `0ec2d907f443` |
| Load average | 12.40 6.63 4.67 before, 2.60 4.63 4.45 after |
| Task role | TASK_FOREGROUND_APPLICATION (set returned 0, get returned 0) |
| Calibration | 0.5239 kernel iterations per ns; serial check 1.000 (1.0 is exact) |
| Runs per case | 2000 |

## Makespan divided by its lower bound (pass)

At most 1.25 at P50 and 1.5 at P99.

| Shape | Background load | P50 | P99 | Max | Greedy P50 / P99 | Longest-first P50 / P99 | Makespan | Lower bound | CPU per run | Work per run |
|---|---|---|---|---|---|---|---|---|---|---|
| wide-shallow | none | 1.076 | 1.133 | 1.395 | 1.017 / 1.044 | 1.016 / 1.047 | 14822 µs | 13735 µs | 58247 µs | 54938 µs |
| wide-shallow | two zstd tasks | 1.075 | 1.127 | 1.175 | 1.017 / 1.044 | 1.016 / 1.047 | 14805 µs | 13735 µs | 103556 µs | 54938 µs |
| deep-chain | none | 1.003 | 1.029 | 1.034 | 1.000 / 1.000 | 1.000 / 1.000 | 31747 µs | 31620 µs | 36211 µs | 34444 µs |
| deep-chain | two zstd tasks | 1.055 | 1.058 | 1.118 | 1.000 / 1.000 | 1.000 / 1.000 | 33360 µs | 31620 µs | 157936 µs | 34444 µs |
| fan-in-fan-out | none | 1.113 | 1.183 | 1.218 | 1.057 / 1.121 | 1.002 / 1.038 | 15766 µs | 14145 µs | 35182 µs | 31180 µs |
| fan-in-fan-out | two zstd tasks | 1.119 | 1.191 | 1.341 | 1.057 / 1.121 | 1.002 / 1.038 | 15869 µs | 14145 µs | 85598 µs | 31180 µs |

Greedy and longest-first are schedulers with no overhead that never idle a worker while a node is ready, simulated on the same draws; they differ in which ready node they take. Neither is a bound. CPU per run is the whole process, gaps between runs and any background load included; with no background load, what it exceeds the work by is what the executor spends around the jobs.

- wide-shallow: 192 independent nodes, 64 of them with one dependent (256 nodes).
- deep-chain: a 128-node chain; every fourth link forks a side node that rejoins two links on (160 nodes).
- fan-in-fan-out: 16 rounds of 1 node spreading to 8 (twice the 4 frame-critical workers) and joining to 1 (145 nodes).

## Wake latency (pass)

Push to a parked pool until the job starts; under 35 µs at P99. 5000 pushes, 2 ms apart.

| P50 | P99 | Max | Process CPU per second of this phase |
|---|---|---|---|
| 14.8 µs | 33.2 µs | 67.6 µs | 0.0411 s |

## Timer wake (pass)

From a deadline until the frame-critical task sleeping until it resumes, as the timer root ticks, with the pool otherwise idle. The 60 Hz P99 must be under 35 µs.

| Period | Deadlines | P50 | P99 | Max | Over 1 ms | Process CPU per second |
|---|---|---|---|---|---|---|
| 2.000 ms | 5000 | 2.1 µs | 26.2 µs | 129.0 µs | 0 | 0.0840 s |
| 16.667 ms | 5000 | 6.5 µs | 34.6 µs | 109.8 µs | 0 | 0.0124 s |

## Steal (pass)

One successful steal including the split, timed inside the executor on a thief thread taking from a deque another thread filled; under 1 µs at P99. 400000 steals. Each figure includes one pair of clock readings, 54 ns here.

| P50 | P99 | Max | Mean |
|---|---|---|---|
| 42 ns | 167 ns | 40334 ns | 68 ns |

Not a steal time, for reference: with every range forced down to single units, a node of empty units costs 469 worker-ns per entry (split, push, pop or steal, dispatch).

## Stalls

Every wake or timer sample over 1 ms and every graph run over 2 times its lower bound, with the wall-clock time it ended, to match against the system's log of sleeps and wakes (`pmset -g log` on a Mac). Times are UTC; this machine is at +1000.

None.
