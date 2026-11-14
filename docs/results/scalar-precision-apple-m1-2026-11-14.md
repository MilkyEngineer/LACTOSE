# Scalar precision probe, Apple M1, 2026-11-14

The precision half of architecture section 16's scalar rule (milestone 1 plan, decision 5.4;
verification row "Scalar precision"), and the reading for the row "Rapier's 64-bit build uses
SIMD".

- CPU and operating system: Apple M1 (4 performance, 4 efficiency cores), macOS 14.7.4 (23H420).
- Topology and workers: do not apply. The probe is one thread: Rapier is built without
  `parallel` and steps on the calling thread. The disassembly is not a timing.
- Commit: `4c2cef5` (clean). The probe is a scratch crate outside the workspace, with its own
  empty `[workspace]` table and its own lock file; its full source is below.
- Build profile: `release` with `panic = "abort"`, toolchain 1.99.0 (the repository's
  `rust-toolchain.toml`).
- Command, from the repository root, with the two files below in a directory `<probe>`:
  `cargo build --release --manifest-path <probe>/Cargo.toml && <probe>/target/release/scalar-probe`.
- Load average: 13.20 15.58 13.87 before, 12.39 15.37 13.81 after. The run is deterministic, so
  load affects no figure: each precision's origin run, stepped a second time in the same
  process, was bit-equal to the first.
- Resolved versions, the same as the workspace's `Cargo.lock`: `rapier3d` and `rapier3d-f64`
  0.36.0 (upstream crates.io, not the vendored copy), `parry3d` and `parry3d-f64` 0.31.1,
  `glamx` 0.3.1, `glam` 0.33.12, `simba` 0.10.2, `wide` 1.7.1, `nalgebra` 0.35.0, `libm` 0.2.16.

## Answer

- **The 64-bit control fails, so the probe is not read.** The pair translated by (6000, 0, 0)
  differs from the origin run by 17.83 mm, from tick 8,044. That is where the walker slides
  off the edge of the floor. The scene amplifies differences through that contact. The
  32-bit figures below are recorded but are not the result. The precision half of the rule
  stays undecided until the redesigned probe proposed below has run.
- **The walk scene does not stay on its floor for ten minutes.** The support scene's square is
  open: its fourth side walks 60 ticks against the second side's 120, because the rest takes
  the last 60 ticks of the fourth side. The walker therefore drifts about 3 m in +z every 480
  ticks. It leaves the 100 m floor at tick 8,105 or 8,106 (64-bit) and 7,990 to 8,039
  (32-bit), then falls for eight minutes to y = −176 km, a depth consistent with Rapier's
  speed cap (400 m/s, `normalized_max_linear_velocity`). For most of the ten minutes it is
  therefore not "at the map edge" but 176 km below it.
- **Row 41: yes for the contact solver, scalar elsewhere.** In the 64-bit build every
  rigid-body contact constraint is solved by 4-lane `wide::f64x4` kernels. Those kernels are
  NEON `.2d` on aarch64 and AVX `ymm` on x86-64-v3. The glam-based code around them is scalar
  f64. Details under "Row 41" below.

## The scene, and how it maps to the support scene

The walk scene of `crates/physics-rapier/tests/support/mod.rs` (`Walk::create`, `Walk::drive`,
`Walk::run`) is rebuilt as `lactose-physics-rapier`'s backend hands it to Rapier, line for line:

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
- Each tick does what `Walk::drive` and the backend do, in the backend's order. The walker's
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

What differs from the support scene, and why none of it is expected to matter:

- The probe steps without `parallel` and without a solver pool. The workspace's scenes report
  bit-equal results on pools of one, two and four threads (verification row 33,
  `crates/physics-rapier/tests/pool.rs`).
- The probe uses upstream Rapier, not the vendored patched copy. The walk scene exercises
  neither patched path: it has no joint, so the angular limit's `sin_cos` is not reached, and
  the deferred tree optimisation is taken only on a pool of more than one thread.
- The backend's `user_data` key codes are replaced by small integers, which the solver does not
  read. Its hashed `BodyHandle` never reaches the solver. Neither events nor snapshots are
  involved.
- No 32-bit `lactose-math::Scalar` exists. The probe assumes that a 32-bit build would hand
  Rapier the same values, rounded once.

## Results

Each translated run is compared with the origin run of the same precision. The comparison
covers every tick and every body: each body's position relative to its own scene origin, the
difference taken in f64. The fixed bodies never differ, so every largest value is the walker's.
Figures are in millimetres, with the tick of the largest value in parentheses. 6 km and 8 km lie
in the same binade, [4,096, 8,192) m, in both precisions, so their spacing is the same. The two
rows agree to the printed precision in every column except the last-tick length of the 32-bit
runs, which differs in the fourth decimal.

