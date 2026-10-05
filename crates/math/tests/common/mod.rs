//! A small seeded generator shared by the property tests. Integer arithmetic only.

#![allow(dead_code, reason = "each test binary uses a different subset")]

use lactose_math::Scalar;

/// SplitMix64.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    /// A finite scalar with a random sign and fraction and a binary exponent in
    /// `[low, high]`.
    pub fn with_exponent(&mut self, low: i64, high: i64) -> Scalar {
        let exponent = (1023 + low + self.below((high - low + 1) as usize) as i64) as u64;
        let bits = (self.next() & 0x800f_ffff_ffff_ffff) | (exponent << 52);
        Scalar::from_canonical_bits(bits)
    }

    /// Any scalar: often an ordinary value, sometimes a zero of either sign, an infinity, a
    /// NaN, a subnormal or an arbitrary bit pattern.
    pub fn any(&mut self) -> Scalar {
        match self.below(12) {
            0 => Scalar::ZERO,
            1 => -Scalar::ZERO,
            2 => Scalar::INFINITY,
            3 => Scalar::NEG_INFINITY,
            4 => Scalar::NAN,
            5 => Scalar::from_canonical_bits(self.next() & 0x800f_ffff_ffff_ffff),
            6 => Scalar::from_f64(f64::from_bits(self.next())),
            7 | 8 => self.with_exponent(-1000, 1000),
            _ => self.with_exponent(-8, 8),
        }
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }
}
