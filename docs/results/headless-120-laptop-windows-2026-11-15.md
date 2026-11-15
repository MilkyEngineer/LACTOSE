# headless-120, laptop-windows, 2026-11-15

The headless binary's 120-tick run and the world hash it prints. Threshold: none: its hash on x86-64 equals the M1's at the same commit (milestone 1 plan, section 4).

| | |
|---|---|
| CPU | 12th Gen Intel(R) Core(TM) i7-12650H (windows Microsoft Windows [Version 10.0.26200.9974], x86_64) |
| Topology | 6 performance + 4 efficiency cores, as the executor detected them |
| Workers | 6 frame workers, the default executor's |
| Commit | `bef3455b4a56` |
| Profile | dev |
| Command | `cargo xtask measure --all`, which ran `cargo run --locked -p lactose-app-headless -- 120` |
| Processor idle | 63.5% over the run |

## Figures

| Figure | Value |
|---|---|
| `ticks` | 120 |
| `frames` | 121 |
| `behind` | 0 |
| `hash` | 0x92ff315464f84575 |

## Output

```text
120 ticks in 121 frames (0 behind), hash 0x92ff315464f84575
```
