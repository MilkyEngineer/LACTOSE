//! What extract handed the renderer, kept on the host for decision 5.1's reference: each
//! sink call of the fixed-frame mode's passes is passed on to the renderer's sink and sent
//! here, and the render side applies them, frame by frame, to the placements the sink
//! contract says the renderer holds (present-api, "What the sink keeps"). The reference is
//! rasterised from those, and the records the device's scatter wrote are held to them.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc;

use lactose_core::{StableKey, Tick};
use lactose_ecs::WorldKind;
use lactose_present_api::{ExtractPass, InstanceRow, InstanceSink, RenderTransform};
use lactose_renderer::{ExtractSink, InstanceRecord, SlotTable};

/// One sink call, with the sink it was made on.
enum Call {
    Begin(ExtractPass),
    Removed(Vec<StableKey>),
    Rows(Vec<InstanceRow>),
    End,
}

/// The renderer's sink, each call also sent to an [`Extracted`]. A write sends on a
/// channel, which takes no lock; the receiver reads it between frames alone.
pub struct Recording {
    inner: ExtractSink,
    sink: u32,
    calls: mpsc::Sender<(u32, Call)>,
}

impl InstanceSink for Recording {
    fn begin(&mut self, pass: ExtractPass) {
        self.inner.begin(pass);
        let _ = self.calls.send((self.sink, Call::Begin(pass)));
    }

    fn removed(&mut self, keys: &[StableKey]) {
        self.inner.removed(keys);
        let _ = self.calls.send((self.sink, Call::Removed(keys.to_vec())));
    }

    fn write(&self, rows: &[InstanceRow]) {
        self.inner.write(rows);
        let _ = self.calls.send((self.sink, Call::Rows(rows.to_vec())));
    }

    fn end(&mut self) {
        self.inner.end();
        let _ = self.calls.send((self.sink, Call::End));
    }
}

/// What the sink holds for one key of one kind.
#[derive(Clone, Copy)]
struct Held {
    previous: RenderTransform,
    current: RenderTransform,
    base: Option<RenderTransform>,
}

/// A pass begun and not ended.
struct Open {
    kind: WorldKind,
    full: bool,
    written: HashSet<StableKey>,
}

/// The placements extract handed the renderer, by kind and key, as the sink contract keeps
/// them. Broken for good once something happened that the contract does not say the
/// placements of: a pass open at a frame or begun beside one, a pass or a row the renderer
/// lost, dropped or did not place, a torn frame, a key with no slot.
pub struct Extracted {
    calls: mpsc::Receiver<(u32, Call)>,
    sender: mpsc::Sender<(u32, Call)>,
    sinks: u32,
    held: HashMap<(WorldKind, StableKey), Held>,
    /// Each kind's tick at its last `begin`.
    ticks: HashMap<WorldKind, Tick>,
    open: HashMap<u32, Open>,
    broken: Option<String>,
}

impl Extracted {
    pub fn new() -> Self {
        let (sender, calls) = mpsc::channel();
        Self {
            calls,
            sender,
            sinks: 0,
            held: HashMap::new(),
            ticks: HashMap::new(),
            open: HashMap::new(),
            broken: None,
        }
    }

    /// `inner`, recorded here.
    pub fn recording(&mut self, inner: ExtractSink) -> Recording {
        self.sinks += 1;
        Recording { inner, sink: self.sinks, calls: self.sender.clone() }
    }

    /// Why the placements are not known, if they are not.
    pub fn broken(&self) -> Option<&str> {
        self.broken.as_deref()
    }

    /// Marks the placements unknown from now on, for `why`, unless they already are.
    pub fn break_for(&mut self, why: impl Into<String>) {
        self.broken.get_or_insert_with(|| why.into());
    }

    /// Applies every call sent since the last, as a frame's `prepare` is about to read them:
    /// a pass still open breaks the placements.
    pub fn before_prepare(&mut self) {
        while let Ok((sink, call)) = self.calls.try_recv() {
            self.apply(sink, call);
        }
        if !self.open.is_empty() {
            self.break_for("a pass was open at a frame's prepare");
        }
    }

