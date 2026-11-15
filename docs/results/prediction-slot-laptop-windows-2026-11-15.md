# prediction-slot, laptop-windows, 2026-11-15

Criterion 5: the take and the restore of a prediction slot on the spike's thrown scene with Rapier. Threshold: architecture section 22; milestone 1 plan, section 4.

| | |
|---|---|
| CPU | 12th Gen Intel(R) Core(TM) i7-12650H (windows Microsoft Windows [Version 10.0.26200.9974], x86_64) |
| Topology | 6 performance + 4 efficiency cores, as the executor detected them |
| Workers | 4 |
| Commit | `bef3455b4a56` |
| Profile | release |
| Command | `cargo xtask measure --all`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-app --test spike --run-ignored only --no-capture --no-tests=fail -E 'test(=cost_of_a_prediction_slot)'` |
| Processor idle | 50.7% over the run |

## Figures

| Figure | Value |
|---|---|
| `workers` | 4 |
| `frames` | 152 |
| `tick` | 157 |
| `rounds` | 200 |
| `entities` | 115 |
| `claimed` | 51 |
| `fixed_colliders` | 5 |
| `proxies` | 59 |
| `world_snapshot_us` | 12.9 |
| `physics_snapshot_us` | 222.6 |
| `snapshot_us` | 235.5 |
| `record_us` | 86.1 |
| `take_us` | 321.6 |
| `world_restore_us` | 23.5 |
| `physics_restore_us` | 304.2 |
| `restore_us` | 327.5 |
| `world_bytes` | 91988 |
| `physics_bytes` | 251041 |
| `record_bytes` | 14892 |

## Output

```text

running 1 test
measure: workers 4
measure: frames 152
measure: tick 157
measure: rounds 200
measure: entities 115
measure: claimed 51
measure: fixed_colliders 5
measure: proxies 59
measure: world_snapshot_us 12.9
measure: physics_snapshot_us 222.6
measure: snapshot_us 235.5
measure: record_us 86.1
measure: take_us 321.6
measure: world_restore_us 23.5
measure: physics_restore_us 304.2
measure: restore_us 327.5
measure: world_bytes 91988
measure: physics_bytes 251041
measure: record_bytes 14892
test cost_of_a_prediction_slot ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 23 filtered out; finished in 0.83s
```
