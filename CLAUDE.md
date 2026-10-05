# LACTOSE

Rust game engine. Read `docs/architecture.md` before any task. It is binding.

## Names

Things you run get codenames, things you import get descriptions. Crates are `lactose-<role>`.
Never use a codename for a crate, type or function.

| Codename | Means |
|---|---|
| LACTOSE | the engine |
| CURD | `lactose-sim`, deterministic simulation core |
| WHISK | `lactose-task`, work-stealing executor |
| CASEIN | `lactose-ecs`, chunked archetype storage |
| COW | copy-on-write snapshots in `lactose-predict` |
| GOUDA | `lactose-render-graph` |
| CHEDDAR | GPU scene in `lactose-renderer` |
| BRIE | bindless heap in `lactose-renderer` |
| CREAM | GPU culling in `lactose-renderer` |
| LATTE | late-latched orbit camera in `lactose-renderer` |
| FROTH | VFX particles in `lactose-renderer` |
| QUARK | `lactose-physics-rapier` |
| MILKMAN | replication and interest management in `lactose-net`, log target `milkman` |
| SKIM | `lactose-persist`, saves |
| CARTON | save container format in `lactose-persist` |
| SILO | `lactose-asset` |
| RICOTTA | `tools/cook`, the asset cooker binary |
| PAIL | `lactose-platform` |
| SOUR | the determinism CI harness |

## Rules that CI enforces

- `#![forbid(unsafe_code)]` outside `task`, `ecs`, `io`, `rhi-*`, `core` and `tools/cook`.
- `crate-tiers.toml` is the only dependency rule: foundation crates may be used by anyone,
  backend crates only by `app`, `tools/*` and the listed exceptions. It also lists the crates
  inside the determinism boundary.
- Inside the boundary: no `Instant` or `SystemTime`; `core::FixedHashMap` only; no
  `thread_local!`; floats only through `math` (no `f32::min`/`max`/`powi`/`total_cmp`/`to_bits`
  outside it); no glam except through Rapier's pinned math in `physics-rapier`; no runtime CPU feature detection selecting different numerics; chunk,
  row, worker and task order are never semantic keys. Every ordering and RNG key is a stable
  key (`NetId`, `SystemKey`, `PredictionKey`), never `EntityId`.
- x86-64 builds pin the x86-64-v3 baseline; `math` fails to compile without `fma`.
- Every simulation test runs under SOUR in workers, repeat, layout and chaos modes and must
  hash identically. Architecture mode compares linux-x64, windows-x64 and macos-arm64.
- Components are `NoUninit + CheckedBitPattern` with explicit field IDs, typed reference
  fields and a persistence policy. `EntityId` never appears in a component, on the wire or on
  disk.
- Panics abort. Tests run under nextest, one process per test.
