//! Chunk storage: archetypes, chunks with one contiguous column per component, the column
//! ticks and row stamps of every chunk, and the entity table (architecture section 6).
//!
//! A chunk's columns are byte ranges of one slab of 8-byte words, each starting at a multiple
//! of eight, so every column is aligned for every registered component (alignment at most
//! eight, checked when the world is built). Typed access is a checked `bytemuck` cast of a
//! row's or a column's bytes, and disjoint columns are reached by splitting the slab, so
//! nothing here is `unsafe`.

use std::collections::BTreeMap;

use bytemuck::CheckedBitPattern;
use lactose_core::{ComponentKey, StableKey, Tick};
use lactose_reflect::TypeInfo;

use crate::change::Stamp;

/// The slab size of a chunk whose row count is derived (architecture section 6).
pub(crate) const SLAB_BYTES: usize = 16 * 1024;

/// The largest component alignment the slab layout supports.
pub(crate) const MAX_ALIGN: usize = 8;

/// Bytes held in 8-byte words, so they are aligned for any registered type.
#[derive(Clone, Default)]
pub(crate) struct AlignedBytes {
    words: Vec<u64>,
    len: usize,
}

impl AlignedBytes {
    pub(crate) fn zeroed(len: usize) -> Self {
        Self { words: vec![0; len.div_ceil(8)], len }
    }

    pub(crate) fn from_bytes(bytes: &[u8]) -> Self {
        let mut aligned = Self::zeroed(bytes.len());
        aligned.as_bytes_mut().copy_from_slice(bytes);
        aligned
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &bytemuck::cast_slice(&self.words)[..self.len]
    }

    pub(crate) fn as_bytes_mut(&mut self) -> &mut [u8] {
        &mut bytemuck::cast_slice_mut(&mut self.words)[..self.len]
    }
}

/// `bytes` as one `T`. The bytes are a valid `T` by the storage invariant, so the check
/// failing is a broken invariant.
pub(crate) fn typed<T: CheckedBitPattern>(bytes: &[u8]) -> &T {
    bytemuck::checked::from_bytes(bytes)
}

/// `bytes` as one `T`, for writing.
pub(crate) fn typed_mut<T: CheckedBitPattern + bytemuck::NoUninit>(bytes: &mut [u8]) -> &mut T {
    bytemuck::checked::from_bytes_mut(bytes)
}

/// `bytes`, which hold `len` values of `T` back to back, as a slice.
pub(crate) fn typed_slice<T: CheckedBitPattern>(bytes: &[u8], len: usize) -> &[T] {
    if size_of::<T>() == 0 {
        // A cast cannot give a zero-sized type a length, so the slice is built from the one
        // value; a `Vec` of a zero-sized type owns no allocation, so leaking it leaks nothing.
        let empty: [u64; 0] = [];
        let one: T = *bytemuck::checked::from_bytes(bytemuck::cast_slice(&empty));
        return vec![one; len].leak();
    }
    bytemuck::checked::cast_slice(bytes)
}

/// `bytes`, which hold `len` values of `T` back to back, as a mutable slice.
pub(crate) fn typed_slice_mut<T: CheckedBitPattern + bytemuck::NoUninit>(
    bytes: &mut [u8],
    len: usize,
) -> &mut [T] {
    if size_of::<T>() == 0 {
        let empty: [u64; 0] = [];
        let one: T = *bytemuck::checked::from_bytes(bytemuck::cast_slice(&empty));
        return vec![one; len].leak();
    }
    bytemuck::checked::cast_slice_mut(bytes)
}

/// One column of an archetype.
#[derive(Clone, Copy)]
pub(crate) struct ColumnInfo {
    pub(crate) key: ComponentKey,
    pub(crate) info: &'static TypeInfo,
    pub(crate) size: usize,
    /// Replicated or extracted: the column carries a stamp per row.
    pub(crate) tracked: bool,
    /// The byte offset of the column in a chunk's slab, a multiple of eight.
    pub(crate) offset: usize,
}

/// The stamps of a tracked column, one per row.
#[derive(Clone)]
pub(crate) struct RowStamps {
    pub(crate) ticks: Vec<Tick>,
    pub(crate) positions: Vec<u32>,
}

impl RowStamps {
    pub(crate) fn get(&self, row: usize) -> Stamp {
        Stamp { tick: self.ticks[row], position: self.positions[row] }
    }

