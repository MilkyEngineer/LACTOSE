# lactose-math

Deterministic simulation math: the same inputs give the same bits on x86-64 and aarch64, in
any build of this workspace, with no dependence on the platform math library.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | inside; the only boundary crate that computes floating point |
| Unsafe | none, `#![forbid(unsafe_code)]` |

## What it holds

| Item | Contents |
|---|---|
| `Scalar`, `ScalarBits` | the simulation float as a newtype with no primitive float methods; arithmetic, conversions, compare-and-select `min`/`max`/`clamp`, roots, transcendentals, canonical bits, `unit_from_bits` for generator output |
| `Vec2`, `Vec3`, `Vec4` | vectors over `Scalar` |
| `Quat` | rotation quaternion |
| `Mat3`, `Mat4` | column-major matrices |
| `Transform` | translation, rotation, scale |
| `Canonical` | the one route from any of the above to bytes for hashing and serialisation, and back |
| `Reducer`, `ExactSum`, `Min`, `Max`, `IntSum` | order-independent reductions (architecture section 5.2) |
| `render::RenderPosition` | render space: 32-bit grid cell plus 32-bit float offset (architecture section 12.4) |

## Rules a caller should know

- **Comparison is IEEE.** `-0 == +0`, and NaN is unequal to and unordered with everything.
  `Scalar` has no `Eq`, `Ord` or `Hash`.
- **`min`, `max` and `clamp` use the numeric order refined by `-0 < +0`, and NaN is
  absorbing.** `min(-0, +0)` is `-0` and `max(-0, +0)` is `+0` in either operand order; a NaN
  in any position gives NaN. `clamp(x, lo, hi)` is `x.max(lo).min(hi)`, never panics, and
  gives `hi` when `lo > hi`. This makes `min` and `max` commutative and associative on every
  input, which is what lets them be reductions.
- **All NaNs are one value.** No method reveals a NaN's sign or payload; `to_f64` and `to_f32`
  return the canonical NaN. Canonical bits map every NaN to `0x7ff8000000000000`.
- **Negative zero is kept.** It is a distinct simulation state (`1 / -0` is `-inf`), so
  canonical bits keep its sign and two worlds that differ in it hash differently.
- **Hash and serialise through `Canonical`, never through a `bytemuck` byte view.** The types
  are `Pod` because components must be, and a byte view shows raw memory, where the NaN that
  `0 / 0` produces has a different sign on x86-64 and aarch64. On a little-endian target the
  canonical bytes of a value equal the memory bytes of `value.canonical()`, so a bulk column
  copy is canonical after one `canonical()` pass.
- **Evaluation order is part of every result.** Each sum of more than two terms is written
  out and documented on the method (`Vec3::dot` is `(x*x' + y*y') + z*z'`). Vector, quaternion
  and matrix code never uses fused multiply-add; only `Scalar::mul_add` does, and only when
  called.
- **There is no `Sum` or `Product` for `Scalar`.** An iterator sum hides its order. Write the
  order out, or use `ExactSum` where the order is not yours to choose.
- **Quaternions are not renormalised behind the caller's back.** Methods that need unit
  length say so.
- **Render space is a different space.** There is no `From`, `Into` or operator between
  `RenderPosition` and the simulation types, only `RenderPosition::from_sim`.
- **`Reducer` results do not depend on order or grouping**, for any input including NaN and
  infinities. `ExactSum` holds the exact sum of every finite value and rounds once at the end;
  the type documents its range.

## Layout

Every type is `repr(C)` (or `repr(transparent)`), `Pod`, and padding-free: an array of
`Scalar`s in declaration order, so its size is the scalar count times `Scalar::BYTES`.

| Type | Scalars, in order |
|---|---|
| `Scalar` | 1 |
| `Vec2`, `Vec3`, `Vec4` | `x, y[, z[, w]]` |
| `Quat` | `x, y, z, w` |
| `Mat3` | 9, column-major: `x_axis, y_axis, z_axis` |
| `Mat4` | 16, column-major: `x_axis, y_axis, z_axis, w_axis` |
| `Transform` | 10: `translation.xyz, rotation.xyzw, scale.xyz` |

`render::RenderPosition` is `cell: [i32; 3]` then `offset: [f32; 3]`, 24 bytes.

## Why the results are the same everywhere

- **Basic operations** (`+ - * /`, comparisons, conversions) are single IEEE 754 operations
  that Rust compiles without reassociation or contraction, so each has one correct result.
  `a * b + c * d` is two multiplications and an addition on every target; the compiler never
  fuses it.
- **`sqrt` and `mul_add` use the standard library operations, not `libm`.** IEEE 754
  requires both to be correctly rounded, so every conforming implementation gives the same
  bits; the question is only which implementation runs. On aarch64 they compile to `fsqrt`
  and `fmadd`. On x86-64, `sqrt` compiles to `sqrtsd`, and `mul_add` compiles to
  `vfmadd213sd` when the `fma` target feature is on and to a call to the platform's `fma`
  otherwise. Section 8.2's hazard is exactly that: fused chosen by target feature, with the
  platform library behind one branch. The workspace pins x86-64 to x86-64-v3, and this crate
  has a `compile_error!` on x86-64 without `fma`, so the instruction is the only path that
  can be built. `libm::fma` would instead go through a function pointer chosen by CPU
  detection at run time, which is slower and gains nothing once the feature is guaranteed.
- **`floor`, `ceil`, `trunc`, `round` and `%`** are exact operations taken from `libm`, since
  the primitive `%` and, without SSE4.1, the rounding functions compile to platform library
  calls.
- **Transcendentals** come from the `libm` crate, pinned in `Cargo.lock`: the same Rust
  source on every target, built from the basic operations above. `powi` is written out here
  as binary exponentiation.
- **`libm` features.** The workspace enables exactly `arch`. In 0.2.16 that feature swaps
  in hardware `sqrt` (x86-64 and aarch64), `fma` (aarch64; on x86-64 by run-time detection
  of FMA or FMA4, else the software fallback) and `rint` (aarch64). All are exact or
  correctly rounded, so no path can change a bit; the golden table passes with and without
  the feature. `unstable-intrinsics`, `unstable-float` and `force-soft-floats` stay off.
- **The golden table** (`tests/golden/vectors.txt`, about five thousand cases) records the
  canonical result bits of every operation as computed on aarch64. CI runs it on x86-64 Linux
  and Windows; a mismatch prints the operation, inputs and both bit patterns.

## The float width

`Scalar` is 64-bit until the milestone 1 precision test (architecture section 16). The
primitive is named only in the private module `raw`, with its bit type, constants and `libm`
bindings; switching is an edit to that file plus a regenerated golden table, never a Cargo
feature, because features unify across a build and an outside crate could then change
boundary numerics. The `from_f64`/`to_f64` conversions keep their names and round if the
width narrows.

## Regenerating the golden table

On aarch64 only, after adding an operation or a case:

```
cargo nextest run -p lactose-math --run-ignored only -E 'test(bless)'
```

Review the diff. A changed line for an existing case is a determinism break, not a refresh.
