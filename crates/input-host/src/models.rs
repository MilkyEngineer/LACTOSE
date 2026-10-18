//! loom models of the latest-sample slot's sequence lock.
//!
//! Run with `cargo test -p lactose-input-host --features loom --release`. Every model calls
//! the functions `LatestSample` ships, `seqlock::publish` and `seqlock::read`, over loom's
//! atomics. A published value is six equal words, so a read that mixes two publishes, or a
//! final state two publishers interleaved, shows as unequal words.
//!
//! The models run at preemption bound 2 unless `LOOM_MAX_PREEMPTIONS` sets another. The
//! `without_the_release_fence` models run the same functions with the publisher's
//! `fence(Release)` removed and must find a torn read: the mutation the contract names.

use core::sync::atomic::Ordering;

use loom::sync::Arc;
use loom::sync::atomic::{AtomicU32, AtomicU64};
use loom::thread;

use crate::seqlock::{self, Memory, WORDS};

const PREEMPTION_BOUND: usize = 2;

/// loom's atomics.
struct Loom;

impl Memory for Loom {
    type Sequence = AtomicU64;
    type Word = AtomicU32;

    fn load_sequence(sequence: &AtomicU64, order: Ordering) -> u64 {
        sequence.load(order)
    }

    fn compare_exchange_sequence(
        sequence: &AtomicU64,
        current: u64,
        new: u64,
        success: Ordering,
        failure: Ordering,
    ) -> Result<u64, u64> {
        sequence.compare_exchange(current, new, success, failure)
    }

    fn store_sequence(sequence: &AtomicU64, value: u64, order: Ordering) {
        sequence.store(value, order);
    }

    fn load_word(word: &AtomicU32, order: Ordering) -> u32 {
        word.load(order)
    }

    fn store_word(word: &AtomicU32, value: u32, order: Ordering) {
        word.store(value, order);
    }

    fn fence(order: Ordering) {
        loom::sync::atomic::fence(order);
    }

    /// A retry loop must yield under loom, or the model explores it forever.
    fn pause(_attempt: u32) {
        thread::yield_now();
    }
}

/// [`Loom`] with the publisher's release fence removed, and nothing else changed.
struct WithoutReleaseFence;

impl Memory for WithoutReleaseFence {
    type Sequence = AtomicU64;
    type Word = AtomicU32;

    fn load_sequence(sequence: &AtomicU64, order: Ordering) -> u64 {
        Loom::load_sequence(sequence, order)
    }

    fn compare_exchange_sequence(
        sequence: &AtomicU64,
        current: u64,
        new: u64,
        success: Ordering,
        failure: Ordering,
    ) -> Result<u64, u64> {
        Loom::compare_exchange_sequence(sequence, current, new, success, failure)
    }

    fn store_sequence(sequence: &AtomicU64, value: u64, order: Ordering) {
        Loom::store_sequence(sequence, value, order);
    }

    fn load_word(word: &AtomicU32, order: Ordering) -> u32 {
        Loom::load_word(word, order)
    }

    fn store_word(word: &AtomicU32, value: u32, order: Ordering) {
        Loom::store_word(word, value, order);
    }

    fn fence(order: Ordering) {
        if order != Ordering::Release {
            Loom::fence(order);
        }
    }

    fn pause(attempt: u32) {
        Loom::pause(attempt);
    }
}

struct Slot {
    sequence: AtomicU64,
    words: [AtomicU32; WORDS],
}

impl Slot {
    fn new() -> Self {
        Self { sequence: AtomicU64::new(0), words: core::array::from_fn(|_| AtomicU32::new(0)) }
    }

    fn publish<M: Memory<Sequence = AtomicU64, Word = AtomicU32>>(&self, value: u32) {
        seqlock::publish::<M>(&self.sequence, &self.words, [value; WORDS]);
    }

    /// The value of a read, which must be whole.
    fn read<M: Memory<Sequence = AtomicU64, Word = AtomicU32>>(&self) -> u32 {
        let words = seqlock::read::<M>(&self.sequence, &self.words);
        assert!(words.iter().all(|&word| word == words[0]), "torn read: {words:?}");
        words[0]
    }
}

fn check(model: impl Fn() + Sync + Send + 'static) {
    let mut builder = loom::model::Builder::new();
    if builder.preemption_bound.is_none() {
        builder.preemption_bound = Some(PREEMPTION_BOUND);
    }
    builder.check(model);
}

/// One publisher publishes 1 then 2 while the main thread reads: every read is 0 (before
/// the first publish), 1 or 2, never a mix, and never older than a value read before it.
/// After the join a read is 2.
fn one_publisher<M: Memory<Sequence = AtomicU64, Word = AtomicU32>>() {
    check(|| {
        let slot = Arc::new(Slot::new());
        let publisher = {
            let slot = Arc::clone(&slot);
            thread::spawn(move || {
                slot.publish::<M>(1);
                slot.publish::<M>(2);
            })
        };
        let first = slot.read::<M>();
        let second = slot.read::<M>();
        assert!(first <= second && second <= 2, "read {first} then {second}");
        publisher.join().unwrap();
        assert_eq!(slot.read::<M>(), 2);
    });
}

/// Two publishers publish 1 and 2 while the main thread reads: the read is 0, 1 or 2, never
/// a mix; after both joins the slot holds one of them whole, never their words interleaved.
fn two_publishers<M: Memory<Sequence = AtomicU64, Word = AtomicU32>>() {
    check(|| {
        let slot = Arc::new(Slot::new());
        let publishers: Vec<_> = [1, 2]
            .into_iter()
            .map(|value| {
                let slot = Arc::clone(&slot);
                thread::spawn(move || slot.publish::<M>(value))
            })
            .collect();
        let seen = slot.read::<M>();
        assert!(seen <= 2, "read {seen}");
        for publisher in publishers {
            publisher.join().unwrap();
        }
        // A publish that gave up on contention would leave the sequence at 2.
        assert_eq!(slot.sequence.load(Ordering::SeqCst), 4, "both publishes must take the lock");
        let last = slot.read::<M>();
        assert!(last == 1 || last == 2, "after both publishes the slot holds {last}");
    });
}

#[test]
fn one_publisher_and_a_reader() {
    one_publisher::<Loom>();
}

#[test]
fn two_publishers_and_a_reader() {
    two_publishers::<Loom>();
}

#[test]
#[should_panic(expected = "torn read")]
fn one_publisher_tears_without_the_release_fence() {
    one_publisher::<WithoutReleaseFence>();
}

#[test]
#[should_panic(expected = "torn read")]
fn two_publishers_tear_without_the_release_fence() {
    two_publishers::<WithoutReleaseFence>();
}
