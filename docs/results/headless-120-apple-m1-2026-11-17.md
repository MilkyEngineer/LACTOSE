# headless-120, apple-m1, 2026-11-17

The headless binary's 120-tick run and the world hash it prints. Threshold: none: its hash on x86-64 equals the M1's at the same commit (milestone 1 plan, section 4).

| | |
|---|---|
| CPU | Apple M1 (macos 27.2.1, aarch64) |
| Topology | 4 performance + 4 efficiency cores, as the executor detected them |
| Workers | 4 frame workers, the default executor's |
| Commit | `a539d60461a8` |
| Profile | dev |
| Command | `cargo xtask measure --all`, which ran `cargo run --locked -p lactose-app-headless -- 120` |
| Load average | 2.98 3.95 3.67 before, 2.98 3.95 3.67 after |

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
