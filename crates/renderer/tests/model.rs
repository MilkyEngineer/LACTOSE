//! Random sequences of passes, remote batches, losses, writes with no pass open, refused
//! and mid-frame prepares and torn frames through the renderer on null without shaders,
//! against a model written from the README's "Slots", "Writers", "Regions" and "The batch",
//! and from present-api's eager sink ("What the sink keeps", with present-api's own
//! `tick_before`). After each `prepare` the batch is scattered on the host over records
//! that start poisoned (LIVE, NaN placements) and are poisoned again at every torn frame,
//! and the test checks the table, the stats, the drawn records and the batch's bound.
//!
//! Ticks step forward and back, by up to 2^31 - 1 either way and by exactly 2^31 back.
//! `MODEL_SEEDS=a..b` picks other seeds than the default 2,000, the count the reviewer's probe
//! it was adopted from ran (renderer-impl round 1).

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU64;

use lactose_asset_api::{AssetId, AssetRef};
use lactose_core::{NetId, PlayerId, PredictionKey, StableKey, Tick};
use lactose_ecs::WorldKind;
use lactose_math::render::RenderPosition;
use lactose_present_api::{ExtractPass, InstanceRow, InstanceSink, RenderTransform};
use lactose_renderer::shader::SCATTER_GROUP;
use lactose_renderer::{
    ExtractSink, FrameStats, InstanceRecord, Opened, RemoteWriter, Renderer, RendererDesc,
    ScatterEntry, StagingRow,
};

use crate::common::{Ender, PATIENCE, null, ok};

const KINDS: [WorldKind; 3] = [WorldKind::Server, WorldKind::Client, WorldKind::Prediction];

