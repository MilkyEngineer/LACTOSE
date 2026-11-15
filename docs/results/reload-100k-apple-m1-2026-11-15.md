# reload-100k, apple-m1, 2026-11-15

The full diff of a 100k-row region reloaded in place on Rapier (D124). Threshold: none: recorded (docs/verification.md).

| | |
|---|---|
| CPU | Apple M1 (macos 14.7.4, aarch64) |
| Topology | 4 performance + 4 efficiency cores, as the executor detected them |
| Workers | 4 |
| Commit | `bef3455b4a56` |
| Profile | release |
| Command | `cargo xtask measure --all`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-app --test cost --run-ignored only --no-capture --no-tests=fail -E 'test(=cost_of_a_100k_reload)'` |
| Load average | 1.43 2.09 3.71 before, 1.56 2.10 3.70 after |

## Figures

| Figure | Value |
|---|---|
| `rows` | 100000 |
| `bodies` | 2000 |
| `statics` | 50001 |
| `frame_workers` | 4 |
| `reloads` | 5 |
| `reload.step_us` | 392459.1 |
| `reload.sync_us` | 126951.0 |
| `tick.step_us` | 2266.2 |
| `tick.sync_us` | 566.2 |

## Output

```text

running 1 test
measure: rows 100000
measure: bodies 2000
measure: statics 50001
measure: frame_workers 4
measure: reloads 5
measure: reload.step_us 392459.1
measure: reload.sync_us 126951.0
measure: tick.step_us 2266.2
measure: tick.sync_us 566.2
test cost_of_a_100k_reload ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 2.58s
```
