//! Identity keys, architecture section 7.
//!
//! `EntityId` is process-local and deliberately has no ordering. Everything that is stored,
//! ordered, sent or used to key randomness is a [`StableKey`].

use core::num::{NonZeroU32, NonZeroU64};

use bytemuck::{CheckedBitPattern, NoUninit, PodInOption, ZeroableInOption};

use crate::hash::xxh64;

/// A simulation tick index. Sixty ticks are one second of simulation time.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default, NoUninit)]
#[repr(transparent)]
pub struct Tick(pub u32);

/// Process-local entity handle: storage index plus generation.
///
/// Used for storage lookup only. It never reaches the wire or disk, and it implements no
/// ordering so it cannot become an ordering rule by accident.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub struct EntityId(NonZeroU64);

impl EntityId {
    /// Builds an ID from a storage index and a generation.
    #[must_use]
    pub const fn new(index: u32, generation: NonZeroU32) -> Self {
        let bits = ((generation.get() as u64) << 32) | index as u64;
        Self(NonZeroU64::new(bits).expect("generation is non-zero"))
    }

    /// The storage index.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0.get() as u32
    }

    /// The generation of the storage slot this ID was issued for.
    #[must_use]
    pub const fn generation(self) -> NonZeroU32 {
        NonZeroU32::new((self.0.get() >> 32) as u32).expect("generation is non-zero")
    }
}

/// Server-assigned network ID, which is also the persistent ID of every entity that has one.
///
/// The top bit separates cooked IDs (a hash the client derives without a message) from
/// runtime IDs (one server counter, persisted in the save).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, NoUninit, CheckedBitPattern)]
#[repr(transparent)]
pub struct NetId(NonZeroU64);

// SAFETY: `NetId` is `repr(transparent)` over `NonZeroU64`, so `Option<NetId>` has the layout
// of `u64` with zero as `None`.
unsafe impl ZeroableInOption for NetId {}
// SAFETY: as above; every non-zero bit pattern is a valid `NetId`.
unsafe impl PodInOption for NetId {}

impl NetId {
    const COOKED_BIT: u64 = 1 << 63;

    /// The ID of a cooked entity, from the hash of (authoring scene, authored placement GUID,
    /// child path). The top bit of `hash` is replaced by the cooked tag.
    #[must_use]
    pub const fn cooked(hash: u64) -> Self {
        Self(NonZeroU64::new(hash | Self::COOKED_BIT).expect("cooked bit is set"))
    }

    /// The ID of a runtime-spawned entity, from the server counter. Returns `None` once the
    /// counter reaches the cooked range.
    #[must_use]
    pub const fn runtime(counter: NonZeroU64) -> Option<Self> {
        if counter.get() & Self::COOKED_BIT == 0 { Some(Self(counter)) } else { None }
    }

    /// Rebuilds an ID from its wire or disk form. Zero is not an ID.
    #[must_use]
    pub const fn from_bits(bits: u64) -> Option<Self> {
        match NonZeroU64::new(bits) {
            Some(bits) => Some(Self(bits)),
            None => None,
        }
    }

    /// The wire and disk form.
    #[must_use]
    pub const fn to_bits(self) -> u64 {
        self.0.get()
    }

    /// Whether this ID belongs to a cooked entity.
    #[must_use]
    pub const fn is_cooked(self) -> bool {
        self.0.get() & Self::COOKED_BIT != 0
    }
}

/// Server-assigned player identity for one session.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, NoUninit)]
#[repr(transparent)]
pub struct PlayerId(pub u32);

/// Key of a spawn the server has not confirmed yet, and the permanent origin key of that
/// entity on both machines afterwards.
///
/// `ordinal` is the spawning command's position in that tick's total command order,
/// restricted to the player's input-driven sources.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct PredictionKey {
    /// The player whose input produced the spawn.
    pub player: PlayerId,
    /// The input tick that produced the spawn.
    pub input_tick: Tick,
    /// Position among that player's input-driven spawns in the tick.
    pub ordinal: u32,
}

/// The key used wherever an entity reference is stored, ordered or used to key randomness.
///
/// The derived ordering (network keys, then prediction keys, then placeholders) is the
/// engine's ordering rule for entities.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum StableKey {
    /// A server-assigned network ID.
    Net(NetId),
    /// A predicted spawn, or an entity whose origin was a predicted spawn.
    Predicted(PredictionKey),
    /// A spawn made earlier in the same command buffer, resolved when the buffer is applied.
    Placeholder(u32),
}