### 64-bit, the control

| Translation | Largest \|Δx\| | Largest \|Δy\| | Largest \|Δz\| | Largest length | Length at tick 36,000 | Largest length before the walker left the floor |
|---|---|---|---|---|---|---|
| (6000, 0, 6000) | 0.0220 (7937) | 0.0650 (10735) | 0.0273 (7937) | 0.0653 (8070) | 0.0653 | 0.0653 (8070) |
| (8000, 0, 8000) | 0.0220 (7937) | 0.0650 (10735) | 0.0273 (7937) | 0.0653 (8070) | 0.0653 | 0.0653 (8070) |
| (6000, 0, 0) | 0.1816 (7947) | 17.8318 (14433) | 0.2447 (8018) | **17.8326** (16439) | 17.8326 | 17.8326 (8044) |

Largest length per minute, minutes 1 to 10:

| Translation | mm |
|---|---|
| (6000, 0, 6000) | 0.0306, 0.0307, 0.0653, 0.0653, 0.0653, 0.0653, 0.0653, 0.0653, 0.0653, 0.0653 |
| (8000, 0, 8000) | 0.0306, 0.0307, 0.0653, 0.0653, 0.0653, 0.0653, 0.0653, 0.0653, 0.0653, 0.0653 |
| (6000, 0, 0) | 0.0307, 0.0300, 17.8326, 17.8326, 17.8326, 17.8326, 17.8326, 17.8326, 17.8326, 17.8326 |

On the floor, among the blocks, the control stays at or under 0.031 mm for the first two
minutes. The difference jumps at the floor's edge (ticks 7,937 to 8,070). The (6000, 0, 0) run
leaves one tick earlier than the origin run, and the offset it takes there holds through the
fall: from minute 3 to minute 10 the largest length does not change, so the two runs fall
at equal speeds, as they would at the speed cap.

**Verdict: the control fails** (17.83 mm ≥ 1 mm). By decision 5.4 the scene amplifies
differences through its contacts, and the probe is redesigned, not read.

### 32-bit, recorded and not read

| Translation | Largest \|Δx\| | Largest \|Δy\| | Largest \|Δz\| | Largest length | Length at tick 36,000 | Largest length before the walker left the floor |
|---|---|---|---|---|---|---|
| (6000, 0, 6000) | 107.80 (10919) | 1,084,515.6 (29209) | 3,036.6 (8099) | 1,084,516.0 (29219) | 1,084,515.7 | 4,024.8 (7989) |
| (8000, 0, 8000) | 107.80 (10919) | 1,084,515.6 (29209) | 3,036.6 (8099) | 1,084,516.0 (29219) | 1,084,515.7 | 4,024.8 (7989) |
| (6000, 0, 0) | 79.56 (1742) | 46.88 (14433) | 4.06 (19914) | 79.56 (1742) | 67.81 | 79.56 (1742) |

Largest length per minute, minutes 1 to 10:

| Translation | mm |
|---|---|
| (6000, 0, 6000) | 119.49, 126.60, 1,065,152.9, 1,081,721.5, 1,081,975.4, 1,081,974.6, 1,079,446.7, 1,079,446.1, 1,084,516.0, 1,084,515.8 |
| (8000, 0, 8000) | the same to the printed precision |
| (6000, 0, 0) | 79.56, 79.55, 67.04, 67.11, 68.43, 68.43, 68.42, 68.41, 68.41, 68.40 |

These numbers are not the result. One observation steers the redesign without deciding
anything: in the first two minutes, on the floor and away from its edge, the 32-bit pairs
differ by 80 to 127 mm. Over the same span the 64-bit control differs by 0.031 mm. The plan
expected about 27 mm after ten minutes.

## Proposed redesign

1. Keep the walker on the floor, among the blocks, for all 36,000 ticks. Close the square:
   make the cycle 540 ticks, four sides of 120 ticks and then 60 ticks of rest, in place of the
   rest taking the last 60 ticks of the fourth side. Keep the support scene's bodies, jump and
   velocity rule unchanged. The support scene itself need not change: the probe owns its copy.
2. The probe checks its own scene and refuses to report if the check fails. The walker's centre
   must stay above y = 0. It must stay at least 1 m inside the floor's edges. Its contacts must
   be only the floor and the blocks.
3. The control's criterion is unchanged: under 1 mm for every pair, at every tick, before any
   32-bit figure is read.
4. Add a translation past 8,192 m, for example 12 km. 6 km and 8 km share a spacing in both
   precisions, so 8 km tests nothing that 6 km does not. Whether the map extent goes past
   8,192 m is the planner's to state. D59's figure is 6 km.

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
object code:

- aarch64 (this machine): `rapier3d_f64::dynamics::solver::contact_constraint::contact_with_coulomb_friction::ContactWithCoulombFriction<simba::simd::WideF64x4>::solve`
  holds 710 floating-point arithmetic and compare instructions. All 710 are vector `.2d`
  (336 `fmul`, 214 `fadd`, 72 `fsub`, 18 `fdiv`, 2 `fsqrt`, the rest compares, `fabs`, `fneg`
  and `fmaxnm`). None is a scalar `d` register operation, and none is fused. Each `f64x4`
  operation is a pair of `.2d` instructions:

  ```text
  6a5c:  fmul.2d  v16, v9, v16
  6a60:  fmul.2d  v17, v12, v17
  6a74:  fmul.2d  v18, v13, v18
  6a78:  fmul.2d  v19, v21, v19
  6a7c:  fadd.2d  v16, v16, v18
  6a80:  fadd.2d  v17, v17, v19
  ```

  For contrast, `RigidBodyVelocity<f64>::integrate`, written on glam, is entirely scalar
  (`fmul d0, d8, d0`, 73 `fmul`, 41 `fadd`, 17 `fsub`, no `.2d`).
- x86-64-v3 (cross-compiled with `--target x86_64-unknown-linux-gnu`, disassembled and not run):
  the same `solve` holds 324 `ymm` arithmetic instructions (`vmulpd`, `vaddpd`, `vsubpd`,
  `vdivpd`, `vmaxpd`), with no `xmm`-only packed or scalar `sd` arithmetic and no `vfmadd`.
  `RigidBodyVelocity<f64>::integrate` is mostly scalar (64 `sd` instructions), with 31 packed
  ones where LLVM paired lanes.

So the 64-bit cost argument stands on this: the contact solver is SIMD in both precisions, at
two f64 lanes per NEON instruction against four f32 lanes. The integration, the narrow phase
and the rest of the glam-based code are scalar f64 on aarch64.

## The probe

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
//! The precision probe of milestone 1 plan decision 5.4: the walk scene of
//! `crates/physics-rapier/tests/support/mod.rs`, rebuilt on Rapier's own API in 32 and 64 bits,
//! stepped 36,000 ticks at 60 Hz at the origin and translated, and compared with the origin run
//! of the same precision, each position taken relative to its own scene origin, in f64.

use std::time::Instant;

const TICKS: u32 = 36_000;
/// The floor (key 1), the five blocks (keys 2 to 6) and the walker (key 10), in key order.
const BODIES: usize = 7;
const WALKER: usize = 6;

/// Each tick's body positions, relative to the run's scene origin, in f64.
type Trace = Vec<[[f64; 3]; BODIES]>;

