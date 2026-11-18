//! loom models of the interpolation slot's sequence lock.
//!
//! Run with `cargo test -p lactose-net-host --features loom --release`. Every model calls
//! the functions the buffer ships, `seqlock::Slot`'s `reset`, `push` and `read`, over loom's
//! atomics, at a ring of two states of two words rather than the buffer's eight of eight:
//! the functions are the same, and loom branches on every word a read loads. Every model
//! lists the slot's contents after each of its writes, and every read must equal one of them,
//! no older than the reader's previous read. No two states the writer pushes share a word, so
//! a read that mixes two writes fails unless the mix is itself contents a write left, which
//! no reader can tell from a whole read. A read may mix three kinds of word, and each model
//! makes one kind's mixes visible: the ring's wrap the state words, the generation change
//! `generation` against the count and the states, and the ring growing at its front `count`
//! against the states, since a stale count over a ring grown at its front is contents no
//! write left.
//!
//! The models run at preemption bound 2 unless `LOOM_MAX_PREEMPTIONS` sets another. Each
//! `*_without_*` model runs the same functions with one of the lock's four orderings
//! weakened to `Relaxed`, or its fence removed, and must find a torn read.

use core::sync::atomic::Ordering::{self, Acquire, Relaxed, Release};

use loom::sync::Arc;
use loom::sync::atomic::{AtomicU32, AtomicU64};
use loom::thread;

use crate::seqlock::{Memory, Slot, Snapshot};

const PREEMPTION_BOUND: usize = 2;

const STATES: usize = 2;
const WORDS: usize = 2;

type Contents = Snapshot<STATES, WORDS>;

/// loom's atomics.
struct Loom;

impl Memory for Loom {
    type U32 = AtomicU32;
    type U64 = AtomicU64;

    fn new_u32(value: u32) -> AtomicU32 {
        AtomicU32::new(value)
    }

    fn new_u64(value: u64) -> AtomicU64 {
        AtomicU64::new(value)
    }

    fn load_u32(atomic: &AtomicU32, order: Ordering) -> u32 {
        atomic.load(order)
    }

    fn store_u32(atomic: &AtomicU32, value: u32, order: Ordering) {
        atomic.store(value, order);
    }

    fn load_u64(atomic: &AtomicU64, order: Ordering) -> u64 {
        atomic.load(order)
    }

    fn store_u64(atomic: &AtomicU64, value: u64, order: Ordering) {
        atomic.store(value, order);
    }

    fn fence(order: Ordering) {
        loom::sync::atomic::fence(order);
    }

    /// A retry loop must yield under loom, or the model explores it forever.
    fn pause() {
        thread::yield_now();
    }
}

/// The lock's orderings stronger than `Relaxed`, one per site: each pair of operation and
/// ordering below occurs once in `seqlock`.
const WRITER_FENCE: u8 = 0;
const EVEN_RELEASE: u8 = 1;
const FIRST_ACQUIRE: u8 = 2;
const READER_FENCE: u8 = 3;

/// [`Loom`] with the one ordering `WEAKENED` names relaxed, and nothing else changed:
/// `WRITER_FENCE` drops the writer's `fence(Release)`, `EVEN_RELEASE` stores the even
/// sequence `Relaxed`, `FIRST_ACQUIRE` loads the reader's first sequence `Relaxed`, and
/// `READER_FENCE` drops the reader's `fence(Acquire)`.
struct Weakened<const WEAKENED: u8>;

impl<const WEAKENED: u8> Memory for Weakened<WEAKENED> {
    type U32 = AtomicU32;
    type U64 = AtomicU64;

    fn new_u32(value: u32) -> AtomicU32 {
        Loom::new_u32(value)
    }

    fn new_u64(value: u64) -> AtomicU64 {
        Loom::new_u64(value)
    }

    fn load_u32(atomic: &AtomicU32, order: Ordering) -> u32 {
        let weakened = WEAKENED == FIRST_ACQUIRE && order == Acquire;
        Loom::load_u32(atomic, if weakened { Relaxed } else { order })
    }

    fn store_u32(atomic: &AtomicU32, value: u32, order: Ordering) {
        let weakened = WEAKENED == EVEN_RELEASE && order == Release;
        Loom::store_u32(atomic, value, if weakened { Relaxed } else { order });
    }

