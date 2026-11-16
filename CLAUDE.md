# LACTOSE

Revision marker: architecture 3.4. At task start run `git show HEAD:CLAUDE.md | head -3` and
confirm the marker matches this text; if not, read the file on disk, not this copy.

Rust game engine. Read `docs/architecture.md` before any task. It is binding. `docs/decisions-log.md`
holds the reasons; `docs/verification.md` holds what must be measured or re-checked first.
The work is done by agents under the process in `docs/agents.md`; their definitions are in
`.claude/agents/`.

## Names

Things you run get codenames, things you import get descriptions. Crates are `lactose-<role>`.
Never use a codename for a crate, type, function, module, constant or static, in test code
as elsewhere; a test or bench target may carry one.

| Codename | Means |
|---|---|
| LACTOSE | the engine |
| CURD | `lactose-sim`, deterministic simulation core |
| WHISK | `lactose-task`, work-stealing executor |
| CASEIN | `lactose-ecs`, chunked archetype storage |
| COW | copy-on-write snapshots in `lactose-predict` (not built: D139, D140) |
| GOUDA | `lactose-render-graph` |
| CHEDDAR | GPU scene in `lactose-renderer` |
| BRIE | bindless heap in `lactose-renderer` |
| CREAM | GPU culling in `lactose-renderer` |
| LATTE | late-latched orbit camera in `lactose-renderer` |
| FROTH | GPU VFX in `lactose-renderer` |
| QUARK | `lactose-physics-rapier` |
| MILKMAN | replication and interest management in `lactose-net`, log target `milkman` |
| SKIM | `lactose-persist`, saves |
| CARTON | save container format in `lactose-persist` |
| SILO | `lactose-asset` |
| RICOTTA | `tools/cook`, the asset cooker binary |
| PAIL | `lactose-platform` |
| SOUR | the determinism CI harness |

## Rules that CI enforces

- `#![forbid(unsafe_code)]` outside the crates in architecture section 21.
- `crate-tiers.toml` is the only dependency rule and the only boundary list. Every crate must
  appear in it.
- Inside the determinism boundary, the rules of architecture section 8.2 apply verbatim; this
  file does not paraphrase them. Every reference, ordering rule and RNG key is a `StableKey`,
  never an `EntityId`.
- x86-64 builds pin the x86-64-v3 baseline through `.cargo/config.toml` target rustflags;
  never set `RUSTFLAGS` in the environment.
- Every simulation test runs under SOUR; a pull request runs workers mode and one layout
  capacity, nightly runs the rest. Hashes must match.
- Components use `#[component(key = "...")]`, are `NoUninit + CheckedBitPattern`, carry
  explicit field IDs, typed reference fields and a persistence policy.
- Panics abort in binaries. Tests run under nextest, one process per test, with no abort hook.
- Systems use the `#[system]` macro of architecture section 6.1 and declare the worlds they run in.
- What a world registers is held to section 8.2's rules wherever it is declared, and is declared in
  a `[boundary]` crate or a crate of `[lint_file_carriers]` in `tools/xtask/ci-policy.toml`, or in
  tests alone: a `#[system]`, a `#[component]` or `#[resource]` that is not `derived`, an applier, a
  hand-written `impl Reflect`, a registration's setup and migration functions. A system that writes
  hashed state is declared in a library and runs in a test that links it and records under SOUR
  (architecture sections 8.1 and 8.3).
