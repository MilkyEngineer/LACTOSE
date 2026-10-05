//! [`Scalar`], the simulation float.

use core::fmt;
use core::ops::{
    Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Rem, RemAssign, Sub, SubAssign,
};

use bytemuck::{Pod, Zeroable};

use crate::raw::{self, Raw};

/// The unsigned integer as wide as [`Scalar`], the type of its canonical bits.
pub type ScalarBits = raw::Bits;

/// The simulation float: 64-bit until the milestone 1 precision test decides finally.
///
/// It exposes no method of the primitive float, so nothing inside the determinism boundary can
/// reach the platform math library or a NaN's sign and payload by accident. Every operation is
/// defined here and gives the same bits on every supported target.
///
/// # Comparison and NaN
///
/// `==`, `<` and the other comparison operators are the IEEE 754 ones: negative zero equals
/// positive zero, and NaN is unordered and unequal to everything including itself. `Scalar`
/// therefore implements neither `Eq`, `Ord` nor `Hash`. All NaNs are indistinguishable through
/// this API; test for one with [`is_nan`](Self::is_nan).
///
/// # Layout
///
/// `repr(transparent)` over the float, so [`Scalar::BYTES`] bytes, no padding, and any bit
/// pattern is valid. A `bytemuck` byte view is the raw memory, in which a NaN's sign and payload
/// differ between architectures; anything hashed, compared bytewise or serialised goes through
/// [`Canonical`](crate::Canonical) instead.
#[derive(Clone, Copy, PartialEq, PartialOrd, Default, Pod, Zeroable)]
#[repr(transparent)]
pub struct Scalar(Raw);

impl Scalar {
    /// Size in bytes, in memory and in canonical form.
    pub const BYTES: usize = raw::BYTES;

    /// Positive zero.
    pub const ZERO: Self = Self(0.0);
    /// One.
    pub const ONE: Self = Self(1.0);
    /// Minus one.
    pub const NEG_ONE: Self = Self(-1.0);
    /// One half.
    pub const HALF: Self = Self(0.5);
    /// Two.
    pub const TWO: Self = Self(2.0);
    /// The ratio of a circle's circumference to its diameter.
    pub const PI: Self = Self(raw::consts::PI);
    /// A full turn in radians.
    pub const TAU: Self = Self(raw::consts::TAU);
    /// A quarter turn in radians.
    pub const FRAC_PI_2: Self = Self(raw::consts::FRAC_PI_2);
    /// The difference between one and the next larger representable value.
    pub const EPSILON: Self = Self(Raw::EPSILON);
    /// The largest finite value.
    pub const MAX: Self = Self(Raw::MAX);
    /// The smallest finite value, `-MAX`.
    pub const MIN: Self = Self(Raw::MIN);
    /// The smallest positive normal value.
    pub const MIN_POSITIVE: Self = Self(Raw::MIN_POSITIVE);
    /// Positive infinity.
    pub const INFINITY: Self = Self(Raw::INFINITY);
    /// Negative infinity.
    pub const NEG_INFINITY: Self = Self(Raw::NEG_INFINITY);
    /// Not a number.
    pub const NAN: Self = Self(Raw::from_bits(raw::CANONICAL_NAN));

    // Conversions. Every one is a single IEEE conversion, rounded to nearest with ties to even
    // where the target cannot hold the value.

    /// From a 64-bit float, typically a literal: `Scalar::from_f64(9.81)`.
    #[must_use]
    pub const fn from_f64(value: f64) -> Self {
        Self(value as Raw)
    }

    /// From a 32-bit float.
    #[must_use]
    pub const fn from_f32(value: f32) -> Self {
        Self(value as Raw)
    }

    /// From a signed 32-bit integer.
    #[must_use]
    pub const fn from_i32(value: i32) -> Self {
        Self(value as Raw)
    }

    /// From an unsigned 32-bit integer.
    #[must_use]
    pub const fn from_u32(value: u32) -> Self {
        Self(value as Raw)
    }

    /// From a signed 64-bit integer.
    #[must_use]
    pub const fn from_i64(value: i64) -> Self {
        Self(value as Raw)
    }

    /// From an unsigned 64-bit integer.
    #[must_use]
    pub const fn from_u64(value: u64) -> Self {
        Self(value as Raw)
    }

    /// To a 64-bit float, for leaving the boundary. A NaN comes out as the canonical NaN, so
    /// the primitive carries no sign or payload out with it.
    #[must_use]
    pub const fn to_f64(self) -> f64 {
        raw::to_f64(self.canonical().0)
    }

