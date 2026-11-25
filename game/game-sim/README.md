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
| `Counter` | The sketch's counter, key `sketch::counter`, persistent and replicated: a `u32` under field ID 1 |
| `Orbit` | The mover of the client's local world (`crates/app/README.md`, "The mover of the local world"), key `sketch::orbit`, persistent and replicated: `centre` (`Vec3`, ID 1), `radius` (`Scalar`, ID 2), `period` (`u32` ticks, ID 3) and `phase` (`u32` ticks, ID 4) |
| `register_types` | A registration's `types`: registers `Counter` and `Orbit` |
| `register_systems` | A registration's `systems`: adds to `Stage::Simulation`, in the server and prediction worlds, `game_sim::count`, which adds one to every `Counter` each tick, wrapping, and `game_sim::orbit`, which writes the `Transform` of every row with an `Orbit` each tick (below) |
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
- `game_sim::orbit` is a function of the row and the tick. With `t` the tick being run
  (`Ctx::tick`): `step = (t + phase) mod period`, the addition wrapping; `angle = TAU × step /
  period`, both made `Scalar` before the division; `(sin, cos) = Scalar::sin_cos(angle)`;
  the translation `centre + radius × (cos, 0, sin)` and the rotation
  `Quat::from_rotation_y(angle)`. It writes `lactose_physics_api::Transform` and nothing else,
  never `Teleported`, so the row is interpolated; a period of zero leaves the row where it is.
  No state accumulates, so a rollback, a save and a load, and a full pass place the row where
  an unbroken run does; at the tick's wrap the row jumps unless `period` divides 2^32. A row
  with `Orbit` has no body. Its arithmetic is `lactose-math`'s alone.
- Its dependencies are `lactose-core`, `lactose-ecs`, `lactose-math`, `lactose-physics-api`,
  `lactose-reflect` and `lactose-sim`, all admitted to a boundary crate without an allowlist
  entry.
