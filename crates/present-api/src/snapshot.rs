//! The presentation snapshot (architecture section 5.3): the only simulation-derived data
//! the render side reads while the next tick runs. Extract writes one after a tick, the
//! presentation schedule reads the newest published and the one it follows and
//! interpolates between them, and three buffers are enough for each side to wait for the
//! other at most for one exchange of buffer handles.
//!
//! A snapshot holds the *presented rows* of its world: the rows presentation works on,
//! each with every extracted component it has. The bulk of a scene, rows that are only an
//! instance, is not in it: extract writes those to the
//! [`InstanceSink`](crate::InstanceSink) and nowhere else (`README.md`, "The snapshot").
//!
//! Safe code only: the buffers are shared by `Arc`, and the one exchange of them between
//! the two sides is under a lock held for the exchange and for nothing else. The events
//! wait there as the batches `publish` was given and are folded and dropped outside it, so
//! the work under the lock does not grow with what the reader has not acquired: `publish`
//! links a list node it allocated before taking the lock.

use core::marker::PhantomData;
use std::collections::{BTreeMap, LinkedList};
use std::sync::{Arc, Mutex, MutexGuard};

use lactose_core::{ComponentKey, FixedHashMap, StableKey, Tick};
use lactose_ecs::{PresentEmission, PresentEventBatch, PresentEventKey, WorldKind};
use lactose_physics_api::{Teleported, Transform};
use lactose_reflect::{Component, bytemuck};

use crate::{tick_after, tick_before};

/// A snapshot channel: the writing half, which [`ExtractState::set_snapshot`] takes, and
/// the reading half, which the presentation schedule keeps. One channel serves one world;
/// a client's two worlds each have their own, and presentation reads both.
///
/// The channel owns exactly three snapshot buffers for its whole life. The reader holds
/// at most two, the pair of its last [`acquire`](SnapshotReader::acquire); the writer
/// writes the third.
///
/// [`ExtractState::set_snapshot`]: crate::ExtractState::set_snapshot
#[must_use]
pub fn snapshot_channel() -> (SnapshotWriter, SnapshotReader) {
    let free = (0..3).map(|_| Arc::new(PresentSnapshot::empty())).collect();
    let exchange = Arc::new(Mutex::new(Exchange { free, ..Exchange::default() }));
    let writer =
        SnapshotWriter { exchange: Arc::clone(&exchange), pending: None, pending_full: false };
    let reader = SnapshotReader { exchange, held: None, events: PresentEventBatch::default() };
    (writer, reader)
}

/// What the two halves share: which buffers are published, which is free, the events
/// published since the reader last acquired, and whether a cut is still to reach it.
/// Snapshots are told apart by their publication number, not their buffer, which a take-back
/// writes again.
#[derive(Debug, Default)]
struct Exchange {
    /// The snapshots published after the reader's newer one, older first: at most two once
    /// the reader has acquired, three before. An acquire stops publishing every one published
    /// before its newer snapshot.
    published: Vec<Published>,
    /// The buffers that are neither published, held by the reader nor being written.
    free: Vec<Arc<PresentSnapshot>>,
    /// Every batch with an event published since the reader's last acquire, in publication
    /// order: the acquire takes them and folds them once the lock is released. None is
    /// kept once the reader is gone.
    events: LinkedList<PresentEventBatch>,
    /// How many snapshots were ever published: the next one's publication number.
    publications: u64,
    /// The snapshot published last, whether or not it is still published: a snapshot of an
    /// earlier tick is a cut, and a prediction world's full pass of its tick takes its place.
    last: Option<LastPublished>,
    /// The publication numbers of the reader's pair, older first, and so the snapshot a
    /// replacement of its newer one follows.
    held: Option<[u64; 2]>,
    /// The mark: the publication number of the last snapshot published that was a cut by its
    /// own cause, until an acquire returns a newer snapshot published at or after it. While
    /// it is set every snapshot published is marked a cut, and a snapshot marked so does not
    /// move it (`README.md`, "The triple buffer").
    cut_mark: Option<u64>,
    /// `publications` at the reader's last acquire that returned a new pair: an acquire with
    /// nothing published since returns the same pair.
    acquired: Option<u64>,
    /// The reading half was dropped: every batch published from then on is dropped.
    reader_gone: bool,
}

