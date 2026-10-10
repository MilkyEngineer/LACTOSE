//! Events between systems within a tick, and presentation events out of the tick
//! (architecture sections 6.1 and 9.4).

use core::any::{Any, TypeId};
use core::marker::PhantomData;
use std::collections::{BTreeMap, BTreeSet};

use bytemuck::{CheckedBitPattern, NoUninit};
use lactose_core::{FixedHashMap, StableKey, Tick};

use crate::access::{Access, AccessKind, events_target, no_target};
use crate::param::{SystemParam, sealed};
use crate::system::{Fetch, Invocation, Slot, mismatch};

/// A simulation event: a cross-entity effect emitted by one system and consumed by a later
/// one, never a direct write to another row. Indexed by its target.
pub trait Event: Copy + Send + Sync + 'static {
    /// The entity the event is for.
    fn target(&self) -> StableKey;
}

/// One emitted event: its target as the origin key and, for a bound predicted entity, the
/// `Net` key that also names it, so a reader asking by either finds it.
pub(crate) struct EventRecord {
    pub(crate) target: StableKey,
    pub(crate) alias: Option<StableKey>,
    pub(crate) event: Box<dyn Any + Send + Sync>,
}

/// Every event type's events this tick, in emission order.
pub(crate) type EventStore = FixedHashMap<TypeId, Vec<EventRecord>>;

/// Emits events of type `E`.
///
/// Events emitted in a tick are visible to every system ordered after the emitter in the
/// same tick, in order of (emitting system, source, sequence) with sources ordered as
/// commands are (the once-per-tick invocation, per-region invocations by `RegionId`, entity
/// sources by origin key), and are cleared at the next input sync point. The schedule requires every reader of `E` to be ordered
/// after every emitter of `E`, so what a reader sees never depends on scheduling. A
/// placeholder target is a debug assertion: the entity does not exist until the buffer
/// applies.
pub struct EventWriter<'w, E: Event> {
    sink: &'w mut Vec<EventRecord>,
    _event: PhantomData<fn() -> E>,
}

impl<E: Event> EventWriter<'_, E> {
    /// Emits `event`.
    pub fn emit(&mut self, event: E) {
        let target = event.target();
        debug_assert!(
            !matches!(target, StableKey::Placeholder(_)),
            "a placeholder is not an event target: the entity does not exist yet"
        );
        self.sink.push(EventRecord { target, alias: None, event: Box::new(event) });
    }
}

impl<E: Event> sealed::Sealed for EventWriter<'_, E> {}

impl<E: Event> SystemParam for EventWriter<'_, E> {
    const ACCESS: Access = Access::new(AccessKind::EmitEvents, events_target::<E>);
}

impl<'r, 'j, E: Event> Fetch<'r, 'j> for EventWriter<'r, E> {
    fn fetch(invocation: &mut Invocation<'r, 'j>, index: usize) -> Self {
        match invocation.take(index) {
            Slot::EmitEvents(sink) => Self { sink, _event: PhantomData },
            _ => mismatch(index),
        }
    }
}

/// Reads the events of type `E` emitted earlier in this tick.
pub struct Events<'w, E: Event> {
    records: &'w [EventRecord],
    _event: PhantomData<fn() -> E>,
}

impl<E: Event> Events<'_, E> {
    /// The events whose target is `key`, in emission order.
    #[must_use]
    pub fn for_target(&self, key: StableKey) -> EventIter<'_, E> {
        EventIter { records: self.records.iter(), target: Some(key), _event: PhantomData }
    }

    /// Every event, in emission order.
    #[must_use]
    pub fn iter(&self) -> EventIter<'_, E> {
        EventIter { records: self.records.iter(), target: None, _event: PhantomData }
    }

    /// How many events there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether there are no events.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<E: Event> sealed::Sealed for Events<'_, E> {}

impl<E: Event> SystemParam for Events<'_, E> {
    const ACCESS: Access = Access::new(AccessKind::ReadEvents, events_target::<E>);
}

impl<'r, 'j, E: Event> Fetch<'r, 'j> for Events<'r, E> {
    fn fetch(invocation: &mut Invocation<'r, 'j>, index: usize) -> Self {
        match invocation.take(index) {
            Slot::ReadEvents(records) => Self { records, _event: PhantomData },
            _ => mismatch(index),
        }
    }
}

/// Iterates events in emission order.
pub struct EventIter<'a, E: Event> {
    records: core::slice::Iter<'a, EventRecord>,
    target: Option<StableKey>,
    _event: PhantomData<fn() -> E>,
}

