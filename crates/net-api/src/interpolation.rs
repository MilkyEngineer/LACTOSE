//! The interpolation buffer (architecture sections 5.3 and 9.3): per remote entity, the
//! received states by server tick, written by the network thread and sampled by the
//! presentation schedule each frame, about two send intervals in the past, through a
//! sequence lock per slot and a batch call that fills the caller's buffer without
//! locking. It never feeds simulation rows.
//!
//! Host-only: the sampler runs on the render side and the writer on the network thread.

use core::sync::atomic::{AtomicU32, AtomicU64};
use std::sync::Arc;

use lactose_core::{Result, Tick};
use lactose_math::{Quat, Vec3};

use crate::staged::InterpolationSlot;

/// How many received states one slot keeps: the newest eight by tick, about 270
/// milliseconds at a 30 Hz send rate, so the sampling point two send intervals back has
/// a state on either side under jitter.
pub const STATES_PER_SLOT: usize = 8;

/// The words one state occupies: the tick and flags packed in one, then three for the
/// position and four for the rotation, each scalar as its canonical bits widened to 64.
const STATE_WORDS: usize = 8;

/// One received state of one remote entity, in world space: the network thread
/// dequantises the wire position, adds the region origin and hands this to the writer.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ReceivedState {
    /// The server tick the state is from.
    pub tick: Tick,
    /// The no-interpolate flag of architecture section 5.3: the entity teleported into
    /// this state, so the sampler holds the previous state until this tick and never
    /// blends across it.
    pub teleport: bool,
    /// The position, in world metres.
    pub position: Vec3,
    /// The rotation, a unit quaternion.
    pub rotation: Quat,
}

/// A point in server time to sample at: a tick and a fraction of the way to the next, in
/// 65536ths. The presentation schedule computes it from the clock estimate, about two
/// send intervals behind the newest received tick.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct PresentationTime {
    /// The tick.
    pub tick: Tick,
    /// The fraction of the way from `tick` to the next, in 65536ths.
    pub fraction: u16,
}

/// How a sample was produced.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SampleStatus {
    /// Blended between the two received states around the presentation time.
    Interpolated,
    /// The newest state, because the presentation time is at or past it (the entity's
    /// updates are late), or the oldest, because the time is before every state kept, or
    /// the state before a teleport while the time is before the teleport's tick.
    Held,
    /// The slot holds no state, or it was released and perhaps reused: the caller keeps
    /// whatever it showed last.
    Missing,
}

/// What the sampler produces for one slot.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RemoteSample {
    /// How it was produced.
    pub status: SampleStatus,
    /// The position, in world metres; `Vec3::ZERO` when missing.
    pub position: Vec3,
    /// The rotation; `Quat::IDENTITY` when missing.
    pub rotation: Quat,
}

impl RemoteSample {
    /// The sample of a slot with nothing in it.
    pub const MISSING: Self =
        Self { status: SampleStatus::Missing, position: Vec3::ZERO, rotation: Quat::IDENTITY };
}

/// One slot: a sequence lock over a ring of states, every word an atomic so the lock
/// needs no `unsafe`. The writer makes `sequence` odd, stores, then makes it even; a
/// reader loads the sequence, the words and the sequence again and retries when the two
/// differ or the first was odd. `generation` is the slot's current allocation, zero when
/// free.
struct Slot {
    sequence: AtomicU32,
    generation: AtomicU32,
    /// How many states the ring holds, up to [`STATES_PER_SLOT`], and which is newest.
    count: AtomicU32,
    states: [[AtomicU64; STATE_WORDS]; STATES_PER_SLOT],
}

impl Slot {
    const fn new() -> Self {
        Self {
            sequence: AtomicU32::new(0),
            generation: AtomicU32::new(0),
            count: AtomicU32::new(0),
            states: [const { [const { AtomicU64::new(0) }; STATE_WORDS] }; STATES_PER_SLOT],
        }
    }
}

/// The buffer: a fixed number of slots, each a sequence-locked ring of received states.
/// Shared between its one [`InterpolationWriter`] and any number of
/// [`InterpolationReader`]s through an `Arc`.
pub struct InterpolationBuffer {
    slots: Box<[Slot]>,
}