    fn set(&mut self, row: usize, stamp: Stamp) {
        self.ticks[row] = stamp.tick;
        self.positions[row] = stamp.position;
    }
}

/// A slab of rows of one archetype.
#[derive(Clone)]
pub(crate) struct Chunk {
    pub(crate) slab: AlignedBytes,
    /// The origin key of each row; its length is the row count.
    pub(crate) keys: Vec<StableKey>,
    /// The coarse change point of each column.
    pub(crate) column_ticks: Vec<Stamp>,
    /// The row stamps of each column; `Some` for a tracked column.
    pub(crate) stamps: Vec<Option<RowStamps>>,
}

impl Chunk {
    pub(crate) fn len(&self) -> usize {
        self.keys.len()
    }

    /// The bytes of column `index` for the live rows.
    pub(crate) fn column(&self, column: &ColumnInfo) -> &[u8] {
        let start = column.offset;
        &self.slab.as_bytes()[start..start + column.size * self.len()]
    }

    /// The bytes of one row of one column.
    pub(crate) fn row(&self, column: &ColumnInfo, row: usize) -> &[u8] {
        let start = column.offset + column.size * row;
        &self.slab.as_bytes()[start..start + column.size]
    }

    fn row_mut(&mut self, column: &ColumnInfo, row: usize) -> &mut [u8] {
        let start = column.offset + column.size * row;
        &mut self.slab.as_bytes_mut()[start..start + column.size]
    }

    /// One row of column `index`, for writing with its stamp.
    pub(crate) fn row_view(
        &mut self,
        columns: &[ColumnInfo],
        index: usize,
        row: usize,
    ) -> RowView<'_> {
        let column = &columns[index];
        let start = column.offset + column.size * row;
        let bytes = &mut self.slab.as_bytes_mut()[start..start + column.size];
        let stamps = self.stamps[index]
            .as_mut()
            .map(|stamps| (&mut stamps.ticks[row], &mut stamps.positions[row]));
        RowView { bytes, stamps }
    }

    /// One row of every column, in archetype column order, for writing several at once.
    pub(crate) fn row_views(&mut self, columns: &[ColumnInfo], row: usize) -> Vec<RowView<'_>> {
        let mut rest = self.slab.as_bytes_mut();
        let mut consumed = 0;
        let mut stamps = self.stamps.iter_mut();
        columns
            .iter()
            .map(|column| {
                let start = column.offset + column.size * row;
                let (_, tail) = core::mem::take(&mut rest).split_at_mut(start - consumed);
                let (bytes, tail) = tail.split_at_mut(column.size);
                rest = tail;
                consumed = start + column.size;
                let stamps = stamps
                    .next()
                    .expect("one stamp entry per column")
                    .as_mut()
                    .map(|stamps| (&mut stamps.ticks[row], &mut stamps.positions[row]));
                RowView { bytes, stamps }
            })
            .collect()
    }

    /// Splits the chunk into one view per column, in archetype column order, for code that
    /// writes several columns at once.
    pub(crate) fn split<'c>(&'c mut self, columns: &[ColumnInfo]) -> ChunkViews<'c> {
        let len = self.keys.len();
        let mut rest = self.slab.as_bytes_mut();
        let mut consumed = 0;
        let mut views = Vec::with_capacity(columns.len());
        let mut ticks = self.column_ticks.iter_mut();
        let mut stamps = self.stamps.iter_mut();
        for column in columns {
            let skip = column.offset - consumed;
            let (_, tail) = rest.split_at_mut(skip);
            let (bytes, tail) = tail.split_at_mut(column.size * len);
            consumed = column.offset + column.size * len;
            rest = tail;
            let stamps = stamps
                .next()
                .expect("one stamp entry per column")
                .as_mut()
                .map(|s| (&mut s.ticks[..len], &mut s.positions[..len]));
            views.push(ColumnView {
                size: column.size,
                bytes,
                stamps,
                column_tick: ticks.next().expect("one column tick per column"),
            });
        }
        ChunkViews { keys: &self.keys, columns: views }
    }
}

/// A chunk split into disjoint column views.
pub(crate) struct ChunkViews<'c> {
    pub(crate) keys: &'c [StableKey],
    pub(crate) columns: Vec<ColumnView<'c>>,
}

/// One column of a split chunk.
pub(crate) struct ColumnView<'c> {
    pub(crate) size: usize,
    pub(crate) bytes: &'c mut [u8],
    pub(crate) stamps: Option<(&'c mut [Tick], &'c mut [u32])>,
    pub(crate) column_tick: &'c mut Stamp,
}

