//! Asset versions, the per-region version table and the per-region asset set.

use core::num::NonZeroU64;
use std::collections::{BTreeMap, BTreeSet};

use crate::id::AssetId;

/// One published version of an asset: a hash of its cooked form.
///
/// The value is computed by [`CookedHeader::version_of`](crate::CookedHeader::version_of)
/// from the cooked header and bytes, so `(AssetId, AssetVersion)` names the same content on
/// every machine and after every cook, and two cooks can never publish different content
/// under one version. Versions are identities and have no order: which version of an asset
/// is the current one is a fact about a store's pack index, never about the number.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AssetVersion(NonZeroU64);

impl AssetVersion {
    /// The version for a content hash. Zero is reserved for "no version" in plain data,
    /// so a hash of zero becomes one.
    pub(crate) const fn from_hash(hash: u64) -> Self {
        match NonZeroU64::new(hash) {
            Some(bits) => Self(bits),
            None => Self(NonZeroU64::MIN),
        }
    }

    /// Rebuilds a version from its wire or disk form. Zero is not a version.
    #[must_use]
    pub const fn from_bits(bits: u64) -> Option<Self> {
        match NonZeroU64::new(bits) {
            Some(bits) => Some(Self(bits)),
            None => None,
        }
    }

    /// The wire and disk form, never zero.
    #[must_use]
    pub const fn to_bits(self) -> u64 {
        self.0.get()
    }
}

impl core::fmt::Debug for AssetVersion {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "AssetVersion({:#018x})", self.0)
    }
}

/// The versions one region sees: a map from asset to published version.
///
/// A region owns one and replaces it only at the input sync point (architecture sections 6
/// and 8.2). Iteration is in ascending [`AssetId`] order and nothing else is observable, so
/// equality, `Hash` and a walk for the world hash depend on the contents alone.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct VersionTable {
    versions: BTreeMap<AssetId, AssetVersion>,
}

impl VersionTable {
    /// An empty table.
    #[must_use]
    pub const fn new() -> Self {
        Self { versions: BTreeMap::new() }
    }

    /// The number of assets in the table.
    #[must_use]
    pub fn len(&self) -> usize {
        self.versions.len()
    }

    /// Whether the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.versions.is_empty()
    }

    /// The version this table gives `id`.
    #[must_use]
    pub fn get(&self, id: AssetId) -> Option<AssetVersion> {
        self.versions.get(&id).copied()
    }

    /// Gives `id` the version `version`, returning the version it replaced.
    /// [`AssetId::NONE`] names no asset and is ignored.
    pub fn set(&mut self, id: AssetId, version: AssetVersion) -> Option<AssetVersion> {
        if id.is_none() {
            return None;
        }
        self.versions.insert(id, version)
    }

    /// Removes `id`, returning the version it had.
    pub fn remove(&mut self, id: AssetId) -> Option<AssetVersion> {
        self.versions.remove(&id)
    }

    /// Every entry, in ascending [`AssetId`] order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (AssetId, AssetVersion)> + '_ {
        self.versions.iter().map(|(&id, &version)| (id, version))
    }
}

impl FromIterator<(AssetId, AssetVersion)> for VersionTable {
    fn from_iter<I: IntoIterator<Item = (AssetId, AssetVersion)>>(entries: I) -> Self {
        let mut table = Self::new();
        for (id, version) in entries {
            table.set(id, version);
        }
        table
    }
}

/// The assets a region declares: those of its cooked baseline plus those of any migrated
/// resident entity (architecture section 14).
///
/// A set need not list dependencies:
/// [`AssetCatalog::resolve`](crate::AssetCatalog::resolve) closes over them. Iteration is
/// in ascending [`AssetId`] order.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct AssetSet {
    ids: BTreeSet<AssetId>,
}

impl AssetSet {
    /// An empty set.
    #[must_use]
    pub const fn new() -> Self {
        Self { ids: BTreeSet::new() }
    }