impl Exchange {
    /// A buffer for the writer: a free one, else the newest published, which the reader does
    /// not hold and which leaves publication. With three buffers one of the two is there:
    /// the reader holds two at most, and the writer holds none when it asks.
    fn take_buffer(&mut self) -> Arc<PresentSnapshot> {
        match self.free.pop() {
            Some(buffer) => buffer,
            None => self.published.pop().expect("three buffers: one free or published").snapshot,
        }
    }

    /// Puts a buffer that left publication or the reader's pair on the free list, once.
    /// Called under the lock with every other handle to it dropped or about to be dropped
    /// under the same lock, so the writer that takes it next finds it unshared.
    fn release(&mut self, buffer: Arc<PresentSnapshot>) {
        if !self.free.iter().any(|free| Arc::ptr_eq(free, &buffer)) {
            self.free.push(buffer);
        }
    }
}

/// Folds `batches`, in publication order, into one by the world's own rule: an emission
/// followed by its cancellation is in neither list, and so is a cancellation followed by an
/// emission of the same key. One take never holds both for a key, so the order within a
/// batch is moot; the world delivers a key again only after cancelling it, so a key is in
/// each list once at most, and an index by key keeps the fold linear in the events. Called
/// with the channel's lock released.
fn fold(batches: LinkedList<PresentEventBatch>) -> PresentEventBatch {
    if batches.len() <= 1 {
        return batches.into_iter().next().unwrap_or_default();
    }
    let mut emitted: Vec<Option<PresentEmission>> = Vec::new();
    let mut cancelled: Vec<Option<PresentEventKey>> = Vec::new();
    let mut emitted_at: FixedHashMap<PresentEventKey, usize> = FixedHashMap::new();
    let mut cancelled_at: FixedHashMap<PresentEventKey, usize> = FixedHashMap::new();
    for batch in batches {
        for key in batch.cancelled {
            match emitted_at.remove(&key) {
                Some(index) => emitted[index] = None,
                None => {
                    cancelled_at.insert(key, cancelled.len());
                    cancelled.push(Some(key));
                }
            }
        }
        for emission in batch.emitted {
            match cancelled_at.remove(&emission.key) {
                Some(index) => cancelled[index] = None,
                None => {
                    emitted_at.insert(emission.key, emitted.len());
                    emitted.push(Some(emission));
                }
            }
        }
    }
    PresentEventBatch {
        emitted: emitted.into_iter().flatten().collect(),
        cancelled: cancelled.into_iter().flatten().collect(),
    }
}

/// Whether `batch` holds no event.
fn is_empty(batch: &PresentEventBatch) -> bool {
    batch.emitted.is_empty() && batch.cancelled.is_empty()
}

/// The channel's lock. A panic while it is held aborts a binary, so a poisoned lock is met
/// only by a test that has already failed.
fn lock(exchange: &Mutex<Exchange>) -> MutexGuard<'_, Exchange> {
    exchange.lock().expect("nothing panics while the channel's lock is held")
}

/// A published snapshot and what an acquire pairs it with.
#[derive(Debug)]
struct Published {
    snapshot: Arc<PresentSnapshot>,
    /// Its publication number.
    number: u64,
    /// The publication number of the snapshot it follows: the newest the channel had when
    /// it was published, or for one that took another's place what that one followed. An
    /// acquire pairs it with that snapshot if the channel still has it, else with the
    /// reader's newer one.
    follows: Option<u64>,
}

/// What the exchange keeps of the snapshot published last once its buffer may be written
/// again.
#[derive(Debug, Clone, Copy)]
struct LastPublished {
    number: u64,
    tick: Tick,
    cut: bool,
    follows: Option<u64>,
}

/// The writing half of a snapshot channel. It has no public operation: a pass writes
/// through it once it is installed with
/// [`ExtractState::set_snapshot`](crate::ExtractState::set_snapshot), and
/// [`ExtractState::publish`](crate::ExtractState::publish) publishes through it.
#[derive(Debug)]
pub struct SnapshotWriter {
    exchange: Arc<Mutex<Exchange>>,
    /// The buffer a pass has written and `publish` has not yet published.
    pending: Option<Arc<PresentSnapshot>>,
    /// Whether a full pass was among those that wrote `pending`: in a prediction world it
    /// then takes the place of a published snapshot of its own tick (`README.md`, "Events
    /// and publication").
    pending_full: bool,
}

