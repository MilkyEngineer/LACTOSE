//! Deterministic simulation math: the same inputs give the same bits on every supported
//! target, with no dependence on the platform math library.
//!
//! Foundation tier, inside the determinism boundary, and the only place in the boundary
//! where floating point is computed (architecture section 8.2). See `README.md`.

#![forbid(unsafe_code)]

// Architecture section 8.2. Fused multiply-add is correctly rounded, so it is safe to use; the
// hazard is a build in which it is sometimes a hardware instruction and sometimes a call into
// the platform library, or in which a dependency picks fused or unfused by target feature.
// Every x86-64 build is pinned to x86-64-v3 by `.cargo/config.toml`; this fails the build if
// the pin is dropped, for instance by setting `RUSTFLAGS` or running cargo outside the
// repository.
#[cfg(all(target_arch = "x86_64", not(target_feature = "fma")))]
compile_error!(
    "lactose-math requires the `fma` target feature on x86-64. The x86-64-v3 pin in \
     .cargo/config.toml is not in effect: unset RUSTFLAGS and run cargo from inside the \
     repository."
);

mod canonical;
mod mat;
mod quat;
mod raw;
mod reduce;
pub mod render;
mod scalar;
mod transform;
mod vec;

pub use canonical::Canonical;
pub use mat::{Mat3, Mat4};
pub use quat::Quat;
pub use reduce::{ExactSum, ExactSummand, IntSum, Max, Min, Reducer};
pub use scalar::{Scalar, ScalarBits};
pub use transform::Transform;
pub use vec::{Vec2, Vec3, Vec4};
