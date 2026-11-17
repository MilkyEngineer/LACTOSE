# reload-100k, apple-m1, 2026-11-17

The full diff of a 100k-row region reloaded in place on Rapier (D124). Threshold: none: recorded (docs/verification.md).

| | |
|---|---|
| CPU | Apple M1 (macos 27.2.1, aarch64) |
| Topology | 4 performance + 4 efficiency cores, as the executor detected them |
| Workers | 4 |
| Commit | `a539d60461a8` |
| Profile | release |
| Command | `cargo xtask measure --all`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-app --test integration --run-ignored only --retries 0 --no-capture --no-tests=fail -E 'test(=cost::cost_of_a_100k_reload)'` |
| Load average | 3.80 2.70 2.91 before, 3.81 2.72 2.92 after |

## Figures

| Figure | Value |
|---|---|
| `rows` | 100000 |
| `bodies` | 2000 |
| `statics` | 50001 |
| `frame_workers` | 4 |
| `reloads` | 5 |
| `reload.step_us` | 338313.2 |
| `reload.sync_us` | 112353.4 |
| `tick.step_us` | 2041.5 |
| `tick.sync_us` | 546.6 |

## Output

```text

running 1 test
measure: rows 100000
measure: bodies 2000
measure: statics 50001
measure: frame_workers 4
measure: reloads 5
measure: reload.step_us 338313.2
measure: reload.sync_us 112353.4
measure: tick.step_us 2041.5
measure: tick.sync_us 546.6
test cost::cost_of_a_100k_reload ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 57 filtered out; finished in 2.25s
```
