//! Order-independent reductions, architecture section 5.2.
//!
//! A system that reduces over entities runs as chunk jobs in an order and a grouping the
//! scheduler chooses. Each job folds its rows into a partial with [`Reducer::add`], and the
//! partials are folded together with [`Reducer::merge`]. Every reducer here gives the same
//! result for every order and grouping, so the result is identical under any thread count and
//! chunk layout without a sort.

use core::marker::PhantomData;

use crate::raw::{self, Raw};
use crate::{Scalar, Vec2, Vec3, Vec4};

/// A reduction whose result does not depend on the order or grouping of its inputs.
///
/// # Contract
///
/// For any multiset of items, [`finish`](Self::finish) returns the same value however the
/// items are split between partials, in whatever order each partial receives its items, and
/// in whatever order and shape the partials are merged. A partial that received no item is
/// neutral in a merge.
///
/// The trait is sealed: architecture section 5.2 has reductions use provided combinators, and
/// a reducer that broke the contract would break determinism without failing any type check.
/// A new reducer is added here, with a test of the contract.
pub trait Reducer: Sized + sealed::SealedReducer {
    /// What is folded in.
    type Item;
    /// What the reduction produces.
    type Output;

    /// The partial that has received nothing.
    #[must_use]
    fn identity() -> Self;

    /// Folds one item in.
    fn add(&mut self, item: Self::Item);

    /// Folds another partial in.
    fn merge(&mut self, other: &Self);

    /// The result over everything folded in.
    #[must_use]
    fn finish(self) -> Self::Output;
}

/// The exact sum of scalars or vectors, rounded once at the end.
///
/// Each scalar lane is kept as an exact wide integer, so nothing is rounded until
/// [`finish`](Reducer::finish), which returns the true sum rounded to nearest with ties to
/// even. The result is therefore independent of order and grouping, and also more accurate
/// than a running sum: `1e300 + 1 - 1e300` is one.
///
/// # Range and special values
///
/// - Every finite value, subnormals included, is added exactly; there is no precision limit.
/// - Intermediate sums beyond the finite range are held exactly, so `MAX + MAX - MAX` is
///   `MAX`. A final sum beyond the finite range rounds to infinity of its sign.
/// - A zero sum is positive zero, whatever the signs of the zeros added.
/// - A NaN input, or infinities of both signs, give NaN. Otherwise an infinite input gives
///   that infinity.
/// - The integer wraps only after more than `2^128` additions of the largest finite value,
///   which no simulation reaches.
///
/// A partial holds 288 bytes per scalar lane, and an addition costs a few integer operations.
#[derive(Clone, Debug)]
pub struct ExactSum<T: ExactSummand = Scalar> {
    lanes: T::Lanes,
}

impl<T: ExactSummand> sealed::SealedReducer for ExactSum<T> {}

impl<T: ExactSummand> Reducer for ExactSum<T> {
    type Item = T;
    type Output = T;

    fn identity() -> Self {
        Self { lanes: T::zero_lanes() }
    }

    fn add(&mut self, item: T) {
        item.add_to(&mut self.lanes);
    }

    fn merge(&mut self, other: &Self) {
        for (lane, other) in self.lanes.as_mut().iter_mut().zip(other.lanes.as_ref()) {
            lane.merge(other);
        }
    }

    fn finish(self) -> T {
        T::from_lanes(&self.lanes)
    }
}

impl<T: ExactSummand> Default for ExactSum<T> {
    fn default() -> Self {
        Self::identity()
    }
}

/// A type [`ExactSum`] can add: [`Scalar`] and the vectors, lane by lane. Sealed.
pub trait ExactSummand: Copy + sealed::Sealed {
    /// One accumulator per scalar lane.
    #[doc(hidden)]
    type Lanes: Clone
        + core::fmt::Debug
        + AsRef<[sealed::Accumulator]>
        + AsMut<[sealed::Accumulator]>;

    #[doc(hidden)]
    fn zero_lanes() -> Self::Lanes;

    #[doc(hidden)]
    fn add_to(self, lanes: &mut Self::Lanes);

    #[doc(hidden)]
    fn from_lanes(lanes: &Self::Lanes) -> Self;
}

