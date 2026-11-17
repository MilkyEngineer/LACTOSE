# rollback-seven-ticks, apple-m1, 2026-11-17

Criterion 7: a seven-tick rollback on the spike's thrown scene with Rapier. Threshold: none: recorded (architecture section 22; milestone 1 plan, section 4).

| | |
|---|---|
| CPU | Apple M1 (macos 27.2.1, aarch64) |
| Topology | 4 performance + 4 efficiency cores, as the executor detected them |
| Workers | 4 |
| Commit | `a539d60461a8` |
| Profile | release |
| Command | `cargo xtask measure --all`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-app --test integration --run-ignored only --retries 0 --no-capture --no-tests=fail -E 'test(=spike::cost_of_a_seven_tick_rollback)'` |
| Load average | 3.59 2.69 2.91 before, 3.59 2.69 2.91 after |

## Figures

| Figure | Value |
|---|---|
| `workers` | 4 |
| `frames` | 158 |
| `rollbacks` | 20 |
| `resimulated` | 7 |
| `rollback_us` | 2674.5 |
| `per_tick_us` | 382.1 |

## Output

```text

running 1 test
measure: workers 4
measure: frames 158
measure: rollbacks 20
measure: resimulated 7
measure: rollback_us 2674.5
measure: per_tick_us 382.1
test spike::cost_of_a_seven_tick_rollback ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 57 filtered out; finished in 0.20s
```