impl SnapshotWriter {
    /// The buffer a pass writes: the unpublished snapshot if a pass since the last
    /// publication wrote one, else a buffer from the channel, not a cut. `full` records that
    /// a full pass wrote it.
    pub(crate) fn buffer(&mut self, full: bool) -> &mut PresentSnapshot {
        self.pending_full |= full;
        let fresh = self.pending.is_none();
        let exchange = &self.exchange;
        let buffer = self.pending.get_or_insert_with(|| lock(exchange).take_buffer());
        let snapshot = Arc::get_mut(buffer).expect("the buffer the writer writes is unshared");
        if fresh {
            snapshot.cut = false;
        }
        snapshot
    }

    /// The buffer the passes since the last publication wrote, if one did: what the unit
    /// tests compare.
    #[cfg(test)]
    pub(crate) fn pending(&self) -> Option<&PresentSnapshot> {
        self.pending.as_deref()
    }

    /// Publishes the unpublished snapshot, if there is one, and adds `events` to those the
    /// reader's next acquire returns, in one exchange (`README.md`, "The triple buffer" and
    /// "Events and publication"). A batch no reader will take is dropped once the lock is
    /// released.
    pub(crate) fn publish(&mut self, events: PresentEventBatch) {
        let full = core::mem::take(&mut self.pending_full);
        let pending = self.pending.take();
        // The list's node is allocated here, so linking it under the lock allocates nothing.
        let mut unread = LinkedList::new();
        if !is_empty(&events) {
            unread.push_back(events);
        }
        let mut exchange = lock(&self.exchange);
        if !exchange.reader_gone {
            exchange.events.append(&mut unread);
        }
        if let Some(snapshot) = pending {
            exchange.publish(snapshot, full);
        }
        drop(exchange);
        drop(unread);
    }
}

impl Exchange {
    /// The snapshot half of [`SnapshotWriter::publish`]: `snapshot`, written by passes of
    /// which one was full if `full`, becomes the newest published.
    fn publish(&mut self, mut snapshot: Arc<PresentSnapshot>, full: bool) {
        let number = self.publications;
        self.publications += 1;
        let last = self.last;
        let written = Arc::get_mut(&mut snapshot).expect("the buffer the writer wrote is unshared");
        // A cut by its own cause: a pass made it one, or its tick is earlier than the tick of
        // the snapshot published last.
        let mut own = written.cut || last.is_some_and(|last| tick_before(written.tick, last.tick));
        let replaced = last.filter(|last| {
            written.kind == WorldKind::Prediction && full && last.tick == written.tick
        });
        let follows = match replaced {
            Some(last) => {
                own |= last.cut;
                last.follows
            }
            None => self
                .published
                .last()
                .map(|published| published.number)
                .or(self.held.map(|[_, newer]| newer)),
        };
        if own {
            self.cut_mark = Some(number);
        }
        written.cut = own || self.cut_mark.is_some();
        let tick = written.tick;
        self.last = Some(LastPublished { number, tick, cut: written.cut, follows });
        let published = Published { snapshot, number, follows };
        let place = replaced.and_then(|last| {
            self.published.iter().position(|published| published.number == last.number)
        });
        match place {
            Some(index) => {
                let replaced = core::mem::replace(&mut self.published[index], published);
                self.release(replaced.snapshot);
            }
            None => self.published.push(published),
        }
    }
}

/// The reading half of a snapshot channel: one reader, on the render side.
#[derive(Debug)]
pub struct SnapshotReader {
    exchange: Arc<Mutex<Exchange>>,
    /// The pair of the last acquire, older first.
    held: Option<[Arc<PresentSnapshot>; 2]>,
    /// The events the last acquire took.
    events: PresentEventBatch,
}

