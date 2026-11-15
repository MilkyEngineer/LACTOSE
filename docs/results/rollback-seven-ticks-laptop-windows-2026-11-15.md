# rollback-seven-ticks, laptop-windows, 2026-11-15

Criterion 7: a seven-tick rollback on the spike's thrown scene with Rapier. Threshold: none: recorded (architecture section 22; milestone 1 plan, section 4).

| | |
|---|---|
| CPU | 12th Gen Intel(R) Core(TM) i7-12650H (windows Microsoft Windows [Version 10.0.26200.9974], x86_64) |
| Topology | 6 performance + 4 efficiency cores, as the executor detected them |
| Workers | 4 |
| Commit | `bef3455b4a56` |
| Profile | release |
| Command | `cargo xtask measure --all`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-app --test spike --run-ignored only --no-capture --no-tests=fail -E 'test(=cost_of_a_seven_tick_rollback)'` |
| Processor idle | unavailable |

## Figures

| Figure | Value |
|---|---|
| `workers` | 4 |
| `frames` | 158 |
| `rollbacks` | 20 |
| `resimulated` | 7 |
| `rollback_us` | 10953.5 |
| `per_tick_us` | 1564.8 |

## Output

```text

running 1 test
measure: workers 4
measure: frames 158
measure: rollbacks 20
measure: resimulated 7
measure: rollback_us 10953.5
measure: per_tick_us 1564.8
test cost_of_a_seven_tick_rollback ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 23 filtered out; finished in 0.52s
```
