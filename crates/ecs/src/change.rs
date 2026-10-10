//! Change tracking read per consumer: column change ticks, the structural change log and
//! whole-column access for engine passes (architecture section 6).

use core::ops::Range;

use lactose_core::{ComponentKey, StableKey, Tick};
use lactose_reflect::Component;

use crate::region::RegionId;

/// A point in a world's stamp order: a tick and a position within it.
///
/// Within a tick, every sync point and every system has a fixed position in execution
/// order, so a system's writes carry the same stamp under every layout; direct writes made
/// inside a slot or between ticks take the next position after the last one used.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub(crate) struct Stamp {
    pub(crate) tick: Tick,
    pub(crate) position: u32,
}

impl Stamp {
    pub(crate) const ZERO: Self = Self { tick: Tick(0), position: 0 };

    pub(crate) const fn new(tick: Tick, position: u32) -> Self {
        Self { tick, position }
    }
}

/// A point in a world's change history, taken with [`World::mark`](crate::World::mark)
/// and given back to [`World::changed_since`](crate::World::changed_since) and
/// [`World::is_current`](crate::World::is_current).
///
/// A mark records the world's epoch and a point in the tick's stamp order: between ticks it
/// is after every position of the tick, so a direct write made after the mark, or a write
/// in a later `run_pass`, is newer than it. A restore, a whole-world load or a change-stamp
/// rebase starts a new epoch, so a mark from before it is stale: the methods that take one
/// then return `None` or `false`, and the consumer does a full pass and takes a fresh mark.
/// A region load does not start an epoch: its rows are stamped at the current tick and
/// logged as spawned, so consumers pick them up incrementally.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ChangeMark {
    epoch: u32,
    tick: Tick,
    position: u32,
}

impl ChangeMark {
    pub(crate) const fn new(epoch: u32, stamp: Stamp) -> Self {
        Self { epoch, tick: stamp.tick, position: stamp.position }
    }

    pub(crate) const fn epoch(&self) -> u32 {
        self.epoch
    }

    pub(crate) const fn stamp(&self) -> Stamp {
        Stamp::new(self.tick, self.position)
    }

    /// The tick the mark was taken at.
    #[must_use]
    pub const fn tick(&self) -> Tick {
        self.tick
    }
}

/// Rows of component `T` changed since a mark: those whose change stamp is newer than the
/// mark. `T` is a tracked column (replicated or extracted). Storage order, which is not a
/// semantic key: the consumer keys what it produces by the row's key.
pub struct ChangedRows<'w, T: Component> {
    pub(crate) rows: std::vec::IntoIter<(StableKey, &'w T)>,
}

impl<'w, T: Component> Iterator for ChangedRows<'w, T> {
    type Item = (StableKey, &'w T);

    fn next(&mut self) -> Option<Self::Item> {
        self.rows.next()
    }
}

/// One chunk's column of `T`, read. Yielded by [`World::columns`](crate::World::columns)
/// for extract, replication gather and hashing, which fork over chunks; the rows are
/// identified by key, never by index.
pub struct ColumnRef<'w, T: Component> {
    pub(crate) keys: &'w [StableKey],
    pub(crate) regions: &'w [RegionId],
    pub(crate) values: &'w [T],
    pub(crate) row_ticks: Option<&'w [Tick]>,
    pub(crate) column_tick: Tick,
}

impl<T: Component> ColumnRef<'_, T> {
    /// The origin key of each row.
    #[must_use]
    pub fn keys(&self) -> &[StableKey] {
        self.keys
    }

    /// The region of each row.
    #[must_use]
    pub fn regions(&self) -> &[RegionId] {
        self.regions
    }

    /// The values, one per row.
    #[must_use]
    pub fn values(&self) -> &[T] {
        self.values
    }

    /// The tick each row last changed at, on a tracked column; `None` on an untracked one,
    /// whose rows share [`column_tick`](Self::column_tick).
    #[must_use]
    pub fn row_ticks(&self) -> Option<&[Tick]> {
        self.row_ticks
    }

    /// The chunk's coarse change tick for this column: the last tick a writing system was
    /// dispatched over the chunk, or any write reached it. A consumer skips a chunk whose
    /// column tick is older than its mark's tick; a write in the mark's own tick, before or
    /// after the mark, shows that tick, so only `changed_since` tells them apart.
    #[must_use]
    pub fn column_tick(&self) -> Tick {
        self.column_tick
    }
}

/// Iterates the chunks holding a column of `T`.
pub struct Columns<'w, T: Component> {
    pub(crate) columns: std::vec::IntoIter<ColumnRef<'w, T>>,
}

impl<'w, T: Component> Iterator for Columns<'w, T> {
    type Item = ColumnRef<'w, T>;

    fn next(&mut self) -> Option<Self::Item> {
        self.columns.next()
    }
}

