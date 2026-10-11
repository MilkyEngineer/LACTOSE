//! The replication gather (architecture section 9.3): the exclusive pass system in
//! `Pass::Gather` that copies every replicated row whose tick advanced, and the structural
//! log since its cursor, into a [`StagingFrame`] that per-client packing reads off the
//! critical path, never against live chunks. [`GatherState`] is the service resource that
//! holds the gather's mark, cursor and the frame under construction; `app` takes the frame
//! between runs and hands it to the packer.
//!
//! Boundary code may use this module: it reads the world through `columns_erased` and the
//! structural log, keys every row by `StableKey`, and computes nothing from a float.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use lactose_core::{ComponentKey, Result, StableKey, Tick};
use lactose_ecs::{
    ChangeMark, ErasedColumn, Job, LogCursor, LogEntry, RegionId, ResMut, StructuralChange, World,
    system,
};
use lactose_reflect::TypeRole;

use crate::staged::after;
use crate::staged::append;

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
        let (start, len) = append(&mut self.bytes, bytes);
        self.rows.push(RowEntry { region, key, component, tick, start, len });
        self.finished = false;
    }

    /// Appends a structural log entry, in log order.
    pub fn push_structural(&mut self, entry: LogEntry) {
        self.structural.push(entry);
    }

    /// Sorts the rows into (region, key, component) order and, where one (key,
    /// component) was pushed more than once, keeps the row with the newest tick (ties by
    /// the later push). Idempotent.
    pub fn finish(&mut self) {
        if self.finished {
            return;
        }
        // A stable sort by (key, component) leaves each pair's pushes in push order, so the
        // later of two equal ticks is met last and kept.
        let mut order: Vec<RowEntry> = self.rows.clone();
        order.sort_by_key(|row| (row.key, row.component));
        let mut kept: Vec<RowEntry> = Vec::with_capacity(order.len());
        for row in order {
            match kept.last_mut() {
                Some(last) if (last.key, last.component) == (row.key, row.component) => {
                    if !after(last.tick, row.tick) {
                        *last = row;
                    }
                }
                _ => kept.push(row),
            }
        }
        kept.sort_unstable_by_key(|row| (row.region, row.key, row.component));
        // The arena is rebuilt in row order, so a finished frame's representation is a
        // function of its rows alone and equal frames compare equal.
        let mut bytes = Vec::with_capacity(kept.iter().map(|row| row.len as usize).sum());
        for row in &mut kept {
            let old = &self.bytes[row.start as usize..(row.start + row.len) as usize];
            (row.start, row.len) = append(&mut bytes, old);
        }
        self.rows = kept;
        self.bytes = bytes;
        self.finished = true;
    }

    /// Folds `newer`, a later gather's frame, into this one, for a packer that fell behind
    /// and finds two frames: a row in `newer` replaces the same (key, component) here,
    /// `newer`'s structural entries follow this frame's, `tick` becomes `newer.tick`, and
    /// a full `newer` makes the result full with `newer`'s rows alone. `newer`'s
    /// structural entries also apply to the rows here: a row of an entity it despawned or
    /// of a component it removed is dropped, and a row of an entity it moved resides in the
    /// entity's new region. A full frame here stays full and keeps no structural entry.
    /// Nothing is lost, the result never holds two values of one row, and it is the frame
    /// one gather over both windows produces.
    pub fn merge(&mut self, mut newer: Self) {
        if newer.full {
            newer.finish();
            *self = newer;
            return;
        }
        // The rows here predate every structural entry of `newer`: a row of an entity it
        // despawned, or of a component it removed, goes, and a row of an entity it moved
        // resides where the entity does now, so the result is what one gather over both
        // windows holds whichever order the packer reads structure and rows in.
        let mut despawned = BTreeSet::new();
        let mut removed = BTreeSet::new();
        let mut moved = BTreeMap::new();
        for entry in &newer.structural {
            match entry.change {
                StructuralChange::Despawned { key, .. } => {
                    despawned.insert(key);
                }
                StructuralChange::Removed { key, component } => {
                    removed.insert((key, component));
                }
                StructuralChange::Moved { key, to, .. } => {
                    moved.insert(key, to);
                }
                StructuralChange::Spawned { .. } | StructuralChange::Added { .. } => {}
            }
        }
        if !(despawned.is_empty() && removed.is_empty() && moved.is_empty()) {
            self.rows.retain(|row| {
                !despawned.contains(&row.key) && !removed.contains(&(row.key, row.component))
            });
            for row in &mut self.rows {
                if let Some(&to) = moved.get(&row.key) {
                    row.region = to;
                }
            }
            self.finished = false;
        }
        if !self.full {
            self.structural.append(&mut newer.structural);
        }
        self.tick = newer.tick;
        self.extend_rows(&newer);
        self.finish();
    }

    /// Appends every row of `other` after this frame's, in `other`'s order.
    fn extend_rows(&mut self, other: &Self) {
        if other.rows.is_empty() {
            return;
        }
        let base = append(&mut self.bytes, &other.bytes).0;
        self.rows.extend(other.rows.iter().map(|row| RowEntry { start: base + row.start, ..*row }));
        self.finished = false;
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
        debug_assert!(self.finished, "a frame is read after `finish`");
        let start = self.rows.partition_point(|row| row.region < region);
        let end = start + self.rows[start..].partition_point(|row| row.region == region);
        GatheredRows { entries: self.rows[start..end].iter(), bytes: &self.bytes }
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
        let log = world.structural_log();
        // Decided before any row is read: a stale mark or cursor makes this a full pass.
        let mark = self.mark.filter(|&mark| world.is_current(mark));
        let mut cursor = self.cursor;
        let entries = cursor.as_mut().and_then(|cursor| log.since(cursor));
        let (mark, entries) = match (mark, entries) {
            (Some(mark), Some(entries)) => (Some(mark), Some(entries)),
            _ => (None, None),
        };
        let mut frame = StagingFrame::new(world.tick());
        frame.full = mark.is_none();

        let mut columns = Vec::new();
        for info in world.registry().iter() {
            if info.role() != TypeRole::Component || !info.tracking().replicated {
                continue;
            }
            columns.extend(world.columns_erased(info.key()).filter(|column| {
                mark.is_none_or(|mark| !tick_before(column.column_tick(), mark.tick()))
            }));
        }
        // One cell per kept chunk, each set by the one child whose range holds it; the
        // cells are read back in chunk order and `finish` sorts by key, so neither the
        // order the children ran in nor the chunk layout reaches the frame.
        let cells: Vec<OnceLock<StagingFrame>> = columns.iter().map(|_| OnceLock::new()).collect();
        let units = u32::try_from(columns.len()).expect("fewer than 2^32 chunks");
        job.context().scope(|scope| {
            scope.fork_range(0..units, |range, _| {
                for unit in range {
                    let index = unit as usize;
                    let partial = gather_column(&columns[index], mark);
                    assert!(cells[index].set(partial).is_ok(), "each chunk is gathered once");
                }
            });
        });
        for cell in cells {
            let partial = cell.into_inner().expect("every kept chunk was gathered");
            frame.extend_rows(&partial);
        }
        for entry in entries.into_iter().flatten() {
            frame.push_structural(*entry);
        }
        frame.finish();

        self.mark = Some(world.mark());
        self.cursor = Some(log.cursor());
        self.frame.merge(frame);
        Ok(())
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
    let world: &World = world;
    state.gather(world, &mut job).expect("the gather reads a world it was given whole");
}

