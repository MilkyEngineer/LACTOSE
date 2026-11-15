# composed-tick, apple-m1, 2026-11-15

The 100k scene with 2,000 bodies on Rapier beside it, on one frame worker and on N. Threshold: none: recorded beside criteria 1 and 4 (milestone 1 plan, section 4).

| | |
|---|---|
| CPU | Apple M1 (macos 14.7.4, aarch64) |
| Topology | 4 performance + 4 efficiency cores, as the executor detected them |
| Workers | 1 and 4 |
| Commit | `bef3455b4a56` |
| Profile | release |
| Command | `cargo xtask measure --all`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-app --test cost --run-ignored only --no-capture --no-tests=fail -E 'test(=composed::cost_of_the_composed_tick)'` |
| Load average | 1.43 2.09 3.71 before, 1.43 2.09 3.71 after |

## Figures

| Figure | Value |
|---|---|
| `w1.step_us` | 13808.9 |
| `w1.physics_us` | 3156.1 |
| `w4.step_us` | 5772.5 |
| `w4.physics_us` | 2265.8 |
| `workers` | 1 and 4 |
| `rows` | 100064 |
| `bodies` | 2000 |
| `ticks` | 60 |
| `warm_up_ticks` | 10 |
| `n` | 4 |
| `t1_us` | 13808.9 |
| `tn_us` | 5772.5 |
| `s_us` | 3093.7 |
| `projection_us` | 4433.1 |

## Output

```text

running 1 test
measure: w1.step_us 13808.9
measure: w1.physics_us 3156.1
measure: w4.step_us 5772.5
measure: w4.physics_us 2265.8
measure: workers 1 and 4
measure: rows 100064
measure: bodies 2000
measure: ticks 60
measure: warm_up_ticks 10
measure: n 4
measure: t1_us 13808.9
measure: tn_us 5772.5
measure: s_us 3093.7
measure: projection_us 4433.1
test composed::cost_of_the_composed_tick ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 1.54s
```
