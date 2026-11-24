//! The batch's builder: what `prepare` does with a sealed region's entries (`README.md`,
//! "Slots", "Writers" and "The batch"). It applies the passes to the slot table kind by
//! kind, decides each entry's base flags on the host, resolves duplicates, and splits the
//! runs into dispatches.

use lactose_asset_api::AssetRef;
use lactose_core::Tick;
use lactose_ecs::WorldKind;
use lactose_present_api::MeshAsset;

use crate::key::KeyBits;
use crate::scatter::{BatchRange, ScatterEntry};
use crate::slots::{Slot, SlotHolder, SlotTable};
use crate::staging::{Counts, Entry, KINDS, Note, kind_index};

/// What the renderer keeps of one kind's passes between frames, for the base rule.
///
/// An *epoch* is the span of a kind's passes at one tick: it advances at each `begin` whose
/// tick is not the tick of the `begin` before it for that kind, and at the first `begin`
/// after the kind's tick was forgotten. A record's base counts in the epoch it was taken
/// in, which is "that tick and that count" of present-api, since a `begin` that drops the
/// bases opens an epoch of its own.
#[derive(Clone, Copy, Debug, Default)]
struct KindState {
    /// The tick of the last `begin` applied, or `None` before the first and after a loss
    /// or a torn frame.
    last: Option<Tick>,
    /// The current epoch. It never goes back, so a stale epoch never matches.
    epoch: u64,
    /// The epoch opened by the last `begin` that dropped the bases, if any.
    dropped: Option<u64>,
}

impl KindState {
    /// Applies a `begin` at `tick`.
    fn begin(&mut self, tick: Tick) {
        match self.last {
            Some(last) if last == tick => {}
            Some(last) => {
                self.epoch += 1;
                // Earlier as present-api's `tick_before` has it: the difference from `last`
                // is at least 2^31, so a tick exactly 2^31 back is earlier.
                if tick.0.wrapping_sub(last.0) >= 1 << 31 {
                    self.dropped = Some(self.epoch);
                }
            }
            None => self.epoch += 1,
        }
        self.last = Some(tick);
    }

    /// The flags of an entry for a held instance written in the current epoch, taking its
    /// base where this is its first write in the epoch.
    fn flags(&self, born: u64, base: &mut Option<u64>) -> u32 {
        if self.dropped == Some(self.epoch) || born == self.epoch {
            0
        } else if *base == Some(self.epoch) {
            ScatterEntry::HAS_BASE
        } else {
            *base = Some(self.epoch);
            ScatterEntry::TAKE_BASE | ScatterEntry::HAS_BASE
        }
    }
}

/// One row of the sealed region.
#[derive(Clone, Copy, Debug)]
struct RowRef {
    /// Its pass's or batch's id.
    pass: u32,
    key: KeyBits,
    /// Its index in the region.
    index: u32,
    mesh: AssetRef<MeshAsset>,
}

/// One pass of the sealed region.
#[derive(Clone, Copy, Debug)]
struct PassRef {
    /// The index of its `begin`.
    id: u32,
    kind: WorldKind,
    tick: Tick,
    full: bool,
    ended: bool,
}

/// What one `prepare` counted, before it saturates into `FrameStats`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Built {
    pub(crate) pass_rows: u64,
    pub(crate) remote_rows: u64,
    pub(crate) duplicates: u64,
    pub(crate) unplaced: u64,
    pub(crate) unplaced_by_kind: [u64; 3],
    pub(crate) allocated: u64,
    pub(crate) freed: u64,
    /// The kinds lost, as bits of `kind_index`.
    pub(crate) lost: u8,
}

/// Per slot index, the run that last did each thing to it. Runs are numbered from one
/// and never reused, so the stamps need no clearing.
#[derive(Clone, Copy, Debug, Default)]
struct Stamp {
    written: u64,
    freed: u64,
    given: u64,
    named: u64,
}

