# Vendored crates

Third-party crates the workspace builds from a patched copy, under the rule of architecture
section 8.2 (D123) and checked by `cargo xtask vendor`. Each `<name>-<version>/` is the
published archive as `cargo vendor --versioned-dirs` unpacks it, `.cargo-checksum.json`
included, with `<name>-<version>.patch` applied; nothing else. The crate keeps its crates.io
name in `crate-tiers.toml` and `deny.toml`, and the root `Cargo.toml` names the directory
under `[patch.crates-io]`. The directory is excluded from the workspace: no workspace lint,
format or test runs over it.

To add or bump a crate: vendor the version from a scratch manifest that depends on it as the
root `[workspace.dependencies]` entry does (`cargo vendor --versioned-dirs` into a temporary
directory, with no `[patch]` entry), move `<name>-<version>/` here, apply the patch with
`git apply`, mark every change with a `LACTOSE patch:` comment, point the `[patch.crates-io]`
entry at it, regenerate `Cargo.lock`, write the section below and run `cargo xtask vendor`.
A patch that no longer applies is rewritten and reviewed as new; a version that carries the
change upstream drops the directory, the patch and the entry.

## rapier3d-f64 0.36.0

- Upstream: crates.io `rapier3d-f64` 0.36.0 (git `b716d375efc0201003f0cd9ef7168eee0b62c177`,
  path `crates/rapier3d-f64`); the archive's sha256 is `package` in `.cargo-checksum.json`.
- Used by: `lactose-physics-rapier`, inside the determinism boundary.
- Why, two defects:
  - The angular joint limit (`AngularLimitParams::new`,
    `src/dynamics/solver/joint_constraint/joint_constraint_helper.rs`) took the sine and cosine
    of the limit centre with the inherent `f64::sin_cos`, the platform maths library's, on the
    solver path of every limited joint. `enhanced-determinism` turns on `simba/libm_force`,
    which routes `ComplexField::sin_cos` to `libm`; the inherent method ignores it. The patch
    calls the trait method, so the value is `libm`'s on every target (architecture section
    8.2).
  - The broad phase's deferred tree optimisation hung the step. On a pool of more than one
    thread `PhysicsPipeline::detect_collisions` (`src/pipeline/physics_pipeline/solve.rs`)
    queued it with `rayon::spawn` and `join_deferred_bvh_optimize`
    (`src/pipeline/physics_pipeline/mod.rs`) blocked the stepping thread, itself a pool thread,
    in `mpsc::Receiver::recv` until it ran. rayon tries to wake a thread for a job a pool
    thread pushes to its own deque but does not guarantee it; it relies on the pushing thread
    to pop the job, which a thread blocked in `recv` never does. The step hung with every other
    thread asleep and the job stealable: once on a four-thread pool (the D124 review's sample),
    in 4 of 9 unpatched runs of a five-scene probe and 4 of 12 of six two-thread walks in a row
    (the S5 review's probes; the patched build hung in none), and every time while every
    other thread of the pool is held (`crates/physics-rapier/tests/pool.rs`). `solve.rs` holds `unsafe`,
    so the patch is in `BroadPhaseBvh::take_deferred_optimize`
    (`src/geometry/broad_phase_bvh/mod.rs`): it runs the pass where the pipeline takes it, on
    the stepping thread, and hands the pipeline nothing, so the detached path is never taken.
    The pipeline joins the pass before every use of the broad phase, and between the take and
    the join the tree and its workspace are out of the broad phase on every path, so nothing
    reads them there; the pass runs the same code on the same tree as the one-thread path does
    at the join, and every report and snapshot byte of `crates/physics-rapier`'s scenes on one,
    two and four threads equals the unpatched build's on one thread. A defect that blocks the
    milestone 1 physics criteria on a pool of more than one thread (section 16), reviewed as a
    determinism review.
- Patch: `rapier3d-f64-0.36.0.patch`, two files, sixteen changed lines: six in the joint
  helper (two removed, four added, two of them the notice), ten in the broad phase (two
  removed, eight added, three of them the notice).
- Upstream report: to be filed, one issue per defect (the owner files them on dimforge/rapier;
  the links go here).
- Dropped when: a bump reaches a version whose `AngularLimitParams::new` takes the trait
  route, and whose step never blocks a pool thread on a job it queued. Each hunk drops on its
  own.

Not patched: the sign of a zero returned by `max` and `min` (`f64::max` and the `wide` SIMD
lanes differ between x86-64 and AArch64 when both operands are zeros of opposite sign). The
call sites are spread over `rapier3d-f64`, `parry3d-f64`, `simba`, `wide` and `glam`; no
patch under fifty lines closes them, so the item stays open in `docs/verification.md`.
