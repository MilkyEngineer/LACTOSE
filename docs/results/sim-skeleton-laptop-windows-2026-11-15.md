# sim-skeleton, laptop-windows, 2026-11-15

Criterion 6: the sim skeleton of 200 characters; figure (b) is the criterion's. Threshold: architecture section 22; milestone 1 plan, section 4.

| | |
|---|---|
| CPU | 12th Gen Intel(R) Core(TM) i7-12650H (windows Microsoft Windows [Version 10.0.26200.9974], x86_64) |
| Topology | 6 performance + 4 efficiency cores, as the executor detected them |
| Workers | 1 and 4 |
| Commit | `bef3455b4a56` |
| Profile | release |
| Command | `cargo xtask measure --all`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-anim --test cost --run-ignored only --no-capture --no-tests=fail -E 'test(=the_sim_skeleton_budget)'` |
| Processor idle | 48.1% over the run |

## Figures

| Figure | Value |
|---|---|
| `a.evaluate_us` | 112.3 |
| `characters` | 200 |
| `bones` | 16 |
| `ticks` | 1000 |
| `workers` | 1 and 4 |
| `b.w1.with_us` | 186.7 |
| `b.w1.without_us` | 38.8 |
| `b.w1.difference_us` | 147.9 |
| `b.w4.with_us` | 111.0 |
| `b.w4.without_us` | 40.9 |
| `b.w4.difference_us` | 70.1 |
| `c.test_backend.w1.with_us` | 4483.0 |
| `c.test_backend.w1.without_us` | 237.1 |
| `c.test_backend.w1.bodies_us` | 4245.9 |
| `c.test_backend.w4.with_us` | 5104.8 |
| `c.test_backend.w4.without_us` | 143.7 |
| `c.test_backend.w4.bodies_us` | 4961.1 |
| `c.no_physics.w1.with_us` | 1785.4 |
| `c.no_physics.w1.without_us` | 246.7 |
| `c.no_physics.w1.bodies_us` | 1538.7 |
| `c.no_physics.w4.with_us` | 1208.1 |
| `c.no_physics.w4.without_us` | 136.0 |
| `c.no_physics.w4.bodies_us` | 1072.1 |

## Output

```text

running 1 test
measure: a.evaluate_us 112.3
(a) SimPose::evaluate, 200 characters, half blending: median 112.3 us, best 111.8 us per tick
measure: characters 200
measure: bones 16
measure: ticks 1000
measure: workers 1 and 4
measure: b.w1.with_us 186.7
measure: b.w1.without_us 38.8
measure: b.w1.difference_us 147.9
(b) Simulation::step, 1 worker(s): median 186.7 us with SimPose, 38.8 us without, 147.9 us difference per tick
measure: b.w4.with_us 111.0
measure: b.w4.without_us 40.9
measure: b.w4.difference_us 70.1
(b) Simulation::step, 4 worker(s): median 111.0 us with SimPose, 40.9 us without, 70.1 us difference per tick
measure: c.test_backend.w1.with_us 4483.0
measure: c.test_backend.w1.without_us 237.1
measure: c.test_backend.w1.bodies_us 4245.9
(c) a body on every bone, test backend, contacts off, 1 worker(s): median 4483.0 us with, 237.1 us without, 4245.9 us of bodies per tick
measure: c.test_backend.w4.with_us 5104.8
measure: c.test_backend.w4.without_us 143.7
measure: c.test_backend.w4.bodies_us 4961.1
(c) a body on every bone, test backend, contacts off, 4 worker(s): median 5104.8 us with, 143.7 us without, 4961.1 us of bodies per tick
measure: c.no_physics.w1.with_us 1785.4
measure: c.no_physics.w1.without_us 246.7
measure: c.no_physics.w1.bodies_us 1538.7
(c) a body on every bone, no physics, 1 worker(s): median 1785.4 us with, 246.7 us without, 1538.7 us of bodies per tick
measure: c.no_physics.w4.with_us 1208.1
measure: c.no_physics.w4.without_us 136.0
measure: c.no_physics.w4.bodies_us 1072.1
(c) a body on every bone, no physics, 4 worker(s): median 1208.1 us with, 136.0 us without, 1072.1 us of bodies per tick
test the_sim_skeleton_budget ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 17.43s
```