    /// The records the scatter of the frame just prepared leaves, slot by slot below the
    /// table's extent, where every record was written (renderer, `InstanceRecord`: a
    /// record no entry has written is undefined, and the draw covers the extent alone), from
    /// the slots `table` gave: a call sent since [`before_prepare`](Self::before_prepare)
    /// breaks the placements, as does a held key the table has no slot for below its
    /// extent or a slot the table holds for no key held here.
    pub fn records(&mut self, table: &SlotTable) -> Option<Vec<InstanceRecord>> {
        if self.calls.try_recv().is_ok() {
            self.break_for("a pass wrote beside a frame's prepare");
        }
        if usize::try_from(table.live()).ok() != Some(self.held.len()) {
            let why = format!("{} slots held for {} placements", table.live(), self.held.len());
            self.break_for(why);
        }
        if self.broken.is_some() {
            return None;
        }
        let free = InstanceRecord::from_bytes(&[0; InstanceRecord::BYTES as usize]);
        let mut records = vec![free; table.extent() as usize];
        for (&(kind, key), held) in &self.held {
            let slot = table.get(kind, key).map(|slot| slot.index() as usize);
            let Some(record) = slot.and_then(|slot| records.get_mut(slot)) else {
                self.broken = Some(format!("{kind:?} {key:?} holds no slot"));
                return None;
            };
            *record = InstanceRecord {
                previous: held.previous,
                current: held.current,
                base: held.current,
                flags: InstanceRecord::LIVE,
            };
        }
        Some(records)
    }

    fn apply(&mut self, sink: u32, call: Call) {
        match call {
            Call::Begin(pass) => {
                if self.open.contains_key(&sink) {
                    self.break_for("a pass began inside another");
                }
                self.begin(pass);
                let written = HashSet::new();
                self.open.insert(sink, Open { kind: pass.kind, full: pass.full, written });
            }
            Call::Removed(keys) => {
                let Some(open) = self.open.get(&sink) else {
                    return self.break_for("a removal outside a pass");
                };
                let kind = open.kind;
                for key in keys {
                    self.held.remove(&(kind, key));
                }
            }
            Call::Rows(rows) => self.write(sink, &rows),
            Call::End => {
                let Some(open) = self.open.remove(&sink) else {
                    return self.break_for("an end outside a pass");
                };
                if open.full {
                    self.held
                        .retain(|&(kind, key), _| kind != open.kind || open.written.contains(&key));
                }
            }
        }
    }

    /// The contract's `begin`: at a later tick every held key of the kind takes its
    /// current placement as its base; at an earlier one, by the wrapping comparison, every
    /// base of the kind is dropped and none taken until the next tick.
    fn begin(&mut self, pass: ExtractPass) {
        let last = self.ticks.insert(pass.kind, pass.tick);
        if last == Some(pass.tick) {
            return;
        }
        let earlier = last.is_some_and(|last| pass.tick.0.wrapping_sub(last.0) >= 1 << 31);
        for ((kind, _), held) in &mut self.held {
            if *kind == pass.kind {
                held.base = if earlier { None } else { Some(held.current) };
            }
        }
    }

    /// The contract's `write`: the previous placement becomes the base and the current one
    /// the row's; with `no_interpolate`, with no base, or for a key new to the kind, both
    /// become the row's.
    fn write(&mut self, sink: u32, rows: &[InstanceRow]) {
        let Some(open) = self.open.get_mut(&sink) else {
            return self.break_for("a write outside a pass");
        };
        let kind = open.kind;
        let mut twice = false;
        for row in rows {
            twice |= !open.written.insert(row.key);
            let to = row.transform;
            let held = self.held.entry((kind, row.key)).or_insert(Held {
                previous: to,
                current: to,
                base: None,
            });
            held.previous = match held.base {
                Some(base) if !row.no_interpolate => base,
                _ => to,
            };
            held.current = to;
        }
        if twice {
            self.break_for("a pass wrote a key twice");
        }
    }
}