macro_rules! exact_summand {
    ($($ty:ty => $count:literal, $to_array:expr, $from_array:expr;)*) => {$(
        impl sealed::Sealed for $ty {}

        impl ExactSummand for $ty {
            type Lanes = [sealed::Accumulator; $count];

            fn zero_lanes() -> Self::Lanes {
                [sealed::Accumulator::ZERO; $count]
            }

            fn add_to(self, lanes: &mut Self::Lanes) {
                let scalars: [Scalar; $count] = $to_array(self);
                for (lane, scalar) in lanes.iter_mut().zip(scalars) {
                    lane.add(scalar.raw());
                }
            }

            fn from_lanes(lanes: &Self::Lanes) -> Self {
                let mut scalars = [Scalar::ZERO; $count];
                for (scalar, lane) in scalars.iter_mut().zip(lanes) {
                    *scalar = Scalar::from_raw(lane.finish());
                }
                $from_array(scalars)
            }
        }
    )*};
}

exact_summand! {
    Scalar => 1, |scalar| [scalar], |[scalar]: [Scalar; 1]| scalar;
    Vec2 => 2, Vec2::to_array, Vec2::from_array;
    Vec3 => 3, Vec3::to_array, Vec3::from_array;
    Vec4 => 4, Vec4::to_array, Vec4::from_array;
}

/// The smallest item, or `None` when there was none.
///
/// For [`Scalar`] the order is that of [`Scalar::min`]: any NaN item makes the result NaN,
/// and negative zero is smaller than positive zero.
#[derive(Clone, Copy, Debug)]
pub struct Min<T>(Option<T>);

/// The largest item, or `None` when there was none.
///
/// For [`Scalar`] the order is that of [`Scalar::max`]: any NaN item makes the result NaN,
/// and positive zero is larger than negative zero.
#[derive(Clone, Copy, Debug)]
pub struct Max<T>(Option<T>);

/// A type [`Min`] and [`Max`] can reduce: [`Scalar`] and the primitive integers. Sealed.
///
/// These are the types for which two items that compare equal are the same value, so which
/// of two equal items is kept cannot be observed. A type whose ordering ignores part of its
/// value would make the result depend on the order of the items.
pub trait Extremum: Copy + sealed::Sealed {
    #[doc(hidden)]
    fn lesser(self, other: Self) -> Self;

    #[doc(hidden)]
    fn greater(self, other: Self) -> Self;
}

impl Extremum for Scalar {
    fn lesser(self, other: Self) -> Self {
        self.min(other)
    }

    fn greater(self, other: Self) -> Self {
        self.max(other)
    }
}

macro_rules! integer_extremum {
    ($($ty:ty),*) => {$(
        impl sealed::Sealed for $ty {}

        impl Extremum for $ty {
            fn lesser(self, other: Self) -> Self {
                Ord::min(self, other)
            }

            fn greater(self, other: Self) -> Self {
                Ord::max(self, other)
            }
        }
    )*};
}

integer_extremum!(i8, i16, i32, i64, i128, u8, u16, u32, u64, u128);

macro_rules! extremum_reducer {
    ($reducer:ident, $select:ident) => {
        impl<T: Extremum> sealed::SealedReducer for $reducer<T> {}

        impl<T: Extremum> Reducer for $reducer<T> {
            type Item = T;
            type Output = Option<T>;

            fn identity() -> Self {
                Self(None)
            }

            fn add(&mut self, item: T) {
                self.0 = Some(match self.0 {
                    Some(current) => current.$select(item),
                    None => item,
                });
            }

            fn merge(&mut self, other: &Self) {
                if let Some(item) = other.0 {
                    self.add(item);
                }
            }

            fn finish(self) -> Option<T> {
                self.0
            }
        }
    };
}

extremum_reducer!(Min, lesser);
extremum_reducer!(Max, greater);

impl<T> Default for Min<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<T> Default for Max<T> {
    fn default() -> Self {
        Self(None)
    }
}

/// The sum of integers, as an `i128`.
///
/// Items are any integer type that converts to `i128` losslessly. The total wraps on `i128`
/// overflow, which keeps the result order-independent and takes more than `2^63` items of
/// the largest 64-bit magnitude to reach.
#[derive(Clone, Copy, Debug)]
pub struct IntSum<T> {
    total: i128,
    item: PhantomData<fn(T)>,
}

impl<T: Into<i128>> sealed::SealedReducer for IntSum<T> {}

impl<T: Into<i128>> Reducer for IntSum<T> {
    type Item = T;
    type Output = i128;