    fn load_u64(atomic: &AtomicU64, order: Ordering) -> u64 {
        Loom::load_u64(atomic, order)
    }

    fn store_u64(atomic: &AtomicU64, value: u64, order: Ordering) {
        Loom::store_u64(atomic, value, order);
    }

    fn fence(order: Ordering) {
        let removed = (WEAKENED == WRITER_FENCE && order == Release)
            || (WEAKENED == READER_FENCE && order == Acquire);
        if !removed {
            Loom::fence(order);
        }
    }

    fn pause() {
        Loom::pause();
    }
}

/// The state `mark` at `tick`: the tick and the mark in word 0, a word derived from the
/// mark alone in word 1, so no two marks share a word.
const fn state(tick: u32, mark: u32) -> [u64; WORDS] {
    [((tick as u64) << 32) | mark as u64, (mark as u64) * 0x0101_0101]
}

const A: [u64; WORDS] = state(1, 1);
const B: [u64; WORDS] = state(2, 2);
const C: [u64; WORDS] = state(3, 3);
const D: [u64; WORDS] = state(4, 4);
const E: [u64; WORDS] = state(5, 5);
const NONE: [u64; WORDS] = [0; WORDS];

/// The slot's contents: `generation`, then the `count` first of `states` in the ring.
const fn contents(generation: u32, count: usize, states: [[u64; WORDS]; STATES]) -> Contents {
    Contents { generation, count, states }
}

/// Where `read` falls in `history`, the slot's contents after each write in order; a read
/// that is none of them is torn.
fn position(history: &[Contents], read: &Contents) -> usize {
    let found = history.iter().position(|contents| contents == read);
    found.unwrap_or_else(|| panic!("torn read: {read:?} is none of {history:?}"))
}

/// Spawns a thread that reads `slot` twice: each read is whole, and the second is not older
/// than the first.
fn reader<M: Memory + 'static>(
    slot: &Arc<Slot<M, STATES, WORDS>>,
    history: &'static [Contents],
) -> thread::JoinHandle<()> {
    let slot = Arc::clone(slot);
    thread::spawn(move || {
        let first = position(history, &slot.read());
        let second = position(history, &slot.read());
        assert!(first <= second, "read {:?} after {:?}", history[second], history[first]);
    })
}

fn check(model: impl Fn() + Sync + Send + 'static) {
    let mut builder = loom::model::Builder::new();
    if builder.preemption_bound.is_none() {
        builder.preemption_bound = Some(PREEMPTION_BOUND);
    }
    builder.check(model);
}

// The writer is the main thread and the reader a spawned one. loom 0.7.2 finds a race from
// the last access to an atomic alone, so a writer thread whose `push` loads a word before it
// stores it hides an earlier load of that word by a reader from the search: with the writer
// spawned and the reader on the main thread, the ring-wrap model ran one execution. One
// reader: a reader only loads, and loads do not order one another, so a second adds no
// interleaving against the writer that the first lacks, and a model of two readers and one
// push exceeded loom's branch limit while both waited out an odd sequence.
//
// loom 0.7.2 keeps seven stores of each atomic, its initial value among them, and silently
// drops the oldest. A reader may read the sequence value it was spawned after and every later
// one, and each model keeps those within the seven: the ring's wrap and the ring growing at
// its front five values (6 to 10 and 4 to 8), a margin of two, and the generation change
// seven, no margin.

/// A full ring of `A` and `B` at generation 1; the writer pushes `C` and `D`, each dropping
/// the oldest, while a reader reads twice: every read is one of the three rings, in order.
fn the_ring_wraps<M: Memory + 'static>() {
    static HISTORY: [Contents; 3] =
        [contents(1, 2, [A, B]), contents(1, 2, [B, C]), contents(1, 2, [C, D])];
    check(|| {
        let slot = Arc::new(Slot::<M, STATES, WORDS>::new());
        slot.reset(1);
        slot.push(A);
        slot.push(B);
        let reader = reader(&slot, &HISTORY);
        slot.push(C);
        slot.push(D);
        reader.join().unwrap();
    });
}

