# rollback-seven-ticks, apple-m1, 2026-11-15

Criterion 7: a seven-tick rollback on the spike's thrown scene with Rapier. Threshold: none: recorded (architecture section 22; milestone 1 plan, section 4).

| | |
|---|---|
| CPU | Apple M1 (macos 14.7.4, aarch64) |
| Topology | 4 performance + 4 efficiency cores, as the executor detected them |
| Workers | 4 |
| Commit | `c50b68899fc5` |
| Profile | release |
| Command | `cargo xtask measure prediction-slot rollback-seven-ticks spike-net`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-app --test spike --run-ignored only --retries 0 --no-capture --no-tests=fail -E 'test(=cost_of_a_seven_tick_rollback)'` |
| Load average | 2.96 2.75 5.85 before, 3.44 2.86 5.87 after |

## Figures

| Figure | Value |
|---|---|
| `workers` | 4 |
| `frames` | 158 |
| `rollbacks` | 20 |
| `resimulated` | 7 |
| `rollback_us` | 2713.7 |
| `per_tick_us` | 387.7 |

## Output

```text

running 1 test
measure: workers 4
measure: frames 158
measure: rollbacks 20
measure: resimulated 7
measure: rollback_us 2713.7
measure: per_tick_us 387.7
test cost_of_a_seven_tick_rollback ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 23 filtered out; finished in 0.22s
```
