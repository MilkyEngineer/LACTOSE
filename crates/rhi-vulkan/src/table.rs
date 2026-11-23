//! The tables an encoder resolves handles through: entries that never move, each behind a
//! lock of its own, so that recording takes no lock an update of the records takes
//! (`lactose-rhi-api`'s README, "Threads").

use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use lactose_core::Handle;

use crate::lock;

/// Entries in one chunk.
const CHUNK: usize = 256;
/// Chunks in one table: a table holds `CHUNK * CHUNKS` live entries at most.
const CHUNKS: usize = 256;
/// The most live entries of one kind a device holds.
pub(crate) const CAPACITY: u32 = (CHUNK * CHUNKS) as u32;

/// The next generation any Vulkan device issues. One count for the process, so that no two
/// devices issue an equal handle and a handle of another device resolves to nothing.
static GENERATION: AtomicU32 = AtomicU32::new(1);

/// A generation no handle of any Vulkan device has had yet, wrapping past zero.
pub(crate) fn generation() -> NonZeroU32 {
    loop {
        // Relaxed: a generation needs only to be unique, which the RMW gives; no other
        // memory is published through it.
        if let Some(generation) = NonZeroU32::new(GENERATION.fetch_add(1, Ordering::Relaxed)) {
            return generation;
        }
    }
}

struct Entry<T> {
    generation: Option<NonZeroU32>,
    value: Option<Arc<T>>,
}

/// One chunk of a table's entries.
type Chunk<T> = Box<[Mutex<Entry<T>>]>;

/// A table of what live handles resolve to. A chunk is made once, by the first insert of
/// an index in it, and never moved or freed while the device lives; a lookup takes one
/// entry's lock alone.
pub(crate) struct Table<T> {
    chunks: Box<[OnceLock<Chunk<T>>]>,
}

impl<T> Table<T> {
    pub(crate) fn new() -> Self {
        Self { chunks: (0..CHUNKS).map(|_| OnceLock::new()).collect() }
    }

    fn entry(&self, index: u32) -> Option<&Mutex<Entry<T>>> {
        let index = usize::try_from(index).ok()?;
        self.chunks.get(index / CHUNK)?.get()?.get(index % CHUNK)
    }

    /// Makes `handle` resolve to `value`. Called after the records' lock that allocated the
    /// index is given up: a previous occupant whose destroy has yet to remove it is
    /// replaced, and its last count may destroy its driver object as it drops, which it
    /// does after the entry's lock.
    pub(crate) fn insert<K>(&self, handle: Handle<K>, value: Arc<T>) {
        let index = handle.index() as usize;
        let chunk = self.chunks[index / CHUNK].get_or_init(|| {
            (0..CHUNK).map(|_| Mutex::new(Entry { generation: None, value: None })).collect()
        });
        let replaced = {
            let mut entry = lock(&chunk[index % CHUNK]);
            entry.generation = Some(handle.generation());
            entry.value.replace(value)
        };
        drop(replaced);
    }

    /// What `handle` resolves to, counted by the `Arc`, or `None` for a stale or foreign
    /// handle.
    pub(crate) fn resolve<K>(&self, handle: Handle<K>) -> Option<Arc<T>> {
        let entry = lock(self.entry(handle.index())?);
        if entry.generation == Some(handle.generation()) { entry.value.clone() } else { None }
    }

    /// Makes the handle stale on every thread. What it resolved to lives on in the counts
    /// already taken; the table's own is dropped after the entry's lock, as the last may
    /// destroy a driver object. Called with no lock held but a queue's or the heap's.
    pub(crate) fn remove<K>(&self, handle: Handle<K>) {
        let Some(entry) = self.entry(handle.index()) else { return };
        let removed = {
            let mut entry = lock(entry);
            if entry.generation == Some(handle.generation()) {
                entry.generation = None;
                entry.value.take()
            } else {
                None
            }
        };
        drop(removed);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Weak};

    use lactose_core::Handle;

    use super::{Table, generation};

    /// A value whose drop checks that its entry's lock is free: the last count of a driver
    /// object destroys it as it drops, which takes other locks.
    struct Probe {
        table: Weak<Table<Probe>>,
        index: u32,
        dropped: Arc<AtomicUsize>,
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            if let Some(table) = self.table.upgrade() {
                let entry = table.entry(self.index).expect("the entry");
                assert!(entry.try_lock().is_ok(), "dropped under its entry's lock");
            }
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn handle(index: u32) -> Handle<()> {
        Handle::from_parts(index, generation())
    }

    #[test]
    fn a_replaced_or_removed_value_drops_after_its_entrys_lock() {
        let table = Arc::new(Table::new());
        let dropped = Arc::new(AtomicUsize::new(0));
        let probe = || {
            Arc::new(Probe {
                table: Arc::downgrade(&table),
                index: 3,
                dropped: Arc::clone(&dropped),
            })
        };
        let first = handle(3);
        table.insert(first, probe());
        let second = handle(3);
        table.insert(second, probe());
        assert_eq!(dropped.load(Ordering::Relaxed), 1, "the replaced value");
        assert!(table.resolve(first).is_none() && table.resolve(second).is_some());
        table.remove(first);
        assert!(table.resolve(second).is_some(), "a stale remove leaves the entry");
        table.remove(second);
        assert_eq!(dropped.load(Ordering::Relaxed), 2, "the removed value");
        assert!(table.resolve(second).is_none());
    }

    /// Inserts, resolves and removes of one index's generations from several threads: a
    /// handle resolves to its own value or to nothing, never another generation's.
    #[test]
    fn a_handle_resolves_to_its_own_value_or_none_across_threads() {
        const ROUNDS: usize = if cfg!(miri) { 8 } else { 500 };
        let table: Arc<Table<Handle<()>>> = Arc::new(Table::new());
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let table = Arc::clone(&table);
                std::thread::spawn(move || {
                    for _ in 0..ROUNDS {
                        let handle = handle(7);
                        table.insert(handle, Arc::new(handle));
                        if let Some(value) = table.resolve(handle) {
                            assert_eq!(*value, handle, "another generation's value");
                        }
                        table.remove(handle);
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().expect("a worker");
        }
        assert!(table.resolve(handle(7)).is_none());
    }
}
