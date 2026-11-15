| | |
|---|---|
| Command | `cargo xtask measure --all`, which ran `WHISK_BENCH_MACHINE=laptop-windows cargo bench --locked -p lactose-task --bench whisk`, its files written under the target directory and moved here |
| Profile | bench |
| OS | windows Microsoft Windows [Version 10.0.26200.9974] (x86_64) |
| Processor idle | 68.0% over the run |

# WHISK benchmark, laptop-windows, 2026-11-15

Written by `cargo bench -p lactose-task --bench whisk`. Thresholds are those of
`crates/task/README.md`.

| | |
|---|---|
| CPU | Intel64 Family 6 Model 154 Stepping 3, GenuineIntel (windows, x86_64) |
| Detected topology | 6 performance, 4 efficiency cores |
| Workers | 6 frame-critical, 4 background |
| Commit | `bef3455b4a56` |
| Load average | unknown before, unknown after |
| Task role | not applicable |
| Calibration | 0.5304 kernel iterations per ns; serial check 1.016 (1.0 is exact) |
| Runs per case | 2000 |

## Makespan divided by its lower bound (FAIL)

At most 1.25 at P50 and 1.5 at P99.

| Shape | Background load | P50 | P99 | Max | Greedy P50 / P99 | Longest-first P50 / P99 | Makespan | Lower bound | CPU per run | Work per run |
|---|---|---|---|---|---|---|---|---|---|---|
| wide-shallow | none | 1.137 | 4.023 | 6.734 | 1.034 / 1.072 | 1.035 / 1.078 | 12650 µs | 9156 µs | null µs | 54938 µs |
| wide-shallow | two zstd tasks | 1.095 | 1.368 | 1.958 | 1.034 / 1.072 | 1.035 / 1.078 | 10121 µs | 9156 µs | null µs | 54938 µs |
| deep-chain | none | 1.078 | 2.768 | 4.921 | 1.000 / 1.000 | 1.000 / 1.000 | 37692 µs | 31620 µs | null µs | 34444 µs |
| deep-chain | two zstd tasks | 1.079 | 1.190 | 1.599 | 1.000 / 1.000 | 1.000 / 1.000 | 34313 µs | 31620 µs | null µs | 34444 µs |
| fan-in-fan-out | none | 2.588 | 6.576 | 7.763 | 1.049 / 1.106 | 1.000 / 1.015 | 44418 µs | 15402 µs | null µs | 45043 µs |
| fan-in-fan-out | two zstd tasks | 1.226 | 2.435 | 2.981 | 1.049 / 1.106 | 1.000 / 1.015 | 21126 µs | 15402 µs | null µs | 45043 µs |

Greedy and longest-first are schedulers with no overhead that never idle a worker while a node is ready, simulated on the same draws; they differ in which ready node they take. Neither is a bound. CPU per run is the whole process, gaps between runs and any background load included; with no background load, what it exceeds the work by is what the executor spends around the jobs.

- wide-shallow: 192 independent nodes, 64 of them with one dependent (256 nodes).
- deep-chain: a 128-node chain; every fourth link forks a side node that rejoins two links on (160 nodes).
- fan-in-fan-out: 16 rounds of 1 node spreading to 12 (twice the 6 frame-critical workers) and joining to 1 (209 nodes).

## Wake latency (FAIL)

Push to a parked pool until the job starts; under 35 µs at P99. 5000 pushes, 2 ms apart.

| P50 | P99 | Max | Process CPU per second of this phase |
|---|---|---|---|
| 145.9 µs | 236.4 µs | 828.6 µs | null s |

## Timer wake (FAIL)

From a deadline until the frame-critical task sleeping until it resumes, as the timer root ticks, with the pool otherwise idle. The 60 Hz P99 must be under 20 µs.

| Period | Deadlines | P50 | P99 | Max | Over 1 ms | Process CPU per second |
|---|---|---|---|---|---|---|
| 2.000 ms | 5000 | 14919.5 µs | 15505.9 µs | 16117.3 µs | 4998 | null s |
| 16.667 ms | 5000 | 4339.8 µs | 15918.4 µs | 16412.3 µs | 4429 | null s |

## Steal (pass)

One successful steal including the split, timed inside the executor on a thief thread taking from a deque another thread filled; under 1 µs at P99. 400000 steals. Each figure includes one pair of clock readings, 72 ns here.

| P50 | P99 | Max | Mean |
|---|---|---|---|
| 100 ns | 700 ns | 245100 ns | 231 ns |

Not a steal time, for reference: with every range forced down to single units, a node of empty units costs 586 worker-ns per entry (split, push, pop or steal, dispatch).
