# composed-tick, apple-m1, 2026-11-17

The 100k scene with 2,000 bodies on Rapier beside it, on one frame worker and on N. Threshold: none: recorded beside criteria 1 and 4 (milestone 1 plan, section 4).

| | |
|---|---|
| CPU | Apple M1 (macos 27.2.1, aarch64) |
| Topology | 4 performance + 4 efficiency cores, as the executor detected them |
| Workers | 1 and 4 |
| Commit | `a539d60461a8` |
| Profile | release |
| Command | `cargo xtask measure --all`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-app --test integration --run-ignored only --retries 0 --no-capture --no-tests=fail -E 'test(=cost::composed::cost_of_the_composed_tick)'` |
| Load average | 3.80 2.70 2.91 before, 3.80 2.70 2.91 after |

## Figures

| Figure | Value |
|---|---|
| `w1.step_us` | 13554.5 |
| `w1.physics_us` | 3083.2 |
| `w4.step_us` | 5728.4 |
| `w4.physics_us` | 2238.1 |
| `workers` | 1 and 4 |
| `rows` | 100064 |
| `bodies` | 2000 |
| `ticks` | 60 |
| `warm_up_ticks` | 10 |
| `n` | 4 |
| `t1_us` | 13554.5 |
| `tn_us` | 5728.4 |
| `s_us` | 3119.7 |
| `projection_us` | 4424.0 |

## Output

```text

running 1 test
measure: w1.step_us 13554.5
measure: w1.physics_us 3083.2
measure: w4.step_us 5728.4
measure: w4.physics_us 2238.1
measure: workers 1 and 4
measure: rows 100064
measure: bodies 2000
measure: ticks 60
measure: warm_up_ticks 10
measure: n 4
measure: t1_us 13554.5
measure: tn_us 5728.4
measure: s_us 3119.7
measure: projection_us 4424.0
test cost::composed::cost_of_the_composed_tick ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 57 filtered out; finished in 1.50s
```
