//! The sequence lock behind [`LatestSample`](crate::LatestSample), written once against
//! [`Memory`] so the loom models (`models`) check these functions, not a copy of them: the
//! slot ships them over the standard atomics ([`Native`]) and the models run them over
//! loom's.
//!
//! The sequence is even while the words are stable and odd while a publisher writes them.
//! It is 64 bits wide so it never wraps in practice: a reader that loads the same even
//! value twice has seen no publish in between.

use core::sync::atomic::Ordering::{self, Acquire, Relaxed, Release};
use core::sync::atomic::{AtomicU32, AtomicU64};

/// The number of 32-bit words a published value has.
pub(crate) const WORDS: usize = 6;

/// The atomic operations the lock is built from.
pub(crate) trait Memory {
    type Sequence;
    type Word;

    fn load_sequence(sequence: &Self::Sequence, order: Ordering) -> u64;
    fn compare_exchange_sequence(
        sequence: &Self::Sequence,
        current: u64,
        new: u64,
        success: Ordering,
        failure: Ordering,
    ) -> Result<u64, u64>;
    fn store_sequence(sequence: &Self::Sequence, value: u64, order: Ordering);
    fn load_word(word: &Self::Word, order: Ordering) -> u32;
    fn store_word(word: &Self::Word, value: u32, order: Ordering);
    fn fence(order: Ordering);
    /// Waits a moment before the `attempt`th retry.
    fn pause(attempt: u32);
}

/// The standard atomics: what [`LatestSample`](crate::LatestSample) runs on.
pub(crate) struct Native;

impl Memory for Native {
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
        core::sync::atomic::fence(order);
    }

    /// Spins briefly, then yields the thread: the other side is a publisher midway through
    /// six stores, which a preemption can stretch to a time slice.
    fn pause(attempt: u32) {
        if attempt < 64 {
            core::hint::spin_loop();
        } else {
            std::thread::yield_now();
        }
    }
}

/// Writes `value` into `words`: compare-exchange (`Acquire`) the sequence from even to odd,
/// `fence(Release)`, the words `Relaxed`, then the next even sequence (`Release`). The
/// acquire orders this publisher's words after those of the publisher before it, so the last
/// to finish wins whole; the fence orders the words after the odd sequence for any reader
/// that sees one of them.
pub(crate) fn publish<M: Memory>(
    sequence: &M::Sequence,
    words: &[M::Word; WORDS],
    value: [u32; WORDS],
) {
    let mut attempt = 0u32;
    let mut current = M::load_sequence(sequence, Relaxed);
    let taken = loop {
        if current & 1 == 0 {
            match M::compare_exchange_sequence(sequence, current, current + 1, Acquire, Relaxed) {
                Ok(_) => break current,
                Err(actual) => current = actual,
            }
        } else {
            M::pause(attempt);
            attempt = attempt.saturating_add(1);
            current = M::load_sequence(sequence, Relaxed);
        }
    };
    M::fence(Release);
    for (word, value) in words.iter().zip(value) {
        M::store_word(word, value, Relaxed);
    }
    M::store_sequence(sequence, taken + 2, Release);
}

/// The words of the last publish to finish: load the sequence (`Acquire`), the words
/// `Relaxed`, `fence(Acquire)`, reload the sequence, and retry on an odd or changed value.
pub(crate) fn read<M: Memory>(sequence: &M::Sequence, words: &[M::Word; WORDS]) -> [u32; WORDS] {
    let mut attempt = 0u32;
    loop {
        let before = M::load_sequence(sequence, Acquire);
        if before & 1 == 0 {
            let value = words.each_ref().map(|word| M::load_word(word, Relaxed));
            M::fence(Acquire);
            if M::load_sequence(sequence, Relaxed) == before {
                return value;
            }
        }
        M::pause(attempt);
        attempt = attempt.saturating_add(1);
    }
}
