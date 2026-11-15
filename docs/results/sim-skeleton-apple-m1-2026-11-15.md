# sim-skeleton, apple-m1, 2026-11-15

Criterion 6: the sim skeleton of 200 characters; figure (b) is the criterion's. Threshold: architecture section 22; milestone 1 plan, section 4.

| | |
|---|---|
| CPU | Apple M1 (macos 14.7.4, aarch64) |
| Topology | 4 performance + 4 efficiency cores, as the executor detected them |
| Workers | 1 and 4 |
| Commit | `bef3455b4a56` |
| Profile | release |
| Command | `cargo xtask measure --all`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-anim --test cost --run-ignored only --no-capture --no-tests=fail -E 'test(=the_sim_skeleton_budget)'` |
| Load average | 1.51 2.09 3.69 before, 1.51 2.07 3.66 after |

## Figures

| Figure | Value |
|---|---|
| `a.evaluate_us` | 160.5 |
| `characters` | 200 |
| `bones` | 16 |
| `ticks` | 1000 |
| `workers` | 1 and 4 |
| `b.w1.with_us` | 229.0 |
| `b.w1.without_us` | 48.2 |
| `b.w1.difference_us` | 180.8 |
| `b.w4.with_us` | 136.0 |
| `b.w4.without_us` | 37.5 |
| `b.w4.difference_us` | 98.4 |
| `c.test_backend.w1.with_us` | 2952.1 |
| `c.test_backend.w1.without_us` | 254.0 |
| `c.test_backend.w1.bodies_us` | 2698.2 |
| `c.test_backend.w4.with_us` | 2769.4 |
| `c.test_backend.w4.without_us` | 157.6 |
| `c.test_backend.w4.bodies_us` | 2611.8 |
| `c.no_physics.w1.with_us` | 1031.1 |
| `c.no_physics.w1.without_us` | 250.8 |
| `c.no_physics.w1.bodies_us` | 780.3 |
| `c.no_physics.w4.with_us` | 775.8 |
| `c.no_physics.w4.without_us` | 134.0 |
| `c.no_physics.w4.bodies_us` | 641.8 |

## Output

```text

running 1 test
measure: a.evaluate_us 160.5
(a) SimPose::evaluate, 200 characters, half blending: median 160.5 us, best 154.6 us per tick
measure: characters 200
measure: bones 16
measure: ticks 1000
measure: workers 1 and 4
measure: b.w1.with_us 229.0
measure: b.w1.without_us 48.2
measure: b.w1.difference_us 180.8
(b) Simulation::step, 1 worker(s): median 229.0 us with SimPose, 48.2 us without, 180.8 us difference per tick
measure: b.w4.with_us 136.0
measure: b.w4.without_us 37.5
measure: b.w4.difference_us 98.4
(b) Simulation::step, 4 worker(s): median 136.0 us with SimPose, 37.5 us without, 98.4 us difference per tick
measure: c.test_backend.w1.with_us 2952.1
measure: c.test_backend.w1.without_us 254.0
measure: c.test_backend.w1.bodies_us 2698.2
(c) a body on every bone, test backend, contacts off, 1 worker(s): median 2952.1 us with, 254.0 us without, 2698.2 us of bodies per tick
measure: c.test_backend.w4.with_us 2769.4
measure: c.test_backend.w4.without_us 157.6
measure: c.test_backend.w4.bodies_us 2611.8
(c) a body on every bone, test backend, contacts off, 4 worker(s): median 2769.4 us with, 157.6 us without, 2611.8 us of bodies per tick
measure: c.no_physics.w1.with_us 1031.1
measure: c.no_physics.w1.without_us 250.8
measure: c.no_physics.w1.bodies_us 780.3
(c) a body on every bone, no physics, 1 worker(s): median 1031.1 us with, 250.8 us without, 780.3 us of bodies per tick
measure: c.no_physics.w4.with_us 775.8
measure: c.no_physics.w4.without_us 134.0
measure: c.no_physics.w4.bodies_us 641.8
(c) a body on every bone, no physics, 4 worker(s): median 775.8 us with, 134.0 us without, 641.8 us of bodies per tick
test the_sim_skeleton_budget ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.63s
```
