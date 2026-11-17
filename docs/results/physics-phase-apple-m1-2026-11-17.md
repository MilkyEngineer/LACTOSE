# physics-phase, apple-m1, 2026-11-17

Criterion 4: the sync, step and write-back of 100k rows with 2,000 awake bodies on Rapier. Threshold: architecture section 22; milestone 1 plan, section 4.

| | |
|---|---|
| CPU | Apple M1 (macos 27.2.1, aarch64) |
| Topology | 4 performance + 4 efficiency cores, as the executor detected them |
| Workers | 4 |
| Commit | `a539d60461a8` |
| Profile | release |
| Command | `cargo xtask measure --all`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-app --test integration --run-ignored only --retries 0 --no-capture --no-tests=fail -E 'test(=cost::cost_of_the_physics_phase_at_100k_rows)'` |
| Load average | 3.80 2.70 2.91 before, 3.80 2.70 2.91 after |

## Figures

| Figure | Value |
|---|---|
| `rows` | 100000 |
| `bodies` | 2000 |
| `statics` | 50001 |
| `plain_rows` | 47999 |
| `frame_workers` | 4 |
| `solver_threads` | 3 |
| `ticks` | 60 |
| `warm_up_ticks` | 30 |
| `sync_us` | 546.3 |
| `step_us` | 1004.5 |
| `write_back_us` | 404.4 |
| `phase_us` | 1953.5 |
| `server_step_us` | 2026.2 |

## Output

```text

running 1 test
measure: rows 100000
measure: bodies 2000
measure: statics 50001
measure: plain_rows 47999
measure: frame_workers 4
measure: solver_threads 3
measure: ticks 60
measure: warm_up_ticks 30
measure: sync_us 546.3
measure: step_us 1004.5
measure: write_back_us 404.4
measure: phase_us 1953.5
measure: server_step_us 2026.2
test cost::cost_of_the_physics_phase_at_100k_rows ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 57 filtered out; finished in 0.44s
```
