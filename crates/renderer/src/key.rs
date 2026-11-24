//! A `StableKey` as two words, for the atomics beside the ring's rows (`README.md`,
//! "Concurrency"), and as one number whose order is the key's.
//!
//! The key's variant is the top of the high word, so that the number orders as
//! `StableKey`'s derived ordering does: network keys, then prediction keys, then
//! placeholders, each by its fields in order.

use lactose_core::{NetId, PlayerId, PredictionKey, StableKey, Tick};

/// A `StableKey` as 98 bits: the variant (bits 96 and 97) and its fields below it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub(crate) struct KeyBits(u128);

const NET: u128 = 0;
const PREDICTED: u128 = 1;
const PLACEHOLDER: u128 = 2;

impl KeyBits {
    /// The number of `key`.
    pub(crate) const fn of(key: StableKey) -> Self {
        let (variant, fields) = match key {
            StableKey::Net(id) => (NET, id.to_bits() as u128),
            StableKey::Predicted(key) => (
                PREDICTED,
                (key.player.0 as u128) << 64
                    | (key.input_tick.0 as u128) << 32
                    | key.ordinal as u128,
            ),
            StableKey::Placeholder(index) => (PLACEHOLDER, index as u128),
        };
        Self(variant << 96 | fields)
    }

    /// The number as the two words a shadow entry holds, low first.
    pub(crate) const fn words(self) -> [u64; 2] {
        [self.0 as u64, (self.0 >> 64) as u64]
    }

    /// The number of the two words [`words`](Self::words) gave.
    pub(crate) const fn from_words(words: [u64; 2]) -> Self {
        Self(words[0] as u128 | (words[1] as u128) << 64)
    }

    /// The key, or `None` for a number no key gives: a shadow entry's words are read only
    /// where its generation says a writer stored them, so this is a broken invariant.
    pub(crate) const fn key(self) -> Option<StableKey> {
        let fields = self.0 & ((1 << 96) - 1);
        match self.0 >> 96 {
            NET => match NetId::from_bits(fields as u64) {
                Some(id) if fields >> 64 == 0 => Some(StableKey::Net(id)),
                _ => None,
            },
            PREDICTED => Some(StableKey::Predicted(PredictionKey {
                player: PlayerId((fields >> 64) as u32),
                input_tick: Tick((fields >> 32) as u32),
                ordinal: fields as u32,
            })),
            PLACEHOLDER if fields >> 32 == 0 => Some(StableKey::Placeholder(fields as u32)),
            _ => None,
        }
    }
}

#[cfg(all(test, not(feature = "loom")))]
mod tests {
    use super::*;

    fn keys() -> Vec<StableKey> {
        let net = |bits| StableKey::Net(NetId::from_bits(bits).unwrap());
        let predicted = |player, tick, ordinal| {
            StableKey::Predicted(PredictionKey {
                player: PlayerId(player),
                input_tick: Tick(tick),
                ordinal,
            })
        };
        vec![
            net(1),
            net(2),
            net(u64::MAX),
            net(1 << 63 | 5),
            predicted(0, 0, 0),
            predicted(0, 0, u32::MAX),
            predicted(0, 1, 0),
            predicted(1, 0, 0),
            predicted(u32::MAX, u32::MAX, u32::MAX),
            StableKey::Placeholder(0),
            StableKey::Placeholder(u32::MAX),
        ]
    }

    #[test]
    fn a_key_comes_back_from_its_words() {
        for key in keys() {
            let bits = KeyBits::of(key);
            assert_eq!(KeyBits::from_words(bits.words()), bits);
            assert_eq!(bits.key(), Some(key));
        }
    }

    #[test]
    fn the_numbers_order_as_the_keys() {
        let keys = keys();
        for a in &keys {
            for b in &keys {
                assert_eq!(KeyBits::of(*a).cmp(&KeyBits::of(*b)), a.cmp(b), "{a:?} {b:?}");
            }
        }
    }
}
