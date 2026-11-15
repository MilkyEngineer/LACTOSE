# physics-phase, laptop-windows, 2026-11-15

Criterion 4: the sync, step and write-back of 100k rows with 2,000 awake bodies on Rapier. Threshold: architecture section 22; milestone 1 plan, section 4.

| | |
|---|---|
| CPU | 12th Gen Intel(R) Core(TM) i7-12650H (windows Microsoft Windows [Version 10.0.26200.9974], x86_64) |
| Topology | 6 performance + 4 efficiency cores, as the executor detected them |
| Workers | 6 |
| Commit | `bef3455b4a56` |
| Profile | release |
| Command | `cargo xtask measure --all`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-app --test cost --run-ignored only --no-capture --no-tests=fail -E 'test(=cost_of_the_physics_phase_at_100k_rows)'` |
| Processor idle | unavailable |

## Figures

| Figure | Value |
|---|---|
| `rows` | 100000 |
| `bodies` | 2000 |
| `statics` | 50001 |
| `plain_rows` | 47999 |
| `frame_workers` | 6 |
| `solver_threads` | 5 |
| `ticks` | 60 |
| `warm_up_ticks` | 30 |
| `sync_us` | 801.0 |
| `step_us` | 2633.4 |
| `write_back_us` | 593.6 |
| `phase_us` | 4028.4 |
| `server_step_us` | 4220.7 |

## Output

```text

running 1 test
measure: rows 100000
measure: bodies 2000
measure: statics 50001
measure: plain_rows 47999
measure: frame_workers 6
measure: solver_threads 5
measure: ticks 60
measure: warm_up_ticks 30
measure: sync_us 801.0
measure: step_us 2633.4
measure: write_back_us 593.6
measure: phase_us 4028.4
measure: server_step_us 4220.7
test cost_of_the_physics_phase_at_100k_rows ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 0.81s
```
