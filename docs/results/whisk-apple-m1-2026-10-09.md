# WHISK benchmark, apple-m1, 2026-10-09

Written by `cargo bench -p lactose-task --bench whisk`. Thresholds are those of
`crates/task/README.md`.

| | |
|---|---|
| CPU | Apple M1 (macos, aarch64) |
| Detected topology | 4 performance, 4 efficiency cores |
| Workers | 4 frame-critical, 4 background |
| Commit | `00f1f3d6d035` |
| Load average | 3.73 3.71 4.86 before, 1.73 3.39 4.42 after |
| Task role | TASK_FOREGROUND_APPLICATION (set returned 0, get returned 0) |
| Calibration | 0.5157 kernel iterations per ns; serial check 1.023 (1.0 is exact) |
| Runs per case | 2000 |

## Makespan divided by its lower bound (pass)

At most 1.25 at P50 and 1.5 at P99.

| Shape | Background load | P50 | P99 | Max | Greedy P50 / P99 | Longest-first P50 / P99 | Makespan | Lower bound | CPU per run | Work per run |
|---|---|---|---|---|---|---|---|---|---|---|
| wide-shallow | none | 1.062 | 1.125 | 1.185 | 1.017 / 1.044 | 1.016 / 1.047 | 14636 µs | 13735 µs | 57521 µs | 54938 µs |
| wide-shallow | two zstd tasks | 1.064 | 1.197 | 1.281 | 1.017 / 1.044 | 1.016 / 1.047 | 14752 µs | 13735 µs | 89361 µs | 54938 µs |
| deep-chain | none | 1.000 | 1.051 | 1.070 | 1.000 / 1.000 | 1.000 / 1.000 | 31848 µs | 31620 µs | 36343 µs | 34444 µs |
| deep-chain | two zstd tasks | 1.038 | 1.041 | 1.057 | 1.000 / 1.000 | 1.000 / 1.000 | 32838 µs | 31620 µs | 153495 µs | 34444 µs |
| fan-in-fan-out | none | 1.099 | 1.170 | 1.202 | 1.057 / 1.121 | 1.002 / 1.038 | 15565 µs | 14145 µs | 34807 µs | 31180 µs |
| fan-in-fan-out | two zstd tasks | 1.103 | 1.172 | 1.263 | 1.057 / 1.121 | 1.002 / 1.038 | 15635 µs | 14145 µs | 84172 µs | 31180 µs |

Greedy and longest-first are schedulers with no overhead that never idle a worker while a node is ready, simulated on the same draws; they differ in which ready node they take. Neither is a bound. CPU per run is the whole process, gaps between runs and any background load included; with no background load, what it exceeds the work by is what the executor spends around the jobs.

- wide-shallow: 192 independent nodes, 64 of them with one dependent (256 nodes).
- deep-chain: a 128-node chain; every fourth link forks a side node that rejoins two links on (160 nodes).
- fan-in-fan-out: 16 rounds of 1 node spreading to 8 (twice the 4 frame-critical workers) and joining to 1 (145 nodes).

## Wake latency (pass)

Push to a parked pool until the job starts; under 35 µs at P99. 5000 pushes, 2 ms apart.

| P50 | P99 | Max | Process CPU per second of this phase |
|---|---|---|---|
| 10.1 µs | 32.8 µs | 55.6 µs | 0.0333 s |

## Timer wake (FAIL)

From a deadline until the frame-critical task sleeping until it resumes, as the timer root ticks, with the pool otherwise idle. The 60 Hz P99 must be under 20 µs.

| Period | Deadlines | P50 | P99 | Max | Over 1 ms | Process CPU per second |
|---|---|---|---|---|---|---|
| 2.000 ms | 5000 | 1.4 µs | 20.5 µs | 126.0 µs | 0 | 0.0761 s |
| 16.667 ms | 5000 | 3.7 µs | 24.5 µs | 46.3 µs | 0 | 0.0102 s |

## Steal (pass)

One successful steal including the split, timed inside the executor on a thief thread taking from a deque another thread filled; under 1 µs at P99. 400000 steals. Each figure includes one pair of clock readings, 50 ns here.

| P50 | P99 | Max | Mean |
|---|---|---|---|
| 42 ns | 125 ns | 17292 ns | 62 ns |

Not a steal time, for reference: with every range forced down to single units, a node of empty units costs 494 worker-ns per entry (split, push, pop or steal, dispatch).