/// The builder: the kinds' base state, and the scratch every `prepare` reuses.
pub(crate) struct Builder {
    kinds: [KindState; 3],
    /// Whether the next batch begins by freeing every slot index ever given.
    after_torn: bool,
    /// The most entries one dispatch runs.
    per_dispatch: u32,
    run: u64,
    stamps: Vec<Stamp>,
    passes: Vec<PassRef>,
    batches: Vec<(u32, bool)>,
    removed: Vec<(u32, KeyBits)>,
    rows: Vec<RowRef>,
    fresh: Vec<RowRef>,
    freed: Vec<u32>,
}

impl Builder {
    /// A builder whose dispatches run at most `per_dispatch` entries, at least one.
    pub(crate) const fn new(per_dispatch: u32) -> Self {
        Self {
            kinds: [KindState { last: None, epoch: 0, dropped: None }; 3],
            after_torn: false,
            per_dispatch,
            run: 0,
            stamps: Vec::new(),
            passes: Vec::new(),
            batches: Vec::new(),
            removed: Vec::new(),
            rows: Vec::new(),
            fresh: Vec::new(),
            freed: Vec::new(),
        }
    }

    /// What `torn` does to the builder: every kind's tick is forgotten, and the next
    /// batch begins by freeing every slot index ever given.
    pub(crate) fn torn(&mut self) {
        for kind in &mut self.kinds {
            kind.last = None;
        }
        self.after_torn = true;
    }

    /// Builds the frame's batch into `entries` and `dispatches` from a sealed region's
    /// `region` entries and `counts`, applying it to `table`.
    ///
    /// # Panics
    ///
    /// If a run names a slot twice: the builder's own assertion, in every build.
    pub(crate) fn build(
        &mut self,
        table: &mut SlotTable,
        region: impl Iterator<Item = Entry>,
        counts: &Counts,
        entries: &mut Vec<ScatterEntry>,
        dispatches: &mut Vec<BatchRange>,
    ) -> Built {
        entries.clear();
        dispatches.clear();
        let mut built = Built { lost: counts.lost, ..Built::default() };
        let last_client_note = self.read(region);
        for pass in &self.passes {
            if !pass.ended {
                built.lost |= 1 << kind_index(pass.kind);
            }
        }
        for row in &self.rows {
            if self.passes.binary_search_by_key(&row.pass, |p| p.id).is_ok() {
                built.pass_rows += 1;
            } else {
                built.remote_rows += 1;
            }
        }
        // Each pass's rows together, in ascending key order, a key's rows in region order.
        self.rows.sort_unstable_by_key(|row| (row.pass, row.key, row.index));
        self.removed.sort_by_key(|&(pass, _)| pass);

        if core::mem::take(&mut self.after_torn) {
            self.open_run();
            for slot in 0..table.given() {
                entries.push(ScatterEntry {
                    slot: Slot::new(slot),
                    row: 0,
                    flags: ScatterEntry::FREE,
                });
            }
            self.close_run(entries, dispatches);
        }

        self.open_run();
        for kind in KINDS {
            if built.lost & 1 << kind_index(kind) == 0 {
                continue;
            }
            self.kinds[kind_index(kind)].last = None;
            let held: Vec<u32> = table
                .iter()
                .filter(|(_, holder)| holder.kind == kind)
                .map(|(slot, _)| slot.index())
                .collect();
            for slot in held {
                table.release(slot);
                built.freed += 1;
                entries.push(ScatterEntry {
                    slot: Slot::new(slot),
                    row: 0,
                    flags: ScatterEntry::FREE,
                });
            }
        }
        self.close_run(entries, dispatches);

        let passes = core::mem::take(&mut self.passes);
        let rows = core::mem::take(&mut self.rows);
        let removed = core::mem::take(&mut self.removed);
        for kind in KINDS {
            if built.lost & 1 << kind_index(kind) != 0 {
                continue;
            }
            for pass in passes.iter().filter(|pass| pass.kind == kind) {
                let removed = of_pass(&removed, pass.id, |&(id, _)| id);
                let rows = of_pass(&rows, pass.id, |row| row.pass);
                self.kinds[kind_index(kind)].begin(pass.tick);
                let removed = removed.iter().map(|&(_, key)| key);
                self.apply(table, kind, false, pass.full, removed, rows, &mut built, entries);
                self.close_run(entries, dispatches);
            }
            if kind != WorldKind::Client {
                continue;
            }
            // The region's last batch, where it is whole and no `Client` note follows its
            // opening.
            if let Some(&(id, ended)) = self.batches.last()
                && ended
                && last_client_note.is_none_or(|note| note < id)
            {
                let rows = of_pass(&rows, id, |row| row.pass);
                self.apply(table, kind, true, false, [].into_iter(), rows, &mut built, entries);
                self.close_run(entries, dispatches);
            }
        }
        self.passes = passes;
        self.rows = rows;
        self.removed = removed;
        built
    }

