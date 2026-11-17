//! The presentation snapshot (architecture section 5.3): the only simulation-derived data
//! the render side reads while the next tick runs. Extract writes one after a tick, the
//! presentation schedule reads the two newest published and interpolates between them, and
//! three buffers are enough for the two never to wait for each other.
//!
//! A snapshot holds the *presented rows* of its world: the rows presentation works on,
//! each with every extracted component it has. The bulk of a scene, rows that are only an
//! instance, is not in it: extract writes those to the
//! [`InstanceSink`](crate::InstanceSink) and nowhere else (`README.md`, "The snapshot").
//!
//! Safe code only: the buffers are shared by `Arc`, and the one exchange of them between
//! the two sides is under a lock held for the exchange and for nothing else.

use core::marker::PhantomData;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use lactose_core::{ComponentKey, StableKey, Tick};
use lactose_ecs::{PresentEventBatch, WorldKind};
use lactose_physics_api::Transform;
use lactose_reflect::Component;

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
    todo!()
}

/// What the two halves share: which buffers are published, which is free, and the events
/// published since the reader last acquired.
#[derive(Debug, Default)]
struct Exchange {
    /// The two newest published snapshots, older first; one after the first publication.
    published: Vec<Arc<PresentSnapshot>>,
    /// The buffers that are neither published nor being written.
    free: Vec<Arc<PresentSnapshot>>,
    /// Every batch published since the reader's last acquire, folded into one.
    events: PresentEventBatch,
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
    /// The two newest published snapshots and every event published since the previous
    /// call, or `None` if nothing was ever published. The frame borrows the reader, so one
    /// frame is alive at a time, and the reader keeps the frame's two snapshots until the
    /// next call: until then no pass writes either.
    ///
    /// It never waits for a pass: it exchanges buffer handles under the channel's lock
    /// and copies nothing. Called again before a publication it returns the same pair and
    /// no event. `README.md`, "The snapshot", states what the pair is when the reader
    /// runs slower than the passes, and that no event is lost then.
    pub fn acquire(&mut self) -> Option<PresentFrame<'_>> {
        todo!()
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
struct SnapshotColumn {
    keys: Vec<StableKey>,
    bytes: Vec<u8>,
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
    /// not a prediction world wrote it, or wrote a snapshot it replaced before the reader
    /// acquired that one (`README.md`, "The triple buffer").
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
        todo!()
    }

    /// Every row that has a `T`, with its value, in ascending key order. Does not build
    /// for a `T` that is not declared `extracted`, as [`get`](Self::get) does not.
    #[must_use]
    pub fn rows<T: Component>(&self) -> SnapshotRows<'_, T> {
        const { assert!(is_extracted::<T>(), "a snapshot holds `extracted` components alone") };
        todo!()
    }
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
        todo!()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.keys.size_hint()
    }
}

impl<T: Component> ExactSizeIterator for SnapshotRows<'_, T> {}

/// What one frame presents from: the two newest published snapshots of a world, and the
/// events published since the previous frame was acquired.
#[derive(Clone, Copy, Debug)]
pub struct PresentFrame<'a> {
    older: &'a PresentSnapshot,
    newer: &'a PresentSnapshot,
    events: &'a PresentEventBatch,
}

impl<'a> PresentFrame<'a> {
    /// The older snapshot of the pair: the state a frame interpolates from. It is the
    /// newer one itself while only one snapshot has been published.
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
        todo!()
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