/// One precision's scene. `$real` is Rapier's `Real`; every value the backend hands Rapier is
/// computed in f64 as the support scene computes it, then rounded once to `$real`.
macro_rules! scene {
    ($module:ident, $rapier:ident, $real:ty) => {
        mod $module {
            use $rapier::dynamics::{LockedAxes, RigidBodyBuilder, RigidBodyHandle, RigidBodyType};
            use $rapier::geometry::{
                ActiveCollisionTypes, ColliderBuilder, Group, InteractionGroups,
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
            ) -> RigidBodyHandle {
                let builder = RigidBodyBuilder::new(RigidBodyType::Fixed)
                    .pose(Pose::from_parts(at, Rotation::IDENTITY))
                    .user_data(key);
                let body = world.insert_body(builder);
                world.insert_collider(collider(shape, offset, key), Some(body));
                recompute_mass(world, body);
                body
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

            /// Steps the walk scene with its every position translated by `origin`.
            pub fn run(origin: [f64; 3]) -> super::Trace {
                let [ox, oy, oz] = origin;
                let mut world = PhysicsWorld::new();
                let mut handles = Vec::with_capacity(super::BODIES);
                let mut last: Option<Vector> = None;
                let mut trace = Vec::with_capacity(super::TICKS as usize);
                for tick in 1..=super::TICKS {
                    // `Walk::drive`: a square at 3 m/s keeping the reported vertical velocity,
                    // a rest for the last 60 ticks of every 480, a jump every 300.
                    let (x, z) = match ((tick / 120) % 4, tick % 480 >= 420) {
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
                        handles.push(fixed(
                            &mut world,
                            1,
                            v(ox, oy, oz),
                            SharedShape::cuboid(r(50.0), r(1.0), r(50.0)),
                            v(0.0, -1.0, 0.0),
                        ));
                        for i in 0..5_u32 {
                            let i = f64::from(i);
                            handles.push(fixed(
                                &mut world,
                                2 + i as u128,
                                v(-4.0 + 2.0 * i + ox, 0.3 + oy, 2.0 + oz),
                                SharedShape::cuboid(r(0.5), r(0.3 + 0.1 * i), r(0.5)),
                                Vector::ZERO,
                            ));
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
                        let capsule = SharedShape::capsule_y(r(0.5), r(0.3));
                        world.insert_collider(collider(capsule, Vector::ZERO, 10), Some(walker));
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
                    if tick % 300 == 150
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
                    trace.push(positions);
                }
                trace
            }
        }
    };
}

scene!(single, rapier3d, f32);
scene!(double, rapier3d_f64, f64);

/// The largest difference between a translated run and the origin run.
struct Largest {
    axis: [(f64, u32); 3],
    length: (f64, u32, usize),
    per_minute: Vec<f64>,
    last_tick: f64,
    /// The largest length over the ticks before either run's walker left the floor.
    on_floor: (f64, u32),
}

/// The first tick the walker's centre is below the floor's underside (y = -2): it has left
/// the floor.
fn departure(trace: &Trace) -> Option<u32> {
    trace.iter().position(|p| p[WALKER][1] < -2.0).map(|index| index as u32 + 1)
}

fn compare(origin: &Trace, moved: &Trace) -> Largest {
    let mut largest = Largest {
        axis: [(0.0, 0); 3],
        length: (0.0, 0, 0),
        per_minute: vec![0.0; 10],
        last_tick: 0.0,
        on_floor: (0.0, 0),
    };
    let left = departure(origin).unwrap_or(TICKS + 1).min(departure(moved).unwrap_or(TICKS + 1));
    for (index, (a, b)) in origin.iter().zip(moved).enumerate() {
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
                largest.length = (length, tick, body);
            }
            let minute = (index / 3_600).min(9);
            if length > largest.per_minute[minute] {
                largest.per_minute[minute] = length;
            }
            if tick < left && length > largest.on_floor.0 {
                largest.on_floor = (length, tick);
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
        ("(8000, 0, 8000)", [8_000.0, 0.0, 8_000.0]),
        ("(6000, 0, 0)", [6_000.0, 0.0, 0.0]),
    ];
    // The 64-bit control first.
    let precisions: [(&str, fn([f64; 3]) -> Trace); 2] =
        [("64-bit", double::run), ("32-bit", single::run)];
    for (name, run) in precisions {
        let traces: Vec<Trace> = runs
            .iter()
            .map(|&(label, origin)| {
                let start = Instant::now();
                let trace = run(origin);
                let w = trace[trace.len() - 1][WALKER];
                let lowest = trace.iter().map(|p| p[WALKER][1]).fold(f64::INFINITY, f64::min);
                println!(
                    "{name} {label}: {} ticks in {:.2} s; walker left the floor at tick {:?}; at the last tick ({:.6}, {:.6}, {:.6}), lowest y {:.6}",
                    trace.len(),
                    start.elapsed().as_secs_f64(),
                    departure(&trace),
                    w[0],
                    w[1],
                    w[2],
                    lowest,
                );
                trace
            })
            .collect();
        println!();
        println!(
            "| {name} run | max |dx| mm (tick) | max |dy| mm (tick) | max |dz| mm (tick) | max length mm (tick, body) | length at tick 36,000 mm |"
        );
        println!("|---|---|---|---|---|---|");
        for (index, (label, _)) in runs.iter().enumerate().skip(1) {
            let l = compare(&traces[0], &traces[index]);
            println!(
                "| {label} | {} ({}) | {} ({}) | {} ({}) | {} ({}, {}) | {} |",
                mm(l.axis[0].0),
                l.axis[0].1,
                mm(l.axis[1].0),
                l.axis[1].1,
                mm(l.axis[2].0),
                l.axis[2].1,
                mm(l.length.0),
                l.length.1,
                l.length.2,
                mm(l.last_tick),
            );
        }
        println!();
        println!(
            "| {name} run | largest length per minute, mm (minutes 1 to 10) | largest length before the walker left the floor, mm (tick) |"
        );
        println!("|---|---|---|");
        for (index, (label, _)) in runs.iter().enumerate().skip(1) {
            let l = compare(&traces[0], &traces[index]);
            let minutes: Vec<String> = l.per_minute.iter().map(|&m| mm(m)).collect();
            println!(
                "| {label} | {} | {} ({}) |",
                minutes.join(", "),
                mm(l.on_floor.0),
                l.on_floor.1
            );
        }
        println!();
        // Same precision, origin against origin: the probe is deterministic in itself.
        let again = run(runs[0].1);
        let equal = again.iter().zip(&traces[0]).all(|(a, b)| {
            a.iter().flatten().zip(b.iter().flatten()).all(|(x, y)| x.to_bits() == y.to_bits())
        });
        println!("{name} origin run repeated: bit-equal {equal}");
        println!();
    }
}
```
