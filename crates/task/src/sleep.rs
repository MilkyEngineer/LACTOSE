//! The sleep-and-wake protocol: how an idle worker parks without missing work that arrives
//! while it is deciding to.
//!
//! Modelled under loom (`models.rs`).
//!
//! One bit per worker says "asleep, or about to be". The two sides are:
//!
//! - **Sleeper**: [`Sleep::announce`] (set the bit, full fence), re-check every source of
//!   work, then either [`Sleep::cancel`] or [`Sleep::park`].
//! - **Waker**: make the work visible, then [`Sleep::notify_one`] (full fence, read the bits,
//!   claim one by clearing it) and unpark the claimed worker.
//!
//! The two fences are what make this sound: whichever fence comes first in the global order,
//! the other side's later read sees the earlier write. Either the sleeper's re-check sees the
//! work, or the waker sees the sleeper's bit. The work itself is published by a third party
//! (`crossbeam-deque`) with a plain release store, which is why the protocol brings its own
//! fences and does not lean on the orderings of the stores around them.
//!
//! A cleared bit is the wake-up. The unpark that follows only cuts the park short, so a park
//! token that arrives late, or that belongs to an earlier wake-up, costs one more trip round
//! a loop and nothing else.

use std::time::Instant;

use crate::sync::{AtomicU64, Ordering, fence, park, park_timeout};

const BITS: usize = u64::BITS as usize;

/// The sleep state of one worker pool.
pub(crate) struct Sleep {
    /// Bit `w % 64` of word `w / 64` is set while worker `w` is asleep or about to be.
    words: Box<[AtomicU64]>,
}

impl Sleep {
    pub(crate) fn new(workers: usize) -> Self {
        Self { words: (0..workers.div_ceil(BITS)).map(|_| AtomicU64::new(0)).collect() }
    }

    fn slot(&self, worker: usize) -> (&AtomicU64, u64) {
        (&self.words[worker / BITS], 1 << (worker % BITS))
    }

    /// Declares that `worker` is about to sleep. The caller re-checks for work afterwards
    /// and then calls [`Sleep::cancel`] or [`Sleep::park`].
    pub(crate) fn announce(&self, worker: usize) {
        let (word, bit) = self.slot(worker);
        word.fetch_or(bit, Ordering::SeqCst);
        fence(Ordering::SeqCst);
    }

    /// Withdraws an announcement because the re-check found work.
    ///
    /// A waker may already have claimed the bit. Its unpark then leaves a stale token, which
    /// is harmless, and its notification is not lost: this worker is awake and about to look
    /// for work, which is all a notification asks for.
    pub(crate) fn cancel(&self, worker: usize) {
        let (word, bit) = self.slot(worker);
        word.fetch_and(!bit, Ordering::SeqCst);
    }

    /// Parks `worker`, which must have announced, until a waker claims it or `deadline`
    /// passes. Returns `false` only on a timeout that no waker raced.
    pub(crate) fn park(&self, worker: usize, deadline: Option<Instant>) -> bool {
        let (word, bit) = self.slot(worker);
        while word.load(Ordering::SeqCst) & bit != 0 {
            match deadline {
                None => park(),
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return word.fetch_and(!bit, Ordering::SeqCst) & bit == 0;
                    }
                    park_timeout(deadline - now);
                }
            }
        }
        true
    }

    /// Claims one sleeping worker, if there is one. The caller made work visible before the
    /// call and unparks the returned worker after it.
    #[must_use]
    pub(crate) fn notify_one(&self) -> Option<usize> {
        fence(Ordering::SeqCst);
        for (index, word) in self.words.iter().enumerate() {
            let mut bits = word.load(Ordering::SeqCst);
            while bits != 0 {
                let bit = bits.isolate_lowest_one();
                match word.compare_exchange_weak(
                    bits,
                    bits & !bit,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => return Some(index * BITS + bit.trailing_zeros() as usize),
                    Err(actual) => bits = actual,
                }
            }
        }
        None
    }

    /// Claims `worker` if it is asleep. The caller changed something that worker waits for
    /// before the call, and unparks it if this returns `true`.
    #[must_use]
    pub(crate) fn notify(&self, worker: usize) -> bool {
        fence(Ordering::SeqCst);
        let (word, bit) = self.slot(worker);
        word.load(Ordering::SeqCst) & bit != 0 && word.fetch_and(!bit, Ordering::SeqCst) & bit != 0
    }

    /// Claims every sleeping worker and passes each to `unpark`.
    pub(crate) fn notify_all(&self, mut unpark: impl FnMut(usize)) {
        fence(Ordering::SeqCst);
        for (index, word) in self.words.iter().enumerate() {
            if word.load(Ordering::SeqCst) == 0 {
                continue;
            }
            let mut bits = word.swap(0, Ordering::SeqCst);
            while bits != 0 {
                unpark(index * BITS + bits.trailing_zeros() as usize);
                bits &= bits - 1;
            }
        }
    }
}

#[cfg(all(test, not(feature = "loom")))]
mod tests {
    use super::*;

    #[test]
    fn claims_each_sleeper_once_across_words() {
        let sleep = Sleep::new(130);
        for worker in [3, 64, 129] {
            sleep.announce(worker);
        }
        let mut claimed = [sleep.notify_one(), sleep.notify_one(), sleep.notify_one()];
        claimed.sort();
        assert_eq!(claimed, [Some(3), Some(64), Some(129)]);
        assert_eq!(sleep.notify_one(), None);
        // A claimed worker's park returns without blocking.
        assert!(sleep.park(64, None));
    }

    #[test]
    fn targeted_and_broadcast_notifications() {
        let sleep = Sleep::new(70);
        sleep.announce(1);
        sleep.announce(69);
        assert!(!sleep.notify(2));
        assert!(sleep.notify(69));
        assert!(!sleep.notify(69));
        sleep.announce(5);
        let mut woken = Vec::new();
        sleep.notify_all(|worker| woken.push(worker));
        assert_eq!(woken, [1, 5]);
        sleep.announce(7);
        sleep.cancel(7);
        assert_eq!(sleep.notify_one(), None);
    }

    #[test]
    fn a_timed_park_withdraws_the_announcement() {
        let sleep = Sleep::new(2);
        sleep.announce(0);
        assert!(!sleep.park(0, Some(Instant::now())));
        assert_eq!(sleep.notify_one(), None);
    }
}
