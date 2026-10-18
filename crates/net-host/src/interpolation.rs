//! The interpolation buffer (architecture sections 5.3 and 9.3): per remote entity, the
//! received states by server tick, written by the network thread and sampled by the
//! presentation schedule each frame, about two send intervals in the past, through a
//! sequence lock per slot and a batch call that fills the caller's buffer without
//! locking. It never feeds simulation rows.
//!
//! Host-only: the sampler runs on the render side and the writer on the network thread.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};
use std::sync::Arc;

use lactose_core::{Error, ErrorKind, Result, Tick};
use lactose_math::{Quat, Scalar, ScalarBits, Vec3};
use lactose_net_api::InterpolationSlot;

/// How many received states one slot keeps: the newest eight by tick, about 270
/// milliseconds at a 30 Hz send rate, so the sampling point two send intervals back has
/// a state on either side under jitter.
pub const STATES_PER_SLOT: usize = 8;

/// The words one state occupies: the tick and the teleport tick packed in one, then three
/// for the position and four for the rotation, each scalar as its canonical bits widened
/// to 64.
const STATE_WORDS: usize = 8;

/// One received state of one remote entity, in world space: the network thread
/// dequantises the wire position, adds the region origin and hands this to the writer.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ReceivedState {
    /// The server tick the state is from.
    pub tick: Tick,
    /// The no-interpolate flag of architecture section 5.3 as D119 defines it: the
    /// `Teleported` value of the entity's row, the last tick it was placed without a
    /// sweep, zero for never. A tick rather than a flag, so a teleport is told from its
    /// states whatever order they arrive in: the sampler never blends across it ("The
    /// sampler" in the README).
    pub teleported: Tick,
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
    /// one of the two states around a teleport, since nothing blends across one.
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
/// needs no `unsafe`. The writer stores the odd sequence (`Relaxed`), issues
/// `fence(Release)`, stores the words (`Relaxed`), and stores the even sequence with
/// `Release`; a reader loads the sequence with `Acquire`, loads the words with `Relaxed`,
/// issues `fence(Acquire)`, and reloads the sequence, retrying when the first load was
/// odd or the two differ. A first load left `Relaxed` admits torn reads on aarch64 (the
/// review measured them); `Acquire` admits none. `generation` and `count` are words read
/// inside that window; `generation` is the slot's current allocation, zero when free, and
/// `allocate` and `release` bump the sequence as `push` does.
struct Slot {
    sequence: AtomicU32,
    generation: AtomicU32,
    /// How many states the ring holds, up to [`STATES_PER_SLOT`]: `states[..count]`,
    /// ascending by tick, so the newest is the last.
    count: AtomicU32,
    states: [[AtomicU64; STATE_WORDS]; STATES_PER_SLOT],
}

/// A slot's contents as one consistent read: the generation, then the states held,
/// ascending by tick, in `states[..count]`.
#[derive(Clone, Copy)]
struct Snapshot {
    generation: u32,
    count: usize,
    states: [ReceivedState; STATES_PER_SLOT],
}

const EMPTY_STATE: ReceivedState = ReceivedState {
    tick: Tick(0),
    teleported: Tick(0),
    position: Vec3::ZERO,
    rotation: Quat::IDENTITY,
};

impl Slot {
    const fn new() -> Self {
        Self {
            sequence: AtomicU32::new(0),
            generation: AtomicU32::new(0),
            count: AtomicU32::new(0),
            states: [const { [const { AtomicU64::new(0) }; STATE_WORDS] }; STATES_PER_SLOT],
        }
    }