impl<'a, E: Event> Iterator for EventIter<'a, E> {
    type Item = &'a E;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let record = self.records.next()?;
            if self.target.is_none_or(|key| record.target == key || record.alias == Some(key)) {
                return Some(record.event.downcast_ref().expect("events of one type"));
            }
        }
    }
}

/// The kind of a presentation event: the xxh64 of a declared name, part of the key that
/// deduplicates re-emission.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PresentKind(pub u64);

impl PresentKind {
    /// The kind named `name`.
    #[must_use]
    pub const fn from_name(name: &str) -> Self {
        Self(lactose_core::xxh64(name.as_bytes(), 0))
    }
}

/// A presentation event: a request the simulation makes of presentation (audio, VFX, UI),
/// declared in `lactose-present-api`. Plain data, so the log that deduplicates it is part
/// of a snapshot.
pub trait PresentEvent: NoUninit + CheckedBitPattern + Copy + Send + Sync + 'static {
    /// The kind.
    const KIND: PresentKind;
}

/// One presentation event as an invocation emitted it.
pub(crate) struct PresentRecord {
    pub(crate) origin: StableKey,
    pub(crate) kind: PresentKind,
    pub(crate) bytes: Vec<u8>,
}

/// Emits presentation events.
///
/// Each emission is keyed by (origin key, tick, kind, sequence), the sequence being the
/// ordinal of this (origin, kind) among the tick's emissions. The world keeps the keys it
/// has delivered, across [`restore`](crate::World::restore), pruned below the oldest tick a
/// restore can reach: a re-emission with a delivered key, as a rollback re-simulation
/// produces, is not delivered again, and a delivered key whose tick is re-simulated without
/// re-emitting it is reported as cancelled in the next
/// [`PresentEventBatch`](crate::PresentEventBatch). A placeholder origin is a debug
/// assertion.
pub struct PresentEvents<'w> {
    sink: &'w mut Vec<PresentRecord>,
}

impl PresentEvents<'_> {
    /// Emits `event` for `origin`, the entity the event is about, at the current tick. A
    /// per-region or once-per-tick system names the entity itself.
    pub fn emit<P: PresentEvent>(&mut self, origin: StableKey, event: P) {
        debug_assert!(
            !matches!(origin, StableKey::Placeholder(_)),
            "a placeholder is not a presentation-event origin: the entity does not exist yet"
        );
        self.sink.push(PresentRecord {
            origin,
            kind: P::KIND,
            bytes: bytemuck::bytes_of(&event).to_vec(),
        });
    }
}

impl sealed::Sealed for PresentEvents<'_> {}

impl SystemParam for PresentEvents<'_> {
    const ACCESS: Access = Access::new(AccessKind::PresentEvents, no_target);
}

impl<'r, 'j> Fetch<'r, 'j> for PresentEvents<'r> {
    fn fetch(invocation: &mut Invocation<'r, 'j>, index: usize) -> Self {
        match invocation.take(index) {
            Slot::Present(sink) => Self { sink },
            _ => mismatch(index),
        }
    }
}

/// The key of one presentation event.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PresentEventKey {
    /// The entity the event is about.
    pub origin: StableKey,
    /// The tick it was emitted at.
    pub tick: Tick,
    /// Its kind.
    pub kind: PresentKind,
    /// Its ordinal among the emissions with the same origin and kind in the tick.
    pub seq: u32,
}

/// One delivered presentation event.
#[derive(Clone, Debug)]
pub struct PresentEmission {
    /// The key.
    pub key: PresentEventKey,
    bytes: Vec<u8>,
    /// The emitting system's position in execution order, for the batch order.
    order: u32,
}

impl PresentEmission {
    /// The event as `P`, or `None` if the key's kind is not `P::KIND` or the bytes are not a
    /// valid `P`.
    #[must_use]
    pub fn get<P: PresentEvent>(&self) -> Option<P> {
        if self.key.kind != P::KIND {
            return None;
        }
        bytemuck::checked::try_pod_read_unaligned(&self.bytes).ok()
    }
}

/// What [`World::take_present_events`](crate::World::take_present_events) returns.
#[derive(Clone, Debug, Default)]
pub struct PresentEventBatch {
    /// Events delivered for the first time, in order of (tick, emitting system, origin
    /// key, sequence).
    pub emitted: Vec<PresentEmission>,
    /// Keys delivered earlier whose tick was re-simulated without re-emitting them.
    pub cancelled: Vec<PresentEventKey>,
}

