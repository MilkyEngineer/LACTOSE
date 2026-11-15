# tick-100k, laptop-windows, 2026-11-15

Criterion 1: the median step of the 100k scene on one frame worker and on N, S and the projected eight-core tick. Threshold: architecture section 22; milestone 1 plan, section 4 and decision 5.1.

| | |
|---|---|
| CPU | 12th Gen Intel(R) Core(TM) i7-12650H (windows Microsoft Windows [Version 10.0.26200.9974], x86_64) |
| Topology | 6 performance + 4 efficiency cores, as the executor detected them |
| Workers | 1 and 6 |
| Commit | `bef3455b4a56` |
| Profile | release |
| Command | `cargo xtask measure --all`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-sim --test cost --run-ignored only --no-capture --no-tests=fail -E 'test(=cost_of_a_tick_at_100k_rows)'` |
| Processor idle | 64.9% over the run |

## Figures

| Figure | Value |
|---|---|
| `rows` | 100064 |
| `archetypes` | 5 |
| `regions` | 16 |
| `systems` | 16 |
| `ticks` | 60 |
| `warm_up_ticks` | 10 |
| `workers` | 1 and 6 |
| `hash` | 0x11c1a653b85d7d3d |
| `topology.performance` | 6 |
| `topology.efficiency` | 4 |
| `n` | 6 |
| `t1_us` | 13856.0 |
| `tn_us` | 4689.9 |
| `s_us` | 2856.7 |
| `projection_us` | 4231.6 |

## Output

```text

running 1 test
1 frame worker(s), 100064 rows, 60 ticks: median / p90 in µs
  step (whole)                               13856.0   15820.2
  node casein::events                           43.9      52.6
  node casein::sync                            532.3     619.8
  sync_point closing                             1.2       1.5
  sync_point sim::input                        301.6     374.9
  sync_point sim::physics                      141.3     164.0
  sync_point sim::post                           0.2       0.2
  sync_point sim::simulation                     0.5       0.6
  system cost::drag                            920.0    1163.5
  system cost::energy                         1217.5    1644.7
  system cost::input_writer                      0.8       1.0
  system cost::integrate                      1767.3    2040.2
  system cost::steer                            16.7      18.0
  system cost::strike                         2409.5    2677.5
  system cost::take_hits                      3927.1    4768.8
  system cost::timers                         1983.2    2296.4
  system cost::touch                           646.7     802.3
  system cost::weather                          11.1      13.8
  system sim::apply_frame_regions                3.8       4.2
  system sim::apply_inputs                      30.1      33.0
  system sim::apply_region_policy                1.2       1.4
  system sim::apply_sessions                     5.5       6.1
  system sim::apply_staged_state                 1.2       1.4
  system sim::begin_tick                         7.8       9.0
  hash (all regions)                         26308.9   33869.5
  hash (one region)                           1574.2
6 frame worker(s), 100064 rows, 60 ticks: median / p90 in µs
  step (whole)                                4689.9    5550.5
  node casein::events                           56.0      65.6
  node casein::sync                            906.7    1049.4
  sync_point closing                             1.4       2.0
  sync_point sim::input                        585.9     697.0
  sync_point sim::physics                      182.4     265.4
  sync_point sim::post                           0.2       0.3
  sync_point sim::simulation                     0.6       0.8
  system cost::drag                           1565.5    1830.7
  system cost::energy                         2222.0    2476.6
  system cost::input_writer                      0.8       1.1
  system cost::integrate                      2633.6    2997.8
  system cost::steer                            23.2      26.9
  system cost::strike                         3916.0    4705.7
  system cost::take_hits                      5260.1    5906.2
  system cost::timers                         3148.9    3635.3
  system cost::touch                          1048.4    1265.2
  system cost::weather                          15.4      23.0
  system sim::apply_frame_regions                5.0       5.9
  system sim::apply_inputs                      39.0      46.5
  system sim::apply_region_policy                1.5       1.8
  system sim::apply_sessions                     6.7       8.1
  system sim::apply_staged_state                 1.5       2.0
  system sim::begin_tick                         9.3      10.8
  hash (all regions)                         34363.6   60631.1
  hash (one region)                           1846.8
measure: rows 100064
measure: archetypes 5
measure: regions 16
measure: systems 16
measure: ticks 60
measure: warm_up_ticks 10
measure: workers 1 and 6
measure: hash 0x11c1a653b85d7d3d
measure: topology.performance 6
measure: topology.efficiency 4
measure: n 6
measure: t1_us 13856.0
measure: tn_us 4689.9
measure: s_us 2856.7
measure: projection_us 4231.6
T1 13856.0 µs, T6 4689.9 µs, S 2856.7 µs, projected eight-core tick 4231.6 µs (at most 4,000)
test cost_of_a_tick_at_100k_rows ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out; finished in 6.44s
```