impl InterpolationBuffer {
    /// A buffer of `capacity` slots, split into its one writer and a reader (which clones
    /// for more). Allocates once.
    #[must_use]
    pub fn split(capacity: u32) -> (InterpolationWriter, InterpolationReader) {
        let slots = (0..capacity).map(|_| Slot::new()).collect();
        let buffer = Arc::new(Self { slots });
        let writer = InterpolationWriter {
            buffer: buffer.clone(),
            free: Vec::new(),
            next_unused: 0,
            generations: vec![0; capacity as usize],
        };
        (writer, InterpolationReader { buffer })
    }

    /// How many slots the buffer has.
    #[must_use]
    pub fn capacity(&self) -> u32 {
        self.slots.len() as u32
    }
}

impl core::fmt::Debug for InterpolationBuffer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InterpolationBuffer").field("capacity", &self.capacity()).finish()
    }
}

/// The writer side, owned by the network thread: allocates a slot when a remote entity
/// first appears (the staged spawn carries it), pushes every received state, and
/// releases the slot on the entity's tombstone. One writer per buffer; `&mut self`
/// methods, so two threads cannot write at once.
pub struct InterpolationWriter {
    buffer: Arc<InterpolationBuffer>,
    free: Vec<u32>,
    next_unused: u32,
    generations: Vec<u32>,
}

impl InterpolationWriter {
    /// Allocates a free slot: the most recently released, else the next never-used, at a
    /// new generation (never zero, wrapping past `u32::MAX` to one). The slot starts
    /// empty. Fails with `ResourceExhausted` when every slot is allocated.
    pub fn allocate(&mut self) -> Result<InterpolationSlot> {
        todo!()
    }

    /// Pushes `state` into `slot`'s ring, which keeps the [`STATES_PER_SLOT`] newest
    /// states by tick: a state older than the oldest kept is dropped, one with the tick of
    /// a kept state replaces it, so arrival order does not matter. Readers never observe
    /// a half-written ring. Fails with `NotFound` if `slot` is stale or unallocated.
    pub fn push(&mut self, slot: InterpolationSlot, state: ReceivedState) -> Result<()> {
        todo!()
    }

    /// Releases `slot`: it reads as missing from now on, under this generation and the
    /// next, until reallocated. Fails with `NotFound` if `slot` is stale or unallocated.
    pub fn release(&mut self, slot: InterpolationSlot) -> Result<()> {
        todo!()
    }

    /// How many slots the buffer has.
    #[must_use]
    pub fn capacity(&self) -> u32 {
        self.buffer.capacity()
    }
}

impl core::fmt::Debug for InterpolationWriter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InterpolationWriter").field("capacity", &self.capacity()).finish()
    }
}

/// The reader side, the `net-api` resource of architecture section 5.3: the presentation
/// schedule holds one and samples every frame. Cloneable; any number of threads may
/// sample at once, none ever blocks, and no sample allocates.
#[derive(Clone)]
pub struct InterpolationReader {
    buffer: Arc<InterpolationBuffer>,
}

impl InterpolationReader {
    /// The state of `slot` at `at`: `Interpolated` between the two kept states around
    /// `at` (position by `Vec3::lerp`, rotation by `Quat::nlerp`, the parameter being
    /// `at`'s distance from the earlier state over the two states' tick distance);
    /// `Held` at the newest state when `at` is not before it, at the oldest when `at` is
    /// before every kept state, or at the state before a teleport while `at` is before
    /// the teleport's tick; `Missing` when the slot holds nothing or `slot` is stale. A
    /// torn read is retried; the call never blocks and never locks.
    #[must_use]
    pub fn sample(&self, slot: InterpolationSlot, at: PresentationTime) -> RemoteSample {
        todo!()
    }

    /// [`sample`](Self::sample) for every slot of `slots` into `out[i]`, which must be at
    /// least as long; the batch call of architecture section 5.3. Allocates nothing.
    pub fn sample_batch(
        &self,
        slots: &[InterpolationSlot],
        at: PresentationTime,
        out: &mut [RemoteSample],
    ) {
        todo!()
    }

    /// How many slots the buffer has.
    #[must_use]
    pub fn capacity(&self) -> u32 {
        self.buffer.capacity()
    }
}

impl core::fmt::Debug for InterpolationReader {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InterpolationReader").field("capacity", &self.capacity()).finish()
    }
}
