//! The phase of a clip as the simulation counts it: a `u32` fraction of the clip, advanced
//! by an integer rate per tick, so two machines agree on it without a float and a mark
//! authored at a phase is crossed exactly once.

use lactose_math::Scalar;
use lactose_reflect::value;

/// Where in its clip a state is: a fraction of the clip in units of `1 / 2^32`. Zero is the
/// clip's start, [`HALF`](Self::HALF) its middle and [`END`](Self::END), `u32::MAX`, the
/// last instant of a one-shot clip. A looping clip's phase wraps and a one-shot's saturates
/// at `END` ([`advanced`](Self::advanced)); a state is finished once its phase holds `END`.
///
/// Integer on purpose (architecture section 8.2): a phase compares, replicates and hashes
/// exactly; the rate per tick is an integer fixed by the clip's length and the state's
/// speed ([`rate`](Self::rate)); and sampling a clip at a phase is integer arithmetic up to
/// the fraction between two frames, which is one `lactose_math` division. Section 6.1's
/// `anim.phase_crossed(0.5)` is `anim.phase_crossed(Phase::HALF)` here.
#[value]
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct Phase(#[field(id = 1)] pub u32);

impl Phase {
    /// The start of the clip.
    pub const ZERO: Self = Self(0);

    /// The middle of the clip.
    pub const HALF: Self = Self(1 << 31);

    /// The last instant of a one-shot clip, where its phase saturates.
    pub const END: Self = Self(u32::MAX);

    /// The number of phase units in one clip, `2^32`.
    pub const UNITS: u64 = 1 << 32;

    /// The fraction bits of the fixed-point speed [`rate`](Self::rate) computes with: a
    /// speed is taken to Q16.16 once, so both scalar widths give one rate.
    pub const SPEED_FRACTION_BITS: u32 = 16;

    /// The phase `numerator / denominator` of the way through the clip, rounded down to a
    /// unit; `END` at or past the whole clip. A zero `denominator` is a panic, a broken
    /// invariant of the caller's.
    #[must_use]
    pub const fn from_fraction(numerator: u32, denominator: u32) -> Self {
        assert!(denominator != 0, "a phase fraction has a nonzero denominator");
        let units = (numerator as u64 * Self::UNITS) / denominator as u64;
        if units >= Self::UNITS { Self::END } else { Self(units as u32) }
    }

    /// The phase as a scalar in `[0, 1]`: the units divided by `2^32`, one `lactose_math`
    /// division, the same bits on every target. Below one for every phase under an
    /// eight-byte scalar; `END` rounds to one under a four-byte scalar.
    #[must_use]
    pub fn fraction(self) -> Scalar {
        Scalar::from_u32(self.0) / Scalar::from_u64(Self::UNITS)
    }

    /// The units a state advances per tick, in integers: `speed` is taken to Q16.16 once
    /// (`speed * 2^16` through `Scalar::to_i64`, toward zero, clamped to `[0, 2^32 - 1]`),
    /// and the rate is `ceil(2^32 * q / (duration_ticks * 2^16))` in 128-bit arithmetic,
    /// clamped to `u32::MAX`. Zero for a speed that is not finite, negative or zero, or for
    /// a zero `duration_ticks`, so the state holds its phase.
    ///
    /// What follows from the ceiling, and is part of this contract: a one-shot of `d`
    /// ticks at speed one holds `END` on tick `d` of its state; a mark at
    /// `from_fraction(f, d)` is crossed on tick `f`; a loop of `d` ticks wraps on tick `d`,
    /// and wraps one tick early once every `rate / (d * rate - 2^32)` cycles (about 1.6
    /// million cycles at `d = 60`), never within a session. Both scalar widths give the
    /// same rate for any speed both can hold, since the only float operation is a product
    /// by a power of two. A speed of `duration_ticks` cycles per tick or more is clamped to
    /// `u32::MAX`: a one-shot then reaches `END` in one tick and a loop's phase moves back
    /// one unit a tick, which is meaningless; a state machine's speed is authored below
    /// its clips' durations, and a `speed_param` that could exceed it is the game's to
    /// bound, since the clip is another asset and neither this function nor the machine's
    /// validation can see its duration.
    #[must_use]
    pub fn rate(duration_ticks: u32, speed: Scalar) -> u32 {
        if duration_ticks == 0 || !speed.is_finite() || speed <= Scalar::ZERO {
            return 0;
        }
        let q = (speed * Scalar::from_u32(1 << Self::SPEED_FRACTION_BITS)).to_i64();
        let q = u128::try_from(q.clamp(0, i64::from(u32::MAX))).unwrap_or(0);
        let divisor = u128::from(duration_ticks) << Self::SPEED_FRACTION_BITS;
        let units = (u128::from(Self::UNITS) * q).div_ceil(divisor);
        u32::try_from(units).unwrap_or(u32::MAX)
    }

    /// This phase `rate` units on: wrapping around the clip when `looping`, saturating at
    /// `END` otherwise.
    #[must_use]
    pub const fn advanced(self, rate: u32, looping: bool) -> Self {
        if looping { Self(self.0.wrapping_add(rate)) } else { Self(self.0.saturating_add(rate)) }
    }

    /// Whether an advance from `from` to `to` passed `mark`: `mark` in `(from, to]` when
    /// `to >= from`, and in `(from, END] ∪ [ZERO, to]` when `to < from`, which is a looping
    /// clip that wrapped this tick (an advance wraps at most once, since a rate is below
    /// `2^32`). Equal `from` and `to` passed nothing.
    #[must_use]
    pub const fn passed(from: Self, to: Self, mark: Self) -> bool {
        if to.0 >= from.0 {
            from.0 < mark.0 && mark.0 <= to.0
        } else {
            mark.0 > from.0 || mark.0 <= to.0
        }
    }
}