/// A slot at generation 1 holding `A` is released, reallocated at generation 2 and pushed
/// `E` while a reader reads twice: every read is one of the four contents, in order, so a
/// reader never pairs one generation with another's count or states.
///
/// The sequence takes eleven stores, and the reader may read seven of them (4 to 10), loom's
/// whole history: one more write here would drop value 4 from what loom offers the reader,
/// which narrows the search without a failure, so a further write needs a model of its own.
fn the_generation_changes<M: Memory + 'static>() {
    static HISTORY: [Contents; 4] = [
        contents(1, 1, [A, NONE]),
        contents(0, 0, [NONE, NONE]),
        contents(2, 0, [NONE, NONE]),
        contents(2, 1, [E, NONE]),
    ];
    check(|| {
        let slot = Arc::new(Slot::<M, STATES, WORDS>::new());
        slot.reset(1);
        slot.push(A);
        let reader = reader(&slot, &HISTORY);
        slot.reset(0);
        slot.reset(2);
        slot.push(E);
        reader.join().unwrap();
    });
}

/// A slot at generation 1 holding `B`; the writer pushes `A`, older, so it lands at the
/// ring's front, then `C`, dropping `A`, while a reader reads twice: every read is one of the
/// three rings, in order. A count from before `A`'s push read over its states gives `A`
/// alone, which no write left.
fn the_ring_grows_at_its_front<M: Memory + 'static>() {
    static HISTORY: [Contents; 3] =
        [contents(1, 1, [B, NONE]), contents(1, 2, [A, B]), contents(1, 2, [B, C])];
    check(|| {
        let slot = Arc::new(Slot::<M, STATES, WORDS>::new());
        slot.reset(1);
        slot.push(B);
        let reader = reader(&slot, &HISTORY);
        slot.push(A);
        slot.push(C);
        reader.join().unwrap();
    });
}

#[test]
fn the_ring_wraps_under_a_reader() {
    the_ring_wraps::<Loom>();
}

#[test]
fn the_generation_changes_under_a_reader() {
    the_generation_changes::<Loom>();
}

#[test]
fn the_ring_grows_at_its_front_under_a_reader() {
    the_ring_grows_at_its_front::<Loom>();
}

#[test]
#[should_panic(expected = "torn read")]
fn the_ring_wrap_tears_without_the_writers_release_fence() {
    the_ring_wraps::<Weakened<WRITER_FENCE>>();
}

#[test]
#[should_panic(expected = "torn read")]
fn the_ring_wrap_tears_without_the_even_sequences_release() {
    the_ring_wraps::<Weakened<EVEN_RELEASE>>();
}

#[test]
#[should_panic(expected = "torn read")]
fn the_ring_wrap_tears_without_the_readers_first_acquire() {
    the_ring_wraps::<Weakened<FIRST_ACQUIRE>>();
}

#[test]
#[should_panic(expected = "torn read")]
fn the_ring_wrap_tears_without_the_readers_acquire_fence() {
    the_ring_wraps::<Weakened<READER_FENCE>>();
}

#[test]
#[should_panic(expected = "torn read")]
fn the_generation_change_tears_without_the_writers_release_fence() {
    the_generation_changes::<Weakened<WRITER_FENCE>>();
}

#[test]
#[should_panic(expected = "torn read")]
fn the_generation_change_tears_without_the_even_sequences_release() {
    the_generation_changes::<Weakened<EVEN_RELEASE>>();
}

#[test]
#[should_panic(expected = "torn read")]
fn the_generation_change_tears_without_the_readers_first_acquire() {
    the_generation_changes::<Weakened<FIRST_ACQUIRE>>();
}

#[test]
#[should_panic(expected = "torn read")]
fn the_generation_change_tears_without_the_readers_acquire_fence() {
    the_generation_changes::<Weakened<READER_FENCE>>();
}

#[test]
#[should_panic(expected = "torn read")]
fn the_front_growth_tears_without_the_writers_release_fence() {
    the_ring_grows_at_its_front::<Weakened<WRITER_FENCE>>();
}

#[test]
#[should_panic(expected = "torn read")]
fn the_front_growth_tears_without_the_even_sequences_release() {
    the_ring_grows_at_its_front::<Weakened<EVEN_RELEASE>>();
}

#[test]
#[should_panic(expected = "torn read")]
fn the_front_growth_tears_without_the_readers_first_acquire() {
    the_ring_grows_at_its_front::<Weakened<FIRST_ACQUIRE>>();
}

#[test]
#[should_panic(expected = "torn read")]
fn the_front_growth_tears_without_the_readers_acquire_fence() {
    the_ring_grows_at_its_front::<Weakened<READER_FENCE>>();
}
