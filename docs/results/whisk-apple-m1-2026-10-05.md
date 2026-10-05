# WHISK benchmark, apple-m1, 2026-10-05

Written by `cargo bench -p lactose-task --bench whisk`. Thresholds are those of
`crates/task/README.md`.

| | |
|---|---|
| CPU | Apple M1 (macos, aarch64) |
| Detected topology | 4 performance, 4 efficiency cores |
| Workers | 4 frame-critical, 4 background |
| Commit | `dfe7e70651cd` |
| Load average | 5.39 3.93 3.82 before, 4.97 4.60 4.16 after |
| Calibration | 0.5332 kernel iterations per ns; serial check 1.000 (1.0 is exact) |
| Runs per case | 2000 |

## Makespan divided by its lower bound (pass)

At most 1.25 at P50 and 1.5 at P99.

| Shape | Background load | P50 | P99 | Max | Greedy P50 / P99 | Longest-first P50 / P99 | Makespan | Lower bound | CPU per run | Work per run |
|---|---|---|---|---|---|---|---|---|---|---|
| wide-shallow | none | 1.099 | 1.312 | 19.641 | 1.017 / 1.044 | 1.016 / 1.047 | 15369 µs | 13735 µs | 59617 µs | 54938 µs |
| wide-shallow | two zstd tasks | 1.131 | 1.372 | 1.668 | 1.017 / 1.044 | 1.016 / 1.047 | 15866 µs | 13735 µs | 100786 µs | 54938 µs |
| deep-chain | none | 1.023 | 1.048 | 1.199 | 1.000 / 1.000 | 1.000 / 1.000 | 32386 µs | 31620 µs | 36973 µs | 34444 µs |
| deep-chain | two zstd tasks | 1.074 | 1.106 | 13.399 | 1.000 / 1.000 | 1.000 / 1.000 | 34183 µs | 31620 µs | 160999 µs | 34444 µs |
| fan-in-fan-out | none | 1.128 | 1.198 | 1.249 | 1.057 / 1.121 | 1.002 / 1.038 | 15978 µs | 14145 µs | 35812 µs | 31180 µs |
| fan-in-fan-out | two zstd tasks | 1.142 | 1.244 | 1.631 | 1.057 / 1.121 | 1.002 / 1.038 | 16205 µs | 14145 µs | 89209 µs | 31180 µs |

Greedy and longest-first are schedulers with no overhead that never idle a worker while a node is ready, simulated on the same draws; they differ in which ready node they take. Neither is a bound. CPU per run is the whole process, gaps between runs and any background load included; with no background load, what it exceeds the work by is what the executor spends around the jobs.

- wide-shallow: 192 independent nodes, 64 of them with one dependent (256 nodes).
- deep-chain: a 128-node chain; every fourth link forks a side node that rejoins two links on (160 nodes).
- fan-in-fan-out: 16 rounds of 1 node spreading to 8 (twice the 4 frame-critical workers) and joining to 1 (145 nodes).

## Wake latency (FAIL)

Push to a parked pool until the job starts; under 20 µs at P99. 5000 pushes, 2 ms apart.

| P50 | P99 | Max | Process CPU per second of this phase |
|---|---|---|---|
| 18.5 µs | 33.7 µs | 234.9 µs | 0.0366 s |

## Steal (pass)

One successful steal including the split, timed inside the executor on a thief thread taking from a deque another thread filled; under 1 µs at P99. 400000 steals. Each figure includes one pair of clock readings, 54 ns here.

| P50 | P99 | Max | Mean |
|---|---|---|---|
| 83 ns | 167 ns | 20458 ns | 70 ns |

Not a steal time, for reference: with every range forced down to single units, a node of empty units costs 493 worker-ns per entry (split, push, pop or steal, dispatch).
