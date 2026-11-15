# composed-tick, laptop-windows, 2026-11-15

The 100k scene with 2,000 bodies on Rapier beside it, on one frame worker and on N. Threshold: none: recorded beside criteria 1 and 4 (milestone 1 plan, section 4).

| | |
|---|---|
| CPU | 12th Gen Intel(R) Core(TM) i7-12650H (windows Microsoft Windows [Version 10.0.26200.9974], x86_64) |
| Topology | 6 performance + 4 efficiency cores, as the executor detected them |
| Workers | 1 and 6 |
| Commit | `bef3455b4a56` |
| Profile | release |
| Command | `cargo xtask measure --all`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-app --test cost --run-ignored only --no-capture --no-tests=fail -E 'test(=composed::cost_of_the_composed_tick)'` |
| Processor idle | 57.0% over the run |

## Figures

| Figure | Value |
|---|---|
| `w1.step_us` | 22858.4 |
| `w1.physics_us` | 8970.4 |
| `w6.step_us` | 8166.0 |
| `w6.physics_us` | 4230.1 |
| `workers` | 1 and 6 |
| `rows` | 100064 |
| `bodies` | 2000 |
| `ticks` | 60 |
| `warm_up_ticks` | 10 |
| `n` | 6 |
| `t1_us` | 22858.4 |
| `tn_us` | 8166.0 |
| `s_us` | 5227.5 |
| `projection_us` | 7431.4 |

## Output

```text

running 1 test
measure: w1.step_us 22858.4
measure: w1.physics_us 8970.4
measure: w6.step_us 8166.0
measure: w6.physics_us 4230.1
measure: workers 1 and 6
measure: rows 100064
measure: bodies 2000
measure: ticks 60
measure: warm_up_ticks 10
measure: n 6
measure: t1_us 22858.4
measure: tn_us 8166.0
measure: s_us 5227.5
measure: projection_us 7431.4
test composed::cost_of_the_composed_tick ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 2.50s
```