impl From<NetId> for StableKey {
    fn from(id: NetId) -> Self {
        Self::Net(id)
    }
}

impl From<PredictionKey> for StableKey {
    fn from(key: PredictionKey) -> Self {
        Self::Predicted(key)
    }
}

/// An optional [`StableKey`] in padding-free plain-data form, for component fields.
///
/// The all-zero value is the empty reference. Bytes arriving from disk or the network are
/// validated through [`CheckedBitPattern`], which accepts only canonical encodings.
#[derive(Clone, Copy, PartialEq, Eq, Hash, NoUninit)]
#[repr(C)]
pub struct EntityRef {
    tag: u32,
    words: [u32; 3],
}

const TAG_NONE: u32 = 0;
const TAG_NET: u32 = 1;
const TAG_PREDICTED: u32 = 2;
const TAG_PLACEHOLDER: u32 = 3;

impl EntityRef {
    /// The empty reference.
    pub const NONE: Self = Self { tag: TAG_NONE, words: [0; 3] };

    /// A reference to `key`.
    #[must_use]
    pub const fn new(key: StableKey) -> Self {
        match key {
            StableKey::Net(id) => {
                let bits = id.to_bits();
                Self { tag: TAG_NET, words: [bits as u32, (bits >> 32) as u32, 0] }
            }
            StableKey::Predicted(key) => {
                Self { tag: TAG_PREDICTED, words: [key.player.0, key.input_tick.0, key.ordinal] }
            }
            StableKey::Placeholder(index) => Self { tag: TAG_PLACEHOLDER, words: [index, 0, 0] },
        }
    }

    /// The referenced key, or `None` for the empty reference.
    #[must_use]
    pub const fn get(self) -> Option<StableKey> {
        let [a, b, c] = self.words;
        match self.tag {
            TAG_NET => match NetId::from_bits(a as u64 | (b as u64) << 32) {
                Some(id) => Some(StableKey::Net(id)),
                None => None,
            },
            TAG_PREDICTED => Some(StableKey::Predicted(PredictionKey {
                player: PlayerId(a),
                input_tick: Tick(b),
                ordinal: c,
            })),
            TAG_PLACEHOLDER => Some(StableKey::Placeholder(a)),
            _ => None,
        }
    }

    /// Whether this is the empty reference.
    #[must_use]
    pub const fn is_none(self) -> bool {
        self.get().is_none()
    }
}

impl Default for EntityRef {
    fn default() -> Self {
        Self::NONE
    }
}

impl core::fmt::Debug for EntityRef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("EntityRef").field(&self.get()).finish()
    }
}

impl From<StableKey> for EntityRef {
    fn from(key: StableKey) -> Self {
        Self::new(key)
    }
}

impl From<Option<StableKey>> for EntityRef {
    fn from(key: Option<StableKey>) -> Self {
        key.map_or(Self::NONE, Self::new)
    }
}

// SAFETY: `EntityRef` is four `u32` words with no padding, so every bit pattern of
// `[u32; 4]` is a memory-valid `EntityRef`. The check below only narrows that to the
// canonical encodings the constructors produce.
unsafe impl CheckedBitPattern for EntityRef {
    type Bits = [u32; 4];

    fn is_valid_bit_pattern(bits: &[u32; 4]) -> bool {
        let [tag, a, b, c] = *bits;
        match tag {
            TAG_NONE => a == 0 && b == 0 && c == 0,
            TAG_NET => (a != 0 || b != 0) && c == 0,
            TAG_PREDICTED => true,
            TAG_PLACEHOLDER => b == 0 && c == 0,
            _ => false,
        }
    }
}

/// Build-time key of a system: xxh64 of its declared name. Breaks command-order ties and
/// selects the RNG stream.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
#[repr(transparent)]
pub struct SystemKey(pub u64);

impl SystemKey {
    /// The key for a declared system name such as `"game_sim::apply_gravity"`.
    #[must_use]
    pub const fn from_name(name: &str) -> Self {
        Self(xxh64(name.as_bytes(), 0))
    }
}

