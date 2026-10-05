//! A hash map whose behaviour is identical on every run, thread count and host.

use core::borrow::Borrow;
use core::hash::{BuildHasherDefault, Hash, Hasher};

/// The fixed, unseeded hasher behind [`FixedHashMap`].
///
/// Hash values never leave the map, so they carry no determinism obligation of their own;
/// the hasher is fixed so that map behaviour (capacity growth, probe lengths) repeats.
#[derive(Clone, Copy, Default, Debug)]
pub struct FixedHasher {
    state: u64,
}

impl FixedHasher {
    const fn add(&mut self, word: u64) {
        self.state = (self.state.rotate_left(5) ^ word).wrapping_mul(0x517C_C1B7_2722_0A95);
    }
}

impl Hasher for FixedHasher {
    fn write(&mut self, bytes: &[u8]) {
        let (chunks, rest) = bytes.as_chunks::<8>();
        for chunk in chunks {
            self.add(u64::from_le_bytes(*chunk));
        }
        if !rest.is_empty() {
            let mut tail = [0u8; 8];
            tail[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(tail));
        }
        self.add(bytes.len() as u64);
    }

    fn write_u8(&mut self, value: u8) {
        self.add(value as u64);
    }

    fn write_u16(&mut self, value: u16) {
        self.add(value as u64);
    }

    fn write_u32(&mut self, value: u32) {
        self.add(value as u64);
    }

    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }

    fn finish(&self) -> u64 {
        let mut hash = self.state;
        hash ^= hash >> 33;
        hash = hash.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
        hash ^= hash >> 33;
        hash = hash.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
        hash ^ (hash >> 33)
    }
}

#[expect(clippy::disallowed_types, reason = "this is the wrapper that makes HashMap deterministic")]
type Inner<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<FixedHasher>>;

/// A hash map with a fixed hasher and no way to observe iteration order.
///
/// The only traversal is [`FixedHashMap::sorted`], which visits entries in key order.
#[derive(Clone)]
pub struct FixedHashMap<K, V> {
    inner: Inner<K, V>,
}

impl<K, V> Default for FixedHashMap<K, V> {
    fn default() -> Self {
        Self { inner: Inner::default() }
    }
}

impl<K, V> FixedHashMap<K, V> {
    /// An empty map. Does not allocate.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Whether the map has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Removes every entry, keeping the allocation.
    pub fn clear(&mut self) {
        self.inner.clear();
    }
}

impl<K: Eq + Hash, V> FixedHashMap<K, V> {
    /// An empty map with room for `capacity` entries.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self { inner: Inner::with_capacity_and_hasher(capacity, BuildHasherDefault::default()) }
    }

    /// The value stored for `key`.
    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.inner.get(key)
    }

    /// Mutable access to the value stored for `key`.
    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.inner.get_mut(key)
    }

    /// Whether `key` has a value.
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.inner.contains_key(key)
    }

    /// Stores `value` for `key`, returning the value it replaced.
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        self.inner.insert(key, value)
    }

    /// Removes and returns the value stored for `key`.
    pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.inner.remove(key)
    }

    /// The value stored for `key`, inserting the result of `make` first if there is none.
    pub fn get_or_insert_with(&mut self, key: K, make: impl FnOnce() -> V) -> &mut V {
        self.inner.entry(key).or_insert_with(make)
    }
}

impl<K: Ord, V> FixedHashMap<K, V> {
    /// Every entry, in ascending key order. Costs a sort; keep it off per-entity paths.
    #[must_use]
    pub fn sorted(&self) -> Vec<(&K, &V)> {
        let mut entries: Vec<_> = self.inner.iter().collect();
        entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
        entries
    }
}

impl<K: core::fmt::Debug + Ord, V: core::fmt::Debug> core::fmt::Debug for FixedHashMap<K, V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_map().entries(self.sorted()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_operations() {
        let mut map = FixedHashMap::new();
        assert!(map.is_empty());
        assert_eq!(map.insert("b", 2), None);
        assert_eq!(map.insert("a", 1), None);
        assert_eq!(map.insert("b", 3), Some(2));
        assert_eq!(map.get("b"), Some(&3));
        *map.get_mut("a").unwrap() += 10;
        assert_eq!(*map.get_or_insert_with("c", || 7), 7);
        assert_eq!(map.remove("a"), Some(11));
        assert!(!map.contains_key("a"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn sorted_is_independent_of_insertion_order() {
        let mut forward = FixedHashMap::new();
        let mut backward = FixedHashMap::new();
        for i in 0..1000u32 {
            forward.insert(i, i * 2);
            backward.insert(999 - i, (999 - i) * 2);
        }
        assert_eq!(forward.sorted(), backward.sorted());
        assert!(forward.sorted().windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn hasher_is_unseeded() {
        let hash = |value: &str| {
            let mut hasher = FixedHasher::default();
            value.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash("region"), hash("region"));
        assert_ne!(hash("region"), hash("regioN"));
    }
}
