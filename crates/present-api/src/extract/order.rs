//! The presented rows' places, kept in key order from pass to pass so that a pass whose rows
//! stayed where they were sorts nothing (D164). A place is a hint and never a key
//! (architecture section 8.2): every pass that copies checks it against the row's key, and no
//! byte of the snapshot depends on it. It says where a row is and not what the row holds,
//! since a row can keep its chunk and its row while its archetype changes: the columns are
//! read from the chunk at every pass.

use lactose_core::{ComponentKey, StableKey};
use lactose_ecs::{ChunkRef, ErasedColumn};

/// Where a presented row was at the pass that last sorted the places: its key, the index of
/// its chunk among that pass's presented chunks, and its row in the chunk.
#[derive(Clone, Copy, Debug)]
struct Place {
    key: StableKey,
    chunk: u32,
    row: u32,
}

impl Place {
    /// Whether the row at this place in `chunks` is still the place's key.
    fn holds(self, chunks: &[ChunkRef<'_>]) -> bool {
        chunks.get(self.chunk as usize).and_then(|chunk| chunk.keys().get(self.row as usize))
            == Some(&self.key)
    }
}

/// The presented rows' places in ascending key order, read through the [`Settled`] that
/// [`settle`](Self::settle) returns, and through nothing else.
#[derive(Debug)]
pub(super) struct Order {
    places: Vec<Place>,
    /// The previous places' allocation, which the next sort fills.
    spare: Vec<Place>,
    /// How many times the places were sorted: what the unit tests read.
    #[cfg(test)]
    sorts: u64,
}

impl Order {
    pub(super) const fn new() -> Self {
        Self {
            places: Vec::new(),
            spare: Vec::new(),
            #[cfg(test)]
            sorts: 0,
        }
    }

    /// How many places there are.
    pub(super) fn len(&self) -> usize {
        self.places.len()
    }

    /// How many times the places were sorted.
    #[cfg(test)]
    pub(super) const fn sorts(&self) -> u64 {
        self.sorts
    }

    /// Makes the places those of `chunks`' rows, in ascending key order: kept when there are
    /// as many as `chunks` has rows and each still holds its key, sorted again from `chunks`
    /// otherwise.
    pub(super) fn settle<'a, 'w>(&'a mut self, chunks: &'a [ChunkRef<'w>]) -> Settled<'a, 'w> {
        let rows: usize = chunks.iter().map(|chunk| chunk.keys().len()).sum();
        // The places are distinct, being one sort of distinct positions, so as many as there
        // are rows, each holding a key, are every row once, and in key order.
        if self.places.len() == rows && self.places.iter().all(|place| place.holds(chunks)) {
            return Settled { places: &self.places, chunks, changed: false };
        }
        let sorted = &mut self.spare;
        sorted.clear();
        sorted.reserve(rows);
        for (index, chunk) in chunks.iter().enumerate() {
            let index = u32::try_from(index).expect("fewer than 2^32 chunks");
            for (row, &key) in chunk.keys().iter().enumerate() {
                let row = u32::try_from(row).expect("fewer than 2^32 rows in a chunk");
                sorted.push(Place { key, chunk: index, row });
            }
        }
        // Keys are unique in a world, so the order is the same whatever the sort.
        sorted.sort_unstable_by_key(|place| place.key);
        let changed =
            !self.places.iter().map(|place| place.key).eq(sorted.iter().map(|place| place.key));
        core::mem::swap(&mut self.places, &mut self.spare);
        #[cfg(test)]
        {
            self.sorts += 1;
        }
        Settled { places: &self.places, chunks, changed }
    }
}

/// An [`Order`] settled on one pass's presented chunks, which it holds: a fill copies through
/// it alone, so a fill from places that were not settled on the chunks it copies from does
/// not compile.
pub(super) struct Settled<'a, 'w> {
    places: &'a [Place],
    chunks: &'a [ChunkRef<'w>],
    changed: bool,
}

impl<'a, 'w> Settled<'a, 'w> {
    /// Whether the keys differ from those the places held before they were settled.
    pub(super) const fn changed(&self) -> bool {
        self.changed
    }

    /// Whether `key` is a presented row's.
    pub(super) fn holds(&self, key: StableKey) -> bool {
        self.places.binary_search_by_key(&key, |place| place.key).is_ok()
    }

    /// The chunks the places were settled on.
    pub(super) const fn chunks(&self) -> &'a [ChunkRef<'w>] {
        self.chunks
    }

    /// Calls `push` with the key and the value's bytes of every row whose chunk has
    /// `component`, in ascending key order, each read from its chunk now.
    pub(super) fn copy(&self, component: ComponentKey, mut push: impl FnMut(StableKey, &[u8])) {
        let columns: Vec<Option<ErasedColumn<'w>>> =
            self.chunks.iter().map(|chunk| chunk.column(component)).collect();
        for place in self.places {
            if let Some(column) = &columns[place.chunk as usize] {
                let size = column.element_size();
                let row = place.row as usize;
                push(place.key, &column.bytes()[row * size..(row + 1) * size]);
            }
        }
    }
}