/// One chunk's column of `T`, written as a slice with an explicit dirty range
/// (architecture section 6). Yielded by [`World::columns_mut`](crate::World::columns_mut)
/// for bulk writes at a sync point.
pub struct ColumnMut<'w, T: Component> {
    pub(crate) keys: &'w [StableKey],
    pub(crate) regions: Vec<RegionId>,
    pub(crate) values: &'w mut [T],
    pub(crate) stamps: Option<(&'w mut [Tick], &'w mut [u32])>,
    pub(crate) column_tick: &'w mut Stamp,
    pub(crate) now: Stamp,
}

impl<T: Component> ColumnMut<'_, T> {
    /// The origin key of each row.
    #[must_use]
    pub fn keys(&self) -> &[StableKey] {
        self.keys
    }

    /// The region of each row.
    #[must_use]
    pub fn regions(&self) -> &[RegionId] {
        &self.regions
    }

    /// Writes through `f`, which receives the rows' keys and values and records every row
    /// it changed; those rows are stamped at the current point when `f` returns. A row
    /// written and not recorded keeps its old stamp, which replication and extract then
    /// miss; that is how `lactose-predict` writes a correction after a restore without
    /// inventing a change.
    pub fn write(self, f: impl FnOnce(&[StableKey], &mut [T], &mut DirtyRows)) {
        let mut dirty = DirtyRows { bits: Vec::new(), rows: self.keys.len() };
        f(self.keys, self.values, &mut dirty);
        let mut any = false;
        if let Some((ticks, positions)) = self.stamps {
            let rows = ticks.len();
            for row in dirty.iter().take_while(|&row| row < rows) {
                ticks[row] = self.now.tick;
                positions[row] = self.now.position;
                any = true;
            }
        } else {
            any = dirty.iter().any(|row| row < self.keys.len());
        }
        if any {
            *self.column_tick = (*self.column_tick).max(self.now);
        }
    }
}

/// Iterates the chunks holding a column of `T`, for writing.
pub struct ColumnsMut<'w, T: Component> {
    pub(crate) columns: std::vec::IntoIter<ColumnMut<'w, T>>,
}

impl<'w, T: Component> Iterator for ColumnsMut<'w, T> {
    type Item = ColumnMut<'w, T>;

    fn next(&mut self) -> Option<Self::Item> {
        self.columns.next()
    }
}

/// The rows a whole-column write changed, as a set of row indices within the chunk's
/// slice. Starts empty. The indices are positions in the slice handed to the write and
/// mean nothing outside it.
#[derive(Clone, Debug, Default)]
pub struct DirtyRows {
    bits: Vec<u64>,
    /// The length of the slice the set belongs to; zero for a set made with `default`.
    rows: usize,
}

impl DirtyRows {
    /// Marks row `index`.
    pub fn mark(&mut self, index: usize) {
        let word = index / 64;
        if self.bits.len() <= word {
            self.bits.resize(word + 1, 0);
        }
        self.bits[word] |= 1 << (index % 64);
    }

    /// Marks every row in `rows`.
    pub fn mark_range(&mut self, rows: Range<usize>) {
        for index in rows {
            self.mark(index);
        }
    }

    /// Marks every row of the slice the write was handed. A set made with `default`
    /// belongs to no slice, and this marks nothing in it.
    pub fn mark_all(&mut self) {
        self.mark_range(0..self.rows);
    }

    /// Whether `index` is marked.
    #[must_use]
    pub fn contains(&self, index: usize) -> bool {
        self.bits.get(index / 64).is_some_and(|word| word & (1 << (index % 64)) != 0)
    }

    /// The marked rows, ascending.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.bits.len() * 64).filter(|&index| self.contains(index))
    }
}

/// One chunk's column of any component, as bytes. Yielded by
/// [`World::columns_erased`](crate::World::columns_erased) for passes that work from the
/// registry rather than a type: the world hasher and bulk export.
pub struct ErasedColumn<'w> {
    pub(crate) component: ComponentKey,
    pub(crate) element_size: usize,
    pub(crate) keys: &'w [StableKey],
    pub(crate) regions: &'w [RegionId],
    pub(crate) bytes: &'w [u8],
    pub(crate) row_ticks: Option<&'w [Tick]>,
    pub(crate) column_tick: Tick,
}

impl ErasedColumn<'_> {
    /// The component.
    #[must_use]
    pub fn component(&self) -> ComponentKey {
        self.component
    }

    /// The size of one value in bytes.
    #[must_use]
    pub fn element_size(&self) -> usize {
        self.element_size
    }

    /// The origin key of each row.
    #[must_use]
    pub fn keys(&self) -> &[StableKey] {
        self.keys
    }

    /// The region of each row.
    #[must_use]
    pub fn regions(&self) -> &[RegionId] {
        self.regions
    }

    /// The values, `element_size` bytes per row, laid out as the component is in memory.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.bytes
    }

    /// The tick each row last changed at, on a tracked column; `None` otherwise.
    #[must_use]
    pub fn row_ticks(&self) -> Option<&[Tick]> {
        self.row_ticks
    }

    /// The chunk's coarse change tick for this column.
    #[must_use]
    pub fn column_tick(&self) -> Tick {
        self.column_tick
    }
}