impl SnapshotReader {
    /// The newest published snapshot, the one it follows, and every event published since
    /// the previous call, or `None` if nothing was ever published. For a reader that
    /// acquires every publication the pair is the two newest published. The frame borrows
    /// the reader, so one frame is alive at a time, and the reader keeps the frame's two
    /// snapshots until the next call: until then no pass writes either.
    ///
    /// It waits for a pass at most for one exchange: it exchanges buffer handles and takes
    /// the published event batches under the channel's lock, copies nothing, and folds the
    /// batches once the lock is released. Called again before a publication it returns the
    /// same pair and the events published since, if any. Called while a pass writes the
    /// snapshot published after the reader's newer one, which the pass took back, it
    /// returns that newer one twice, so the pass after finds a buffer free. Before the
    /// first publication it returns `None` and leaves the events published so far for the
    /// first frame. `README.md`, "The snapshot", states what the pair is when the reader
    /// runs slower than the passes, and that no event is lost then.
    pub fn acquire(&mut self) -> Option<PresentFrame<'_>> {
        let mut exchange = lock(&self.exchange);
        if exchange.publications == 0 {
            return None;
        }
        let batches = core::mem::take(&mut exchange.events);
        if exchange.acquired != Some(exchange.publications) {
            exchange.acquired = Some(exchange.publications);
            let held = self.held.take().zip(exchange.held);
            let (pair, numbers) = match exchange.published.pop() {
                Some(newer) => pair_with(&mut exchange, held, newer),
                // Nothing published after the reader's newer snapshot is still published: a
                // pass took it back and is writing it. Keeping the newer one twice and
                // letting the older go leaves the next pass a free buffer.
                None => {
                    let ([older, newer], [_, number]) =
                        held.expect("what was published and is no more was acquired");
                    if !Arc::ptr_eq(&older, &newer) {
                        exchange.release(older);
                    }
                    ([Arc::clone(&newer), newer], [number, number])
                }
            };
            // Once acquired, a snapshot follows the older of its pair, and a replacement of
            // it follows what it did.
            if let Some(last) = exchange.last.as_mut().filter(|last| last.number == numbers[1]) {
                last.follows = Some(numbers[0]);
            }
            if exchange.cut_mark.is_some_and(|mark| numbers[1] >= mark) {
                exchange.cut_mark = None;
            }
            exchange.held = Some(numbers);
            self.held = Some(pair);
        }
        drop(exchange);
        self.events = fold(batches);
        let [older, newer] = self.held.as_ref().expect("a publication was acquired");
        Some(PresentFrame { older, newer, events: &self.events })
    }
}

/// The pair of an acquire whose newer snapshot is `newer`, the newest published: its older
/// one is the snapshot it follows if the channel still has it (published, or in `held`, the
/// reader's last pair), else the reader's newer one, else `newer` itself. Every other
/// buffer published or held goes back to the free list: no later pair holds it.
fn pair_with(
    exchange: &mut Exchange,
    held: Option<([Arc<PresentSnapshot>; 2], [u64; 2])>,
    newer: Published,
) -> ([Arc<PresentSnapshot>; 2], [u64; 2]) {
    let mut candidates: Vec<(u64, Arc<PresentSnapshot>)> = Vec::new();
    let mut fallback = None;
    if let Some(([held_older, held_newer], [older_number, newer_number])) = held {
        candidates.push((older_number, held_older));
        candidates.push((newer_number, held_newer));
        fallback = Some(newer_number);
    }
    candidates.extend(
        core::mem::take(&mut exchange.published)
            .into_iter()
            .map(|published| (published.number, published.snapshot)),
    );
    let older_number = newer
        .follows
        .filter(|&number| candidates.iter().any(|&(candidate, _)| candidate == number))
        .or(fallback)
        .unwrap_or(newer.number);
    let mut older: Option<Arc<PresentSnapshot>> = None;
    for (number, buffer) in candidates {
        if number == older_number && older.is_none() {
            older = Some(buffer);
        } else if older.as_ref().is_none_or(|kept| !Arc::ptr_eq(kept, &buffer)) {
            exchange.release(buffer);
        }
    }
    let older = older.unwrap_or_else(|| Arc::clone(&newer.snapshot));
    ([older, newer.snapshot], [older_number, newer.number])
}

impl Drop for SnapshotReader {
    /// Nothing will acquire again: `publish` drops every batch from now on, and the
    /// reader's buffers go back to the channel. The batches it never took are dropped once
    /// the lock is released.
    fn drop(&mut self) {
        let mut exchange = lock(&self.exchange);
        exchange.reader_gone = true;
        let unread = core::mem::take(&mut exchange.events);
        exchange.held = None;
        if let Some([older, newer]) = self.held.take() {
            exchange.release(older);
            exchange.release(newer);
        }
        drop(exchange);
        drop(unread);
    }
}

/// What extract copied out of one world at one pass: the world's presented rows, each
/// with every extracted component it has, by `StableKey`.
#[derive(Debug)]
pub struct PresentSnapshot {
    kind: WorldKind,
    tick: Tick,
    cut: bool,
    columns: BTreeMap<ComponentKey, SnapshotColumn>,
}

/// One component's rows in a snapshot: the keys in ascending order and the values'
/// bytes in the same order.
#[derive(Debug, Default)]
#[cfg_attr(test, derive(PartialEq, Eq))]
struct SnapshotColumn {
    keys: Vec<StableKey>,
    bytes: Vec<u8>,
}

