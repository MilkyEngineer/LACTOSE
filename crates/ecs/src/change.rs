//! Change tracking read per consumer: column change ticks, the structural change log and
//! whole-column access for engine passes (architecture section 6).

use core::marker::PhantomData;
use core::ops::Range;

use lactose_core::{ComponentKey, StableKey, Tick};
use lactose_reflect::Component;

use crate::region::RegionId;

/// A point in a world's change history, taken with [`World::mark`](crate::World::mark)
/// and given back to [`World::changed_since`](crate::World::changed_since).
///
/// A mark records the tick and the world's epoch. A restore or a load rebases ticks and
/// starts a new epoch, so a mark from before it is stale: the methods that take one then
/// return `None`, and the consumer does a full pass and takes a fresh mark.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ChangeMark {
    epoch: u32,
    tick: Tick,
}

impl ChangeMark {
    /// The tick the mark was taken at.
    #[must_use]
    pub const fn tick(&self) -> Tick {
        self.tick
    }
}

/// Rows of component `T` changed since a mark: those whose change tick is newer than the
/// mark's tick. Exact per row on a tracked column (replicated or extracted), per chunk
/// otherwise. Storage order, which is not a semantic key.
pub struct ChangedRows<'w, T: Component> {
    _world: PhantomData<&'w T>,
}

impl<'w, T: Component> Iterator for ChangedRows<'w, T> {
    type Item = (StableKey, &'w T);

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

/// One chunk's column of `T`, read. Yielded by [`World::columns`](crate::World::columns)
/// for extract, replication gather and hashing, which fork over chunks; the rows are
/// identified by key, never by index.
pub struct ColumnRef<'w, T: Component> {
    _world: PhantomData<&'w T>,
}

impl<T: Component> ColumnRef<'_, T> {
    /// The origin key of each row.
    #[must_use]
    pub fn keys(&self) -> &[StableKey] {
        todo!()
    }

    /// The region of each row.
    #[must_use]
    pub fn regions(&self) -> &[RegionId] {
        todo!()
    }

    /// The values, one per row.
    #[must_use]
    pub fn values(&self) -> &[T] {
        todo!()
    }

    /// The change tick of each row on a tracked column, or `None` on an untracked one, whose
    /// rows share [`column_tick`](Self::column_tick).
    #[must_use]
    pub fn row_ticks(&self) -> Option<&[Tick]> {
        todo!()
    }

    /// The chunk's coarse change tick for this column: the last tick a writing system was
    /// dispatched over the chunk.
    #[must_use]
    pub fn column_tick(&self) -> Tick {
        todo!()
    }
}

/// Iterates the chunks holding a column of `T`.
pub struct Columns<'w, T: Component> {
    _world: PhantomData<&'w T>,
}

impl<'w, T: Component> Iterator for Columns<'w, T> {
    type Item = ColumnRef<'w, T>;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

/// One chunk's column of `T`, written as a slice with an explicit dirty range
/// (architecture section 6). Yielded by [`World::columns_mut`](crate::World::columns_mut)
/// for bulk writes at a sync point.
pub struct ColumnMut<'w, T: Component> {
    _world: PhantomData<&'w mut T>,
}

impl<T: Component> ColumnMut<'_, T> {
    /// The origin key of each row.
    #[must_use]
    pub fn keys(&self) -> &[StableKey] {
        todo!()
    }

    /// The region of each row.
    #[must_use]
    pub fn regions(&self) -> &[RegionId] {
        todo!()
    }

    /// Writes through `f`, which receives the values and must record every row it changed
    /// in the dirty range; those rows are marked changed at the current tick when `f`
    /// returns. A row written and not recorded keeps its old tick, which replication and
    /// extract then miss.
    pub fn write(self, f: impl FnOnce(&mut [T], &mut DirtyRange)) {
        todo!()
    }
}

