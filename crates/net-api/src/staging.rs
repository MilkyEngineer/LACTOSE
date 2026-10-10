//! The replication gather (architecture section 9.3): the exclusive pass system in
//! `Pass::Gather` that copies every replicated row whose tick advanced, and the structural
//! log since its cursor, into a [`StagingFrame`] that per-client packing reads off the
//! critical path, never against live chunks. [`GatherState`] is the service resource that
//! holds the gather's mark, cursor and the frame under construction; `app` takes the frame
//! between runs and hands it to the packer.
//!
//! Boundary code may use this module: it reads the world through `columns_erased` and the
//! structural log, keys every row by `StableKey`, and computes nothing from a float.

use lactose_core::{ComponentKey, Result, StableKey, Tick};
use lactose_ecs::{ChangeMark, Job, LogCursor, LogEntry, RegionId, ResMut, World, system};

/// What one gather produced: the rows of every replicated column whose tick advanced
/// since the previous gather, in one byte arena, grouped by region so that co-located
/// clients share per-region dirty lists, and the structural log entries since the
/// previous gather.
///
/// Rows are in ascending (region, origin key, component key) order once
/// [`finish`](Self::finish) has run, which [`GatherState::gather`] and
/// [`merge`](Self::merge) do; the order is a function of the keys, never of chunk layout
/// or worker count, so two servers with equal state produce equal frames.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StagingFrame {
    /// The world's tick when the gather ran, between ticks: every row is from an earlier
    /// tick.
    pub tick: Tick,
    /// Whether this frame is a full pass: the gather's mark or cursor was stale (a
    /// whole-world load or the change-stamp rebase, architecture section 6), so every row
    /// of every replicated column is here, `structural` is empty, and every client's
    /// acked ticks are void: the packer resyncs every client from it.
    pub full: bool,
    rows: Vec<RowEntry>,
    bytes: Vec<u8>,
    structural: Vec<LogEntry>,
    finished: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct RowEntry {
    region: RegionId,
    key: StableKey,
    component: ComponentKey,
    tick: Tick,
    start: u32,
    len: u32,
}

impl StagingFrame {
    /// An empty, finished frame for `tick`.
    #[must_use]
    pub fn new(tick: Tick) -> Self {
        Self {
            tick,
            full: false,
            rows: Vec::new(),
            bytes: Vec::new(),
            structural: Vec::new(),
            finished: true,
        }
    }

    /// Appends the component `component` of `key`, residing in `region`, as its row tick
    /// `tick` and bytes `bytes` (the component's image as laid out in memory). The frame
    /// is unordered from here until [`finish`](Self::finish).
    pub fn push_row(
        &mut self,
        region: RegionId,
        key: StableKey,
        component: ComponentKey,
        tick: Tick,
        bytes: &[u8],
    ) {
        todo!()
    }

    /// Appends a structural log entry, in log order.
    pub fn push_structural(&mut self, entry: LogEntry) {
        self.structural.push(entry);
    }

    /// Sorts the rows into (region, key, component) order and, where one (key,
    /// component) was pushed more than once, keeps the row with the newest tick (ties by
    /// the later push). Idempotent.
    pub fn finish(&mut self) {
        todo!()
    }

    /// Folds `newer`, a later gather's frame, into this one, for a packer that fell behind
    /// and finds two frames: a row in `newer` replaces the same (key, component) here,
    /// `newer`'s structural entries follow this frame's, `tick` becomes `newer.tick`, and
    /// a full `newer` makes the result full with `newer`'s rows alone. Nothing is lost, and
    /// the result never holds two values of one row.
    pub fn merge(&mut self, newer: Self) {
        todo!()
    }

    /// Every row, in ascending (region, key, component) order.
    #[must_use]
    pub fn rows(&self) -> GatheredRows<'_> {
        debug_assert!(self.finished, "a frame is read after `finish`");
        GatheredRows { entries: self.rows.iter(), bytes: &self.bytes }
    }

    /// The rows residing in `region`, in ascending (key, component) order: the region's
    /// dirty list, which every client to whom the region is relevant reads.
    #[must_use]
    pub fn region(&self, region: RegionId) -> GatheredRows<'_> {
        todo!()
    }

    /// The structural log entries since the previous gather, in log order.
    #[must_use]
    pub fn structural(&self) -> &[LogEntry] {
        &self.structural
    }

    /// How many rows the frame holds.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Whether the frame holds no row and no structural entry and is not full.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.full && self.rows.is_empty() && self.structural.is_empty()
    }
}

