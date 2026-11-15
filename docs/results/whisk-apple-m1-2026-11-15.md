| | |
|---|---|
| Command | `cargo xtask measure --all`, which ran `WHISK_BENCH_MACHINE=apple-m1 cargo bench --locked -p lactose-task --bench whisk`, its files written under the target directory and moved here |
| Profile | bench |
| OS | macos 14.7.4 (aarch64) |
| Load average | 1.51 2.07 3.66 before, 4.04 7.23 5.90 after |

# WHISK benchmark, apple-m1, 2026-11-15

Written by `cargo bench -p lactose-task --bench whisk`. Thresholds are those of
`crates/task/README.md`.

| | |
|---|---|
| CPU | Apple M1 (macos, aarch64) |
| Detected topology | 4 performance, 4 efficiency cores |
| Workers | 4 frame-critical, 4 background |
| Commit | `bef3455b4a56` |
| Load average | 1.51 2.07 3.66 before, 4.04 7.23 5.90 after |
| Task role | TASK_FOREGROUND_APPLICATION (set returned 0, get returned 0) |
| Calibration | 0.5330 kernel iterations per ns; serial check 1.002 (1.0 is exact) |
| Runs per case | 2000 |

## Makespan divided by its lower bound (pass)

At most 1.25 at P50 and 1.5 at P99.

| Shape | Background load | P50 | P99 | Max | Greedy P50 / P99 | Longest-first P50 / P99 | Makespan | Lower bound | CPU per run | Work per run |
|---|---|---|---|---|---|---|---|---|---|---|
| wide-shallow | none | 1.095 | 1.137 | 1.203 | 1.017 / 1.044 | 1.016 / 1.047 | 15063 µs | 13735 µs | 59211 µs | 54938 µs |
| wide-shallow | two zstd tasks | 1.094 | 1.138 | 1.167 | 1.017 / 1.044 | 1.016 / 1.047 | 15056 µs | 13735 µs | 106632 µs | 54938 µs |
| deep-chain | none | 1.022 | 1.055 | 1.156 | 1.000 / 1.000 | 1.000 / 1.000 | 32357 µs | 31620 µs | 36881 µs | 34444 µs |
| deep-chain | two zstd tasks | 1.031 | 1.075 | 1.098 | 1.000 / 1.000 | 1.000 / 1.000 | 32840 µs | 31620 µs | 137702 µs | 34444 µs |
| fan-in-fan-out | none | 1.143 | 1.219 | 1.272 | 1.057 / 1.121 | 1.002 / 1.038 | 16193 µs | 14145 µs | 36228 µs | 31180 µs |
| fan-in-fan-out | two zstd tasks | 1.140 | 1.235 | 1.296 | 1.057 / 1.121 | 1.002 / 1.038 | 16169 µs | 14145 µs | 76232 µs | 31180 µs |

Greedy and longest-first are schedulers with no overhead that never idle a worker while a node is ready, simulated on the same draws; they differ in which ready node they take. Neither is a bound. CPU per run is the whole process, gaps between runs and any background load included; with no background load, what it exceeds the work by is what the executor spends around the jobs.

- wide-shallow: 192 independent nodes, 64 of them with one dependent (256 nodes).
- deep-chain: a 128-node chain; every fourth link forks a side node that rejoins two links on (160 nodes).
- fan-in-fan-out: 16 rounds of 1 node spreading to 8 (twice the 4 frame-critical workers) and joining to 1 (145 nodes).

## Wake latency (pass)

Push to a parked pool until the job starts; under 35 µs at P99. 5000 pushes, 2 ms apart.

| P50 | P99 | Max | Process CPU per second of this phase |
|---|---|---|---|
| 13.3 µs | 31.6 µs | 102.9 µs | 0.0379 s |

## Timer wake (FAIL)

From a deadline until the frame-critical task sleeping until it resumes, as the timer root ticks, with the pool otherwise idle. The 60 Hz P99 must be under 20 µs.

| Period | Deadlines | P50 | P99 | Max | Over 1 ms | Process CPU per second |
|---|---|---|---|---|---|---|
| 2.000 ms | 5000 | 2.1 µs | 23.2 µs | 191.2 µs | 0 | 0.0787 s |
| 16.667 ms | 5000 | 2.6 µs | 27.0 µs | 69.6 µs | 0 | 0.0108 s |

## Steal (pass)

One successful steal including the split, timed inside the executor on a thief thread taking from a deque another thread filled; under 1 µs at P99. 400000 steals. Each figure includes one pair of clock readings, 54 ns here.

| P50 | P99 | Max | Mean |
|---|---|---|---|
| 42 ns | 167 ns | 27625 ns | 68 ns |

Not a steal time, for reference: with every range forced down to single units, a node of empty units costs 491 worker-ns per entry (split, push, pop or steal, dispatch).