fn ki(kind: WorldKind) -> usize {
    match kind {
        WorldKind::Server => 0,
        WorldKind::Client => 1,
        WorldKind::Prediction => 2,
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn pct(&mut self, p: u64) -> bool {
        self.below(100) < p
    }
}

/// Key `n` of the universe, over the three variants so that the allocation order is
/// `StableKey`'s across them.
fn the_key(n: u64) -> StableKey {
    match n % 3 {
        0 => StableKey::Net(NetId::runtime(NonZeroU64::new(n + 1).unwrap()).unwrap()),
        1 => StableKey::Predicted(PredictionKey {
            player: PlayerId((n % 5) as u32),
            input_tick: Tick((n * 7919) as u32),
            ordinal: (n * 31) as u32,
        }),
        _ => StableKey::Placeholder((n * 13) as u32),
    }
}

fn placement(serial: u32, x: f32) -> RenderTransform {
    RenderTransform {
        position: RenderPosition { cell: [serial as i32, 0, 0], offset: [x, 0.0, 0.0] },
        rotation: [0.0, 0.0, 0.0, 1.0],
    }
}

fn poison() -> InstanceRecord {
    let nan = RenderTransform {
        position: RenderPosition { cell: [-7; 3], offset: [f32::NAN; 3] },
        rotation: [f32::NAN; 4],
    };
    InstanceRecord { previous: nan, current: nan, base: nan, flags: InstanceRecord::LIVE }
}

/// present-api's `tick_before`.
fn before(tick: Tick, than: Tick) -> bool {
    tick.0.wrapping_sub(than.0) >= 1 << 31
}

#[derive(Clone, Copy, Debug)]
struct Inst {
    slot: u32,
    mesh: u64,
    previous: RenderTransform,
    current: RenderTransform,
    base: Option<RenderTransform>,
}

#[derive(Clone, Copy, Debug)]
enum Note {
    Begin { kind: WorldKind, tick: Tick, full: bool },
    End(u32),
    Removed(u32, StableKey),
    Row(u32, InstanceRow),
    Open,
    Close(u32),
}

#[derive(Clone, Copy, Default, Debug)]
struct Counts {
    lost: u8,
    dropped: u64,
    by_kind: [u64; 3],
}

impl Counts {
    fn lose(&mut self, kind: WorldKind) {
        self.lost |= 1 << ki(kind);
    }
    fn drop_rows(&mut self, n: u64, kind: Option<WorldKind>) {
        self.dropped += n;
        if let Some(kind) = kind {
            self.by_kind[ki(kind)] += n;
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct MPass {
    kind: WorldKind,
    at: bool,
    id: u32,
    lost: bool,
}

#[derive(Clone, Copy, Debug)]
struct MRemote {
    at: bool,
    opening: Option<u32>,
}

#[derive(Default, Debug, PartialEq, Eq, Clone, Copy)]
struct Expect {
    pass_rows: u64,
    remote_rows: u64,
    dropped: u64,
    dropped_in: [u64; 3],
    duplicates: u64,
    unplaced: u64,
    unplaced_in: [u64; 3],
    allocated: u64,
    freed: u64,
    entries: u64,
    live: u64,
    lost: u8,
}

struct Model {
    rows: u32,
    instances: u32,
    open: bool,
    pending: bool,
    cursor: u64,
    notes: Vec<(u32, Note)>,
    log: Vec<Option<InstanceRow>>,
    counts: Counts,
    wide: Counts,
    counted: u32,
    ticks: [Option<Tick>; 3],
    held: BTreeMap<(usize, StableKey), Inst>,
    free: BTreeSet<u32>,
    given: u32,
    torn: bool,
    seen_remote: core::cell::Cell<u64>,
    seen_regiven: core::cell::Cell<u64>,
}

impl Model {
    fn new(rows: u32, instances: u32) -> Self {
        let mut model = Self {
            rows,
            instances,
            open: false,
            pending: false,
            cursor: 0,
            notes: Vec::new(),
            log: Vec::new(),
            counts: Counts::default(),
            wide: Counts::default(),
            counted: 0,
            ticks: [None; 3],
            held: BTreeMap::new(),
            free: BTreeSet::new(),
            given: 0,
            torn: false,
            seen_remote: Default::default(),
            seen_regiven: Default::default(),
        };
        model.open_region();
        model
    }

    fn open_region(&mut self) {
        self.open = true;
        self.cursor = 0;
        self.notes.clear();
        self.log = vec![None; self.rows as usize];
        self.counts = Counts::default();
    }

    fn reserve(&mut self, count: u64) -> (u32, u32) {
        let start = self.cursor;
        self.cursor += count;
        let rows = u64::from(self.rows);
        if start >= rows {
            return (self.rows, 0);
        }
        (start as u32, (rows - start).min(count) as u32)
    }

    fn store(&mut self, index: u32, note: Note) {
        if let Note::Row(_, row) = note {
            self.log[index as usize] = Some(row);
        }
        self.notes.push((index, note));
    }

    fn abandon(&mut self, sink: &mut Option<MPass>) {
        if let Some(pass) = sink.take()
            && pass.at
        {
            self.counts.lose(pass.kind);
            self.counted -= 1;
        }
    }

    fn begin(&mut self, sink: &mut Option<MPass>, pass: ExtractPass) {
        self.abandon(sink);
        let lost = Some(MPass { kind: pass.kind, at: false, id: 0, lost: true });
        if !self.open {
            self.wide.lose(pass.kind);
            *sink = lost;
            return;
        }
        let (index, fit) = self.reserve(1);
        if fit == 0 {
            self.counts.lose(pass.kind);
            *sink = lost;
            return;
        }
        self.store(index, Note::Begin { kind: pass.kind, tick: pass.tick, full: pass.full });
        self.counted += 1;
        *sink = Some(MPass { kind: pass.kind, at: true, id: index, lost: false });
    }

    fn removed(&mut self, sink: &mut Option<MPass>, keys: &[StableKey]) {
        let Some(pass) = sink else { return };
        if !pass.at || pass.lost || keys.is_empty() {
            return;
        }
        let (start, fit) = self.reserve(keys.len() as u64);
        for (index, key) in (start..).zip(&keys[..fit as usize]) {
            self.store(index, Note::Removed(pass.id, *key));
        }
        if (fit as usize) < keys.len() {
            self.counts.lose(pass.kind);
            pass.lost = true;
        }
    }

    fn write(&mut self, sink: &Option<MPass>, rows: &[InstanceRow]) {
        let n = rows.len() as u64;
        match *sink {
            None => self.wide.drop_rows(n, None),
            Some(pass) if !pass.at => self.wide.drop_rows(n, Some(pass.kind)),
            Some(pass) => {
                let dropped = if pass.lost {
                    n
                } else {
                    let (start, fit) = self.reserve(n);
                    for (index, row) in (start..).zip(&rows[..fit as usize]) {
                        self.store(index, Note::Row(pass.id, *row));
                    }
                    n - u64::from(fit)
                };
                self.counts.drop_rows(dropped, Some(pass.kind));
            }
        }
    }

    fn end(&mut self, sink: &mut Option<MPass>) {
        let Some(pass) = sink.take() else { return };
        if !pass.at {
            return;
        }
        if !pass.lost {
            let (index, fit) = self.reserve(1);
            if fit == 0 {
                self.counts.lose(pass.kind);
            } else {
                self.store(index, Note::End(pass.id));
            }
        }
        self.counted -= 1;
    }

    fn remote_open(&mut self) -> MRemote {
        if !self.open {
            return MRemote { at: false, opening: None };
        }
        let (index, fit) = self.reserve(1);
        if fit == 0 {
            return MRemote { at: true, opening: None };
        }
        self.store(index, Note::Open);
        MRemote { at: true, opening: Some(index) }
    }

    fn remote_write(&mut self, remote: MRemote, rows: &[InstanceRow]) {
        let n = rows.len() as u64;
        match (remote.at, remote.opening) {
            (false, _) => self.wide.drop_rows(n, None),
            (true, None) => self.counts.drop_rows(n, None),
            (true, Some(opening)) => {
                let (start, fit) = self.reserve(n);
                for (index, row) in (start..).zip(&rows[..fit as usize]) {
                    self.store(index, Note::Row(opening, *row));
                }
                self.counts.drop_rows(n - u64::from(fit), None);
            }
        }
    }

    fn remote_close(&mut self, remote: MRemote) {
        if let (true, Some(opening)) = (remote.at, remote.opening) {
            let (index, fit) = self.reserve(1);
            if fit != 0 {
                self.store(index, Note::Close(opening));
            }
        }
    }

    fn refuses_prepare(&self) -> bool {
        self.pending || !self.open || self.counted != 0
    }

    /// The README's `prepare`, applied to the model; returns what the stats must say.
    fn prepare(&mut self) -> Expect {
        let mut out = Expect::default();
        self.open = false;
        self.pending = true;
        let mut counts = self.counts;
        let wide = core::mem::take(&mut self.wide);
        counts.lost |= wide.lost;
        counts.dropped += wide.dropped;
        for k in 0..3 {
            counts.by_kind[k] += wide.by_kind[k];
        }
        out.dropped = counts.dropped;
        out.dropped_in = counts.by_kind;
        // The passes, in begin order.
        let mut passes: Vec<(u32, WorldKind, Tick, bool, bool)> = Vec::new();
        for &(index, note) in &self.notes {
            if let Note::Begin { kind, tick, full } = note {
                passes.push((index, kind, tick, full, false));
            }
        }
        for &(_, note) in &self.notes {
            if let Note::End(id) = note
                && let Some(pass) = passes.iter_mut().find(|p| p.0 == id)
            {
                pass.4 = true;
            }
        }
        let mut lost = counts.lost;
        for pass in &passes {
            if !pass.4 {
                lost |= 1 << ki(pass.1);
            }
        }
        out.lost = lost;
        let kind_of = |id: u32| passes.iter().find(|p| p.0 == id).map(|p| p.1);
        let mut last_client_note = None;
        let mut openings = Vec::new();
        let mut closed = BTreeSet::new();
        for &(index, note) in &self.notes {
            match note {
                Note::Begin { kind: WorldKind::Client, .. } => last_client_note = Some(index),
                Note::End(id) | Note::Removed(id, _) if kind_of(id) == Some(WorldKind::Client) => {
                    last_client_note = Some(index);
                }
                Note::Row(id, _) => {
                    if kind_of(id).is_some() {
                        out.pass_rows += 1;
                    } else {
                        out.remote_rows += 1;
                    }
                }
                Note::Open => openings.push(index),
                Note::Close(opening) => {
                    closed.insert(opening);
                }
                _ => {}
            }
        }
        if core::mem::take(&mut self.torn) {
            out.entries += u64::from(self.given);
        }
        for kind in KINDS {
            if lost & 1 << ki(kind) == 0 {
                continue;
            }
            self.ticks[ki(kind)] = None;
            let gone: Vec<_> = self.held.keys().filter(|k| k.0 == ki(kind)).copied().collect();
            for key in gone {
                let inst = self.held.remove(&key).unwrap();
                self.free.insert(inst.slot);
                out.freed += 1;
                out.entries += 1;
            }
        }
        for kind in KINDS {
            if lost & 1 << ki(kind) != 0 {
                continue;
            }
            for &(id, _, tick, full, _) in passes.iter().filter(|p| p.1 == kind) {
                self.begin_rule(kind, tick);
                let removed: Vec<StableKey> = self
                    .notes
                    .iter()
                    .filter_map(|&(_, n)| match n {
                        Note::Removed(p, key) if p == id => Some(key),
                        _ => None,
                    })
                    .collect();
                let rows = self.rows_of(id);
                self.apply(kind, false, full, &removed, &rows, &mut out);
            }
            if kind == WorldKind::Client
                && let Some(&opening) = openings.last()
                && closed.contains(&opening)
                && last_client_note.is_none_or(|note| note < opening)
            {
                let rows = self.rows_of(opening);
                self.apply(kind, true, false, &[], &rows, &mut out);
            }
        }
        out.live = self.held.len() as u64;
        out
    }

    fn rows_of(&self, id: u32) -> Vec<InstanceRow> {
        self.notes
            .iter()
            .filter_map(|&(_, n)| match n {
                Note::Row(p, row) if p == id => Some(row),
                _ => None,
            })
            .collect()
    }

    /// present-api's eager sink at a `begin`.
    fn begin_rule(&mut self, kind: WorldKind, tick: Tick) {
        let k = ki(kind);
        match self.ticks[k] {
            Some(last) if last == tick => {}
            Some(last) if before(tick, last) => {
                for (_, inst) in self.held.iter_mut().filter(|(key, _)| key.0 == k) {
                    inst.base = None;
                }
            }
            _ => {
                for (_, inst) in self.held.iter_mut().filter(|(key, _)| key.0 == k) {
                    inst.base = Some(inst.current);
                }
            }
        }
        self.ticks[k] = Some(tick);
    }

    fn apply(
        &mut self,
        kind: WorldKind,
        remote: bool,
        full: bool,
        removed: &[StableKey],
        rows: &[InstanceRow],
        out: &mut Expect,
    ) {
        let k = ki(kind);
        self.seen_remote.set(self.seen_remote.get() + u64::from(remote));
        let mut freed = Vec::new();
        for key in removed {
            if let Some(inst) = self.held.remove(&(k, *key)) {
                self.free.insert(inst.slot);
                freed.push(inst.slot);
            }
        }
        let mut last: BTreeMap<StableKey, InstanceRow> = BTreeMap::new();
        for row in rows {
            if last.insert(row.key, *row).is_some() {
                out.duplicates += 1;
            }
        }
        let mut written = BTreeSet::new();
        let mut fresh = Vec::new();
        for (key, row) in &last {
            if let Some(inst) = self.held.get_mut(&(k, *key)) {
                inst.mesh = row.mesh.id().to_bits();
                match inst.base {
                    Some(base) if !remote && !row.no_interpolate => {
                        inst.previous = base;
                        inst.current = row.transform;
                    }
                    _ => {
                        inst.previous = row.transform;
                        inst.current = row.transform;
                    }
                }
                out.entries += 1;
                written.insert(*key);
            } else {
                fresh.push(*row);
            }
        }
        if full {
            let swept: Vec<_> = self
                .held
                .keys()
                .filter(|key| key.0 == k && !written.contains(&key.1))
                .copied()
                .collect();
            for key in swept {
                let inst = self.held.remove(&key).unwrap();
                self.free.insert(inst.slot);
                freed.push(inst.slot);
            }
        }
        let mut regiven = BTreeSet::new();
        for row in fresh {
            let slot = if let Some(slot) = self.free.pop_first() {
                slot
            } else if self.given < self.instances {
                self.given += 1;
                self.given - 1
            } else {
                out.unplaced += 1;
                if !remote {
                    out.unplaced_in[k] += 1;
                }
                continue;
            };
            let inst = Inst {
                slot,
                mesh: row.mesh.id().to_bits(),
                previous: row.transform,
                current: row.transform,
                base: None,
            };
            self.held.insert((k, row.key), inst);
            out.allocated += 1;
            out.entries += 1;
            if freed.contains(&slot) {
                regiven.insert(slot);
            }
        }
        self.seen_regiven.set(self.seen_regiven.get() + regiven.len() as u64);
        out.freed += freed.len() as u64;
        out.entries += freed.iter().filter(|slot| !regiven.contains(slot)).count() as u64;
    }

    fn torn(&mut self) {
        self.pending = false;
        for (_, inst) in core::mem::take(&mut self.held) {
            self.free.insert(inst.slot);
        }
        self.ticks = [None; 3];
        self.torn = true;
    }
}

fn stats_of(stats: &FrameStats) -> Expect {
    Expect {
        pass_rows: u64::from(stats.pass_rows),
        remote_rows: u64::from(stats.remote_rows),
        dropped: u64::from(stats.dropped),
        dropped_in: KINDS.map(|k| u64::from(stats.dropped_in(k))),
        duplicates: u64::from(stats.duplicates),
        unplaced: u64::from(stats.unplaced),
        unplaced_in: KINDS.map(|k| u64::from(stats.unplaced_in(k))),
        allocated: u64::from(stats.allocated),
        freed: u64::from(stats.freed),
        entries: u64::from(stats.entries),
        live: u64::from(stats.live),
        lost: KINDS.iter().fold(0, |bits, &k| if stats.lost(k) { bits | 1 << ki(k) } else { bits }),
    }
}

/// One operation of a frame's script.
#[derive(Clone, Debug)]
enum Op {
    Begin(ExtractPass),
    Removed(Vec<StableKey>),
    Write(Vec<InstanceRow>),
    End,
    /// The sink is dropped and replaced, with whatever pass it has open.
    Replace,
    RemoteOpen,
    RemoteWrite(Vec<InstanceRow>),
    RemoteClose,
    RemoteForget,
    /// A `prepare` called in the middle of the frame.
    Prepare,
}

struct Run {
    rng: Rng,
    serial: u32,
    universe: u64,
    ticks: [u32; 3],
}

impl Run {
    fn rows(&mut self, kind: WorldKind) -> Vec<InstanceRow> {
        let count = self.rng.below(self.universe + 1);
        let mut keys: Vec<u64> = (0..self.universe).collect();
        for at in (1..keys.len()).rev() {
            keys.swap(at, self.rng.below(at as u64 + 1) as usize);
        }
        keys.truncate(count as usize);
        if !keys.is_empty() && self.rng.pct(6) {
            let again = keys[self.rng.below(keys.len() as u64) as usize];
            keys.push(again);
        }
        keys.iter()
            .map(|&n| {
                self.serial += 1;
                InstanceRow {
                    key: the_key(n),
                    mesh: AssetRef::new(AssetId::from_bits(
                        u64::from(self.serial) * 3 + ki(kind) as u64,
                    )),
                    transform: placement(self.serial, n as f32),
                    no_interpolate: self.rng.pct(15),
                }
            })
            .collect()
    }

    fn split(&mut self, rows: Vec<InstanceRow>, op: fn(Vec<InstanceRow>) -> Op) -> Vec<Op> {
        let mut out = Vec::new();
        let mut rest = rows;
        while !rest.is_empty() {
            let take = 1 + self.rng.below(rest.len() as u64) as usize;
            let tail = rest.split_off(take);
            out.push(op(rest));
            rest = tail;
        }
        // Batches carry no order: shuffle them.
        for at in (1..out.len()).rev() {
            out.swap(at, self.rng.below(at as u64 + 1) as usize);
        }
        out
    }

    fn tick(&mut self, k: usize) -> Tick {
        let t = &mut self.ticks[k];
        let roll = self.rng.below(100);
        *t = match roll {
            0..25 => *t,
            25..70 => t.wrapping_add(1),
            70..80 => t.wrapping_add(2 + self.rng.below(4) as u32),
            80..92 => t.wrapping_sub(1 + self.rng.below(4) as u32),
            92..94 => t.wrapping_sub(1 << 31),
            94..95 => t.wrapping_sub((1 << 31) - 1),
            _ => t.wrapping_add((1 << 31) - 1),
        };
        Tick(*t)
    }

    fn pass(&mut self, kind: WorldKind) -> Vec<Op> {
        let tick = self.tick(ki(kind));
        let full = self.rng.pct(20);
        let mut ops = vec![Op::Begin(ExtractPass { kind, tick, full })];
        if self.rng.pct(20) {
            let keys = (0..1 + self.rng.below(3)).map(|_| the_key(self.rng.below(self.universe)));
            ops.push(Op::Removed(keys.collect()));
        }
        if kind != WorldKind::Client {
            let rows = self.rows(kind);
            ops.extend(self.split(rows, Op::Write));
        }
        if self.rng.pct(3) {
            ops.push(Op::Replace);
        } else {
            ops.push(Op::End);
        }
        ops
    }

    fn remote(&mut self) -> Vec<Op> {
        let rows = self.rows(WorldKind::Client);
        let mut ops = vec![Op::RemoteOpen];
        ops.extend(self.split(rows, Op::RemoteWrite));
        ops.push(if self.rng.pct(3) { Op::RemoteForget } else { Op::RemoteClose });
        ops
    }
}

fn check_records(
    renderer: &Renderer,
    model: &Model,
    records: &mut Vec<InstanceRecord>,
) -> Result<(), String> {
    let batch = renderer.batch();
    let mut next = 0_u64;
    // The README's "The batch": a dispatch is at most as many groups as the device's first
    // dimension allows, of `SCATTER_GROUP` entries each.
    let groups = renderer.device().capabilities().compute_groups[0].max(1);
    let cap = u64::from(groups.min(u32::MAX / SCATTER_GROUP) * SCATTER_GROUP);
    for &range in batch.dispatches() {
        if u64::from(range.first) != next || range.count == 0 || u64::from(range.count) > cap {
            return Err(format!("dispatch {range:?} after {next}, cap {cap}"));
        }
        next += u64::from(range.count);
    }
    if next != batch.entries().len() as u64 {
        return Err(format!("dispatches cover {next} of {} entries", batch.entries().len()));
    }
    for &range in batch.dispatches() {
        let mut seen = BTreeSet::new();
        for entry in batch.run(range) {
            let slot = entry.slot.index();
            if !seen.insert(slot) {
                return Err(format!("dispatch names slot {slot} twice"));
            }
            if records.len() <= slot as usize {
                records.resize(slot as usize + 1, poison());
            }
            let record = &mut records[slot as usize];
            if entry.flags & ScatterEntry::FREE != 0 {
                record.free();
            } else {
                let row = model.log[entry.row as usize]
                    .ok_or_else(|| format!("entry reads row {} which holds no row", entry.row))?;
                record.scatter(entry.flags, &StagingRow::of(&row));
            }
        }
    }
    let table = renderer.slots();
    let mut seen = BTreeMap::new();
    for (slot, holder) in table.iter() {
        seen.insert((ki(holder.kind), holder.key), (slot.index(), holder.mesh.id().to_bits()));
    }
    let want: BTreeMap<_, _> = model.held.iter().map(|(k, i)| (*k, (i.slot, i.mesh))).collect();
    if seen != want {
        return Err(format!("table differs:\n  renderer {seen:?}\n  model    {want:?}"));
    }
    let extent = model.held.values().map(|i| i.slot + 1).max().unwrap_or(0);
    if table.extent() != extent {
        return Err(format!("extent {} != model {extent}", table.extent()));
    }
    for slot in 0..extent {
        let record = records.get(slot as usize).copied().unwrap_or_else(poison);
        let inst = model.held.values().find(|i| i.slot == slot);
        match inst {
            None => {
                if record.flags & InstanceRecord::LIVE != 0 {
                    return Err(format!("slot {slot} is free but its record is LIVE: stale draw"));
                }
            }
            Some(inst) => {
                if record.flags & InstanceRecord::LIVE == 0 {
                    return Err(format!("slot {slot} is held but its record is not LIVE"));
                }
                if record.previous != inst.previous || record.current != inst.current {
                    return Err(format!(
                        "slot {slot}: record (prev cell {:?}, cur cell {:?}) model (prev cell {:?}, cur cell {:?}) base {:?}",
                        record.previous.position.cell[0],
                        record.current.position.cell[0],
                        inst.previous.position.cell[0],
                        inst.current.position.cell[0],
                        inst.base.map(|b| b.position.cell[0]),
                    ));
                }
            }
        }
    }
    let bound = u64::from(model.instances) + 2 * u64::from(model.rows);
    if batch.entries().len() as u64 > bound {
        return Err(format!("batch of {} entries above the bound {bound}", batch.entries().len()));
    }
    Ok(())
}

#[derive(Default, Debug)]
struct Tally {
    prepares: u64,
    refused: u64,
    torn: u64,
    lost: u64,
    unplaced: u64,
    duplicates: u64,
    dropped: u64,
    remote_applied: u64,
    take_base: u64,
    has_base_only: u64,
    regiven: u64,
    frees: u64,
    interpolated: u64,
}

fn tally(t: &mut Tally, stats: &Expect, renderer: &Renderer, model: &Model) {
    t.prepares += 1;
    t.lost += u64::from(stats.lost != 0);
    t.unplaced += stats.unplaced;
    t.duplicates += stats.duplicates;
    t.dropped += stats.dropped;
    for e in renderer.batch().entries() {
        if e.flags & ScatterEntry::FREE != 0 {
            t.frees += 1;
        } else if e.flags & ScatterEntry::TAKE_BASE != 0 {
            t.take_base += 1;
        } else if e.flags & ScatterEntry::HAS_BASE != 0 {
            t.has_base_only += 1;
        }
    }
    t.interpolated += model.held.values().filter(|i| i.previous != i.current).count() as u64;
    t.remote_applied += model.seen_remote.take();
    t.regiven += model.seen_regiven.take();
}

fn run_seed(seed: u64, t: &mut Tally) -> Result<(), String> {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let instances = 3 + rng.below(8) as u32;
    let rows = 6 + rng.below(40) as u32;
    let regions = 2 + rng.below(2) as u32;
    let universe = 4 + rng.below(9);
    let desc = RendererDesc { instances, rows, regions, shaders: None };
    let mut renderer = ok(Renderer::new(null(), &desc));
    let mut ender = Ender::new(&**renderer.device());
    let mut model = Model::new(rows, instances);
    let mut run = Run { rng, serial: 0, universe, ticks: [100, 100, 100] };
    let mut sinks: Vec<ExtractSink> = KINDS.iter().map(|_| renderer.sink()).collect();
    let mut msinks: [Option<MPass>; 3] = [None; 3];
    let mut records: Vec<InstanceRecord> = Vec::new();
    let mut history: Vec<String> = Vec::new();

    for frame in 0..80 {
        // Open, as the frame does before its writers, most of the time.
        if !model.open && run.rng.pct(92) {
            loop {
                match renderer.open().expect("a timeline value") {
                    Opened::Ready => break,
                    Opened::Wait(point) => {
                        assert!(renderer.device().wait_timeline(point, PATIENCE).unwrap());
                    }
                    Opened::Unsubmitted => {
                        if model.pending {
                            return Err(format!(
                                "frame {frame}: Unsubmitted with a frame prepared"
                            ));
                        }
                        ender.end(&mut renderer);
                    }
                }
            }
            model.open_region();
            history.push(format!("f{frame} open"));
        }
        // The frame's script: one queue per actor, interleaved at random.
        let mut queues: Vec<(usize, Vec<Op>)> = Vec::new();
        for kind in KINDS {
            if run.rng.pct(70) {
                let mut ops = run.pass(kind);
                if run.rng.pct(15) {
                    ops.extend(run.pass(kind));
                }
                ops.reverse();
                queues.push((ki(kind), ops));
            }
        }
        let overlap = run.rng.pct(10);
        let mut remote_ops = Vec::new();
        if run.rng.pct(55) {
            remote_ops = run.remote();
            if run.rng.pct(5) {
                remote_ops.extend(run.remote());
            }
        }
        remote_ops.reverse();
        if run.rng.pct(4) {
            queues.push((4, vec![Op::Prepare]));
        }
        if run.rng.pct(3) {
            queues.push((
                run.rng.below(3) as usize,
                vec![Op::Write(run.rows(WorldKind::Server)), Op::Removed(vec![the_key(0)])],
            ));
        }
        let client_pending = |queues: &Vec<(usize, Vec<Op>)>| queues.iter().any(|q| q.0 == 1);
        let mut remote_queue = remote_ops;
        loop {
            // A segment: the remote writer borrows the renderer, so a mid-frame `prepare`
            // ends the segment and runs outside it, only while no writer is open.
            let mut spares: Vec<ExtractSink> = (0..4).map(|_| renderer.sink()).collect();
            let stop = {
                let mut handle: Option<&mut Renderer> = Some(&mut renderer);
                let mut writer: Option<(RemoteWriter<'_>, MRemote)> = None;
                let stop = loop {
                    let remote_ready =
                        !remote_queue.is_empty() && (overlap || !client_pending(&queues));
                    let actors = queues.len() + usize::from(remote_ready);
                    if actors == 0 {
                        break 0;
                    }
                    let pick = run.rng.below(actors as u64) as usize;
                    if pick == queues.len() {
                        let op = remote_queue.pop().unwrap();
                        history.push(format!("f{frame} remote {}", short(&op)));
                        match op {
                            Op::RemoteOpen => {
                                assert!(writer.is_none());
                                let Some(r) = handle.take() else {
                                    remote_queue.push(Op::RemoteOpen);
                                    history.pop();
                                    break 2;
                                };
                                let m = model.remote_open();
                                writer = Some((r.remote(), m));
                            }
                            Op::RemoteWrite(rows) => {
                                let (w, m) = writer.as_ref().unwrap();
                                w.write(&rows);
                                let m = *m;
                                model.remote_write(m, &rows);
                            }
                            Op::RemoteClose => {
                                let (w, m) = writer.take().unwrap();
                                drop(w);
                                model.remote_close(m);
                            }
                            Op::RemoteForget => {
                                let (w, _) = writer.take().unwrap();
                                core::mem::forget(w);
                            }
                            _ => unreachable!(),
                        }
                        continue;
                    }
                    let (actor, ops) = &mut queues[pick];
                    let actor = *actor;
                    if matches!(ops.last(), Some(Op::Prepare)) {
                        if writer.is_some() {
                            continue;
                        }
                        queues.remove(pick);
                        break 1;
                    }
                    let op = ops.pop().unwrap();
                    if ops.is_empty() {
                        queues.remove(pick);
                    }
                    history.push(format!("f{frame} a{actor} {}", short(&op)));
                    match op {
                        Op::Begin(pass) => {
                            sinks[actor].begin(pass);
                            model.begin(&mut msinks[actor], pass);
                        }
                        Op::Removed(keys) => {
                            sinks[actor].removed(&keys);
                            model.removed(&mut msinks[actor], &keys);
                        }
                        Op::Write(rows) => {
                            sinks[actor].write(&rows);
                            model.write(&msinks[actor], &rows);
                        }
                        Op::End => {
                            sinks[actor].end();
                            model.end(&mut msinks[actor]);
                        }
                        Op::Replace => {
                            sinks[actor] = spares.pop().expect("a spare sink");
                            let mut m = msinks[actor];
                            model.abandon(&mut m);
                            msinks[actor] = None;
                        }
                        _ => unreachable!(),
                    }
                };
                assert!(writer.is_none() || stop == 0);
                drop(writer);
                stop
            };
            drop(spares);
            match stop {
                0 => break,
                2 => continue,
                _ => {}
            }
            history.push(format!("f{frame} mid prepare"));
            let refused = model.refuses_prepare();
            match renderer.prepare() {
                Ok(stats) => {
                    if refused {
                        return Err(fail(
                            seed,
                            frame,
                            &history,
                            "mid prepare succeeded where refused",
                        ));
                    }
                    let want = model.prepare();
                    if stats_of(&stats) != want {
                        return Err(fail(
                            seed,
                            frame,
                            &history,
                            &format!("stats {:?}\n   != model {want:?}", stats_of(&stats)),
                        ));
                    }
                    let mut sim = records.clone();
                    check_records(&renderer, &model, &mut sim)
                        .map_err(|e| fail(seed, frame, &history, &e))?;
                    records = sim;
                    tally(t, &want, &renderer, &model);
                }
                Err(_) if refused => t.refused += 1,
                Err(e) => {
                    return Err(fail(seed, frame, &history, &format!("mid prepare refused: {e}")));
                }
            }
        }
        // The frame's end.
        if !model.pending && run.rng.pct(85) {
            let refused = model.refuses_prepare();
            history.push(format!("f{frame} prepare (refused {refused})"));
            match renderer.prepare() {
                Ok(stats) => {
                    if refused {
                        return Err(fail(seed, frame, &history, "prepare succeeded where refused"));
                    }
                    let want = model.prepare();
                    if stats_of(&stats) != want {
                        return Err(fail(
                            seed,
                            frame,
                            &history,
                            &format!("stats {:?}\n   != model {want:?}", stats_of(&stats)),
                        ));
                    }
                    let mut sim = records.clone();
                    check_records(&renderer, &model, &mut sim)
                        .map_err(|e| fail(seed, frame, &history, &e))?;
                    records = sim;
                    tally(t, &want, &renderer, &model);
                }
                Err(_) if refused => t.refused += 1,
                Err(e) => {
                    return Err(fail(seed, frame, &history, &format!("prepare refused: {e}")));
                }
            }
        }
        if model.pending {
            if run.rng.pct(14) {
                history.push(format!("f{frame} torn"));
                renderer.torn();
                model.torn();
                t.torn += 1;
                for record in &mut records {
                    *record = poison();
                }
            } else {
                history.push(format!("f{frame} submitted"));
                ender.end(&mut renderer);
                model.pending = false;
            }
        }
    }
    Ok(())
}

fn short(op: &Op) -> String {
    match op {
        Op::Begin(p) => format!("begin {:?} t{} full {}", p.kind, p.tick.0, p.full),
        Op::Removed(keys) => format!("removed {}", keys.len()),
        Op::Write(rows) => format!("write {}", rows.len()),
        other => format!("{other:?}"),
    }
}

fn fail(seed: u64, frame: u32, history: &[String], what: &str) -> String {
    let tail: Vec<_> = history.iter().rev().take(40).rev().cloned().collect();
    format!("seed {seed} frame {frame}: {what}\n  history tail:\n    {}", tail.join("\n    "))
}

/// The seeds: `MODEL_SEEDS=a..b`, or the first 2,000, which run in about a second.
fn seeds() -> core::ops::Range<u64> {
    std::env::var("MODEL_SEEDS")
        .ok()
        .and_then(|s| {
            let (a, b) = s.split_once("..")?;
            Some(a.parse().ok()?..b.parse().ok()?)
        })
        .unwrap_or(0..2000)
}

#[test]
fn random_sequences_match_the_model() {
    let mut failures = Vec::new();
    let mut t = Tally::default();
    for seed in seeds() {
        if let Err(e) = run_seed(seed, &mut t) {
            failures.push(e);
            if failures.len() >= 3 {
                break;
            }
        }
    }
    eprintln!("{t:?}");
    assert!(
        failures.is_empty(),
        "{} failing seeds; first:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
    // The seeds reach each path the model checks.
    let reached = [
        ("a refused prepare", t.refused),
        ("a torn frame", t.torn),
        ("a lost kind", t.lost),
        ("an unplaced row", t.unplaced),
        ("a duplicate", t.duplicates),
        ("a dropped row", t.dropped),
        ("an applied remote batch", t.remote_applied),
        ("a base taken", t.take_base),
        ("a base kept", t.has_base_only),
        ("a slot given again in its frame", t.regiven),
        ("a free", t.frees),
        ("an interpolated record", t.interpolated),
    ];
    for (what, count) in reached {
        assert!(count > 0, "no seed reached {what}: {t:?}");
    }
}

/// The realistic shape of the half-ring case: a prediction world at tick `t`, then a resync
/// to `t - back` whose pass flags every row (as extract does at a tick `tick_before` calls
/// earlier), then a rollback at that same tick in a frame with no step, unflagged. Returns
/// the record's (previous, current) x offsets after the rollback.
fn resync_then_rollback(back: u32) -> (f32, f32) {
    let desc = RendererDesc { instances: 2, rows: 16, regions: 2, shaders: None };
    let mut renderer = ok(Renderer::new(null(), &desc));
    let mut ender = Ender::new(&**renderer.device());
    let mut records: Vec<InstanceRecord> = Vec::new();
    let t = 1000_u32;
    let mut pass = |renderer: &mut Renderer, tick: u32, full: bool, x: f32, flag: bool| {
        let row = InstanceRow {
            key: the_key(0),
            mesh: AssetRef::new(AssetId::from_bits(1)),
            transform: placement(0, x),
            no_interpolate: flag,
        };
        let mut log = vec![None; 16];
        let mut sink = renderer.sink();
        sink.begin(ExtractPass { kind: WorldKind::Prediction, tick: Tick(tick), full });
        sink.write(&[row]);
        sink.end();
        drop(sink);
        log[1] = Some(row);
        ok(renderer.prepare());
        let batch = renderer.batch();
        for &range in batch.dispatches() {
            for entry in batch.run(range) {
                let slot = entry.slot.index() as usize;
                if records.len() <= slot {
                    records.resize(slot + 1, poison());
                }
                if entry.flags & ScatterEntry::FREE != 0 {
                    records[slot].free();
                } else {
                    records[slot]
                        .scatter(entry.flags, &StagingRow::of(&log[entry.row as usize].unwrap()));
                }
            }
        }
        ender.end(renderer);
        loop {
            match renderer.open().unwrap() {
                Opened::Ready => break,
                Opened::Wait(point) => {
                    renderer.device().wait_timeline(point, PATIENCE).unwrap();
                }
                Opened::Unsubmitted => unreachable!(),
            }
        }
    };
    pass(&mut renderer, t, true, 1.0, false);
    pass(&mut renderer, t + 1, false, 2.0, false);
    pass(&mut renderer, (t + 1).wrapping_sub(back), true, 3.0, true);
    pass(&mut renderer, (t + 1).wrapping_sub(back), true, 4.0, false);
    let record = records[0];
    (record.previous.position.offset[0], record.current.position.offset[0])
}

#[test]
fn a_resync_exactly_half_the_ring_back_drops_the_bases() {
    // present-api's `tick_before` calls both earlier: the sink holds no base after either,
    // so the rollback draws its row at both ends.
    let near = resync_then_rollback((1 << 31) - 1);
    let half = resync_then_rollback(1 << 31);
    eprintln!("2^31 - 1 back: {near:?}; 2^31 back: {half:?}");
    assert_eq!(near, (4.0, 4.0));
    assert_eq!(half, (4.0, 4.0), "the rollback interpolates from the placement before the resync");
}