/// One component's column as [`PresentSnapshot::fill`] hands it out: a row goes in with its
/// key and its value's bytes together, so the two cannot part.
pub(crate) struct ColumnRows<'a>(&'a mut SnapshotColumn);

impl ColumnRows<'_> {
    /// Appends a row: its key, above every key appended before it, and its value's bytes, as
    /// many as every other row's.
    pub(crate) fn push(&mut self, key: StableKey, bytes: &[u8]) {
        let column = &mut *self.0;
        debug_assert!(
            column.keys.last().is_none_or(|&last| last < key),
            "rows in ascending key order"
        );
        debug_assert_eq!(column.bytes.len(), column.keys.len() * bytes.len(), "rows of one size");
        column.keys.push(key);
        column.bytes.extend_from_slice(bytes);
    }
}

impl PresentSnapshot {
    /// The kind of the world the snapshot is of.
    #[must_use]
    pub const fn kind(&self) -> WorldKind {
        self.kind
    }

    /// The world's tick at the pass that wrote the snapshot.
    #[must_use]
    pub const fn tick(&self) -> Tick {
        self.tick
    }

    /// Whether nothing before this snapshot leads into it, so that presentation shows its
    /// state at once and blends nothing across it: set when a full pass in a world that is
    /// not a prediction world wrote it, when its tick is earlier than the tick of the
    /// snapshot published last, when it takes the place of a cut, and on every snapshot
    /// published after a cut until an acquire returns one published at or after it, so the
    /// first acquire whose newer snapshot was published since a cut has a cut as that
    /// snapshot (`README.md`, "The triple buffer").
    #[must_use]
    pub const fn cut(&self) -> bool {
        self.cut
    }

    /// The `T` of the row `key`, by value, or `None` if the snapshot holds no such row or
    /// the row has no `T`.
    ///
    /// Does not build for a `T` that is not declared `extracted`, which no snapshot holds;
    /// the assertion is evaluated where the call is instantiated, so a build reports it
    /// and `cargo check` does not.
    #[must_use]
    pub fn get<T: Component>(&self, key: StableKey) -> Option<T> {
        const { assert!(is_extracted::<T>(), "a snapshot holds `extracted` components alone") };
        let column = self.columns.get(&T::KEY)?;
        let row = column.keys.binary_search(&key).ok()?;
        let size = size_of::<T>();
        Some(read(&column.bytes[row * size..(row + 1) * size]))
    }

    /// Every row that has a `T`, with its value, in ascending key order. Does not build
    /// for a `T` that is not declared `extracted`, as [`get`](Self::get) does not.
    #[must_use]
    pub fn rows<T: Component>(&self) -> SnapshotRows<'_, T> {
        const { assert!(is_extracted::<T>(), "a snapshot holds `extracted` components alone") };
        let (keys, bytes) = match self.columns.get(&T::KEY) {
            Some(column) => (&column.keys[..], &column.bytes[..]),
            None => (&[][..], &[][..]),
        };
        SnapshotRows { keys: keys.iter(), bytes, marker: PhantomData }
    }

    /// A buffer no pass has written yet.
    pub(crate) fn empty() -> Self {
        Self { kind: WorldKind::Server, tick: Tick(0), cut: false, columns: BTreeMap::new() }
    }

    /// Makes the snapshot `kind`'s at `tick`, holding for each of `components` the rows
    /// `copy` pushes to the component's column, in ascending key order; the
    /// columns' allocations are kept for the next pass. `cut` makes it a cut; a snapshot a
    /// pass made one stays one until it is published.
    pub(crate) fn fill(
        &mut self,
        kind: WorldKind,
        tick: Tick,
        cut: bool,
        components: &[ComponentKey],
        mut copy: impl FnMut(ComponentKey, &mut ColumnRows<'_>),
    ) {
        self.kind = kind;
        self.tick = tick;
        self.cut |= cut;
        for column in self.columns.values_mut() {
            column.keys.clear();
            column.bytes.clear();
        }
        for &component in components {
            copy(component, &mut ColumnRows(self.columns.entry(component).or_default()));
        }
        self.columns.retain(|_, column| !column.keys.is_empty());
    }

    /// Whether the two hold the same world kind, tick and rows, whatever their cuts.
    #[cfg(test)]
    pub(crate) fn same_rows(&self, other: &Self) -> bool {
        self.kind == other.kind && self.tick == other.tick && self.columns == other.columns
    }
}