    fn identity() -> Self {
        Self { total: 0, item: PhantomData }
    }

    fn add(&mut self, item: T) {
        self.total = self.total.wrapping_add(item.into());
    }

    fn merge(&mut self, other: &Self) {
        self.total = self.total.wrapping_add(other.total);
    }

    fn finish(self) -> i128 {
        self.total
    }
}

impl<T: Into<i128>> Default for IntSum<T> {
    fn default() -> Self {
        Self::identity()
    }
}

mod sealed {
    use super::{Raw, raw};

    pub trait Sealed {}

    pub trait SealedReducer {}

    const LIMB_BITS: usize = u64::BITS as usize;
    const FRACTION_BITS: usize = raw::FRACTION_BITS as usize;
    const FRACTION_MASK: u64 = (1 << FRACTION_BITS) - 1;
    const EXPONENT_MASK: u64 = (1 << raw::EXPONENT_BITS) - 1;
    const SIGN_SHIFT: usize = FRACTION_BITS + raw::EXPONENT_BITS as usize;

    /// The largest left shift of a significand: the top finite exponent, in units of the
    /// smallest subnormal.
    const MAX_SHIFT: usize = EXPONENT_MASK as usize - 2;

    /// Bits kept above the largest finite value before the integer can wrap.
    const HEADROOM_BITS: usize = 128;

    const LIMBS: usize = (MAX_SHIFT + FRACTION_BITS + 1 + HEADROOM_BITS).div_ceil(LIMB_BITS);

    const NAN: u8 = 1;
    const POSITIVE_INFINITY: u8 = 2;
    const NEGATIVE_INFINITY: u8 = 4;

    /// An exact sum of floats: a two's-complement integer in units of the smallest subnormal,
    /// little-endian limbs, plus flags for the non-finite inputs seen.
    ///
    /// Addition modulo a power of two and bitwise or are both commutative and associative,
    /// so the state after any sequence of `add` and `merge` depends only on the multiset of
    /// inputs, and `finish` is a function of the state.
    #[derive(Clone, Debug)]
    pub struct Accumulator {
        limbs: [u64; LIMBS],
        specials: u8,
    }

    impl Accumulator {
        pub(super) const ZERO: Self = Self { limbs: [0; LIMBS], specials: 0 };

        pub(super) fn add(&mut self, value: Raw) {
            let bits = raw::bits_u64(value);
            let negative = bits >> SIGN_SHIFT != 0;
            let exponent = (bits >> FRACTION_BITS) & EXPONENT_MASK;
            let fraction = bits & FRACTION_MASK;
            if exponent == EXPONENT_MASK {
                // The sign of a NaN is read above but never used.
                self.specials |= if fraction != 0 {
                    NAN
                } else if negative {
                    NEGATIVE_INFINITY
                } else {
                    POSITIVE_INFINITY
                };
                return;
            }
            // A subnormal has no implicit bit and the exponent of the smallest normal.
            let (significand, shift) = if exponent == 0 {
                (fraction, 0)
            } else {
                (fraction | (1 << FRACTION_BITS), exponent as usize - 1)
            };
            if significand == 0 {
                return;
            }
            let wide = u128::from(significand) << (shift % LIMB_BITS);
            let (low, high) = (wide as u64, (wide >> LIMB_BITS) as u64);
            let index = shift / LIMB_BITS;
            if negative {
                self.subtract_at(index, low, high);
            } else {
                self.add_at(index, low, high);
            }
        }

        /// Adds `low + high * 2^64` at limb `index`, wrapping at the top limb.
        fn add_at(&mut self, index: usize, low: u64, high: u64) {
            let (sum, mut carry) = self.limbs[index].overflowing_add(low);
            self.limbs[index] = sum;
            let mut addend = high;
            for limb in &mut self.limbs[index + 1..] {
                if addend == 0 && !carry {
                    break;
                }
                let (sum, carry_a) = limb.overflowing_add(addend);
                let (sum, carry_b) = sum.overflowing_add(u64::from(carry));
                *limb = sum;
                carry = carry_a || carry_b;
                addend = 0;
            }
        }