    /// Reads the region's entries into the builder's lists: the passes and the batches in
    /// index order, their removed keys and rows. Returns the index of the last note of a
    /// `Client` pass, if any.
    fn read(&mut self, region: impl Iterator<Item = Entry>) -> Option<u32> {
        self.passes.clear();
        self.batches.clear();
        self.removed.clear();
        self.rows.clear();
        let mut last_client_note = None;
        for Entry { index, pass, note } in region {
            let of_client = |passes: &[PassRef]| {
                passes
                    .binary_search_by_key(&pass, |p| p.id)
                    .is_ok_and(|at| passes[at].kind == WorldKind::Client)
            };
            match note {
                Note::Begin { kind, tick, full } => {
                    self.passes.push(PassRef { id: index, kind, tick, full, ended: false });
                    if kind == WorldKind::Client {
                        last_client_note = Some(index);
                    }
                }
                Note::End => {
                    if let Ok(at) = self.passes.binary_search_by_key(&pass, |p| p.id) {
                        self.passes[at].ended = true;
                        if self.passes[at].kind == WorldKind::Client {
                            last_client_note = Some(index);
                        }
                    }
                }
                Note::Removed(key) => {
                    if of_client(&self.passes) {
                        last_client_note = Some(index);
                    }
                    self.removed.push((pass, key));
                }
                Note::Row { key, mesh } => self.rows.push(RowRef { pass, key, index, mesh }),
                Note::RemoteOpen => self.batches.push((index, false)),
                Note::RemoteEnd => {
                    if let Ok(at) = self.batches.binary_search_by_key(&pass, |&(id, _)| id) {
                        self.batches[at].1 = true;
                    }
                }
            }
        }
        // Rows of a batch that is not the region's last, or of no pass or batch, stay in
        // `rows` to be counted and are applied by no run.
        last_client_note
    }

    fn open_run(&mut self) {
        self.run += 1;
    }

