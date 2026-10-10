//! The counter-based random number generator (architecture section 8.2).

use lactose_core::{StableKey, SystemKey, Tick, xxh64};
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

const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

/// The SplitMix64 finaliser: a bijection with full avalanche.
const fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The canonical bytes of a key, as the predicted ordinal and the RNG hash them: a tag
/// byte (1 `Net`, 2 `Predicted`, 3 `Placeholder`) and the key's words, little-endian.
pub(crate) fn key_bytes(key: StableKey, out: &mut Vec<u8>) {
    match key {
        StableKey::Net(id) => {
            out.push(1);
            out.extend_from_slice(&id.to_bits().to_le_bytes());
        }
        StableKey::Predicted(key) => {
            out.push(2);
            out.extend_from_slice(&key.player.0.to_le_bytes());
            out.extend_from_slice(&key.input_tick.0.to_le_bytes());
            out.extend_from_slice(&key.ordinal.to_le_bytes());
        }
        StableKey::Placeholder(index) => {
            out.push(3);
            out.extend_from_slice(&index.to_le_bytes());
        }
    }
}

impl Rng {
    /// The stream for `key` drawn by `invoker` (none for a per-region invocation) in
    /// `system` at `tick` in the region with `seed`.
    pub(crate) fn keyed(
        seed: u64,
        tick: Tick,
        system: SystemKey,
        invoker: Option<StableKey>,
        key: StableKey,
    ) -> Self {
        let mut bytes = Vec::with_capacity(32);
        match invoker {
            Some(invoker) => key_bytes(invoker, &mut bytes),
            None => bytes.push(0),
        }
        key_bytes(key, &mut bytes);
        Self { key: [seed, u64::from(tick.0), system.0, xxh64(&bytes, 0)], counter: 0 }
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let [seed, tick, system, entity] = self.key;
        let mut z = mix(seed ^ GOLDEN);
        z = mix(z ^ tick);
        z = mix(z ^ system);
        z = mix(z ^ entity);
        z = mix(z ^ self.counter.wrapping_mul(GOLDEN));
        self.counter = self.counter.wrapping_add(1);
        z
    }

    /// The next 32 random bits.
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// A uniform integer in `0..bound`, without modulo bias. `bound` of zero is a panic.
    pub fn next_below(&mut self, bound: u32) -> u32 {
        assert!(bound != 0, "`next_below(0)` has no value to return");
        // Lemire's multiply-and-reject: the low half of the product falls below the
        // threshold for exactly the draws that would bias the high half.
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let product = u64::from(self.next_u32()) * u64::from(bound);
            if (product as u32) >= threshold {
                return (product >> 32) as u32;
            }
        }
    }

    /// A uniform scalar in `[0, 1)`, through [`Scalar::unit_from_bits`].
    pub fn next_scalar(&mut self) -> Scalar {
        Scalar::unit_from_bits(self.next_u64())
    }
}

/// The layout-mode generator: seeded by SOUR's layout seed and the call site's counters.
/// What it decides (chunk order, row permutation, batch splits) is never a result.
pub(crate) struct LayoutRng(u64);

impl LayoutRng {
    pub(crate) fn new(seed: u64, a: u64, b: u64) -> Self {
        Self(mix(mix(seed ^ GOLDEN) ^ a) ^ b)
    }

    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(GOLDEN);
        mix(self.0)
    }

    /// A uniform index below `bound`, which is not zero. The tiny modulo bias is harmless
    /// here: nothing it decides is a result.
    pub(crate) fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    /// A permutation of `0..len`.
    pub(crate) fn permutation(&mut self, len: usize) -> Vec<usize> {
        let mut order: Vec<usize> = (0..len).collect();
        for i in (1..len).rev() {
            order.swap(i, self.below(i + 1));
        }
        order
    }
}