impl<'c> ColumnView<'c> {
    /// Splits off the first remaining row.
    pub(crate) fn next_row(&mut self) -> RowView<'c> {
        let (bytes, rest) = core::mem::take(&mut self.bytes).split_at_mut(self.size);
        self.bytes = rest;
        let stamps = self.stamps.take().map(|(ticks, positions)| {
            let (tick, ticks) = ticks.split_first_mut().expect("a stamp per row");
            let (position, positions) = positions.split_first_mut().expect("a stamp per row");
            self.stamps = Some((ticks, positions));
            (tick, position)
        });
        RowView { bytes, stamps }
    }
}

/// One row of one column of a split chunk.
pub(crate) struct RowView<'c> {
    pub(crate) bytes: &'c mut [u8],
    pub(crate) stamps: Option<(&'c mut Tick, &'c mut u32)>,
}

impl RowView<'_> {
    pub(crate) fn stamp(&self) -> Option<Stamp> {
        self.stamps.as_ref().map(|(tick, position)| Stamp { tick: **tick, position: **position })
    }
}

/// The rows of one exact component set.
#[derive(Clone)]
pub(crate) struct Archetype {
    /// Ascending by key.
    pub(crate) columns: Vec<ColumnInfo>,
    pub(crate) capacity: usize,
    slab_bytes: usize,
    pub(crate) chunks: Vec<Chunk>,
}

impl Archetype {
    fn new(infos: &[&'static TypeInfo], chunk_rows: Option<u32>) -> Self {
        let sizes: Vec<usize> = infos.iter().map(|info| info.layout().size()).collect();
        let slab = |rows: usize| -> usize {
            sizes.iter().map(|size| (size * rows).next_multiple_of(8)).sum()
        };
        let capacity = match chunk_rows {
            Some(rows) => rows as usize,
            None => {
                let row: usize = sizes.iter().sum();
                let mut rows = SLAB_BYTES.checked_div(row).unwrap_or(SLAB_BYTES).max(1);
                while rows > 1 && slab(rows) > SLAB_BYTES {
                    rows -= 1;
                }
                rows
            }
        };
        let mut offset = 0;
        let columns = infos
            .iter()
            .zip(&sizes)
            .map(|(info, &size)| {
                let tracking = info.tracking();
                let column = ColumnInfo {
                    key: info.key(),
                    info,
                    size,
                    tracked: tracking.replicated || tracking.extracted,
                    offset,
                };
                offset += (size * capacity).next_multiple_of(8);
                column
            })
            .collect();
        Self { columns, capacity, slab_bytes: slab(capacity), chunks: Vec::new() }
    }

    pub(crate) fn column_index(&self, key: ComponentKey) -> Option<usize> {
        self.columns.binary_search_by_key(&key, |column| column.key).ok()
    }

    pub(crate) fn has(&self, key: ComponentKey) -> bool {
        self.column_index(key).is_some()
    }

    fn new_chunk(&self) -> Chunk {
        Chunk {
            slab: AlignedBytes::zeroed(self.slab_bytes),
            keys: Vec::with_capacity(self.capacity),
            column_ticks: vec![Stamp::ZERO; self.columns.len()],
            stamps: self
                .columns
                .iter()
                .map(|column| {
                    column.tracked.then(|| RowStamps {
                        ticks: vec![Tick(0); self.capacity],
                        positions: vec![0; self.capacity],
                    })
                })
                .collect(),
        }
    }
}

/// Where an entity's row is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Loc {
    pub(crate) archetype: u32,
    pub(crate) chunk: u32,
    pub(crate) row: u32,
}

/// One component of a row in transit: being spawned, or moved between archetypes.
#[derive(Clone)]
pub(crate) struct RowValue {
    pub(crate) info: &'static TypeInfo,
    pub(crate) bytes: Vec<u8>,
    /// The row's stamp on a tracked column.
    pub(crate) stamp: Stamp,
    /// The column tick of the chunk the value comes from, so a move never hides an
    /// untracked change from a consumer that skips chunks by their column tick.
    pub(crate) column_tick: Stamp,
}

/// The entity table: every live entity's row, by origin key.
pub(crate) type StableKeyMap = BTreeMap<StableKey, Loc>;

