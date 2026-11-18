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

/// The next generation any null device issues. One count for the process, so that no two
/// devices issue an equal handle and a handle of another device resolves to nothing.
static GENERATION: AtomicU32 = AtomicU32::new(1);

/// A generation no handle of any null device has had yet, wrapping past zero.
pub(crate) fn generation() -> NonZeroU32 {
    loop {
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

/// A table of what live handles resolve to. Chunks are made under the records' lock and
/// never moved or freed while the device lives; a lookup takes one entry's lock alone.
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

    /// Makes `handle` resolve to `value`. Called under the records' lock, which allocated
    /// the index.
    pub(crate) fn insert<K>(&self, handle: Handle<K>, value: Arc<T>) {
        let index = handle.index() as usize;
        let chunk = self.chunks[index / CHUNK].get_or_init(|| {
            (0..CHUNK).map(|_| Mutex::new(Entry { generation: None, value: None })).collect()
        });
        let mut entry = lock(&chunk[index % CHUNK]);
        entry.generation = Some(handle.generation());
        entry.value = Some(value);
    }

    /// What `handle` resolves to, counted by the `Arc`, or `None` for a stale or foreign
    /// handle.
    pub(crate) fn resolve<K>(&self, handle: Handle<K>) -> Option<Arc<T>> {
        let entry = lock(self.entry(handle.index())?);
        if entry.generation == Some(handle.generation()) { entry.value.clone() } else { None }
    }

    /// Makes the handle stale on every thread. What it resolved to lives on in the counts
    /// already taken.
    pub(crate) fn remove<K>(&self, handle: Handle<K>) {
        if let Some(entry) = self.entry(handle.index()) {
            let mut entry = lock(entry);
            if entry.generation == Some(handle.generation()) {
                entry.generation = None;
                entry.value = None;
            }
        }
    }
}
