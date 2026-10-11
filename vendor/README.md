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
- Why: the angular joint limit (`AngularLimitParams::new`,
  `src/dynamics/solver/joint_constraint/joint_constraint_helper.rs`) took the sine and cosine
  of the limit centre with the inherent `f64::sin_cos`, the platform maths library's, on the
  solver path of every limited joint. `enhanced-determinism` turns on `simba/libm_force`,
  which routes `ComplexField::sin_cos` to `libm`; the inherent method ignores it. The patch
  calls the trait method, so the value is `libm`'s on every target (architecture section 8.2).
- Patch: `rapier3d-f64-0.36.0.patch`, one file, six changed lines (two removed, four added,
  two of them the notice).
- Upstream report: to be filed (the owner files it on dimforge/rapier; the link goes here).
- Dropped when: a bump reaches a version whose `AngularLimitParams::new` takes the trait
  route.

Not patched: the sign of a zero returned by `max` and `min` (`f64::max` and the `wide` SIMD
lanes differ between x86-64 and AArch64 when both operands are zeros of opposite sign). The
call sites are spread over `rapier3d-f64`, `parry3d-f64`, `simba`, `wide` and `glam`; no
patch under fifty lines closes them, so the item stays open in `docs/verification.md`.
