# game-sim

The inside half of the game (architecture sections 3 and 8.1): gameplay that every world kind
registers and runs, held to every rule of section 8.2. At milestone 2 it holds the sketch game
the headless binary runs, moved out of the binary so that what a world registers is declared
inside the boundary and the recorded test runs the system that ships (D152; milestone 2 plan,
decision 5.18). The game arrives at milestone 5 (section 22).

| | |
|---|---|
| Tier | game |
| Determinism boundary | inside; carries `clippy.toml`, a byte-identical copy of `crates/core/clippy.toml` |
| Unsafe | none, `#![forbid(unsafe_code)]` |

## What it holds

| Item | Contents |
|---|---|
| `Counter` | The sketch's one component, key `sketch::counter`, persistent and replicated: a `u32` under field ID 1 |
| `register_types` | A registration's `types`: registers `Counter` |
| `register_systems` | A registration's `systems`: adds `sketch::count`, which adds one to every `Counter` each tick, wrapping, in the server and prediction worlds, to `Stage::Simulation` |
| `sessions` | A `Game`'s `sessions`: the session applier. A `Join` spawns the player's `Counter` at zero with `ClaimedBy`, under a key from `World::allocate_net_id`, in the join's region; a `Leave` keeps the row, claim included |

## Rules a caller should know

- A game crate may name neither `lactose-app` nor `lactose-persist` (`crate-tiers.toml`), so
  the `CrateRegistration` and `Game` values are the binary's: `crates/app-headless/src/main.rs`
  builds them from the functions above, with `setup` and `migrations` both `None`. Where a game
  crate's registration value and its migration functions are declared is decided before
  milestone 5 (`docs/verification.md`).
- The component key and the system name keep the `sketch::` prefix they had in the binary. The
  component key enters the world hash, so renaming it moves every recorded stream of the
  sketch.
- `sketch::count` runs in `lactose_app::headless::a_recorded_headless_run`, which links this
  crate. The session applier runs in no recorded test: that test spawns its player through an
  applier of its own, and only the headless binary's own run calls this one
  (`docs/verification.md`).
- The applier is the sketch's, not the contract of `crates/sim/README.md` ("What
  `SessionApplier::apply` must do"): its row carries no `StreamingAnchor`, which under
  `Directed` regions has no policy effect, and a `Leave` neither despawns the row nor removes
  its claim. The game's applier meets that contract.
- Its dependencies are `lactose-core`, `lactose-ecs`, `lactose-reflect` and `lactose-sim`, all
  admitted to a boundary crate without an allowlist entry.