/// Every archetype and the entity table.
#[derive(Clone)]
pub(crate) struct Storage {
    pub(crate) archetypes: Vec<Archetype>,
    by_components: BTreeMap<Vec<ComponentKey>, u32>,
    pub(crate) entities: StableKeyMap,
    chunk_rows: Option<u32>,
    /// The world's tick, from which stamps are ordered across the `u32` wrap when a moved
    /// row's stamps meet a chunk's.
    pub(crate) clock: Tick,
}

impl Storage {
    pub(crate) fn with_clock(chunk_rows: Option<u32>, clock: Tick) -> Self {
        Self {
            archetypes: Vec::new(),
            by_components: BTreeMap::new(),
            entities: BTreeMap::new(),
            chunk_rows,
            clock,
        }
    }

    pub(crate) fn loc(&self, key: StableKey) -> Option<Loc> {
        self.entities.get(&key).copied()
    }

    /// The bytes of component `component` of the entity at `loc`.
    pub(crate) fn component(&self, loc: Loc, component: ComponentKey) -> Option<&[u8]> {
        let archetype = &self.archetypes[loc.archetype as usize];
        let column = archetype.column_index(component)?;
        Some(archetype.chunks[loc.chunk as usize].row(&archetype.columns[column], loc.row as usize))
    }

    /// Overwrites component `component` of the entity at `loc`, which has it, stamping the
    /// row and the column at `stamp`.
    pub(crate) fn write(&mut self, loc: Loc, component: ComponentKey, bytes: &[u8], stamp: Stamp) {
        let archetype = &mut self.archetypes[loc.archetype as usize];
        let index = archetype.column_index(component).expect("the entity has the component");
        let column = archetype.columns[index];
        let chunk = &mut archetype.chunks[loc.chunk as usize];
        chunk.row_mut(&column, loc.row as usize).copy_from_slice(bytes);
        mark(chunk, index, loc.row as usize, stamp);
    }

    /// Overwrites component `component` of the entity at `loc` without stamping it: a
    /// rewrite that changes no value, only how a reference is spelled.
    pub(crate) fn overwrite(&mut self, loc: Loc, component: ComponentKey, bytes: &[u8]) {
        let archetype = &mut self.archetypes[loc.archetype as usize];
        let index = archetype.column_index(component).expect("the entity has the component");
        let column = archetype.columns[index];
        archetype.chunks[loc.chunk as usize]
            .row_mut(&column, loc.row as usize)
            .copy_from_slice(bytes);
    }

