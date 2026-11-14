# Scalar precision probe, Apple M1, 2026-11-14

The precision half of architecture section 16's scalar rule (milestone 1 plan, decision 5.4;
verification row "Scalar precision"), and the reading for the row "Rapier's 64-bit build uses
SIMD". Run 2 is the result, with the straight walk as an added experiment. Run 1 is kept below:
its control failed, and the probe was redesigned.

## Answer

- **The 64-bit control passes.** Every 64-bit pair stays at or under 0.0319 mm at every tick
  of the ten minutes. Every run in both precisions passes the self-check.
- **32 bits fails the one-millimetre rule at 6 km.** The largest difference is 157.75 mm on the
  (6000, 0, 6000) pair and 94.38 mm on the (6000, 0, 0) pair. Both are about a hundred times
  the limit; both are reached in the first 30 seconds, after which the closed square undoes
  each lap's error; a walk that does not retrace keeps growing. Before the walker first comes
  within the 2 cm prediction distance of a block, the two pairs differ by 88.14 mm and
  34.07 mm. The 12 km pair, which is informative only, reaches 120.84 mm without coming within
  it. A straight walk, which does not retrace, drifts 6.1 mm per metre walked at 6 km and
  13.7 mm per metre at 12 km. By section 16's rule 64 bits stays, and the rule's SOUR half is
  moot.
- **Row 41: yes for the contact solver.** In the 64-bit build every rigid-body contact
  constraint is solved by 4-lane `wide::f64x4` kernels. The one the default friction model
  selects, `ContactWithTwistFriction`, is NEON `.2d` on aarch64 and AVX `ymm` on x86-64-v3. The
  glam-based code examined is scalar f64; the substep integration was not isolated. Details
  under "Row 41" below.

## Run 2: the result

- CPU and operating system: Apple M1 (4 performance, 4 efficiency cores), macOS 14.7.4 (23H420).
- Topology and workers: do not apply. The probe is one thread: Rapier is built without
  `parallel` and steps on the calling thread.
- Commit: `017823e` (clean). This tree's code is that of `4c2cef5`; only `docs/results` differs.
  The probe is a scratch crate outside the workspace, with its own empty `[workspace]` table
  and its own lock file, which this file does not carry; a rebuild must resolve the versions
  listed below (for example with `cargo update -p <crate> --precise <version>`), or it is not
  the run recorded here. Its full source, as run 2 ran it, is at the end of this file.
- Build profile: `release` with `panic = "abort"`, toolchain 1.99.0 (the repository's
  `rust-toolchain.toml`).
- Command, from the repository root, with the two files below in a directory `<probe>`:
  `cargo build --release --manifest-path <probe>/Cargo.toml && <probe>/target/release/scalar-probe`.
- Load average: 7.30 7.99 13.79 before, 8.71 8.27 13.85 after. The run is deterministic, so
  load affects no figure: each precision's origin run, stepped a second time in the same
  process, was bit-equal to the first. Run 2 first ran at `a6d8fba`, without the straight
  walk, with load averages 5.76 14.48 16.56 before and 5.38 14.25 16.47 after; this run
  reproduces every figure of that one exactly.
- Resolved versions of the numerical crates in the probe's lock file:

| Crate | Version | Note |
|---|---|---|
| `rapier3d`, `rapier3d-f64` | 0.36.0 | Upstream crates.io, pinned by the manifest; not the vendored copy |
| `parry3d`, `parry3d-f64` | 0.31.1 | |
| `glamx` | 0.3.1 | |
| `glam` | 0.33.12 | The version `glamx` uses; the lock also holds 0.30.10, 0.31.1 and 0.32.1 for `nalgebra`'s conversions |
| `simba` | 0.10.2 | |
| `wide` | 1.7.1 | |
| `safe_arch` | 1.2.0 | `wide`'s x86 intrinsics |
| `nalgebra` | 0.35.0 | |
| `matrixmultiply` | 0.3.11 | |
| `num-traits` | 0.2.19 | |
| `num-complex` | 0.4.6 | |
| `approx` | 0.5.1 | |
| `libm` | 0.2.16 | |
| `ordered-float` | 5.5.1 | The workspace's `Cargo.lock` has 5.5.0 |
| `robust` | 1.2.0 | |
| `spade` | 2.15.1 | |
| `rstar` | 0.13.1 | The workspace's `Cargo.lock` has 0.13.0 |

  Every other crate in the table has the same version in the workspace's `Cargo.lock`.

### The scene, and how it maps to the support scene

The walk scene of `crates/physics-rapier/tests/support/mod.rs` (`Walk::create`, `Walk::drive`,
`Walk::run`) is rebuilt as `lactose-physics-rapier`'s backend hands it to Rapier, line for line,
with one change, the closed square:

- The bodies, in key order: the floor (key 1), a fixed body with a 50 × 1 × 50 half-extent cuboid
  offset by (0, −1, 0); five fixed blocks (keys 2 to 6) at (−4 + 2i, 0.3, 2), half extents
  (0.5, 0.3 + 0.1i, 0.5); the walker (key 10), dynamic, at (−3, 0.81, −3), rotation locked, a
  `capsule_y` of half-height 0.5 and radius 0.3. Every position is translated by the run's
  origin. The colliders take `Collider::new`'s values as `build_collider` builds them: friction
  0.5, restitution 0, density 1, every group, no events, the default collision types, force
  threshold 0. The walker takes `RigidBody::dynamic`'s: no damping, gravity scale 1, allowed to
  sleep.