/// The value `bytes` hold, read unaligned. They were copied from a column of `T`, so a
/// failure is a broken invariant.
fn read<T: Component>(bytes: &[u8]) -> T {
    bytemuck::checked::try_pod_read_unaligned(bytes).expect("a snapshot holds valid values")
}

/// Whether `T` is declared `extracted`.
const fn is_extracted<T: Component>() -> bool {
    T::SCHEMA.info().tracking().extracted
}

/// The iterator of [`PresentSnapshot::rows`]: the key and the value of every row that has
/// a `T`, in ascending key order.
#[derive(Debug)]
pub struct SnapshotRows<'a, T> {
    keys: core::slice::Iter<'a, StableKey>,
    bytes: &'a [u8],
    marker: PhantomData<fn() -> T>,
}

impl<T: Component> Iterator for SnapshotRows<'_, T> {
    type Item = (StableKey, T);

    fn next(&mut self) -> Option<(StableKey, T)> {
        let key = *self.keys.next()?;
        let (value, rest) = self.bytes.split_at(size_of::<T>());
        self.bytes = rest;
        Some((key, read(value)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.keys.size_hint()
    }
}

impl<T: Component> ExactSizeIterator for SnapshotRows<'_, T> {}

/// What one frame presents from: the newest published snapshot of a world and the one it
/// follows, and the events published since the previous frame was acquired.
#[derive(Clone, Copy, Debug)]
pub struct PresentFrame<'a> {
    older: &'a PresentSnapshot,
    newer: &'a PresentSnapshot,
    events: &'a PresentEventBatch,
}

impl<'a> PresentFrame<'a> {
    /// The older snapshot of the pair: the state a frame interpolates from. It is the
    /// newer one itself while one snapshot has been published, and at an acquire made while
    /// a pass writes a snapshot it took back (`README.md`, "The triple buffer").
    #[must_use]
    pub const fn older(&self) -> &'a PresentSnapshot {
        self.older
    }

    /// The newer snapshot of the pair: the state a frame interpolates to.
    #[must_use]
    pub const fn newer(&self) -> &'a PresentSnapshot {
        self.newer
    }

    /// The presentation events published since the previous frame was acquired: every
    /// batch `app` published, in publication order, folded as the world folds one take (an
    /// emission cancelled by a later batch is in neither list). Each emission is in
    /// exactly one frame's batch, whether or not the snapshot it was published with was
    /// ever part of a pair.
    #[must_use]
    pub const fn events(&self) -> &'a PresentEventBatch {
        self.events
    }

    /// The previous and current transform of the row `key`, or `None` if the newer
    /// snapshot holds no transform for it: the row is not a presented row, or has no
    /// `Transform`, or the world is a client's replicated world, whose snapshots hold no
    /// transform (`README.md`, "The snapshot").
    ///
    /// `current` is the newer snapshot's. `previous` is the older snapshot's, or `current`
    /// where the older holds none. `no_interpolate` is set when the newer snapshot is a
    /// [`cut`](PresentSnapshot::cut), when the older holds no transform for the key, when
    /// the two snapshots are of one tick, or when the row's `Teleported` in the newer
    /// snapshot is not zero and is a tick after the older snapshot's, by the wrapping
    /// comparison of ticks.
    #[must_use]
    pub fn transform(&self, key: StableKey) -> Option<TransformPair> {
        let current = self.newer.get::<Transform>(key)?;
        let previous = self.older.get::<Transform>(key);
        let teleported =
            self.newer.get::<Teleported>(key).map_or(Tick(0), |teleported| teleported.0);
        let no_interpolate = self.newer.cut
            || previous.is_none()
            || self.older.tick == self.newer.tick
            || (teleported != Tick(0) && tick_after(teleported, self.older.tick));
        Some(TransformPair { previous: previous.unwrap_or(current), current, no_interpolate })
    }
}

/// The two transforms a frame interpolates a presented row between, in simulation space:
/// presentation interpolates, then converts with `RenderPosition::from_sim` or
/// [`RenderTransform::from_sim`](crate::RenderTransform::from_sim).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TransformPair {
    /// The transform in the older snapshot.
    pub previous: Transform,
    /// The transform in the newer snapshot.
    pub current: Transform,
    /// Whether `current` is a jump and not a movement: show `current` and draw nothing
    /// between the two.
    pub no_interpolate: bool,
}
