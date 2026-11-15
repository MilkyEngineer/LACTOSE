# prediction-slot, apple-m1, 2026-11-15

Criterion 5: the take and the restore of a prediction slot on the spike's thrown scene with Rapier. Threshold: architecture section 22; milestone 1 plan, section 4.

| | |
|---|---|
| CPU | Apple M1 (macos 14.7.4, aarch64) |
| Topology | 4 performance + 4 efficiency cores, as the executor detected them |
| Workers | 4 |
| Commit | `c50b68899fc5` |
| Profile | release |
| Command | `cargo xtask measure prediction-slot rollback-seven-ticks spike-net`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-app --test spike --run-ignored only --retries 0 --no-capture --no-tests=fail -E 'test(=cost_of_a_prediction_slot)'` |
| Load average | 2.96 2.75 5.85 before, 2.96 2.75 5.85 after |

## Figures

| Figure | Value |
|---|---|
| `workers` | 4 |
| `frames` | 152 |
| `tick` | 157 |
| `rounds` | 200 |
| `ring_depth` | 60 |
| `entities` | 115 |
| `claimed` | 51 |
| `fixed_colliders` | 5 |
| `proxies` | 59 |
| `world_snapshot_us` | 11.2 |
| `physics_snapshot_us` | 78.9 |
| `snapshot_us` | 90.0 |
| `record_us` | 61.5 |
| `take_us` | 151.5 |
| `world_restore_us` | 14.6 |
| `physics_restore_us` | 62.0 |
| `restore_us` | 76.5 |
| `world_bytes` | 91988 |
| `physics_bytes` | 251041 |
| `record_bytes` | 14892 |
| `ring_physics_bytes` | 15062460 |
| `ring_bytes_first_us` | 16268.0 |
| `ring_bytes_again_us` | 1.0 |

## Output

```text

running 1 test
measure: workers 4
measure: frames 152
measure: tick 157
measure: rounds 200
measure: ring_depth 60
measure: entities 115
measure: claimed 51
measure: fixed_colliders 5
measure: proxies 59
measure: world_snapshot_us 11.2
measure: physics_snapshot_us 78.9
measure: snapshot_us 90.0
measure: record_us 61.5
measure: take_us 151.5
measure: world_restore_us 14.6
measure: physics_restore_us 62.0
measure: restore_us 76.5
measure: world_bytes 91988
measure: physics_bytes 251041
measure: record_bytes 14892
measure: ring_physics_bytes 15062460
measure: ring_bytes_first_us 16268.0
measure: ring_bytes_again_us 1.0
test cost_of_a_prediction_slot ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 23 filtered out; finished in 0.25s
```