- The bodies enter the solver at the first step, as the backend's `flush` puts them there:
  inserted in key order, the walker with the tick-1 velocity in its builder, then mass
  recomputed from the colliders.
- **The square is closed.** `Walk::drive` rests for the last 60 ticks of every 480, which cuts
  the fourth side to 60 ticks, so run 1's walker drifted off the floor. Run 2's cycle is 540
  ticks: four sides of 120 ticks at 3 m/s (+x, +z, −x, −z), then 60 ticks of rest. The jump
  every 300 ticks and the velocity rule are unchanged. The support scene itself is not changed:
  the probe owns its copy.
- Each tick does what the drive and the backend do, in the backend's order. The walker's
  velocity (3 m/s along the square, the vertical component the last report gave) is written
  only if its bits differ from the solver's, and then the walker is woken and its mass
  recomputed. The impulse (0, 3, 0) is applied at every tick that is 150 modulo 300. Gravity is
  (0, −10, 0) and dt is 1/60, as `support::step` sets them. Then one `PhysicsWorld::step`.
- "The last report" follows the backend's rule: the walker is reported when it is an active
  dynamic body before the step or after it. Otherwise the next tick keeps a vertical velocity
  of zero.
- 36,000 ticks at the workspace's 60 Hz (`support::step`), ten minutes.
- In 32 bits, each value the support scene computes in f64 is rounded once to f32. Positions
  are read back as f32, widened to f64, and the run's origin is subtracted in f64.

What differs from the support scene besides the closed square, and why none of it is expected
to matter:

- The probe builds Rapier without `parallel`. With it, the workspace's scenes are bit-equal on
  pools of one, two and four threads (verification row 33); without it, the staged solver runs
  the same stages on one worker, which was read from the source, not measured.
- The probe uses upstream Rapier, not the vendored patched copy. The walk scene exercises
  neither patched path: it has no joint, so the angular limit's `sin_cos` is not reached, and
  the deferred tree optimisation is taken only on a pool of more than one thread.
- The backend's `user_data` key codes are replaced by small integers, which the solver does not
  read. Its hashed `BodyHandle` never reaches the solver. Neither events nor snapshots are
  involved.
- No 32-bit `lactose-math::Scalar` exists. The probe assumes that a 32-bit build would hand
  Rapier the same values, rounded once.

### The self-check

Every run, in both precisions and at every translation, checks its own scene after every step,
in its scene's coordinates. The run fails the check if any of these happens:

- the walker's centre goes below y = 0;
- the walker's capsule comes within 1 m of the floor's edge, that is, the centre's |x| or |z|
  exceeds 48.7 m;
- the walker has a solver contact (Rapier's active contact, which includes speculative
  contacts within the 2 cm prediction distance) with any collider other than the floor and the
  five blocks.

A failed run is reported as a failed check, and no difference is read from it. Every run
passed:

| Run | 64-bit: check; first comes within the 2 cm prediction distance of a block | 32-bit: check; first comes within the 2 cm prediction distance of a block | Walker's range, x and z (m) |
|---|---|---|---|
| origin | passed; never | passed; never | x −3.13 to 2.85, z −3.00 to 2.97 in both precisions |
| (6000, 0, 6000) | passed; never | passed; tick 749 (345 ticks within it) | 32-bit: x −3.22 to 2.85, z −3.00 to 3.08 |
| (6000, 0, 0) | passed; never | passed; tick 750 (344 ticks within it) | 32-bit: x −3.22 to 2.85, z −3.00 to 3.02 |
| (12000, 0, 12000) | passed; never | passed; never | 32-bit: x −3.14 to 2.93, z −3.00 to 3.06 |

In 64 bits the walker never comes within the 2 cm prediction distance of a block, at any
translation. In 32 bits it does so only in the two 6 km runs, from tick 749 or 750. It does
not touch the block: the review measured its closest approach at about 6.8 mm, but the solver
acts on such speculative contacts. Its centre stays between y = 0.76 m and 3.66 m in every
run. The probe's output and source still say "touch" for this.

### Translations

The runs are (6000, 0, 6000), (6000, 0, 0) and (12000, 0, 12000), each against the origin. The
result is read at 6 km, D59's figure. 12 km lies past the next binade boundary, 8,192 m, and is
informative only unless the planner states a larger extent. Run 1's 8 km run is dropped: 6 km
and 8 km lie in the same binade, [4,096, 8,192) m, in both precisions, so they test the same
spacing.

### Results

Each translated run is compared with the origin run of the same precision. The comparison
covers every tick and every body: each body's position relative to its own scene origin, the
difference taken in f64. The fixed bodies never differ, so every largest value is the walker's.
Figures are in millimetres, with the tick of the largest value in parentheses. The last column
is informative: the largest difference over the ticks before either run of the pair first
comes within the 2 cm prediction distance of anything other than the floor.

#### 64-bit, the control

| Translation | Largest \|Δx\| | Largest \|Δy\| | Largest \|Δz\| | Largest length | Length at tick 36,000 | Largest before either run comes within 2 cm of a block |
|---|---|---|---|---|---|---|
| (6000, 0, 6000) | 0.0000 | 0.0318 (23352) | 0.0000 | **0.0318** (23352) | 0.0000 | 0.0318 (23352); never within 2 cm |
| (6000, 0, 0) | 0.0000 | 0.0319 (3851) | 0.0000 | **0.0319** (3851) | 0.0000 | 0.0319 (3851); never within 2 cm |
| (12000, 0, 12000) | 0.0000 | 0.0319 (26651) | 0.0000 | **0.0319** (26651) | 0.0000 | 0.0319 (26651); never within 2 cm |