    /// The writer's side of the lock: `write` stores words with `Relaxed` between the odd
    /// and the even sequence. Only the one writer calls it, so its own loads of the
    /// sequence need no ordering.
    fn write(&self, write: impl FnOnce(&Self)) {
        let sequence = self.sequence.load(Ordering::Relaxed);
        self.sequence.store(sequence.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        write(self);
        self.sequence.store(sequence.wrapping_add(2), Ordering::Release);
    }

    /// One consistent read, retried until no write overlapped it. The writer reads its
    /// own words through this too; it never retries there, since nothing else writes.
    fn read(&self) -> Snapshot {
        loop {
            let before = self.sequence.load(Ordering::Acquire);
            if before & 1 == 1 {
                core::hint::spin_loop();
                continue;
            }
            let generation = self.generation.load(Ordering::Relaxed);
            // A torn count is caught by the sequence check below; the bound keeps the
            // indexing in range meanwhile.
            let count = (self.count.load(Ordering::Relaxed) as usize).min(STATES_PER_SLOT);
            let mut words = [[0; STATE_WORDS]; STATES_PER_SLOT];
            for (held, words) in self.states[..count].iter().zip(&mut words) {
                for (word, value) in held.iter().zip(words) {
                    *value = word.load(Ordering::Relaxed);
                }
            }
            fence(Ordering::Acquire);
            if self.sequence.load(Ordering::Relaxed) == before {
                let mut states = [EMPTY_STATE; STATES_PER_SLOT];
                for (state, words) in states.iter_mut().zip(&words).take(count) {
                    *state = decode(words);
                }
                return Snapshot { generation, count, states };
            }
            core::hint::spin_loop();
        }
    }

    /// Stores `states` as the ring, inside a write.
    fn store_states(&self, states: &[ReceivedState]) {
        for (held, state) in self.states.iter().zip(states) {
            for (word, value) in held.iter().zip(encode(state)) {
                word.store(value, Ordering::Relaxed);
            }
        }
        let count = u32::try_from(states.len()).expect("at most STATES_PER_SLOT");
        self.count.store(count, Ordering::Relaxed);
    }
}

fn encode(state: &ReceivedState) -> [u64; STATE_WORDS] {
    let p = state.position;
    let r = state.rotation;
    [
        (u64::from(state.tick.0) << 32) | u64::from(state.teleported.0),
        widen(p.x.to_canonical_bits()),
        widen(p.y.to_canonical_bits()),
        widen(p.z.to_canonical_bits()),
        widen(r.x.to_canonical_bits()),
        widen(r.y.to_canonical_bits()),
        widen(r.z.to_canonical_bits()),
        widen(r.w.to_canonical_bits()),
    ]
}

fn decode(words: &[u64; STATE_WORDS]) -> ReceivedState {
    let scalar = |word: u64| Scalar::from_canonical_bits(narrow(word));
    ReceivedState {
        tick: Tick((words[0] >> 32) as u32),
        teleported: Tick(words[0] as u32),
        position: Vec3::new(scalar(words[1]), scalar(words[2]), scalar(words[3])),
        rotation: Quat::new(scalar(words[4]), scalar(words[5]), scalar(words[6]), scalar(words[7])),
    }
}

/// A scalar's canonical bits as one word, whatever the scalar's width.
#[allow(clippy::useless_conversion, reason = "an identity under the 64-bit scalar")]
fn widen(bits: ScalarBits) -> u64 {
    u64::from(bits)
}

/// The inverse of [`widen`].
#[allow(
    clippy::useless_conversion,
    clippy::unnecessary_fallible_conversions,
    reason = "an identity under the 64-bit scalar; a word `widen` wrote always fits"
)]
fn narrow(word: u64) -> ScalarBits {
    ScalarBits::try_from(word).unwrap_or_default()
}

/// Whether `tick` is after `than` under the wrapping comparison the frames use.
const fn tick_after(tick: Tick, than: Tick) -> bool {
    let distance = tick.0.wrapping_sub(than.0);
    distance != 0 && distance < 1 << 31
}

