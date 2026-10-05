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
| LATTE | late-latching in `lactose-renderer` |
| FROTH | particles in `lactose-renderer` |
| QUARK | `lactose-physics-rapier` |
| MILKMAN | replication in `lactose-net`, log target `milkman` |
| SKIM | `lactose-persist`, saves |
| CARTON | save container format in `lactose-persist` |
| SILO | `lactose-asset` |
| RICOTTA | `tools/cook`, the asset cooker binary |
| PAIL | `lactose-platform` |
| SOUR | the determinism CI harness |

## Rules that CI enforces

- `#![forbid(unsafe_code)]` outside the crates listed in the architecture doc.
- No `std::time`, default-hasher `HashMap`, `thread_local!` or platform float functions inside
  the determinism boundary.
- Every simulation test runs twice with different worker counts and must hash identically.
- A crate depends only on `-api` crates, never on another implementation crate. `app` is the
  exception.