The largest length in each minute, minutes 1 to 10, stays between 0.024 and 0.032 mm on every
pair. It does not grow.

**Verdict: the control passes.** Every 64-bit pair is under 1 mm at every tick, by a factor of
about 30. The 0.03 mm is not the f64 spacing magnified: it is the same at 6 km and 12 km, whose
spacings differ by two, and it lies in y only, where the jump landings make the scene sensitive
to any perturbation. It bounds what the scene adds to a difference, so 32-bit figures thousands
of times larger, in x and z, are read as 32-bit error.

#### 32-bit

| Translation | Largest \|Δx\| | Largest \|Δy\| | Largest \|Δz\| | Largest length | Length at tick 36,000 | Largest before either run comes within 2 cm of a block |
|---|---|---|---|---|---|---|
| (6000, 0, 6000) | 94.38 (1439) | 0.107 (3240) | 142.67 (3900) | **157.75** (1649) | 110.92 | 88.14 (449); first within 2 cm at tick 749 |
| (6000, 0, 0) | 94.38 (1439) | 0.172 (1440) | 43.39 (3147) | **94.38** (1440) | 53.03 | 34.07 (242); first within 2 cm at tick 750 |
| (12000, 0, 12000) | 80.09 (120) | 0.116 (3540) | 90.49 (239) | 120.84 (239) | 90.14 | 120.84 (239); never within 2 cm |

The largest length in each minute, minutes 1 to 10:

| Translation | mm |
|---|---|
| (6000, 0, 6000) | 157.75, then 157.75 in every later minute |
| (6000, 0, 0) | 94.38, then 82.26 to 82.27 in every later minute |
| (12000, 0, 12000) | 120.84, then 120.84 in every later minute |

**Verdict at 6 km: fail.** Both 6 km pairs exceed 1 mm, by about a hundred times: 157.75 mm
and 94.38 mm. The difference needs no contact to arise. On the floor alone, before the walker
comes within the 2 cm prediction distance of a block, the 6 km pairs reach 88.14 mm and
34.07 mm within 8 seconds. The 12 km pair never comes within it and reaches 120.84 mm.

