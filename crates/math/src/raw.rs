//! The one module that names the simulation float.
//!
//! Architecture section 16 decides the width in milestone 1. Switching it means editing this
//! file and nothing else in the crate: the alias, the matching unsigned integer, the constants
//! module and the `libm` bindings (the `f`-suffixed functions for 32 bits). It is deliberately
//! not a Cargo feature, because features unify across a build and an outside crate could then
//! change boundary numerics.

/// The simulation float.
pub(crate) type Raw = f64;

/// The unsigned integer as wide as [`Raw`].
pub(crate) type Bits = u64;

pub(crate) use core::f64::consts;

/// Transcendentals, exact rounding functions and `fmod` for [`Raw`], all from the `libm`
/// crate so the platform math library is never reached.
pub(crate) mod lib {
    pub(crate) use libm::{
        acos, asin, atan, atan2, cbrt, ceil, cos, cosh, exp, exp2, expm1, floor, fmod, hypot, log,
        log1p, log2, log10, pow, round, sin, sinh, tan, tanh, trunc,
    };
}

/// Width of [`Raw`] in bytes.
pub(crate) const BYTES: usize = size_of::<Raw>();

/// Bits of the fraction field.
pub(crate) const FRACTION_BITS: u32 = Raw::MANTISSA_DIGITS - 1;

/// Bits of the exponent field.
pub(crate) const EXPONENT_BITS: u32 = Bits::BITS - 1 - FRACTION_BITS;

const SIGN_BIT: Bits = 1 << (Bits::BITS - 1);

/// The single NaN that canonical bits ever contain: positive, quiet, zero payload.
pub(crate) const CANONICAL_NAN: Bits = ((1 << (EXPONENT_BITS + 1)) - 1) << (FRACTION_BITS - 1);

/// The in-memory bits of `x`, NaN sign and payload included.
///
/// Callers must not let the sign or payload of a NaN reach a result.
#[expect(
    clippy::disallowed_methods,
    reason = "the one raw bit read in the boundary; every caller handles NaN before using the bits"
)]
pub(crate) const fn bits(x: Raw) -> Bits {
    x.to_bits()
}

/// [`bits`] widened to 64 bits, for code that is written once for either width.
pub(crate) const fn bits_u64(x: Raw) -> u64 {
    bits(x) as u64
}

/// `x` widened to 64 bits.
#[expect(clippy::unnecessary_cast, reason = "a real conversion when `Raw` is the 32-bit float")]
pub(crate) const fn to_f64(x: Raw) -> f64 {
    x as f64
}

/// The bits of `x` with every NaN replaced by [`CANONICAL_NAN`].
pub(crate) const fn canonical_bits(x: Raw) -> Bits {
    if x.is_nan() { CANONICAL_NAN } else { bits(x) }
}

/// Whether `x` is negative zero. False for NaN.
pub(crate) const fn is_negative_zero(x: Raw) -> bool {
    bits(x) == SIGN_BIT
}