/// Iterates the chunks holding a column, as bytes.
pub struct ErasedColumns<'w> {
    pub(crate) columns: std::vec::IntoIter<ErasedColumn<'w>>,
}

impl<'w> Iterator for ErasedColumns<'w> {
    type Item = ErasedColumn<'w>;

    fn next(&mut self) -> Option<Self::Item> {
        self.columns.next()
    }
}

/// One structural change.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StructuralChange {
    /// An entity came into being.
    Spawned {
        /// Its origin key.
        key: StableKey,
        /// Where it resides.
        region: RegionId,
    },
    /// An entity ceased to exist. Replication sends this as a tombstone and the renderer
    /// frees the slot.
    Despawned {
        /// Its origin key.
        key: StableKey,
        /// Where it resided.
        region: RegionId,
    },
    /// An entity gained a component.
    Added {
        /// The entity.
        key: StableKey,
        /// The component.
        component: ComponentKey,
    },
    /// An entity lost a component.
    Removed {
        /// The entity.
        key: StableKey,
        /// The component.
        component: ComponentKey,
    },
    /// An entity moved between regions.
    Moved {
        /// The entity.
        key: StableKey,
        /// Where it resided.
        from: RegionId,
        /// Where it resides now.
        to: RegionId,
    },
}

/// One entry of the structural log.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LogEntry {
    /// The tick the change applied at.
    pub tick: Tick,
    /// The change.
    pub change: StructuralChange,
}

/// A consumer's position in the structural log, taken with
/// [`StructuralLog::cursor`] and advanced by [`StructuralLog::since`].
///
/// Like a [`ChangeMark`], a cursor carries the world's epoch and is stale after a restore
/// or a whole-world load. Cursors order by epoch, then position, so the owner of several
/// consumers trims at their minimum.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct LogCursor {
    epoch: u32,
    index: u64,
}

/// The per-world log of spawns, despawns, component additions and removals and region
/// moves, each with its tick, read per consumer (architecture section 6).
///
/// Entries accumulate until [`World::trim_structural_log`](crate::World::trim_structural_log)
/// drops those every consumer has passed. A restore or a whole-world load clears the log
/// and starts a new epoch.
pub struct StructuralLog {
    epoch: u32,
    /// The absolute index of `entries[0]`.
    base: u64,
    entries: Vec<LogEntry>,
}

impl StructuralLog {
    pub(crate) fn new(epoch: u32) -> Self {
        Self { epoch, base: 0, entries: Vec::new() }
    }

    pub(crate) fn push(&mut self, tick: Tick, change: StructuralChange) {
        self.entries.push(LogEntry { tick, change });
    }

    /// Empties the log and moves it to `epoch`.
    pub(crate) fn reset(&mut self, epoch: u32) {
        self.epoch = epoch;
        self.base = 0;
        self.entries.clear();
    }

    pub(crate) fn trim(&mut self, cursor: LogCursor) {
        if cursor.epoch != self.epoch || cursor.index <= self.base {
            return;
        }
        let end = self.base + self.entries.len() as u64;
        let drop = (cursor.index.min(end) - self.base) as usize;
        self.entries.drain(..drop);
        self.base += drop as u64;
    }

    /// A cursor at the end of the log: a consumer that starts here sees only later
    /// changes.
    #[must_use]
    pub fn cursor(&self) -> LogCursor {
        LogCursor { epoch: self.epoch, index: self.base + self.entries.len() as u64 }
    }

    /// A cursor at the start of the retained log.
    #[must_use]
    pub fn cursor_at_start(&self) -> LogCursor {
        LogCursor { epoch: self.epoch, index: self.base }
    }

    /// The entries after `cursor`, in order, and `cursor` advanced past them; or `None` if
    /// the cursor is stale or its entries have been trimmed, in which case the consumer
    /// does a full pass and takes [`cursor`](Self::cursor).
    #[must_use]
    pub fn since(&self, cursor: &mut LogCursor) -> Option<LogEntries<'_>> {
        let end = self.base + self.entries.len() as u64;
        if cursor.epoch != self.epoch || cursor.index < self.base || cursor.index > end {
            return None;
        }
        let start = (cursor.index - self.base) as usize;
        cursor.index = end;
        Some(LogEntries { entries: self.entries[start..].iter() })
    }
}

/// Iterates log entries in order.
pub struct LogEntries<'a> {
    entries: core::slice::Iter<'a, LogEntry>,
}

impl<'a> Iterator for LogEntries<'a> {
    type Item = &'a LogEntry;

    fn next(&mut self) -> Option<Self::Item> {
        self.entries.next()
    }
}