/// The rows of one chunk's column the gather takes: those `changed_since(mark)` selects,
/// or every row on a full pass (`mark` `None`).
fn gather_column(column: &ErasedColumn<'_>, mark: Option<ChangeMark>) -> StagingFrame {
    let mut partial = StagingFrame::new(Tick(0));
    let size = column.element_size();
    let mut push = |row: usize| {
        partial.push_row(
            column.regions()[row],
            column.keys()[row],
            column.component(),
            column.row_ticks().expect("a replicated column is tracked")[row],
            &column.bytes()[row * size..][..size],
        );
    };
    match mark {
        Some(mark) => {
            column.changed_since(mark).expect("the mark is current").for_each(&mut push);
        }
        None => (0..column.keys().len()).for_each(&mut push),
    }
    partial
}

/// Whether tick `tick` is before tick `than`, across the wrap, as the world orders change
/// ticks.
const fn tick_before(tick: Tick, than: Tick) -> bool {
    tick.0.wrapping_sub(than.0) >= 1 << 31
}

#[cfg(test)]
mod tests {
    use super::*;
    use lactose_core::NetId;
    use lactose_ecs::{Executor, RegionConfig, RegionMode, WorldKind};
    use lactose_reflect::component;
    use lactose_sim::{RegionControl, Simulation, SimulationBuilder, TickInput, TypeTable};