Each pair reaches its largest difference in the first 30 seconds, and the largest in each later
minute does not grow, because the closed square retraces itself. The error is not a random walk
but a speed bias: each substep moves the walker about 12.5 mm, and the f32 sum is rounded to
the spacing at the walker's coordinate (0.49 mm at 6 km, 0.98 mm at 12 km) with a consistent
sign along a side, so a side ends tens of millimetres long or short and the opposite side
undoes it. Each 32-bit run becomes periodic bit for bit with the scene's 2,700-tick period and
returns to the same offset every period (the review's measurement). A path that does not
retrace accumulates the bias with distance walked, so these figures are lower bounds for the
scene's motion, not a ceiling; the straight walk below measures that growth. The bias depends
on the step's fraction of the spacing and on the offset the first period leaves, not on the
spacing alone, which is why the 12 km pair can lie below the 6 km diagonal.

#### The straight walk, an added experiment

The square retraces its path, so it cannot show how the error grows with distance. The
straight walk is the same scene, bodies and velocity rule, with the walker driven along +x at
3 m/s for 600 ticks and no jumps. It runs at the origin and at (64, 0, 0), (1024, 0, 0),
(6000, 0, 0) and (12000, 0, 0), in both precisions, each against its precision's origin. The
walker stays at z = −3, 4.5 m from the blocks' row, and ends near x = 26.5 m. Every run passed
the self-check, and none came within 2 cm of a block. From its start at x = −3, the origin
walker walks 5.90 m by tick 120, 14.74 m by tick 300 and 29.48 m by tick 600, in both
precisions.

The walker's Δx in mm, and Δx at tick 600 per metre the origin walker walked:

| Translation | 32-bit, tick 120 | 32-bit, tick 300 | 32-bit, tick 600 | 32-bit, mm per metre | 64-bit, ticks 120, 300 and 600 |
|---|---|---|---|---|---|
| (64, 0, 0) | 0.1454 | 0.4616 | 0.6390 | 0.022 | 0.0000 |
| (1024, 0, 0) | 3.8915 | 15.1939 | 33.6819 | 1.14 | 0.0000 |
| (6000, 0, 0) | −33.5841 | −88.1996 | −179.5750 | −6.09 | 0.0000 |
| (12000, 0, 0) | 80.6737 | 201.8394 | 403.4328 | 13.69 | 0.0000 |

In 32 bits, Δy stays under 0.003 mm and Δz is 0 in every pair; in 64 bits every difference is
0.0000 mm to four decimals. At 6 km and 12 km Δx grows in proportion to the distance walked:
5.70, 5.98 and 6.09 mm per metre at the three ticks at 6 km, and 13.68, 13.69 and 13.69 at
12 km. That is a speed bias of about 0.6 % at 6 km and 1.4 % at 12 km, which no closed path
bounds. These figures agree with the review's own straight walk to the precision it gave:
−33.6, −88.2 and −179.6 mm at 6 km, +80.7, +201.8 and +403.4 mm at 12 km, 33.7 mm at 1024 m
and 0.64 mm at 64 m at tick 600, and 0.0000 in 64 bits. The review's 29.43 m for the distance
walked is taken from the walker's position after the first step, 0.05 m past its start.

## Run 1: the control failed

Run 1 stepped the support scene's walk unchanged, at (6000, 0, 6000), (8000, 0, 8000) and
(6000, 0, 0), at commit `4c2cef5`, with load averages 13.20 15.58 13.87 before and 12.39 15.37
13.81 after. Its probe differed from run 2's in three ways:

- its drive was `Walk::drive`'s own, with the open square;
- it had no self-check, only a record of the tick the walker fell below y = −2;
- it ran 8 km in place of 12 km.

Its source is in this file's first commit.

The 64-bit control, largest length in mm (tick):

| Translation | Whole run | Before the walker left the floor | First 2 minutes |
|---|---|---|---|
| (6000, 0, 6000) | 0.0653 (8070) | 0.0653 (8070) | 0.031 |
| (8000, 0, 8000) | 0.0653 (8070) | 0.0653 (8070) | 0.031 |
| (6000, 0, 0) | **17.8326** (16439) | 17.8326 (8044) | 0.031 |

- **The cause.** `Walk::drive`'s rest takes the last 60 ticks of the fourth side of the square.
  The walker therefore drifts about 3 m in +z every 480 ticks. It slides off the 100 m floor's
  edge near tick 8,100 and falls for the rest of the run, to y = −176 km, a depth consistent
  with Rapier's 400 m/s speed cap. The edge contact amplified the (6000, 0, 0) pair's
  difference to 17.83 mm at tick 8,044, so the control failed. Its 32-bit figures were not
  read.
- **The redesign,** accepted by the coordinator: close the square; check the scene in every
  run; keep the control's criterion; and replace 8 km by 12 km, past the binade boundary. Run 2
  is that redesign.

## Row 41: Rapier's 64-bit build uses SIMD

From the vendored source (`vendor/rapier3d-f64-0.36.0`) and the pinned registry sources:

- **No feature gates SIMD in the f64 build. It is always on.** `rapier3d-f64` 0.36.0 has no SIMD
  feature. `simd8` (8 lanes) is refused together with `enhanced-determinism` (`src/lib.rs`, a
  `compile_error!`), and it only widens f32 in any case. Rapier's `SimdReal` is parry's:
  `parry3d-f64` 0.31.1 `src/lib.rs` sets `SimdReal = simba::simd::WideF64x4` for f64
  unconditionally ("f64 stays 4-lane regardless of `simd8`"). That is `wide::f64x4`.
- **What `wide::f64x4` is.** In `wide` 1.7.1, `f64x4` is one `__m256d` where the `avx` target
  feature is on, which x86-64-v3 includes. Elsewhere it is two `f64x2`, and on aarch64 each
  `f64x2` is a NEON `float64x2_t` (`vaddq_f64`, `vmulq_f64`).
- **Every rigid-body contact goes through it.** The staged island solver
  (`src/dynamics/solver/staged_island_solver/init.rs`) packs body-disjoint contact manifolds
  into 4-lane chunks. A manifold it cannot group becomes a 1-lane chunk, solved by the same
  kernel with padding lanes. `solve.rs` calls `ContactWithCoulombFriction<SimdReal>::solve` or
  `ContactWithTwistFriction<SimdReal>::solve` for every chunk. Joints have a `WideF64x4` chunk
  path and a scalar `f64` path. Only multibody (generic) constraints are scalar throughout. A
  scene with few contacts, such as the walk, therefore pays the 4-lane kernel for one lane.
- **glam's `DVec3` is scalar.** `glam` 0.33.12 has SSE2, NEON, wasm and portable-SIMD backends
  only under `src/f32`. `DVec3`, `DVec4` and `DQuat` are plain `#[repr(C)]` structs of f64 on
  every target. Moreover, `enhanced-determinism` turns on `glamx/scalar-math` through
  `parry3d-f64`, which makes glam's f32 types scalar too. Code written on `DVec3` is scalar f64
  unless LLVM pairs lanes by itself.

From one release build of `lactose-physics-rapier` at `4c2cef5`, the vendored crate's own
object code. The backend leaves `friction_model` at its default, `FrictionModel::Simplified`, so
the staged solver sets `use_twist` (`init.rs`) and solves every contact chunk with
`ContactWithTwistFriction<SimdReal>::solve` (`solve.rs`); the Coulomb kernel is compiled but
not called.

- aarch64 (this machine): `rapier3d_f64::dynamics::solver::contact_constraint::contact_with_twist_friction::ContactWithTwistFriction<simba::simd::WideF64x4>::solve`
  holds 832 floating-point arithmetic and compare instructions. All 832 are vector `.2d`
  (396 `fmul`, 254 `fadd`, 88 `fsub`, 18 `fdiv`, 2 `fsqrt`, the rest compares, `fabs`, `fneg`,
  `fmaxnm` and `fminnm`). None is a scalar `d` register operation, and none is fused. An
  excerpt, products and sums compared with zero two lanes at a time:

  ```text
  50d8:  fmul.2d  v14, v19, v12
  50dc:  fadd.2d  v14, v17, v14
  50e0:  fcmge.2d v14, v14, #0.0
  50e4:  orn.16b  v8, v8, v14
  50e8:  fcmge.2d v14, v30, #0.0
  50ec:  mvn.16b  v14, v14
  50f0:  fmul.2d  v25, v6, v30
  50f4:  fadd.2d  v25, v18, v25
  50f8:  fcmge.2d v25, v25, #0.0
  50fc:  orn.16b  v14, v14, v25
  ```

- The substep integration, `integrate_linearized`, is inlined into the solver's worker and was
  not isolated. For contrast, `RigidBodyVelocity<f64>::integrate`, written on glam and reached
  in a step only through CCD and the public prediction calls, is entirely scalar
  (`fmul d0, d8, d0`, 73 `fmul`, 41 `fadd`, 17 `fsub`, no `.2d`).
- x86-64-v3 (cross-compiled with `--target x86_64-unknown-linux-gnu`, disassembled and not run):
  the twist kernel has no symbol of its own. It is inlined into the solve pass's chunk closure
  (`staged_island_solver::solve::solve_pass::{closure}`), which holds 502 `ymm` floating-point
  instructions: 479 arithmetic (239 `vmulpd`, 165 `vaddpd`, 59 `vsubpd`, 9 `vdivpd`, 5 `vmaxpd`,
  1 `vminpd`, 1 `vsqrtpd`), 5 `vcmpunordpd` compares and 18 `vandpd` and `vxorpd` masks, with
  55 `vblendpd` and `vblendvpd` blends besides. It holds one scalar `vdivsd`, no `xmm`-only
  packed arithmetic and no `vfmadd`. The closure calls the Coulomb solve out of line, on the
  branch the twist setting does not take. `RigidBodyVelocity<f64>::integrate` is mostly scalar
  (64 `sd` instructions), with 31 packed ones where LLVM paired lanes.

So the 64-bit cost argument stands on this: the contact solver is SIMD in both precisions, at
two f64 lanes per NEON instruction against four f32 lanes. The glam-based code examined is
scalar f64 on aarch64; the substep integration was not isolated.

## The probe, as run 2 ran it

`Cargo.toml`:

```toml
[package]
name = "scalar-probe"
version = "0.0.0"
edition = "2024"
publish = false

# Outside the repository's workspace: its own lock file, no [patch], upstream crates.io Rapier.
[workspace]

[dependencies]
# The workspace's feature set on its vendored rapier3d-f64 (root Cargo.toml) less `parallel`,
# in both precisions.
rapier3d = { version = "=0.36.0", default-features = false, features = [
    "dim3",
    "f32",
    "std",
    "block-solver",
    "enhanced-determinism",
    "serde-serialize",
] }
rapier3d-f64 = { version = "=0.36.0", default-features = false, features = [
    "dim3",
    "f64",
    "std",
    "block-solver",
    "enhanced-determinism",
    "serde-serialize",
] }

[profile.release]
panic = "abort"
```

`src/main.rs`:

```rust
//! The precision probe of milestone 1 plan decision 5.4, run 2: the walk scene of
//! `crates/physics-rapier/tests/support/mod.rs` with its square closed, rebuilt on Rapier's own
//! API in 32 and 64 bits, stepped 36,000 ticks at 60 Hz at the origin and translated, checked
//! to stay on its floor, and compared with the origin run of the same precision, each position
//! taken relative to its own scene origin, in f64. Then the straight walk: the same scene
//! walked along +x for 600 ticks without jumps, a path that does not retrace itself.

const TICKS: u32 = 36_000;
const STRAIGHT_TICKS: u32 = 600;
/// The floor (key 1), the five blocks (keys 2 to 6) and the walker (key 10), in key order.
const BODIES: usize = 7;
const FLOOR: usize = 0;
const WALKER: usize = 6;
/// The floor's half extent in x and z, the walker's radius, and the margin the walker keeps
/// from the floor's edge.
const FLOOR_HALF: f64 = 50.0;
const RADIUS: f64 = 0.3;
const MARGIN: f64 = 1.0;

/// One run: each tick's body positions relative to the run's scene origin, in f64, and what
/// the walker touched. A touch is a solver contact: Rapier's active contact, which includes
/// speculative contacts within the 2 cm prediction distance.
struct Run {
    trace: Vec<[[f64; 3]; BODIES]>,
    /// The first tick the walker touched a body other than the floor.
    first_touch: Option<u32>,
    /// How many ticks the walker touched a body other than the floor.
    touch_ticks: u32,
    /// The first failure of the self-check, with its tick.
    failed: Option<(u32, String)>,
}

/// One precision's scene. `$real` is Rapier's `Real`; every value the backend hands Rapier is
/// computed in f64 as the support scene computes it, then rounded once to `$real`.
macro_rules! scene {
    ($module:ident, $rapier:ident, $real:ty) => {
        mod $module {
            use $rapier::dynamics::{LockedAxes, RigidBodyBuilder, RigidBodyHandle, RigidBodyType};
            use $rapier::geometry::{
                ActiveCollisionTypes, ColliderBuilder, ColliderHandle, Group, InteractionGroups,
                InteractionTestMode, SharedShape,
            };
            use $rapier::math::{Pose, Rotation, Vector};
            use $rapier::pipeline::{ActiveEvents, PhysicsWorld};

            type Real = $real;

            fn r(value: f64) -> Real {
                value as Real
            }

            fn v(x: f64, y: f64, z: f64) -> Vector {
                Vector::new(r(x), r(y), r(z))
            }

            fn same(a: Vector, b: Vector) -> bool {
                a.x.to_bits() == b.x.to_bits()
                    && a.y.to_bits() == b.y.to_bits()
                    && a.z.to_bits() == b.z.to_bits()
            }

            /// `lactose-physics-rapier`'s `build_collider` for a solid collider without
            /// events, `Collider::new`'s defaults: friction 0.5, restitution 0, density 1.
            fn collider(shape: SharedShape, offset: Vector, key: u128) -> ColliderBuilder {
                ColliderBuilder::new(shape)
                    .position(Pose::from_parts(offset, Rotation::IDENTITY))
                    .friction(r(0.5))
                    .restitution(r(0.0))
                    .collision_groups(InteractionGroups::new(
                        Group::from_bits_retain(u32::MAX),
                        Group::from_bits_retain(u32::MAX),
                        InteractionTestMode::And,
                    ))
                    .sensor(false)
                    .active_events(ActiveEvents::empty())
                    .active_collision_types(ActiveCollisionTypes::default())
                    .contact_force_event_threshold(r(0.0))
                    .user_data(key)
                    .density(r(1.0))
            }

            /// A body with a collider and no `RigidBody`: fixed, as the backend's `insert`
            /// builds it.
            fn fixed(
                world: &mut PhysicsWorld,
                key: u128,
                at: Vector,
                shape: SharedShape,
                offset: Vector,
            ) -> (RigidBodyHandle, ColliderHandle) {
                let builder = RigidBodyBuilder::new(RigidBodyType::Fixed)
                    .pose(Pose::from_parts(at, Rotation::IDENTITY))
                    .user_data(key);
                let body = world.insert_body(builder);
                let collider = world.insert_collider(collider(shape, offset, key), Some(body));
                recompute_mass(world, body);
                (body, collider)
            }

            fn recompute_mass(world: &mut PhysicsWorld, handle: RigidBodyHandle) {
                let PhysicsWorld { bodies, colliders, .. } = world;
                if let Some(body) = bodies.get_mut(handle) {
                    body.recompute_mass_properties_from_colliders(colliders);
                }
            }

            /// Whether the backend's `moving()` would list the walker: an active dynamic body.
            fn moving(world: &PhysicsWorld, walker: RigidBodyHandle) -> bool {
                world.islands.active_bodies().any(|handle| {
                    handle == walker && world.bodies.get(handle).is_some_and(|b| b.is_dynamic())
                })
            }

            /// Steps the walk scene for `ticks` ticks with its every position translated by
            /// `origin`; `straight` walks along +x without jumps instead of the square.
            pub fn run(origin: [f64; 3], ticks: u32, straight: bool) -> super::Run {
                let [ox, oy, oz] = origin;
                let mut world = PhysicsWorld::new();
                let mut handles = Vec::with_capacity(super::BODIES);
                let mut colliders = Vec::with_capacity(super::BODIES);
                let mut last: Option<Vector> = None;
                let mut run = super::Run {
                    trace: Vec::with_capacity(ticks as usize),
                    first_touch: None,
                    touch_ticks: 0,
                    failed: None,
                };
                for tick in 1..=ticks {
                    // `Walk::drive` with the square closed: a 540-tick cycle of four 120-tick
                    // sides at 3 m/s, then 60 ticks of rest, keeping the reported vertical
                    // velocity; a jump every 300 ticks. The straight walk keeps the first side.
                    let phase = tick % 540;
                    let (x, z) = match (phase / 120, phase >= 480) {
                        _ if straight => (3.0, 0.0),
                        (_, true) => (0.0, 0.0),
                        (0, false) => (3.0, 0.0),
                        (1, false) => (0.0, 3.0),
                        (2, false) => (-3.0, 0.0),
                        _ => (0.0, -3.0),
                    };
                    let vy = last.map_or(r(0.0), |velocity| velocity.y);
                    let linear = Vector::new(r(x), vy, r(z));

                    // The backend's `flush`: the bodies created before tick 1 enter the solver
                    // at the first step, in key order, the walker with its written velocity;
                    // later, a velocity write that differs in bits wakes the walker.
                    if tick == 1 {
                        let (body, floor) = fixed(
                            &mut world,
                            1,
                            v(ox, oy, oz),
                            SharedShape::cuboid(r(super::FLOOR_HALF), r(1.0), r(super::FLOOR_HALF)),
                            v(0.0, -1.0, 0.0),
                        );
                        handles.push(body);
                        colliders.push(floor);
                        for i in 0..5_u32 {
                            let i = f64::from(i);
                            let (body, block) = fixed(
                                &mut world,
                                2 + i as u128,
                                v(-4.0 + 2.0 * i + ox, 0.3 + oy, 2.0 + oz),
                                SharedShape::cuboid(r(0.5), r(0.3 + 0.1 * i), r(0.5)),
                                Vector::ZERO,
                            );
                            handles.push(body);
                            colliders.push(block);
                        }
                        let builder = RigidBodyBuilder::new(RigidBodyType::Dynamic)
                            .pose(Pose::from_parts(
                                v(-3.0 + ox, 0.81 + oy, -3.0 + oz),
                                Rotation::IDENTITY,
                            ))
                            .user_data(10)
                            .linear_damping(r(0.0))
                            .angular_damping(r(0.0))
                            .gravity_scale(r(1.0))
                            .locked_axes(LockedAxes::ROTATION_LOCKED)
                            .can_sleep(true)
                            .linvel(linear)
                            .angvel(Vector::ZERO);
                        let walker = world.insert_body(builder);
                        let capsule = SharedShape::capsule_y(r(0.5), r(super::RADIUS));
                        let collider = collider(capsule, Vector::ZERO, 10);
                        colliders.push(world.insert_collider(collider, Some(walker)));
                        recompute_mass(&mut world, walker);
                        handles.push(walker);
                    } else {
                        let walker = handles[super::WALKER];
                        let differs = world.bodies.get(walker).is_some_and(|body| {
                            !same(body.linvel(), linear) || !same(body.angvel(), Vector::ZERO)
                        });
                        if differs && let Some(body) = world.bodies.get_mut(walker) {
                            body.set_linvel(linear, true);
                            body.set_angvel(Vector::ZERO, true);
                            body.wake_up(true);
                            recompute_mass(&mut world, walker);
                        }
                    }
                    let walker = handles[super::WALKER];

                    // The backend's `apply_commands`, then the step's gravity and dt.
                    if !straight
                        && tick % 300 == 150
                        && let Some(body) = world.bodies.get_mut(walker)
                    {
                        body.apply_impulse(v(0.0, 3.0, 0.0), true);
                    }
                    world.gravity = v(0.0, -10.0, 0.0);
                    world.integration_parameters.dt = r(1.0 / 60.0);

                    // Reported if moving before the step or after it, as the backend reports.
                    let before = moving(&world, walker);
                    world.step();
                    let after = moving(&world, walker);
                    last = (before || after)
                        .then(|| world.bodies.get(walker).map(|body| body.linvel()))
                        .flatten();

                    let mut positions = [[0.0; 3]; super::BODIES];
                    for (slot, &handle) in positions.iter_mut().zip(&handles) {
                        let t = world.bodies[handle].position().translation;
                        *slot = [f64::from(t.x) - ox, f64::from(t.y) - oy, f64::from(t.z) - oz];
                    }

                    // The self-check: what the walker touches after this step, and where it is.
                    let own = colliders[super::WALKER];
                    let mut touched_other = false;
                    for pair in world.contact_pairs_with(own) {
                        if !pair.has_any_active_contact() {
                            continue;
                        }
                        let other =
                            if pair.collider1 == own { pair.collider2 } else { pair.collider1 };
                        match colliders.iter().position(|&c| c == other) {
                            Some(super::FLOOR) => {}
                            Some(index) if index < super::WALKER => touched_other = true,
                            _ => {
                                run.failed
                                    .get_or_insert((tick, "touched an unknown collider".into()));
                            }
                        }
                    }
                    if touched_other {
                        run.first_touch.get_or_insert(tick);
                        run.touch_ticks += 1;
                    }
                    let [wx, wy, wz] = positions[super::WALKER];
                    let inside = super::FLOOR_HALF - super::MARGIN - super::RADIUS;
                    if wy < 0.0 {
                        run.failed.get_or_insert((tick, format!("centre below y = 0 ({wy:.6})")));
                    }
                    if wx.abs() > inside || wz.abs() > inside {
                        run.failed.get_or_insert((
                            tick,
                            format!("within 1 m of the floor's edge ({wx:.6}, {wz:.6})"),
                        ));
                    }
                    run.trace.push(positions);
                }
                run
            }
        }
    };
}

scene!(single, rapier3d, f32);
scene!(double, rapier3d_f64, f64);

/// The largest difference between a translated run and the origin run.
struct Largest {
    axis: [(f64, u32); 3],
    length: (f64, u32),
    last_tick: f64,
    /// Over the ticks before either run's walker first touched a body other than the floor.
    before_touch: (f64, u32),
    per_minute: Vec<f64>,
}

fn compare(origin: &Run, moved: &Run) -> Largest {
    let mut largest = Largest {
        axis: [(0.0, 0); 3],
        length: (0.0, 0),
        last_tick: 0.0,
        before_touch: (0.0, 0),
        per_minute: vec![0.0; 10],
    };
    let touch = origin.first_touch.unwrap_or(TICKS + 1).min(moved.first_touch.unwrap_or(TICKS + 1));
    for (index, (a, b)) in origin.trace.iter().zip(&moved.trace).enumerate() {
        let tick = index as u32 + 1;
        for body in 0..BODIES {
            let d = [b[body][0] - a[body][0], b[body][1] - a[body][1], b[body][2] - a[body][2]];
            for axis in 0..3 {
                if d[axis].abs() > largest.axis[axis].0 {
                    largest.axis[axis] = (d[axis].abs(), tick);
                }
            }
            let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if length > largest.length.0 {
                largest.length = (length, tick);
            }
            if tick < touch && length > largest.before_touch.0 {
                largest.before_touch = (length, tick);
            }
            let minute = (index / 3_600).min(9);
            if length > largest.per_minute[minute] {
                largest.per_minute[minute] = length;
            }
            if tick == TICKS && body == WALKER {
                largest.last_tick = length;
            }
        }
    }
    largest
}

fn mm(metres: f64) -> String {
    format!("{:.4}", metres * 1_000.0)
}

fn main() {
    let runs: [(&str, [f64; 3]); 4] = [
        ("origin", [0.0, 0.0, 0.0]),
        ("(6000, 0, 6000)", [6_000.0, 0.0, 6_000.0]),
        ("(6000, 0, 0)", [6_000.0, 0.0, 0.0]),
        ("(12000, 0, 12000)", [12_000.0, 0.0, 12_000.0]),
    ];
    // The 64-bit control first.
    let precisions: [(&str, fn([f64; 3], u32, bool) -> Run); 2] =
        [("64-bit", double::run), ("32-bit", single::run)];
    for (name, step) in precisions {
        let results: Vec<Run> = runs
            .iter()
            .map(|&(label, origin)| {
                let run = step(origin, TICKS, false);
                let w = run.trace[run.trace.len() - 1][WALKER];
                let (mut low, mut high) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                for p in &run.trace {
                    for axis in 0..3 {
                        low[axis] = low[axis].min(p[WALKER][axis]);
                        high[axis] = high[axis].max(p[WALKER][axis]);
                    }
                }
                let check = match &run.failed {
                    None => "passed".to_string(),
                    Some((tick, why)) => format!("FAILED at tick {tick}: {why}"),
                };
                println!(
                    "{name} {label}: check {check}; first touch of a body other than the floor {:?}, {} ticks touching; walker at the last tick ({:.6}, {:.6}, {:.6}); range x [{:.3}, {:.3}] y [{:.3}, {:.3}] z [{:.3}, {:.3}]",
                    run.first_touch,
                    run.touch_ticks,
                    w[0],
                    w[1],
                    w[2],
                    low[0],
                    high[0],
                    low[1],
                    high[1],
                    low[2],
                    high[2],
                );
                run
            })
            .collect();
        println!();
        println!(
            "{name}: translation; largest dx, dy, dz mm (tick); largest length mm (tick); length at tick 36,000 mm; largest length before the first touch of a body other than the floor mm (tick)"
        );
        for (index, (label, _)) in runs.iter().enumerate().skip(1) {
            if results[0].failed.is_some() || results[index].failed.is_some() {
                println!("{label}: not compared, a run of the pair failed its check");
                continue;
            }
            let l = compare(&results[0], &results[index]);
            println!(
                "{label}: {} ({}), {} ({}), {} ({}); {} ({}); {}; {} ({})",
                mm(l.axis[0].0),
                l.axis[0].1,
                mm(l.axis[1].0),
                l.axis[1].1,
                mm(l.axis[2].0),
                l.axis[2].1,
                mm(l.length.0),
                l.length.1,
                mm(l.last_tick),
                mm(l.before_touch.0),
                l.before_touch.1,
            );
            let minutes: Vec<String> = l.per_minute.iter().map(|&m| mm(m)).collect();
            println!("{label} largest length per minute, mm: {}", minutes.join(", "));
        }
        println!();
        // Same precision, origin against origin: the probe is deterministic in itself.
        let again = step(runs[0].1, TICKS, false);
        let equal = again.trace.iter().zip(&results[0].trace).all(|(a, b)| {
            a.iter().flatten().zip(b.iter().flatten()).all(|(x, y)| x.to_bits() == y.to_bits())
        });
        println!("{name} origin run repeated: bit-equal {equal}");
        println!();
    }

    // The straight walk: each translation against its precision's origin, the walker's
    // difference at three ticks, and its growth with the distance the origin walker walked.
    let walks: [(&str, [f64; 3]); 5] = [
        ("origin", [0.0, 0.0, 0.0]),
        ("(64, 0, 0)", [64.0, 0.0, 0.0]),
        ("(1024, 0, 0)", [1_024.0, 0.0, 0.0]),
        ("(6000, 0, 0)", [6_000.0, 0.0, 0.0]),
        ("(12000, 0, 0)", [12_000.0, 0.0, 0.0]),
    ];
    let at = [120_u32, 300, STRAIGHT_TICKS];
    for (name, step) in precisions {
        let results: Vec<Run> =
            walks.iter().map(|&(_, origin)| step(origin, STRAIGHT_TICKS, true)).collect();
        let walked = |tick: u32| results[0].trace[tick as usize - 1][WALKER][0] + 3.0;
        println!(
            "straight {name}: origin walker walked {:.4} m, {:.4} m, {:.4} m by ticks 120, 300, 600",
            walked(at[0]),
            walked(at[1]),
            walked(at[2]),
        );
        for (index, (label, _)) in walks.iter().enumerate() {
            let check = match (&results[index].failed, results[index].first_touch) {
                (Some((tick, why)), _) => format!("FAILED at tick {tick}: {why}"),
                (None, Some(tick)) => format!("FAILED: touched a block at tick {tick}"),
                (None, None) => "passed".to_string(),
            };
            if index == 0 {
                println!("straight {name} {label}: check {check}");
                continue;
            }
            let d = |tick: u32, axis: usize| {
                let i = tick as usize - 1;
                results[index].trace[i][WALKER][axis] - results[0].trace[i][WALKER][axis]
            };
            let cells: Vec<String> = at
                .iter()
                .map(|&t| format!("{} {} {} ({t})", mm(d(t, 0)), mm(d(t, 1)), mm(d(t, 2))))
                .collect();
            println!(
                "straight {name} {label}: check {check}; walker dx dy dz mm (tick): {}; dx per metre walked at tick 600: {:.4} mm/m",
                cells.join("; "),
                d(STRAIGHT_TICKS, 0) * 1_000.0 / walked(STRAIGHT_TICKS),
            );
        }
        println!();
    }
}
```
