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
| COW | copy-on-write snapshots in `lactose-sim` |
| GOUDA | `lactose-render-graph` |
| CHEDDAR | GPU scene in `lactose-renderer` |
| BRIE | bindless heap in `lactose-renderer` |
| CREAM | GPU culling in `lactose-renderer` |
| LATTE | late-latched view rotation in `lactose-renderer` |
| FROTH | VFX particles in `lactose-renderer` |
| QUARK | `lactose-physics-rapier` |
| MILKMAN | replication in `lactose-net`, log target `milkman` |
| SKIM | `lactose-persist`, saves |
| CARTON | save container format in `lactose-persist` |
| SILO | `lactose-asset` |
| RICOTTA | `tools/cook`, the asset cooker binary |
| PAIL | `lactose-platform` |
| SOUR | the determinism CI harness |

## Rules that CI enforces

- `#![forbid(unsafe_code)]` outside `task`, `ecs`, `io`, `rhi-*` and `core`.
- Dependency tiers from `crate-tiers.toml`: foundation crates may be used by anyone, backend
  crates only by `app`, `tools/*` and the exceptions listed there.
- Inside the determinism boundary (section 7 of the architecture doc): no `Instant` or
  `SystemTime`, no default-hasher `HashMap` (use `core::FixedHashMap`), no `thread_local!`,
  no platform float functions or `mul_add`, no runtime CPU feature detection, no chunk index
  or worker index as a semantic key. Enforced by clippy config and `cargo-deny` allowlists.
- Every simulation test runs under SOUR in workers, repeat and layout modes and must hash
  identically. Architecture mode compares x86-64 and aarch64.
- Components are padding-free plain data with explicit field IDs and a persistence policy.
