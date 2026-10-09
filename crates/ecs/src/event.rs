//! Events between systems within a tick, and presentation events out of the tick
//! (architecture sections 6.1 and 9.4).

use core::marker::PhantomData;

use bytemuck::{CheckedBitPattern, NoUninit};
use lactose_core::{StableKey, Tick};

use crate::access::{Access, AccessKind, events_target, no_target};
use crate::param::{SystemParam, sealed};

/// A simulation event: a cross-entity effect emitted by one system and consumed by a later
/// one, never a direct write to another row. Indexed by its target.
pub trait Event: Copy + Send + Sync + 'static {
    /// The entity the event is for.
    fn target(&self) -> StableKey;
}

/// Emits events of type `E`.
///
/// Events emitted in a tick are visible to every system ordered after the emitter in the
/// same tick, in order of (emitting system, source origin key, sequence), and are cleared
/// at the next input sync point. The schedule requires every reader of `E` to be ordered
/// after every emitter of `E`, so what a reader sees never depends on scheduling.
pub struct EventWriter<'w, E: Event> {
    _buffer: PhantomData<&'w mut ()>,
    _event: PhantomData<fn() -> E>,
}

impl<E: Event> EventWriter<'_, E> {
    /// Emits `event`.
    pub fn emit(&mut self, event: E) {
        todo!()
    }
}

impl<E: Event> sealed::Sealed for EventWriter<'_, E> {}

impl<E: Event> SystemParam for EventWriter<'_, E> {
    const ACCESS: Access = Access::new(AccessKind::EmitEvents, events_target::<E>);
}

/// Reads the events of type `E` emitted earlier in this tick.
pub struct Events<'w, E: Event> {
    _buffer: PhantomData<&'w ()>,
    _event: PhantomData<fn() -> E>,
}

impl<E: Event> Events<'_, E> {
    /// The events whose target is `key`, in emission order.
    #[must_use]
    pub fn for_target(&self, key: StableKey) -> EventIter<'_, E> {
        todo!()
    }

    /// Every event, in emission order.
    #[must_use]
    pub fn iter(&self) -> EventIter<'_, E> {
        todo!()
    }

    /// How many events there are.
    #[must_use]
    pub fn len(&self) -> usize {
        todo!()
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

/// Iterates events in emission order.
pub struct EventIter<'a, E: Event> {
    _buffer: PhantomData<&'a ()>,
    _event: PhantomData<fn() -> E>,
}

impl<'a, E: Event> Iterator for EventIter<'a, E> {
    type Item = &'a E;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
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

/// Emits presentation events.
///
/// Each emission is keyed by (origin key, tick, kind). The world keeps the keys it has
/// delivered: a re-emission with a delivered key, as a rollback re-simulation produces, is
/// not delivered again, and a delivered key that a re-simulation of its tick does not
/// re-emit is reported as cancelled in the next
/// [`PresentEventBatch`](crate::PresentEventBatch).
pub struct PresentEvents<'w> {
    _buffer: PhantomData<&'w mut ()>,
}

impl PresentEvents<'_> {
    /// Emits `event` for `origin` at the current tick.
    pub fn emit<P: PresentEvent>(&mut self, origin: StableKey, event: P) {
        todo!()
    }
}

impl sealed::Sealed for PresentEvents<'_> {}

impl SystemParam for PresentEvents<'_> {
    const ACCESS: Access = Access::new(AccessKind::PresentEvents, no_target);
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
}

/// One delivered presentation event.
#[derive(Clone, Debug)]
pub struct PresentEmission {
    /// The key.
    pub key: PresentEventKey,
    bytes: Vec<u8>,
}

impl PresentEmission {
    /// The event as `P`, or `None` if the key's kind is not `P::KIND` or the bytes are not a
    /// valid `P`.
    #[must_use]
    pub fn get<P: PresentEvent>(&self) -> Option<P> {
        todo!()
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
