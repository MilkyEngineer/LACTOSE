//! Typed generational handles and the table that issues them.

use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::num::NonZeroU32;

/// A typed index plus generation into a [`HandleTable`] or a table with the same contract.
///
/// A handle is plain data whatever `T` is: it is `Copy`, `Send` and `Sync`, and a stale
/// handle fails lookup instead of reaching a reused slot.
pub struct Handle<T> {
    index: u32,
    generation: NonZeroU32,
    marker: PhantomData<fn() -> T>,
}

impl<T> Handle<T> {
    /// Builds a handle from its parts. Tables other than [`HandleTable`] use this.
    #[must_use]
    pub const fn from_parts(index: u32, generation: NonZeroU32) -> Self {
        Self { index, generation, marker: PhantomData }
    }

    /// The slot index.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.index
    }

    /// The generation of the slot this handle was issued for.
    #[must_use]
    pub const fn generation(self) -> NonZeroU32 {
        self.generation
    }
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Handle<T> {}

impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}

impl<T> Eq for Handle<T> {}

impl<T> Hash for Handle<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.generation.hash(state);
    }
}

impl<T> core::fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Handle({}v{})", self.index, self.generation)
    }
}

const FIRST_GENERATION: NonZeroU32 = NonZeroU32::MIN;

struct Slot<T> {
    generation: NonZeroU32,
    value: Option<T>,
}

/// A generational slot table. Slot reuse order depends only on the sequence of calls.
pub struct HandleTable<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
    len: u32,
}

impl<T> Default for HandleTable<T> {
    fn default() -> Self {
        Self { slots: Vec::new(), free: Vec::new(), len: 0 }
    }
}

impl<T> HandleTable<T> {
    /// An empty table. Does not allocate.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of live values.
    #[must_use]
    pub fn len(&self) -> u32 {
        self.len
    }

    /// Whether the table holds no live value.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Stores `value` and returns its handle.
    ///
    /// # Panics
    ///
    /// Panics if the table would exceed `u32::MAX` slots.
    pub fn insert(&mut self, value: T) -> Handle<T> {
        self.len += 1;
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.value = Some(value);
            return Handle::from_parts(index, slot.generation);
        }
        let index = u32::try_from(self.slots.len()).expect("handle table is full");
        self.slots.push(Slot { generation: FIRST_GENERATION, value: Some(value) });
        Handle::from_parts(index, FIRST_GENERATION)
    }

    /// The value `handle` refers to, or `None` if it was removed.
    #[must_use]
    pub fn get(&self, handle: Handle<T>) -> Option<&T> {
        let slot = self.slots.get(handle.index as usize)?;
        if slot.generation == handle.generation { slot.value.as_ref() } else { None }
    }

    /// Mutable access to the value `handle` refers to.
    pub fn get_mut(&mut self, handle: Handle<T>) -> Option<&mut T> {
        let slot = self.slots.get_mut(handle.index as usize)?;
        if slot.generation == handle.generation { slot.value.as_mut() } else { None }
    }

    /// Removes and returns the value `handle` refers to. Every copy of the handle goes stale.
    pub fn remove(&mut self, handle: Handle<T>) -> Option<T> {
        let slot = self.slots.get_mut(handle.index as usize)?;
        if slot.generation != handle.generation {
            return None;
        }
        let value = slot.value.take()?;
        self.len -= 1;
        // A slot whose generation would wrap is retired, so a stale handle can never match.
        if let Some(next) = slot.generation.checked_add(1) {
            slot.generation = next;
            self.free.push(handle.index);
        }
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_handles_fail_lookup() {
        let mut table = HandleTable::new();
        let a = table.insert("a");
        let b = table.insert("b");
        assert_eq!(table.get(a), Some(&"a"));
        assert_eq!(table.remove(a), Some("a"));
        assert_eq!(table.remove(a), None);
        let c = table.insert("c");
        assert_eq!(c.index(), a.index());
        assert_ne!(c, a);
        assert_eq!(table.get(a), None);
        assert_eq!(table.get(c), Some(&"c"));
        *table.get_mut(b).unwrap() = "B";
        assert_eq!(table.get(b), Some(&"B"));
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn handles_are_plain_data_for_any_type() {
        fn assert_plain<T: Copy + Send + Sync>() {}
        assert_plain::<Handle<std::rc::Rc<()>>>();
        assert_eq!(size_of::<Option<Handle<String>>>(), 8);
    }
}