/// Build, wire and disk key of a component type: xxh64 of its declared name.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, NoUninit)]
#[repr(transparent)]
pub struct ComponentKey(pub u64);

impl ComponentKey {
    /// The key for a declared component name.
    #[must_use]
    pub const fn from_name(name: &str) -> Self {
        Self(xxh64(name.as_bytes(), 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(bits: u64) -> NetId {
        NetId::from_bits(bits).unwrap()
    }

    #[test]
    fn entity_id_round_trips_and_is_free_in_option() {
        let id = EntityId::new(7, NonZeroU32::new(3).unwrap());
        assert_eq!(id.index(), 7);
        assert_eq!(id.generation().get(), 3);
        assert_eq!(size_of::<Option<EntityId>>(), size_of::<u64>());
    }

    #[test]
    fn net_id_tags() {
        assert!(NetId::cooked(0).is_cooked());
        assert!(NetId::cooked(5).is_cooked());
        let runtime = NetId::runtime(NonZeroU64::new(5).unwrap()).unwrap();
        assert!(!runtime.is_cooked());
        assert_eq!(runtime.to_bits(), 5);
        assert!(NetId::runtime(NonZeroU64::new(1 << 63).unwrap()).is_none());
        assert!(NetId::from_bits(0).is_none());
    }

    #[test]
    fn option_net_id_is_plain_data() {
        let none: Option<NetId> = None;
        assert_eq!(bytemuck::bytes_of(&none), &[0u8; 8]);
        let some = Some(net(0x0102));
        assert_eq!(bytemuck::cast::<Option<NetId>, u64>(some), 0x0102);
        assert!(bytemuck::checked::try_from_bytes::<NetId>(&[0u8; 8]).is_err());
    }

    #[test]
    fn stable_key_order_is_net_then_predicted_then_placeholder() {
        let predicted = |player, tick, ordinal| {
            StableKey::Predicted(PredictionKey {
                player: PlayerId(player),
                input_tick: Tick(tick),
                ordinal,
            })
        };
        let mut keys = [
            StableKey::Placeholder(0),
            predicted(1, 9, 0),
            StableKey::Net(net(9)),
            predicted(1, 2, 5),
            predicted(0, 100, 0),
            StableKey::Net(net(2)),
        ];
        keys.sort();
        assert_eq!(
            keys,
            [
                StableKey::Net(net(2)),
                StableKey::Net(net(9)),
                predicted(0, 100, 0),
                predicted(1, 2, 5),
                predicted(1, 9, 0),
                StableKey::Placeholder(0),
            ]
        );
    }

    #[test]
    fn entity_ref_round_trips_every_arm() {
        let keys = [
            StableKey::Net(net(u64::MAX)),
            StableKey::Net(NetId::cooked(0)),
            StableKey::Predicted(PredictionKey {
                player: PlayerId(4),
                input_tick: Tick(77),
                ordinal: 2,
            }),
            StableKey::Placeholder(12),
        ];
        for key in keys {
            let reference = EntityRef::new(key);
            assert_eq!(reference.get(), Some(key));
            let bits: [u32; 4] = bytemuck::cast(reference);
            assert!(EntityRef::is_valid_bit_pattern(&bits));
        }
        assert_eq!(EntityRef::NONE.get(), None);
        assert_eq!(bytemuck::bytes_of(&EntityRef::NONE), &[0u8; 16]);
        assert_eq!(EntityRef::from(None), EntityRef::NONE);
    }

    #[test]
    fn entity_ref_rejects_non_canonical_bytes() {
        for bits in [[0, 1, 0, 0], [1, 0, 0, 0], [1, 1, 0, 1], [3, 0, 1, 0], [4, 0, 0, 0]] {
            assert!(!EntityRef::is_valid_bit_pattern(&bits), "{bits:?}");
            assert!(bytemuck::checked::try_cast::<[u32; 4], EntityRef>(bits).is_err());
        }
    }

    #[test]
    fn name_keys_are_const_and_distinct() {
        const GRAVITY: SystemKey = SystemKey::from_name("game_sim::apply_gravity");
        assert_ne!(GRAVITY, SystemKey::from_name("game_sim::total_mass"));
        assert_eq!(ComponentKey::from_name("velocity"), ComponentKey::from_name("velocity"));
    }
}