    /// The archetype of the exact component set `infos`, ascending by key.
    pub(crate) fn archetype(&mut self, infos: &[&'static TypeInfo]) -> u32 {
        let keys: Vec<ComponentKey> = infos.iter().map(|info| info.key()).collect();
        debug_assert!(keys.windows(2).all(|pair| pair[0] < pair[1]), "sorted, no duplicates");
        if let Some(&index) = self.by_components.get(&keys) {
            return index;
        }
        let index = self.archetypes.len() as u32;
        self.archetypes.push(Archetype::new(infos, self.chunk_rows));
        self.by_components.insert(keys, index);
        index
    }

    /// Appends a row for `key` to `archetype`. `values` holds exactly the archetype's
    /// components, ascending by key.
    pub(crate) fn push(&mut self, archetype: u32, key: StableKey, values: &[RowValue]) -> Loc {
        let clock = self.clock;
        let arch = &mut self.archetypes[archetype as usize];
        debug_assert_eq!(arch.columns.len(), values.len());
        if arch.chunks.last().is_none_or(|chunk| chunk.len() == arch.capacity) {
            let chunk = arch.new_chunk();
            arch.chunks.push(chunk);
        }
        let chunk_index = arch.chunks.len() - 1;
        let chunk = &mut arch.chunks[chunk_index];
        let row = chunk.len();
        chunk.keys.push(key);
        for (index, (column, value)) in arch.columns.iter().zip(values).enumerate() {
            debug_assert_eq!(column.key, value.info.key());
            chunk.row_mut(column, row).copy_from_slice(&value.bytes);
            if let Some(stamps) = &mut chunk.stamps[index] {
                stamps.set(row, value.stamp);
            }
            let tick = &mut chunk.column_ticks[index];
            *tick = tick.later(value.column_tick, clock).later(value.stamp, clock);
        }
        let loc = Loc { archetype, chunk: chunk_index as u32, row: row as u32 };
        self.entities.insert(key, loc);
        loc
    }

    /// Removes `key`'s row and returns its archetype and components, ascending by key.
    pub(crate) fn take(&mut self, key: StableKey) -> Option<(u32, Vec<RowValue>)> {
        let loc = self.entities.remove(&key)?;
        let arch = &mut self.archetypes[loc.archetype as usize];
        let chunk = &arch.chunks[loc.chunk as usize];
        let row = loc.row as usize;
        let values = arch
            .columns
            .iter()
            .enumerate()
            .map(|(index, column)| RowValue {
                info: column.info,
                bytes: chunk.row(column, row).to_vec(),
                stamp: chunk.stamps[index].as_ref().map_or(Stamp::ZERO, |s| s.get(row)),
                column_tick: chunk.column_ticks[index],
            })
            .collect();

        // Fill the hole with the archetype's last row, so every chunk but the last is full.
        let last_chunk = arch.chunks.len() - 1;
        let last_row = arch.chunks[last_chunk].len() - 1;
        if (loc.chunk as usize, row) != (last_chunk, last_row) {
            let moved =
                copy_row(arch, (last_chunk, last_row), (loc.chunk as usize, row), self.clock);
            self.entities.insert(moved, loc);
        }
        let arch = &mut self.archetypes[loc.archetype as usize];
        let chunk = &mut arch.chunks[last_chunk];
        chunk.keys.pop();
        if chunk.keys.is_empty() {
            arch.chunks.pop();
        }
        Some((loc.archetype, values))
    }

    /// Reorders the rows of every chunk by a permutation `shuffle` draws, keeping every
    /// identity: SOUR's layout mode (architecture section 8.3).
    pub(crate) fn permute(&mut self, shuffle: &mut impl FnMut(usize) -> Vec<usize>) {
        for (archetype, arch) in self.archetypes.iter_mut().enumerate() {
            for (chunk_index, chunk) in arch.chunks.iter_mut().enumerate() {
                let order = shuffle(chunk.len());
                let old = chunk.clone();
                for (row, &from) in order.iter().enumerate() {
                    for (index, column) in arch.columns.iter().enumerate() {
                        chunk.row_mut(column, row).copy_from_slice(old.row(column, from));
                        if let (Some(stamps), Some(source)) =
                            (&mut chunk.stamps[index], &old.stamps[index])
                        {
                            stamps.set(row, source.get(from));
                        }
                    }
                    chunk.keys[row] = old.keys[from];
                    self.entities.insert(
                        old.keys[from],
                        Loc {
                            archetype: archetype as u32,
                            chunk: chunk_index as u32,
                            row: row as u32,
                        },
                    );
                }
            }
        }
    }
}

/// Stamps row `row` of column `index` and raises the column tick.
pub(crate) fn mark(chunk: &mut Chunk, index: usize, row: usize, stamp: Stamp) {
    if let Some(stamps) = &mut chunk.stamps[index] {
        stamps.set(row, stamp);
    }
    let tick = &mut chunk.column_ticks[index];
    // A write is stamped at the newest point there is.
    *tick = stamp;
}

/// Copies the row at `from` over the row at `to` within one archetype and returns the key
/// of the row copied. The destination chunk's column ticks rise to the source's.
fn copy_row(
    arch: &mut Archetype,
    from: (usize, usize),
    to: (usize, usize),
    clock: Tick,
) -> StableKey {
    let source = arch.chunks[from.0].clone_row(&arch.columns, from.1);
    let chunk = &mut arch.chunks[to.0];
    for (index, column) in arch.columns.iter().enumerate() {
        chunk.row_mut(column, to.1).copy_from_slice(&source.values[index].bytes);
        if let Some(stamps) = &mut chunk.stamps[index] {
            stamps.set(to.1, source.values[index].stamp);
        }
        let tick = &mut chunk.column_ticks[index];
        *tick = tick.later(source.values[index].column_tick, clock);
    }
    chunk.keys[to.1] = source.key;
    source.key
}

struct ClonedRow {
    key: StableKey,
    values: Vec<RowValue>,
}

impl Chunk {
    fn clone_row(&self, columns: &[ColumnInfo], row: usize) -> ClonedRow {
        ClonedRow {
            key: self.keys[row],
            values: columns
                .iter()
                .enumerate()
                .map(|(index, column)| RowValue {
                    info: column.info,
                    bytes: self.row(column, row).to_vec(),
                    stamp: self.stamps[index].as_ref().map_or(Stamp::ZERO, |s| s.get(row)),
                    column_tick: self.column_ticks[index],
                })
                .collect(),
        }
    }
}
