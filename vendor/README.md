# Vendored crates

Third-party crates the workspace builds from a patched copy, through `[patch.crates-io]` in
the root `Cargo.toml`. Each directory is the published `.crate` archive, unpacked, with only
the change listed here applied. The directory is excluded from the workspace: none of the
workspace's lints, formatting or tests run over it.

A bump of a vendored crate replaces the directory with the new archive and re-applies the
patch, or drops the patch when upstream has made the same change.

## rapier3d-f64 0.36.0

- Upstream: crates.io `rapier3d-f64` 0.36.0, archive sha256
  `c28c8d3fba1ae7bd3051fa8295217a695f5cac724c5be71921514fe3e14c5dd9`, git
  `b716d375efc0201003f0cd9ef7168eee0b62c177`, path `crates/rapier3d-f64`.
- Used by: `lactose-physics-rapier`, inside the determinism boundary.
- Why: the angular joint limit (`AngularLimitParams::new`) took the sine and cosine of the
  limit centre with the inherent `f64::sin_cos`, the platform maths library's, on the solver
  path of every limited joint. `enhanced-determinism` turns on `simba/libm_force`, which routes
  `ComplexField::sin_cos` to `libm`; the inherent method ignores it. The patch calls the trait
  method, so the value is `libm`'s on every target (architecture section 8.2).
- Size: 4 changed lines and 2 comment lines, one file.

```diff
--- a/src/dynamics/solver/joint_constraint/joint_constraint_helper.rs
+++ b/src/dynamics/solver/joint_constraint/joint_constraint_helper.rs
@@ -58,10 +58,12 @@
         }
 
         let center = (min + max) * 0.5;
+        // LACTOSE patch: through `ComplexField`, which `libm_force` routes to `libm`; the
+        // inherent `f64::sin_cos` is the platform library's.
         #[cfg(feature = "dim2")]
-        let (sin, cos) = center.sin_cos();
+        let (sin, cos) = simba::scalar::ComplexField::sin_cos(center);
         #[cfg(feature = "dim3")]
-        let (sin, cos) = (center * 0.5).sin_cos();
+        let (sin, cos) = simba::scalar::ComplexField::sin_cos(center * 0.5);
 
         Self {
             center: [cos, sin],
```

Not patched: the sign of a zero returned by `max` and `min` (`f64::max` and the `wide` SIMD
lanes differ between x86-64 and AArch64 when both operands are zeros of opposite sign). The
call sites are spread over `rapier3d-f64`, `parry3d-f64`, `simba`, `wide` and `glam`; no
patch under fifty lines closes them, so the item stays open in `docs/verification.md`.