    #[component(key = "net_api_test::value", persistent, replicated)]
    struct Value(#[field(id = 1)] u64);

    fn key(n: u64) -> StableKey {
        StableKey::Net(NetId::cooked(n))
    }

    fn passes(executor: &Executor, simulation: Simulation) -> Simulation {
        executor.block_on(simulation.run_passes(executor)).unwrap()
    }

    fn state(simulation: &mut Simulation) -> &mut GatherState {
        simulation.world_mut().service_mut::<GatherState>().unwrap()
    }

    /// A server that has gathered twice, the second time incrementally, with one row
    /// written since.
    fn gathered() -> (Executor, Simulation) {
        let executor = Executor::builder().frame_workers(2).background_workers(1).build().unwrap();
        let mut types = TypeTable::new();
        types.register::<Value>().unwrap();
        let mut simulation = SimulationBuilder::new(WorldKind::Server, types)
            .regions(RegionControl::Directed)
            .setup(crate::setup)
            .systems(crate::register_systems)
            .build()
            .unwrap();
        let world = simulation.world_mut();
        let config = RegionConfig { id: RegionId(1), seed: 1, mode: RegionMode::Simulated };
        world.create_region(config).unwrap();
        for n in 1..=3 {
            world.spawn(key(n), RegionId(1), (Value(n),)).unwrap();
        }
        simulation = executor.block_on(simulation.step(&executor, TickInput::default())).unwrap();
        simulation = passes(&executor, simulation);
        assert!(state(&mut simulation).take().full);
        simulation.world_mut().insert(key(2), Value(20)).unwrap();
        (executor, simulation)
    }

    #[test]
    fn a_stale_mark_with_a_current_cursor_is_a_full_pass_with_every_row() {
        let (executor, mut simulation) = gathered();
        // Every epoch change also resets the log, so the public operations only ever make
        // both stale at once; the pair is set here as the decision must handle it.
        let stale = simulation.world().mark();
        let next = Tick(simulation.tick().0 + 1);
        simulation.world_mut().set_tick(next).unwrap();
        let cursor = simulation.world().structural_log().cursor();
        assert!(!simulation.world().is_current(stale));
        let gather = state(&mut simulation);
        gather.mark = Some(stale);
        gather.cursor = Some(cursor);
        simulation = passes(&executor, simulation);
        let frame = state(&mut simulation).take();
        assert!(frame.full);
        assert!(frame.structural().is_empty());
        let rows: Vec<_> = frame.rows().map(|row| (row.key, row.bytes.to_vec())).collect();
        let expected: Vec<_> = [(1, 1), (2, 20), (3, 3)]
            .into_iter()
            .map(|(n, value): (u64, u64)| (key(n), value.to_ne_bytes().to_vec()))
            .collect();
        assert_eq!(rows, expected);
    }

    #[test]
    fn a_current_mark_and_cursor_gather_incrementally() {
        let (executor, mut simulation) = gathered();
        simulation = passes(&executor, simulation);
        let frame = state(&mut simulation).take();
        assert!(!frame.full);
        assert_eq!(frame.rows().map(|row| row.key).collect::<Vec<_>>(), vec![key(2)]);
    }
}
