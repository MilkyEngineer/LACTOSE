# physics-phase, apple-m1, 2026-11-15

Criterion 4: the sync, step and write-back of 100k rows with 2,000 awake bodies on Rapier. Threshold: architecture section 22; milestone 1 plan, section 4.

| | |
|---|---|
| CPU | Apple M1 (macos 14.7.4, aarch64) |
| Topology | 4 performance + 4 efficiency cores, as the executor detected them |
| Workers | 4 |
| Commit | `bef3455b4a56` |
| Profile | release |
| Command | `cargo xtask measure --all`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-app --test cost --run-ignored only --no-capture --no-tests=fail -E 'test(=cost_of_the_physics_phase_at_100k_rows)'` |
| Load average | 1.43 2.09 3.71 before, 1.43 2.09 3.71 after |

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
| `sync_us` | 554.3 |
| `step_us` | 985.4 |
| `write_back_us` | 419.0 |
| `phase_us` | 1954.3 |
| `server_step_us` | 2027.6 |

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
measure: sync_us 554.3
measure: step_us 985.4
measure: write_back_us 419.0
measure: phase_us 1954.3
measure: server_step_us 2027.6
test cost_of_the_physics_phase_at_100k_rows ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 0.45s
```
