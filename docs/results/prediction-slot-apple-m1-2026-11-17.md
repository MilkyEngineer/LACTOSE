# prediction-slot, apple-m1, 2026-11-17

Criterion 5: the take and the restore of a prediction slot on the spike's thrown scene with Rapier. Threshold: architecture section 22; milestone 1 plan, section 4.

| | |
|---|---|
| CPU | Apple M1 (macos 27.2.1, aarch64) |
| Topology | 4 performance + 4 efficiency cores, as the executor detected them |
| Workers | 4 |
| Commit | `a539d60461a8` |
| Profile | release |
| Command | `cargo xtask measure --all`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-app --test integration --run-ignored only --retries 0 --no-capture --no-tests=fail -E 'test(=spike::cost_of_a_prediction_slot)'` |
| Load average | 3.59 2.69 2.91 before, 3.59 2.69 2.91 after |

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
| `world_snapshot_us` | 11.0 |
| `physics_snapshot_us` | 63.5 |
| `snapshot_us` | 74.4 |
| `record_us` | 56.2 |
| `take_us` | 130.6 |
| `world_restore_us` | 12.1 |
| `physics_restore_us` | 55.9 |
| `restore_us` | 68.1 |
| `world_bytes` | 91988 |
| `physics_bytes` | 251041 |
| `record_bytes` | 14892 |
| `ring_physics_bytes` | 15062460 |
| `ring_bytes_first_us` | 13223.1 |
| `ring_bytes_again_us` | 0.5 |

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
measure: world_snapshot_us 11.0
measure: physics_snapshot_us 63.5
measure: snapshot_us 74.4
measure: record_us 56.2
measure: take_us 130.6
measure: world_restore_us 12.1
measure: physics_restore_us 55.9
measure: restore_us 68.1
measure: world_bytes 91988
measure: physics_bytes 251041
measure: record_bytes 14892
measure: ring_physics_bytes 15062460
measure: ring_bytes_first_us 13223.1
measure: ring_bytes_again_us 0.5
test spike::cost_of_a_prediction_slot ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 57 filtered out; finished in 0.24s
```
