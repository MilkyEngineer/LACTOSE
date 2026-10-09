//! The counter-based random number generator (architecture section 8.2).

use lactose_math::Scalar;

/// A stream of random numbers keyed by (region seed, tick, system key, stable entity key),
/// obtained from [`Ctx::rng`](crate::Ctx::rng). Each draw is a pure function of the key and
/// a counter the stream holds; no state is shared and no draw depends on which worker runs
/// it or on any other stream.
///
/// Every stream mixes the invoking entity's origin key, and a placeholder from the same
/// invocation is keyed by its spawn index, which is the same on every machine that runs the
/// same invocation.
#[derive(Clone, Debug)]
pub struct Rng {
    key: [u64; 4],
    counter: u64,
}

impl Rng {
    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        todo!()
    }

    /// The next 32 random bits.
    pub fn next_u32(&mut self) -> u32 {
        todo!()
    }

    /// A uniform integer in `0..bound`, without modulo bias. `bound` of zero is a panic.
    pub fn next_below(&mut self, bound: u32) -> u32 {
        todo!()
    }

    /// A uniform scalar in `[0, 1)`, through [`Scalar::unit_from_bits`].
    pub fn next_scalar(&mut self) -> Scalar {
        todo!()
    }
}