        /// Subtracts `low + high * 2^64` at limb `index`, wrapping at the top limb.
        fn subtract_at(&mut self, index: usize, low: u64, high: u64) {
            let (difference, mut borrow) = self.limbs[index].overflowing_sub(low);
            self.limbs[index] = difference;
            let mut subtrahend = high;
            for limb in &mut self.limbs[index + 1..] {
                if subtrahend == 0 && !borrow {
                    break;
                }
                let (difference, borrow_a) = limb.overflowing_sub(subtrahend);
                let (difference, borrow_b) = difference.overflowing_sub(u64::from(borrow));
                *limb = difference;
                borrow = borrow_a || borrow_b;
                subtrahend = 0;
            }
        }

        pub(super) fn merge(&mut self, other: &Self) {
            let mut carry = false;
            for (limb, other) in self.limbs.iter_mut().zip(&other.limbs) {
                let (sum, carry_a) = limb.overflowing_add(*other);
                let (sum, carry_b) = sum.overflowing_add(u64::from(carry));
                *limb = sum;
                carry = carry_a || carry_b;
            }
            self.specials |= other.specials;
        }

        /// The sum, rounded to nearest with ties to even.
        pub(super) fn finish(&self) -> Raw {
            let both_infinities = POSITIVE_INFINITY | NEGATIVE_INFINITY;
            if self.specials & NAN != 0 || self.specials & both_infinities == both_infinities {
                return Raw::from_bits(raw::CANONICAL_NAN);
            }
            if self.specials & POSITIVE_INFINITY != 0 {
                return Raw::INFINITY;
            }
            if self.specials & NEGATIVE_INFINITY != 0 {
                return Raw::NEG_INFINITY;
            }

            let negative = self.limbs[LIMBS - 1] >> (LIMB_BITS - 1) != 0;
            let magnitude = if negative { negated(&self.limbs) } else { self.limbs };
            let Some(top_limb) = magnitude.iter().rposition(|&limb| limb != 0) else {
                return 0.0;
            };
            // Index of the highest set bit; bit `i` is worth `2^i` smallest subnormals.
            let top = top_limb * LIMB_BITS
                + (LIMB_BITS - 1 - magnitude[top_limb].leading_zeros() as usize);

            let mut encoded = if top < FRACTION_BITS {
                // Below the smallest normal: the integer is the subnormal's fraction field.
                magnitude[0]
            } else {
                // Keep the top `FRACTION_BITS + 1` bits; the rest decides the rounding.
                let shift = top - FRACTION_BITS;
                let kept = bits_from(&magnitude, shift);
                let exponent = (shift + 1) as u64;
                let truncated = (exponent << FRACTION_BITS) | (kept & FRACTION_MASK);
                let round_up = shift > 0 && {
                    let half = bit(&magnitude, shift - 1);
                    let below_half = any_below(&magnitude, shift - 1);
                    half && (below_half || kept & 1 == 1)
                };
                // A carry out of the fraction increments the exponent, which is the correct
                // encoding of the next binade.
                truncated + u64::from(round_up)
            };
            let infinity = EXPONENT_MASK << FRACTION_BITS;
            if encoded > infinity {
                encoded = infinity;
            }
            if negative {
                encoded |= 1 << SIGN_SHIFT;
            }
            Raw::from_bits(encoded as raw::Bits)
        }
    }

    fn negated(limbs: &[u64; LIMBS]) -> [u64; LIMBS] {
        let mut out = [0; LIMBS];
        let mut carry = true;
        for (out, limb) in out.iter_mut().zip(limbs) {
            let (sum, overflow) = (!limb).overflowing_add(u64::from(carry));
            *out = sum;
            carry = overflow;
        }
        out
    }

    fn bit(limbs: &[u64; LIMBS], index: usize) -> bool {
        (limbs[index / LIMB_BITS] >> (index % LIMB_BITS)) & 1 == 1
    }

    /// Whether any bit below `index` is set.
    fn any_below(limbs: &[u64; LIMBS], index: usize) -> bool {
        let (whole, partial) = (index / LIMB_BITS, index % LIMB_BITS);
        limbs[..whole].iter().any(|&limb| limb != 0) || limbs[whole] & ((1 << partial) - 1) != 0
    }

    /// The 64 bits starting at bit `start`.
    fn bits_from(limbs: &[u64; LIMBS], start: usize) -> u64 {
        let (whole, partial) = (start / LIMB_BITS, start % LIMB_BITS);
        let low = limbs[whole] >> partial;
        if partial == 0 || whole + 1 == LIMBS {
            low
        } else {
            low | (limbs[whole + 1] << (LIMB_BITS - partial))
        }
    }
}