/// One gathered row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GatheredRow<'a> {
    /// The region the entity resides in.
    pub region: RegionId,
    /// The entity's origin key.
    pub key: StableKey,
    /// The component.
    pub component: ComponentKey,
    /// The tick the row last changed at.
    pub tick: Tick,
    /// The component's bytes, as laid out in memory.
    pub bytes: &'a [u8],
}

/// Iterates gathered rows in ascending (region, key, component) order.
pub struct GatheredRows<'a> {
    entries: core::slice::Iter<'a, RowEntry>,
    bytes: &'a [u8],
}

impl<'a> Iterator for GatheredRows<'a> {
    type Item = GatheredRow<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = self.entries.next()?;
        Some(GatheredRow {
            region: entry.region,
            key: entry.key,
            component: entry.component,
            tick: entry.tick,
            bytes: &self.bytes[entry.start as usize..(entry.start + entry.len) as usize],
        })
    }
}

/// The gather's state beside the world: its change mark and log cursor, and the frame
/// under construction. A service resource that [`setup`](crate::setup) inserts and the
/// `net::gather` system takes as `ResMut<GatherState>`; `app` reaches it through
/// `Simulation::world_mut().service_mut::<GatherState>()` between runs to
/// [`take`](Self::take) the frame.
#[derive(Debug)]
pub struct GatherState {
    mark: Option<ChangeMark>,
    cursor: Option<LogCursor>,
    frame: StagingFrame,
}

impl GatherState {
    /// A gather that has not run: its first run is a full pass.
    #[must_use]
    pub fn new() -> Self {
        Self { mark: None, cursor: None, frame: StagingFrame::new(Tick(0)) }
    }

    /// One gather over `world`. It first decides full or incremental: with no mark, a
    /// stale mark (`World::is_current(mark)` false: a whole-world load or the change-stamp
    /// rebase) or a stale cursor (`StructuralLog::since` on a copy of the cursor returning
    /// `None`), the gather is a full pass and every row of every replicated column goes
    /// into the frame, `full` set and `structural` empty. Otherwise, incrementally: for
    /// every registered type whose tracking is `replicated` and whose role is
    /// `Component`, in ascending key order, every chunk of `World::columns_erased` whose
    /// column tick is not before the mark's tick is kept (the skip is by tick, before any
    /// fork, and costs nothing per row; only an incremental gather skips); the kept chunks
    /// are forked over inside `job`'s scope with `fork_range`; in each, the rows
    /// `ErasedColumn::changed_since(mark)` selects, by stamp and exactly as the typed
    /// `World::changed_since` would, are pushed into the frame with their region, key,
    /// row tick and bytes; then the structural log since the cursor is appended. Either
    /// way the frame is finished and a fresh mark and cursor are taken. The frame is
    /// merged into whatever `app` has not taken, so a frame not taken loses nothing.
    /// Keyed by `StableKey` throughout: the result is the same under every chunk layout
    /// and worker count, and a gather with nothing changed since the last is empty.
    pub fn gather(&mut self, world: &World, job: &mut Job<'_, '_>) -> Result<()> {
        todo!()
    }

    /// Takes the frame gathered since the last take, leaving an empty frame at the same
    /// tick. `app` calls it between runs and hands the frame to the packer, which owns and
    /// drops it; nothing else clears the frame.
    pub fn take(&mut self) -> StagingFrame {
        let tick = self.frame.tick;
        core::mem::replace(&mut self.frame, StagingFrame::new(tick))
    }

    /// The frame gathered since the last take.
    #[must_use]
    pub fn pending(&self) -> &StagingFrame {
        &self.frame
    }
}

impl Default for GatherState {
    fn default() -> Self {
        Self::new()
    }
}

/// The replication gather of architecture section 9.3, placed in `Pass::Gather` on the
/// server by [`register_systems`](crate::register_systems): [`GatherState::gather`] over
/// the world.
#[system(name = "net::gather", worlds = [server], exclusive)]
pub(crate) fn gather(world: &mut World, job: Job, state: ResMut<GatherState>) {
    todo!()
}