    /// Applies one pass or remote batch of `kind` as a run, after its `begin` was applied:
    /// its removed keys, then its rows, then a full pass's sweep, then its new instances.
    #[expect(clippy::too_many_arguments, reason = "one step of `build`, which owns them all")]
    fn apply(
        &mut self,
        table: &mut SlotTable,
        kind: WorldKind,
        remote: bool,
        full: bool,
        removed: impl Iterator<Item = KeyBits>,
        rows: &[RowRef],
        built: &mut Built,
        entries: &mut Vec<ScatterEntry>,
    ) {
        self.open_run();
        let run = self.run;
        self.freed.clear();
        self.fresh.clear();
        for key in removed {
            if let Some(slot) = table.lookup(kind, key) {
                table.release(slot);
                self.stamp(slot).freed = run;
                self.freed.push(slot);
            }
        }
        let state = self.kinds[kind_index(kind)];
        for group in rows.chunk_by(|a, b| a.key == b.key) {
            // Of the rows that name one instance, the one later in the region is kept.
            let row = group[group.len() - 1];
            built.duplicates += group.len() as u64 - 1;
            let Some(slot) = table.lookup(kind, row.key) else {
                self.fresh.push(row);
                continue;
            };
            self.stamp(slot).written = run;
            let record = table.record_mut(slot);
            if let Some(holder) = &mut record.holder {
                holder.mesh = row.mesh;
            }
            let flags = if remote { 0 } else { state.flags(record.born, &mut record.base) };
            entries.push(ScatterEntry { slot: Slot::new(slot), row: row.index, flags });
        }
        if full {
            for slot in 0..table.given() {
                let held = table.records()[slot as usize].holder.is_some_and(|h| h.kind == kind);
                if held && self.stamp(slot).written != run {
                    table.release(slot);
                    self.stamp(slot).freed = run;
                    self.freed.push(slot);
                }
            }
        }
        for index in 0..self.fresh.len() {
            let row = self.fresh[index];
            let key = row.key.key().expect("a row's key comes back from its words");
            let holder = SlotHolder { kind, key, mesh: row.mesh };
            match table.allocate(holder, state.epoch) {
                Some(slot) => {
                    built.allocated += 1;
                    self.stamp(slot).given = run;
                    entries.push(ScatterEntry { slot: Slot::new(slot), row: row.index, flags: 0 });
                }
                None => {
                    built.unplaced += 1;
                    if !remote {
                        built.unplaced_by_kind[kind_index(kind)] += 1;
                    }
                }
            }
        }
        built.freed += self.freed.len() as u64;
        for index in 0..self.freed.len() {
            let slot = self.freed[index];
            if self.stamp(slot).given != run {
                entries.push(ScatterEntry {
                    slot: Slot::new(slot),
                    row: 0,
                    flags: ScatterEntry::FREE,
                });
            }
        }
    }

    /// Ends the run that began at the last dispatch's end: asserts that it names each slot
    /// once, then splits it into dispatches.
    fn close_run(&mut self, entries: &[ScatterEntry], dispatches: &mut Vec<BatchRange>) {
        let run = self.run;
        let first = dispatches.last().map_or(0, |last| last.first + last.count);
        for entry in &entries[first as usize..] {
            let stamp = self.stamp(entry.slot.index());
            assert!(stamp.named != run, "a run of the batch names slot {:?} twice", entry.slot);
            stamp.named = run;
        }
        // At most `instances + 2 * rows` entries, which `Renderer::new` refuses past
        // `u32::MAX`, so the `expect` is an assertion that cannot fail.
        let mut at = first;
        let end = u32::try_from(entries.len()).expect("`new` bounds a batch at `u32::MAX`");
        while at < end {
            let count = (end - at).min(self.per_dispatch);
            dispatches.push(BatchRange { first: at, count });
            at += count;
        }
    }

    fn stamp(&mut self, slot: u32) -> &mut Stamp {
        let slot = slot as usize;
        if slot >= self.stamps.len() {
            self.stamps.resize(slot + 1, Stamp::default());
        }
        &mut self.stamps[slot]
    }
}

/// The items of `pass` in `items`, which are sorted by `id`.
fn of_pass<T>(items: &[T], pass: u32, id: impl Fn(&T) -> u32) -> &[T] {
    let start = items.partition_point(|item| id(item) < pass);
    let end = items.partition_point(|item| id(item) <= pass);
    &items[start..end]
}

#[cfg(all(test, not(feature = "loom")))]
mod tests {
    use super::*;

    /// A run longer than a dispatch is split into dispatches of `per_dispatch` entries and
    /// one of the rest, each beginning where the one before it ended.
    #[test]
    fn a_run_of_130_entries_is_split_into_dispatches_of_64_64_and_2() {
        let mut builder = Builder::new(64);
        builder.run = 1;
        let entries: Vec<ScatterEntry> = (0..130)
            .map(|index| ScatterEntry { slot: Slot::new(index), row: 0, flags: ScatterEntry::FREE })
            .collect();
        let mut dispatches = Vec::new();
        builder.close_run(&entries, &mut dispatches);
        let split: Vec<(u32, u32)> = dispatches.iter().map(|d| (d.first, d.count)).collect();
        assert_eq!(split, [(0, 64), (64, 64), (128, 2)]);
    }
}
