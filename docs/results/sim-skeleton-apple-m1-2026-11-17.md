# sim-skeleton, apple-m1, 2026-11-17

Criterion 6: the sim skeleton of 200 characters; figure (b) is the criterion's. Threshold: architecture section 22; milestone 1 plan, section 4.

| | |
|---|---|
| CPU | Apple M1 (macos 27.2.1, aarch64) |
| Topology | 4 performance + 4 efficiency cores, as the executor detected them |
| Workers | 1 and 4 |
| Commit | `a539d60461a8` |
| Profile | release |
| Command | `cargo xtask measure --all`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-anim --test integration --run-ignored only --retries 0 --no-capture --no-tests=fail -E 'test(=cost::the_sim_skeleton_budget)'` |
| Load average | 4.88 3.88 3.38 before, 4.58 3.85 3.38 after |

## Figures

| Figure | Value |
|---|---|
| `a.evaluate_us` | 160.3 |
| `characters` | 200 |
| `bones` | 16 |
| `ticks` | 1000 |
| `workers` | 1 and 4 |
| `b.w1.with_us` | 227.9 |
| `b.w1.without_us` | 45.7 |
| `b.w1.difference_us` | 182.2 |
| `b.w4.with_us` | 123.8 |
| `b.w4.without_us` | 38.8 |
| `b.w4.difference_us` | 85.0 |
| `c.test_backend.w1.with_us` | 2883.5 |
| `c.test_backend.w1.without_us` | 254.3 |
| `c.test_backend.w1.bodies_us` | 2629.2 |
| `c.test_backend.w4.with_us` | 2698.4 |
| `c.test_backend.w4.without_us` | 154.1 |
| `c.test_backend.w4.bodies_us` | 2544.2 |
| `c.no_physics.w1.with_us` | 997.9 |
| `c.no_physics.w1.without_us` | 251.8 |
| `c.no_physics.w1.bodies_us` | 746.1 |
| `c.no_physics.w4.with_us` | 759.4 |
| `c.no_physics.w4.without_us` | 150.9 |
| `c.no_physics.w4.bodies_us` | 608.5 |

## Output

```text

running 1 test
measure: a.evaluate_us 160.3
(a) SimPose::evaluate, 200 characters, half blending: median 160.3 us, best 132.0 us per tick
measure: characters 200
measure: bones 16
measure: ticks 1000
measure: workers 1 and 4
measure: b.w1.with_us 227.9
measure: b.w1.without_us 45.7
measure: b.w1.difference_us 182.2
(b) Simulation::step, 1 worker(s): median 227.9 us with SimPose, 45.7 us without, 182.2 us difference per tick
measure: b.w4.with_us 123.8
measure: b.w4.without_us 38.8
measure: b.w4.difference_us 85.0
(b) Simulation::step, 4 worker(s): median 123.8 us with SimPose, 38.8 us without, 85.0 us difference per tick
measure: c.test_backend.w1.with_us 2883.5
measure: c.test_backend.w1.without_us 254.3
measure: c.test_backend.w1.bodies_us 2629.2
(c) a body on every bone, test backend, contacts off, 1 worker(s): median 2883.5 us with, 254.3 us without, 2629.2 us of bodies per tick
measure: c.test_backend.w4.with_us 2698.4
measure: c.test_backend.w4.without_us 154.1
measure: c.test_backend.w4.bodies_us 2544.2
(c) a body on every bone, test backend, contacts off, 4 worker(s): median 2698.4 us with, 154.1 us without, 2544.2 us of bodies per tick
measure: c.no_physics.w1.with_us 997.9
measure: c.no_physics.w1.without_us 251.8
measure: c.no_physics.w1.bodies_us 746.1
(c) a body on every bone, no physics, 1 worker(s): median 997.9 us with, 251.8 us without, 746.1 us of bodies per tick
measure: c.no_physics.w4.with_us 759.4
measure: c.no_physics.w4.without_us 150.9
measure: c.no_physics.w4.bodies_us 608.5
(c) a body on every bone, no physics, 4 worker(s): median 759.4 us with, 150.9 us without, 608.5 us of bodies per tick
test cost::the_sim_skeleton_budget ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 83 filtered out; finished in 9.39s
```
