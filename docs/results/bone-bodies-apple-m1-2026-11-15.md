# bone-bodies, apple-m1, 2026-11-15

200 characters with a Rapier body on each of their sixteen bones, against none. Threshold: none: recorded (docs/verification.md).

| | |
|---|---|
| CPU | Apple M1 (macos 14.7.4, aarch64) |
| Topology | 4 performance + 4 efficiency cores, as the executor detected them |
| Workers | 1 and 4 |
| Commit | `bef3455b4a56` |
| Profile | release |
| Command | `cargo xtask measure --all`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-app --test cost --run-ignored only --no-capture --no-tests=fail -E 'test(=cost_of_bone_bodies_on_rapier)'` |
| Load average | 1.56 2.10 3.70 before, 1.51 2.09 3.69 after |

## Figures

| Figure | Value |
|---|---|
| `characters` | 200 |
| `bones` | 16 |
| `ticks` | 300 |
| `warm_up_ticks` | 50 |
| `workers` | 1 and 4 |
| `w1.with_us` | 7622.5 |
| `w1.without_us` | 239.9 |
| `w1.bodies_us` | 7382.5 |
| `w4.with_us` | 6277.2 |
| `w4.without_us` | 179.0 |
| `w4.bodies_us` | 6098.2 |

## Output

```text

running 1 test
measure: characters 200
measure: bones 16
measure: ticks 300
measure: warm_up_ticks 50
measure: workers 1 and 4
measure: w1.with_us 7622.5
measure: w1.without_us 239.9
measure: w1.bodies_us 7382.5
measure: w4.with_us 6277.2
measure: w4.without_us 179.0
measure: w4.bodies_us 6098.2
test cost_of_bone_bodies_on_rapier ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 5.11s
```
