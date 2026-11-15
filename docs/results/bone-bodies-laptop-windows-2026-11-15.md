# bone-bodies, laptop-windows, 2026-11-15

200 characters with a Rapier body on each of their sixteen bones, against none. Threshold: none: recorded (docs/verification.md).

| | |
|---|---|
| CPU | 12th Gen Intel(R) Core(TM) i7-12650H (windows Microsoft Windows [Version 10.0.26200.9974], x86_64) |
| Topology | 6 performance + 4 efficiency cores, as the executor detected them |
| Workers | 1 and 6 |
| Commit | `bef3455b4a56` |
| Profile | release |
| Command | `cargo xtask measure --all`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-app --test cost --run-ignored only --no-capture --no-tests=fail -E 'test(=cost_of_bone_bodies_on_rapier)'` |
| Processor idle | 56.9% over the run |

## Figures

| Figure | Value |
|---|---|
| `characters` | 200 |
| `bones` | 16 |
| `ticks` | 300 |
| `warm_up_ticks` | 50 |
| `workers` | 1 and 6 |
| `w1.with_us` | 21190.0 |
| `w1.without_us` | 329.8 |
| `w1.bodies_us` | 20860.2 |
| `w6.with_us` | 13094.9 |
| `w6.without_us` | 267.8 |
| `w6.bodies_us` | 12827.1 |

## Output

```text

running 1 test
measure: characters 200
measure: bones 16
measure: ticks 300
measure: warm_up_ticks 50
measure: workers 1 and 6
measure: w1.with_us 21190.0
measure: w1.without_us 329.8
measure: w1.bodies_us 20860.2
measure: w6.with_us 13094.9
measure: w6.without_us 267.8
measure: w6.bodies_us 12827.1
test cost_of_bone_bodies_on_rapier ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 14.80s
```
