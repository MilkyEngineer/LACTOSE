# reload-100k, laptop-windows, 2026-11-15

The full diff of a 100k-row region reloaded in place on Rapier (D124). Threshold: none: recorded (docs/verification.md).

| | |
|---|---|
| CPU | 12th Gen Intel(R) Core(TM) i7-12650H (windows Microsoft Windows [Version 10.0.26200.9974], x86_64) |
| Topology | 6 performance + 4 efficiency cores, as the executor detected them |
| Workers | 6 |
| Commit | `bef3455b4a56` |
| Profile | release |
| Command | `cargo xtask measure --all`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-app --test cost --run-ignored only --no-capture --no-tests=fail -E 'test(=cost_of_a_100k_reload)'` |
| Processor idle | 58.0% over the run |

## Figures

| Figure | Value |
|---|---|
| `rows` | 100000 |
| `bodies` | 2000 |
| `statics` | 50001 |
| `frame_workers` | 6 |
| `reloads` | 5 |
| `reload.step_us` | 671913.7 |
| `reload.sync_us` | 206936.1 |
| `tick.step_us` | 4189.1 |
| `tick.sync_us` | 818.1 |

## Output

```text

running 1 test
measure: rows 100000
measure: bodies 2000
measure: statics 50001
measure: frame_workers 6
measure: reloads 5
measure: reload.step_us 671913.7
measure: reload.sync_us 206936.1
measure: tick.step_us 4189.1
measure: tick.sync_us 818.1
test cost_of_a_100k_reload ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 4.57s
```