/// One emission of a run, before its sequence is known.
pub(crate) struct PendingEmission {
    /// Execution position, source order, parameter and emission order: the canonical order
    /// the sequence is counted in.
    pub(crate) order: (u32, crate::schedule::Source, u32, u32),
    pub(crate) record: PresentRecord,
}

type KeyParts = (StableKey, PresentKind, u32);

/// The first sequence number of a pass's emissions: a pass between ticks numbers its
/// events apart from the tick's, so neither shifts the other across a rollback.
pub(crate) const PASS_SEQ: u32 = 1 << 31;

/// The presentation-event state a world keeps: emissions not yet taken, and the keys taken,
/// which survive a restore (architecture section 10).
#[derive(Default)]
pub(crate) struct PresentLog {
    emitted: Vec<PresentEmission>,
    cancelled: Vec<PresentEventKey>,
    delivered: BTreeMap<Tick, BTreeSet<KeyParts>>,
    /// The next sequence per (origin, kind, run is a pass) at tick `counted`, so a later
    /// pass at the same tick continues the passes' sequence.
    counts: BTreeMap<(StableKey, PresentKind, bool), u32>,
    counted: Option<Tick>,
}

impl PresentLog {
    /// Records the emissions of a run at `tick`, whose origins are already origin keys:
    /// those already delivered are dropped. A tick's run numbers each (origin, kind) from
    /// zero and cancels the delivered keys of its tick that it did not emit again, which
    /// only a re-simulated tick can leave. A pass between ticks numbers from
    /// [`PASS_SEQ`], continuing within the tick, and its keys are never cancelled by a
    /// re-simulation of the tick, which does not run passes.
    pub(crate) fn end_run(&mut self, tick: Tick, mut emissions: Vec<PendingEmission>, pass: bool) {
        emissions.sort_by_key(|emission| emission.order);
        if !pass || self.counted != Some(tick) {
            self.counts.clear();
        }
        self.counted = Some(tick);
        let mut delivered = if pass {
            self.delivered.get(&tick).cloned().unwrap_or_default()
        } else {
            self.delivered.remove(&tick).unwrap_or_default()
        };
        let mut keys = BTreeSet::new();
        for emission in emissions {
            let PresentRecord { origin, kind, bytes } = emission.record;
            let base = if pass { PASS_SEQ } else { 0 };
            let count = self.counts.entry((origin, kind, pass)).or_insert(base);
            let seq = *count;
            *count += 1;
            keys.insert((origin, kind, seq));
            let key = PresentEventKey { origin, tick, kind, seq };
            if delivered.contains(&(origin, kind, seq)) {
                continue;
            }
            // Cancelled by an earlier rollback and emitted again before presentation took
            // either: it was never stopped, so it is neither cancelled nor delivered again.
            if let Some(index) = self.cancelled.iter().position(|cancelled| *cancelled == key) {
                self.cancelled.remove(index);
                delivered.insert((origin, kind, seq));
                continue;
            }
            self.emitted.push(PresentEmission { key, bytes, order: emission.order.0 });
        }
        if pass {
            if !delivered.is_empty() {
                self.delivered.insert(tick, delivered);
            }
            return;
        }
        let mut kept = BTreeSet::new();
        for parts in delivered {
            let (origin, kind, seq) = parts;
            if seq >= PASS_SEQ || keys.contains(&parts) {
                kept.insert(parts);
            } else {
                self.cancelled.push(PresentEventKey { origin, tick, kind, seq });
            }
        }
        if !kept.is_empty() {
            self.delivered.insert(tick, kept);
        }
    }

    pub(crate) fn take(&mut self) -> PresentEventBatch {
        let mut emitted = core::mem::take(&mut self.emitted);
        emitted.sort_by(|a, b| {
            let key =
                |e: &PresentEmission| (e.key.tick, e.order, e.key.origin, e.key.seq, e.key.kind);
            key(a).cmp(&key(b))
        });
        for emission in &emitted {
            let PresentEventKey { origin, tick, kind, seq } = emission.key;
            self.delivered.entry(tick).or_default().insert((origin, kind, seq));
        }
        PresentEventBatch { emitted, cancelled: core::mem::take(&mut self.cancelled) }
    }

    /// Drops the emissions after `tick` that were never taken: a restore to `tick`.
    pub(crate) fn rewind(&mut self, tick: Tick) {
        self.emitted.retain(|emission| emission.key.tick <= tick);
        self.counts.clear();
        self.counted = None;
    }

    /// Forgets delivered keys below `oldest`, which no restore can reach.
    pub(crate) fn prune(&mut self, oldest: Tick) {
        self.delivered = self.delivered.split_off(&oldest);
    }
}