    /// To a 32-bit float, for leaving the boundary. A NaN comes out as `f32::NAN`.
    #[must_use]
    pub const fn to_f32(self) -> f32 {
        if self.0.is_nan() { f32::NAN } else { self.0 as f32 }
    }

    /// To a signed 32-bit integer: truncated toward zero, saturated at the integer's range,
    /// zero for NaN.
    #[must_use]
    pub const fn to_i32(self) -> i32 {
        self.0 as i32
    }

    /// To a signed 64-bit integer: truncated toward zero, saturated at the integer's range,
    /// zero for NaN.
    #[must_use]
    pub const fn to_i64(self) -> i64 {
        self.0 as i64
    }

    /// A uniform value in `[0, 1)` from raw generator output.
    ///
    /// Uses the top bits of `bits`, as many as the fraction holds, so the result is an exact
    /// multiple of the spacing just below one and every such multiple is equally likely.
    #[must_use]
    pub const fn unit_from_bits(bits: u64) -> Self {
        let digits = Raw::MANTISSA_DIGITS;
        let top = bits >> (u64::BITS - digits);
        // Both the conversion and the product are exact: `top` has at most `digits` bits and
        // the factor is a power of two.
        Self(top as Raw * (1.0 / (1u64 << digits) as Raw))
    }

    // Canonical bits.

    /// The bits of the value, with every NaN replaced by one canonical quiet NaN.
    ///
    /// Negative zero keeps its sign: it is a distinct simulation state (its reciprocal and its
    /// `atan2` differ from positive zero's), so two worlds that differ in it must hash apart.
    #[must_use]
    pub const fn to_canonical_bits(self) -> ScalarBits {
        raw::canonical_bits(self.0)
    }

    /// The value with the given bits. Any NaN pattern decodes to the canonical NaN, so
    /// encoding the result always gives canonical bits.
    #[must_use]
    pub const fn from_canonical_bits(bits: ScalarBits) -> Self {
        Self(Raw::from_bits(bits)).canonical()
    }

    pub(crate) const fn canonical(self) -> Self {
        if self.0.is_nan() { Self::NAN } else { self }
    }

    pub(crate) const fn raw(self) -> Raw {
        self.0
    }

    pub(crate) const fn from_raw(value: Raw) -> Self {
        Self(value)
    }

    // Classification.

    /// Whether the value is NaN.
    #[must_use]
    pub const fn is_nan(self) -> bool {
        self.0.is_nan()
    }

    /// Whether the value is neither infinite nor NaN.
    #[must_use]
    pub const fn is_finite(self) -> bool {
        self.0.is_finite()
    }

    // Compare-and-select. The order is the numeric one refined by `-0 < +0`, and NaN is
    // absorbing, which makes `min` and `max` commutative and associative on every input. That
    // is what lets them be order-independent reductions.

    /// The smaller of two values.
    ///
    /// NaN if either operand is NaN. Negative zero is smaller than positive zero, so
    /// `min(-0, +0)` and `min(+0, -0)` are both `-0`.
    #[must_use]
    pub const fn min(self, other: Self) -> Self {
        let (a, b) = (self.0, other.0);
        if a.is_nan() || b.is_nan() {
            Self::NAN
        } else if a < b || (a == b && raw::is_negative_zero(a)) {
            // Equal operands are the same value or zeros of either sign.
            self
        } else {
            other
        }
    }

    /// The larger of two values.
    ///
    /// NaN if either operand is NaN. Positive zero is larger than negative zero, so
    /// `max(-0, +0)` and `max(+0, -0)` are both `+0`.
    #[must_use]
    pub const fn max(self, other: Self) -> Self {
        let (a, b) = (self.0, other.0);
        if a.is_nan() || b.is_nan() {
            Self::NAN
        } else if b > a || (a == b && raw::is_negative_zero(a)) {
            other
        } else {
            self
        }
    }

    /// `self` limited to `[lo, hi]`, defined as `self.max(lo).min(hi)`.
    ///
    /// NaN if any operand is NaN. Zeros follow `-0 < +0`: clamping `-0` to `[+0, 1]` gives
    /// `+0`. It never panics; if `lo > hi` the result is `hi`.
    #[must_use]
    pub const fn clamp(self, lo: Self, hi: Self) -> Self {
        self.max(lo).min(hi)
    }

    // Exact and correctly rounded operations.

    /// The absolute value. NaN stays NaN.
    #[must_use]
    pub const fn abs(self) -> Self {
        Self(self.0.abs())
    }

