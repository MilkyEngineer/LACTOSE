//! The name-derived keys of authored animation data: the xxh64 of a name, computed at
//! compile time where the name is a literal, so a game names a bone, a state or an event
//! without a string at run time and the cooker records the same key.

use lactose_core::xxh64;

/// The key of a bone of a [`Skeleton`](crate::Skeleton): the xxh64 of its authored name,
/// with seed zero. The cooker checks the bones of one skeleton for collisions.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct BoneKey(pub u64);

impl BoneKey {
    /// The key of the bone named `name`.
    #[must_use]
    pub const fn from_name(name: &str) -> Self {
        Self(xxh64(name.as_bytes(), 0))
    }
}

/// The key of a state of a [`StateMachine`](crate::StateMachine): the xxh64 of its authored
/// name, with seed zero. The cooker checks the states of one machine for collisions.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct StateKey(pub u64);

impl StateKey {
    /// The key of the state named `name`.
    #[must_use]
    pub const fn from_name(name: &str) -> Self {
        Self(xxh64(name.as_bytes(), 0))
    }
}

/// The key of an authored clip event ([`ClipEvent`](crate::ClipEvent)): the xxh64 of its
/// authored name, with seed zero. A game declares the keys it reacts to as constants.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct AnimEventKey(pub u64);

impl AnimEventKey {
    /// The key of the event named `name`.
    #[must_use]
    pub const fn from_name(name: &str) -> Self {
        Self(xxh64(name.as_bytes(), 0))
    }
}