    /// The number of assets in the set.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Whether the set is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Whether the set contains `id`.
    #[must_use]
    pub fn contains(&self, id: AssetId) -> bool {
        self.ids.contains(&id)
    }

    /// Adds `id`, returning whether it was absent. [`AssetId::NONE`] names no asset and is
    /// ignored, so the IDs of reference fields can be added without checking each one.
    pub fn insert(&mut self, id: AssetId) -> bool {
        !id.is_none() && self.ids.insert(id)
    }

    /// Removes `id`, returning whether it was present.
    pub fn remove(&mut self, id: AssetId) -> bool {
        self.ids.remove(&id)
    }

    /// Every asset, in ascending [`AssetId`] order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = AssetId> + '_ {
        self.ids.iter().copied()
    }
}

impl FromIterator<AssetId> for AssetSet {
    fn from_iter<I: IntoIterator<Item = AssetId>>(ids: I) -> Self {
        let mut set = Self::new();
        set.extend(ids);
        set
    }
}

impl Extend<AssetId> for AssetSet {
    fn extend<I: IntoIterator<Item = AssetId>>(&mut self, ids: I) {
        for id in ids {
            self.insert(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use core::hash::{Hash, Hasher};

    use lactose_core::FixedHasher;

    use super::*;

    fn id(bits: u64) -> AssetId {
        AssetId::from_bits(bits)
    }

    fn version(bits: u64) -> AssetVersion {
        AssetVersion::from_bits(bits).unwrap()
    }

    fn hash_of(value: &impl Hash) -> u64 {
        let mut hasher = FixedHasher::default();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn version_reserves_zero() {
        assert_eq!(AssetVersion::from_bits(0), None);
        assert_eq!(version(7).to_bits(), 7);
        assert_eq!(AssetVersion::from_hash(0).to_bits(), 1);
        assert_eq!(AssetVersion::from_hash(9), version(9));
        assert_eq!(size_of::<Option<AssetVersion>>(), size_of::<u64>());
    }

    #[test]
    fn table_is_independent_of_insertion_order() {
        let forward: VersionTable = (1..200).map(|i| (id(i), version(i))).collect();
        let backward: VersionTable = (1..200).rev().map(|i| (id(i), version(i))).collect();
        assert_eq!(forward, backward);
        assert_eq!(hash_of(&forward), hash_of(&backward));
        assert!(forward.iter().map(|(id, _)| id).is_sorted());
        assert_eq!(forward.iter().len(), 199);
    }

    #[test]
    fn table_set_get_remove() {
        let mut table = VersionTable::new();
        assert!(table.is_empty());
        assert_eq!(table.set(id(3), version(1)), None);
        assert_eq!(table.set(id(3), version(2)), Some(version(1)));
        assert_eq!(table.get(id(3)), Some(version(2)));
        assert_eq!(table.get(id(4)), None);
        let other: VersionTable = [(id(3), version(1))].into_iter().collect();
        assert_ne!(table, other);
        assert_eq!(table.remove(id(3)), Some(version(2)));
        assert_eq!(table.len(), 0);
    }

    #[test]
    fn none_is_never_stored() {
        let mut table = VersionTable::new();
        assert_eq!(table.set(AssetId::NONE, version(1)), None);
        assert!(table.is_empty());
        let mut set = AssetSet::new();
        assert!(!set.insert(AssetId::NONE));
        assert!(set.is_empty());
    }

    #[test]
    fn set_is_sorted_and_deduplicated() {
        let mut set: AssetSet = [id(9), id(2), id(9), id(5)].into_iter().collect();
        assert_eq!(set.iter().collect::<Vec<_>>(), [id(2), id(5), id(9)]);
        assert!(set.contains(id(5)));
        assert!(!set.insert(id(5)));
        assert!(set.remove(id(5)));
        assert!(!set.remove(id(5)));
        assert_eq!(set.len(), 2);
        let same: AssetSet = [id(2), id(9)].into_iter().collect();
        assert_eq!(set, same);
        assert_eq!(hash_of(&set), hash_of(&same));
    }
}