    /// One for a positive value, minus one for a negative value, positive zero for a zero of
    /// either sign, NaN for NaN.
    #[must_use]
    pub const fn sign(self) -> Self {
        if self.0 > 0.0 {
            Self::ONE
        } else if self.0 < 0.0 {
            Self::NEG_ONE
        } else if self.0.is_nan() {
            Self::NAN
        } else {
            Self::ZERO
        }
    }

    /// `1 / self`.
    #[must_use]
    pub const fn recip(self) -> Self {
        Self(1.0 / self.0)
    }

    /// The square root, correctly rounded. NaN for a negative value; `-0` gives `-0`.
    #[must_use]
    pub fn sqrt(self) -> Self {
        Self(self.0.sqrt())
    }

    /// `self * a + b` with a single rounding (fused multiply-add).
    // Never inlined into another crate. An inlined copy is code-generated with the caller's
    // target features, and a caller built without the x86-64-v3 pin (a doctest, a crate built
    // through a wrapper) would turn it into a call to the platform `fma`. Kept out of line,
    // the instruction is always generated here, where the `compile_error!` in `lib.rs`
    // guarantees the `fma` feature.
    #[inline(never)]
    #[must_use]
    #[expect(
        clippy::disallowed_methods,
        reason = "the sanctioned fused multiply-add: one hardware instruction under the fma guard"
    )]
    pub fn mul_add(self, a: Self, b: Self) -> Self {
        Self(self.0.mul_add(a.0, b.0))
    }

    /// The largest integer not above `self`.
    #[must_use]
    pub fn floor(self) -> Self {
        Self(raw::lib::floor(self.0))
    }

    /// The smallest integer not below `self`.
    #[must_use]
    pub fn ceil(self) -> Self {
        Self(raw::lib::ceil(self.0))
    }

    /// The integer part, rounding toward zero.
    #[must_use]
    pub fn trunc(self) -> Self {
        Self(raw::lib::trunc(self.0))
    }

    /// The nearest integer, with halfway cases away from zero.
    #[must_use]
    pub fn round(self) -> Self {
        Self(raw::lib::round(self.0))
    }

    /// The non-negative remainder: `self % rhs`, plus `|rhs|` when that is negative.
    ///
    /// In `[0, |rhs|)` for finite operands and non-zero `rhs`, except that the addition can
    /// round up to `|rhs|` itself when `self` is negative and tiny beside `rhs`. NaN if `rhs`
    /// is zero or `self` is infinite.
    #[must_use]
    pub fn rem_euclid(self, rhs: Self) -> Self {
        let r = raw::lib::fmod(self.0, rhs.0);
        if r < 0.0 { Self(r + rhs.0.abs()) } else { Self(r) }
    }

    /// `self + (end - self) * t`, evaluated in exactly that order. Not clamped.
    #[must_use]
    pub fn lerp(self, end: Self, t: Self) -> Self {
        self + (end - self) * t
    }

    /// Degrees to radians: `self * (PI / 180)`.
    #[must_use]
    pub fn to_radians(self) -> Self {
        Self(self.0 * (raw::consts::PI / 180.0))
    }

    /// Radians to degrees: `self * (180 / PI)`.
    #[must_use]
    pub fn to_degrees(self) -> Self {
        Self(self.0 * (180.0 / raw::consts::PI))
    }

    /// `self` raised to an integer power by binary exponentiation.
    ///
    /// The product is accumulated from the lowest exponent bit upward over repeated squares
    /// of `self`, each multiplication rounded, so the error grows with the exponent and the
    /// result is not the correctly rounded power; use [`powf`](Self::powf) where that
    /// matters. A negative exponent computes the positive power and takes its reciprocal at
    /// the end, so a positive power that overflows gives zero even where the true result is
    /// a representable subnormal: `2.powi(-1074)` is zero. `powi(0)` is one for every `self`,
    /// NaN included.
    #[must_use]
    pub const fn powi(self, exponent: i32) -> Self {
        let mut square = self.0;
        let mut remaining = exponent.unsigned_abs();
        let mut product: Raw = 1.0;
        while remaining != 0 {
            if remaining & 1 == 1 {
                product *= square;
            }
            remaining >>= 1;
            if remaining != 0 {
                square *= square;
            }
        }
        if exponent < 0 { Self(1.0 / product) } else { Self(product) }
    }

    // Transcendentals, from the `libm` crate.

    /// The sine, in radians.
    #[must_use]
    pub fn sin(self) -> Self {
        Self(raw::lib::sin(self.0))
    }

    /// The cosine, in radians.
    #[must_use]
    pub fn cos(self) -> Self {
        Self(raw::lib::cos(self.0))
    }

    /// `(self.sin(), self.cos())`, bit-identical to calling the two separately.
    #[must_use]
    pub fn sin_cos(self) -> (Self, Self) {
        (self.sin(), self.cos())
    }

    /// The tangent, in radians.
    #[must_use]
    pub fn tan(self) -> Self {
        Self(raw::lib::tan(self.0))
    }

    /// The arcsine, in `[-PI/2, PI/2]`. NaN outside `[-1, 1]`.
    #[must_use]
    pub fn asin(self) -> Self {
        Self(raw::lib::asin(self.0))
    }

    /// The arccosine, in `[0, PI]`. NaN outside `[-1, 1]`.
    #[must_use]
    pub fn acos(self) -> Self {
        Self(raw::lib::acos(self.0))
    }

    /// The arctangent, in `[-PI/2, PI/2]`.
    #[must_use]
    pub fn atan(self) -> Self {
        Self(raw::lib::atan(self.0))
    }

    /// The angle of the point `(x, self)` from the positive x axis, in `[-PI, PI]`.
    #[must_use]
    pub fn atan2(self, x: Self) -> Self {
        Self(raw::lib::atan2(self.0, x.0))
    }

    /// The hyperbolic sine.
    #[must_use]
    pub fn sinh(self) -> Self {
        Self(raw::lib::sinh(self.0))
    }

    /// The hyperbolic cosine.
    #[must_use]
    pub fn cosh(self) -> Self {
        Self(raw::lib::cosh(self.0))
    }

    /// The hyperbolic tangent.
    #[must_use]
    pub fn tanh(self) -> Self {
        Self(raw::lib::tanh(self.0))
    }

    /// `e` raised to `self`.
    #[must_use]
    pub fn exp(self) -> Self {
        Self(raw::lib::exp(self.0))
    }

    /// Two raised to `self`.
    #[must_use]
    pub fn exp2(self) -> Self {
        Self(raw::lib::exp2(self.0))
    }

    /// `e` raised to `self`, minus one, accurate near zero.
    #[must_use]
    pub fn exp_m1(self) -> Self {
        Self(raw::lib::expm1(self.0))
    }

    /// The natural logarithm.
    #[must_use]
    pub fn ln(self) -> Self {
        Self(raw::lib::log(self.0))
    }

    /// The natural logarithm of `1 + self`, accurate near zero.
    #[must_use]
    pub fn ln_1p(self) -> Self {
        Self(raw::lib::log1p(self.0))
    }

    /// The base-two logarithm.
    #[must_use]
    pub fn log2(self) -> Self {
        Self(raw::lib::log2(self.0))
    }

    /// The base-ten logarithm.
    #[must_use]
    pub fn log10(self) -> Self {
        Self(raw::lib::log10(self.0))
    }

    /// `self` raised to a real power.
    #[must_use]
    pub fn powf(self, exponent: Self) -> Self {
        Self(raw::lib::pow(self.0, exponent.0))
    }

    /// The cube root.
    #[must_use]
    pub fn cbrt(self) -> Self {
        Self(raw::lib::cbrt(self.0))
    }

    /// `sqrt(self² + other²)` without intermediate overflow or underflow.
    #[must_use]
    pub fn hypot(self, other: Self) -> Self {
        Self(raw::lib::hypot(self.0, other.0))
    }
}

impl fmt::Debug for Scalar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for Scalar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl Add for Scalar {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Scalar {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

impl Mul for Scalar {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        Self(self.0 * rhs.0)
    }
}

impl Div for Scalar {
    type Output = Self;
    fn div(self, rhs: Self) -> Self {
        Self(self.0 / rhs.0)
    }
}

/// The remainder of truncating division, with the sign of the dividend. It is exact, and
/// computed by `libm` because the primitive `%` compiles to a call into the platform library.
impl Rem for Scalar {
    type Output = Self;
    fn rem(self, rhs: Self) -> Self {
        Self(raw::lib::fmod(self.0, rhs.0))
    }
}

impl Neg for Scalar {
    type Output = Self;
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

impl AddAssign for Scalar {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl SubAssign for Scalar {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl MulAssign for Scalar {
    fn mul_assign(&mut self, rhs: Self) {
        *self = *self * rhs;
    }
}

impl DivAssign for Scalar {
    fn div_assign(&mut self, rhs: Self) {
        *self = *self / rhs;
    }
}

impl RemAssign for Scalar {
    fn rem_assign(&mut self, rhs: Self) {
        *self = *self % rhs;
    }
}