/// The generation after `generation`, skipping zero, which marks a free slot.
const fn next_generation(generation: u32) -> u32 {
    if generation == u32::MAX { 1 } else { generation + 1 }
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
        let index = match self.free.pop() {
            Some(index) => index,
            None if self.next_unused < self.capacity() => {
                self.next_unused += 1;
                self.next_unused - 1
            }
            None => {
                return Err(Error::new(
                    ErrorKind::ResourceExhausted,
                    "every interpolation slot is allocated",
                ));
            }
        };
        let generation = next_generation(self.generations[index as usize]);
        self.generations[index as usize] = generation;
        self.buffer.slots[index as usize].write(|slot| {
            slot.generation.store(generation, Ordering::Relaxed);
            slot.count.store(0, Ordering::Relaxed);
        });
        Ok(InterpolationSlot { index, generation })
    }

    /// Pushes `state` into `slot`'s ring, which keeps the [`STATES_PER_SLOT`] newest
    /// states by tick: a state older than the oldest kept is dropped, one with the tick of
    /// a kept state replaces it, so arrival order does not matter. Readers never observe
    /// a half-written ring. Fails with `NotFound` if `slot` is stale or unallocated.
    pub fn push(&mut self, slot: InterpolationSlot, state: ReceivedState) -> Result<()> {
        let held = self.allocated(slot)?;
        let snapshot = held.read();
        let mut states = [EMPTY_STATE; STATES_PER_SLOT + 1];
        let kept = &snapshot.states[..snapshot.count];
        // Ascending by tick: the states before `state`, `state` (replacing an equal tick),
        // the states after it; then the oldest dropped if there are too many.
        let at = kept.iter().take_while(|kept| tick_after(state.tick, kept.tick)).count();
        let replaces = kept.get(at).is_some_and(|kept| kept.tick == state.tick);
        if !replaces && at == 0 && kept.len() == STATES_PER_SLOT {
            return Ok(());
        }
        let after = if replaces { at + 1 } else { at };
        let mut count = 0;
        for next in kept[..at].iter().chain([&state]).chain(&kept[after..]) {
            states[count] = *next;
            count += 1;
        }
        let first = count.saturating_sub(STATES_PER_SLOT);
        held.write(|slot| slot.store_states(&states[first..count]));
        Ok(())
    }

    /// Releases `slot`: it reads as missing from now on, under this generation and the
    /// next, until reallocated. Fails with `NotFound` if `slot` is stale or unallocated.
    pub fn release(&mut self, slot: InterpolationSlot) -> Result<()> {
        let held = self.allocated(slot)?;
        held.write(|held| {
            held.generation.store(0, Ordering::Relaxed);
            held.count.store(0, Ordering::Relaxed);
        });
        self.free.push(slot.index);
        Ok(())
    }

    /// The slot `slot` names, if it is allocated at `slot`'s generation.
    fn allocated(&self, slot: InterpolationSlot) -> Result<&Slot> {
        let held = self.buffer.slots.get(slot.index as usize).filter(|held| {
            slot.generation != 0 && held.generation.load(Ordering::Relaxed) == slot.generation
        });
        held.ok_or_else(|| Error::new(ErrorKind::NotFound, "a stale or unallocated slot"))
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
    /// before every kept state, and around a teleport: with `later.teleported` in
    /// `(earlier.tick, later.tick]`, at `earlier` while `at` is before `later.teleported`
    /// and at `later` from then on; `Missing` when the slot holds nothing or `slot` is
    /// stale. A torn read is retried; the call never blocks and never locks.
    #[must_use]
    pub fn sample(&self, slot: InterpolationSlot, at: PresentationTime) -> RemoteSample {
        let Some(held) = self.buffer.slots.get(slot.index as usize) else {
            return RemoteSample::MISSING;
        };
        let snapshot = held.read();
        if slot.generation == 0 || snapshot.generation != slot.generation || snapshot.count == 0
        {
            return RemoteSample::MISSING;
        }
        let kept = &snapshot.states[..snapshot.count];
        // `at` is `at.tick + fraction`, the fraction below one, so against an integer tick
        // the comparisons are those of `at.tick`.
        let later_at = kept.iter().position(|state| tick_after(state.tick, at.tick));
        let held_at = |state: &ReceivedState| RemoteSample {
            status: SampleStatus::Held,
            position: state.position,
            rotation: state.rotation,
        };
        let (earlier, later) = match later_at {
            None => return held_at(&kept[kept.len() - 1]),
            Some(0) => return held_at(&kept[0]),
            Some(later) => (&kept[later - 1], &kept[later]),
        };
        let teleport = later.teleported;
        if tick_after(teleport, earlier.tick) && !tick_after(teleport, later.tick) {
            return held_at(if tick_after(teleport, at.tick) { earlier } else { later });
        }
        let elapsed = Scalar::from_u32(at.tick.0.wrapping_sub(earlier.tick.0))
            + Scalar::from_u32(u32::from(at.fraction)) / Scalar::from_u32(1 << 16);
        let t = elapsed / Scalar::from_u32(later.tick.0.wrapping_sub(earlier.tick.0));
        RemoteSample {
            status: SampleStatus::Interpolated,
            position: earlier.position.lerp(later.position, t),
            rotation: earlier.rotation.nlerp(later.rotation, t),
        }
    }

    /// [`sample`](Self::sample) for every slot of `slots` into `out[i]`, which must be at
    /// least as long; the batch call of architecture section 5.3. Allocates nothing.
    pub fn sample_batch(
        &self,
        slots: &[InterpolationSlot],
        at: PresentationTime,
        out: &mut [RemoteSample],
    ) {
        assert!(out.len() >= slots.len(), "the output is shorter than the slots");
        for (sample, &slot) in out.iter_mut().zip(slots) {
            *sample = self.sample(slot, at);
        }
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
