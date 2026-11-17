# bone-bodies, apple-m1, 2026-11-17

200 characters with a Rapier body on each of their sixteen bones, against none. Threshold: none: recorded (docs/verification.md).

| | |
|---|---|
| CPU | Apple M1 (macos 27.2.1, aarch64) |
| Topology | 4 performance + 4 efficiency cores, as the executor detected them |
| Workers | 1 and 4 |
| Commit | `a539d60461a8` |
| Profile | release |
| Command | `cargo xtask measure --all`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-app --test integration --run-ignored only --retries 0 --no-capture --no-tests=fail -E 'test(=cost::cost_of_bone_bodies_on_rapier)'` |
| Load average | 3.81 2.72 2.92 before, 3.59 2.69 2.91 after |

## Figures

| Figure | Value |
|---|---|
| `characters` | 200 |
| `bones` | 16 |
| `ticks` | 300 |
| `warm_up_ticks` | 50 |
| `workers` | 1 and 4 |
| `w1.with_us` | 7314.9 |
| `w1.without_us` | 238.8 |
| `w1.bodies_us` | 7076.1 |
| `w4.with_us` | 6028.5 |
| `w4.without_us` | 182.7 |
| `w4.bodies_us` | 5845.8 |

## Output

```text

running 1 test
measure: characters 200
measure: bones 16
measure: ticks 300
measure: warm_up_ticks 50
measure: workers 1 and 4
measure: w1.with_us 7314.9
measure: w1.without_us 238.8
measure: w1.bodies_us 7076.1
measure: w4.with_us 6028.5
measure: w4.without_us 182.7
measure: w4.bodies_us 5845.8
test cost::cost_of_bone_bodies_on_rapier ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 57 filtered out; finished in 4.96s
```
