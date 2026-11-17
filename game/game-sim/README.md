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
| `register_systems` | A registration's `systems`: adds `game_sim::count`, which adds one to every `Counter` each tick, wrapping, in the server and prediction worlds, to `Stage::Simulation` |
| `sessions` | A `Game`'s `sessions`: the session applier, which meets `crates/sim/README.md`'s contract ("What `SessionApplier::apply` must do"). A `Join` spawns the player's row in the join's region, under a key from `World::allocate_net_id`: a `Counter` at zero, `ClaimedBy` and `StreamingAnchor`. A `Leave` removes the claim and the anchor from that row and keeps the row |

## Rules a caller should know

- A game crate may name neither `lactose-app` nor `lactose-persist` (`crate-tiers.toml`), so
  the `CrateRegistration` and `Game` values are the binary's: `crates/app-headless/src/main.rs`
  builds them from the functions above, with `setup` and `migrations` both `None`. Where a game
  crate's registration value and its migration functions are declared is decided before
  milestone 5 (`docs/verification.md`).
- The component key keeps the `sketch::` prefix it had in the binary: it enters the world
  hash, so renaming it moves every recorded stream of the sketch. The system is named as
  architecture section 6.1 names a `game-sim` system; its name is in no hash.
- Both run in recorded tests of `crates/app/tests/headless.rs`, which link this crate.
  `a_recorded_headless_run` steps `game_sim::count`, its player spawned through an applier of
  the test's own. `a_recorded_join_leave_and_rejoin` runs the game as the binary ships it,
  `sessions` included, through a join, a leave and a second join with a sample every tick, and
  checks after every tick that the player resolves to one claimed row that holds the sample.
  The unrecorded tests of that file and the headless binary's own run call `sessions` too.
- Its dependencies are `lactose-core`, `lactose-ecs`, `lactose-reflect` and `lactose-sim`, all
  admitted to a boundary crate without an allowlist entry.