/// Iterates the chunks holding a column of `T`, for writing.
pub struct ColumnsMut<'w, T: Component> {
    _world: PhantomData<&'w mut T>,
}

impl<'w, T: Component> Iterator for ColumnsMut<'w, T> {
    type Item = ColumnMut<'w, T>;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

/// The rows a whole-column write changed, as one range of row indices within the chunk.
/// Starts empty.
#[derive(Clone, Debug, Default)]
pub struct DirtyRange {
    range: Option<Range<usize>>,
}

impl DirtyRange {
    /// Widens the range to include row `index`.
    pub fn mark(&mut self, index: usize) {
        todo!()
    }

    /// Widens the range to include `rows`.
    pub fn mark_range(&mut self, rows: Range<usize>) {
        todo!()
    }

    /// Marks every row.
    pub fn mark_all(&mut self) {
        todo!()
    }

    /// The marked rows, or `None` if nothing was marked.
    #[must_use]
    pub fn get(&self) -> Option<Range<usize>> {
        self.range.clone()
    }
}

/// One chunk's column of any component, as bytes. Yielded by
/// [`World::columns_erased`](crate::World::columns_erased) for passes that work from the
/// registry rather than a type: the world hasher and bulk export.
pub struct ErasedColumn<'w> {
    _world: PhantomData<&'w ()>,
}

impl ErasedColumn<'_> {
    /// The component.
    #[must_use]
    pub fn component(&self) -> ComponentKey {
        todo!()
    }

    /// The size of one value in bytes.
    #[must_use]
    pub fn element_size(&self) -> usize {
        todo!()
    }

    /// The origin key of each row.
    #[must_use]
    pub fn keys(&self) -> &[StableKey] {
        todo!()
    }

    /// The region of each row.
    #[must_use]
    pub fn regions(&self) -> &[RegionId] {
        todo!()
    }

    /// The values, `element_size` bytes per row, laid out as the component is in memory.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        todo!()
    }

    /// The change tick of each row on a tracked column, or `None`.
    #[must_use]
    pub fn row_ticks(&self) -> Option<&[Tick]> {
        todo!()
    }

    /// The chunk's coarse change tick for this column.
    #[must_use]
    pub fn column_tick(&self) -> Tick {
        todo!()
    }
}

/// Iterates the chunks holding a column, as bytes.
pub struct ErasedColumns<'w> {
    _world: PhantomData<&'w ()>,
}

impl<'w> Iterator for ErasedColumns<'w> {
    type Item = ErasedColumn<'w>;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
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
/// or a load.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LogCursor {
    epoch: u32,
    index: u64,
}

/// The per-world log of spawns, despawns, component additions and removals and region
/// moves, each with its tick, read per consumer (architecture section 6).
///
/// Entries accumulate until [`World::trim_structural_log`](crate::World::trim_structural_log)
/// drops those every consumer has passed. A restore or a load clears the log and starts a
/// new epoch.
pub struct StructuralLog {
    _private: (),
}

impl StructuralLog {
    /// A cursor at the end of the log: a consumer that starts here sees only later
    /// changes.
    #[must_use]
    pub fn cursor(&self) -> LogCursor {
        todo!()
    }

    /// A cursor at the start of the retained log.
    #[must_use]
    pub fn cursor_at_start(&self) -> LogCursor {
        todo!()
    }

    /// The entries after `cursor`, in order, and `cursor` advanced past them; or `None` if
    /// the cursor is stale or its entries have been trimmed, in which case the consumer
    /// does a full pass and takes [`cursor`](Self::cursor).
    #[must_use]
    pub fn since(&self, cursor: &mut LogCursor) -> Option<LogEntries<'_>> {
        todo!()
    }
}

/// Iterates log entries in order.
pub struct LogEntries<'a> {
    _log: PhantomData<&'a ()>,
}

impl<'a> Iterator for LogEntries<'a> {
    type Item = &'a LogEntry;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}
