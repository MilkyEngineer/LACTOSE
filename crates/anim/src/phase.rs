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

    /// The phase `numerator / denominator` of the way through the clip, rounded down to a
    /// unit; `END` at or past the whole clip. A zero `denominator` is a panic, a broken
    /// invariant of the caller's.
    #[must_use]
    pub const fn from_fraction(numerator: u32, denominator: u32) -> Self {
        assert!(denominator != 0, "a phase fraction has a nonzero denominator");
        let units = (numerator as u64 * Self::UNITS) / denominator as u64;
        if units >= Self::UNITS { Self::END } else { Self(units as u32) }
    }

    /// The phase as a scalar in `[0, 1)`: the units divided by `2^32`, one `lactose_math`
    /// division, the same bits on every target.
    #[must_use]
    pub fn fraction(self) -> Scalar {
        Scalar::from_u32(self.0) / Scalar::from_u64(Self::UNITS)
    }

    /// The units a state advances per tick: `2^32 * speed / duration_ticks` computed through
    /// `lactose_math`, truncated toward zero and clamped to `[0, u32::MAX]`. A `speed` that
    /// is not finite, negative or zero, or a zero `duration_ticks`, gives zero, so the state
    /// holds its phase. At speed one a clip of `d` ticks takes `d` ticks per cycle and wraps
    /// one tick late once every `2^32 / d` cycles because of the truncation; the rounding is
    /// part of this contract and never changes, since a change would desynchronise the
    /// phases of two builds.
    #[must_use]
    pub fn rate(duration_ticks: u32, speed: Scalar) -> u32 {
        todo!()
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
        todo!()
    }
}
