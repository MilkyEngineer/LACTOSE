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
//!   claim one by clearing it) and wake the claimed worker with [`Sleep::wake_claimed`].
//!
//! The two fences are what make this sound: whichever fence comes first in the global order,
//! the other side's later read sees the earlier write. Either the sleeper's re-check sees the
//! work, or the waker sees the sleeper's bit. The work itself is published by a third party
//! (`crossbeam-deque`) with a plain release store, which is why the protocol brings its own
//! fences and does not lean on the orderings of the stores around them.
//!
//! A cleared bit is the wake-up. The wake that follows only cuts the park short, so a park
//! token or a timed wait's signal that arrives late, or that belongs to an earlier wake-up,
//! costs one more trip round a loop and nothing else.

use std::time::{Duration, Instant};

use crate::span::TARGET;
use crate::sync::{AtomicU64, Ordering, TimedPark, fence, park};

const BITS: usize = u64::BITS as usize;

/// How much of the time left before a deadline one timed park asks for.
///
/// macOS coalesces timed waits: a `park_timeout` returns late by a leeway proportional to
/// the timeout, about a quarter of it at user-interactive priority and half at default. A
/// keeper that asked for the whole 16.7 ms to the next tick woke milliseconds late. Asking
/// for 0.6 of what is left keeps every return before the deadline even at half leeway, and
/// the remaining steps shrink geometrically: three to five parks per tick. Windows has no
/// such leeway and keeps the step, so every platform runs one loop. Under loom and Miri
/// there is no real clock, so one park as before.
const TIMER_STEP: f64 = if cfg!(any(feature = "loom", miri)) { 1.0 } else { 0.6 };

/// How close to a deadline a timed park stops parking and spins instead, re-checking its
/// bit. About 11 µs of spinning per tick at 60 Hz, which is what puts the wake within a
/// few microseconds of the deadline. On Windows 500 µs: the high-resolution timer that ends
/// a wait there overshoots by about half a millisecond, so the last wait must be asked to end
/// that far ahead (D144).
const TIMER_SPIN: Duration = Duration::from_micros(if cfg!(any(feature = "loom", miri)) {
    0
} else if cfg!(windows) {
    500
} else {
    50
});

/// The sleep state of one worker pool.
pub(crate) struct Sleep {
    /// Bit `w % 64` of word `w / 64` is set while worker `w` is asleep or about to be.
    words: Box<[AtomicU64]>,
    /// Worker `w`'s timed wait, where the workers keep the timers; empty where they never
    /// park with a deadline.
    parks: Box<[TimedPark]>,
}

impl Sleep {
    /// The sleep state of workers that never park with a deadline: the background pool's,
    /// and the openers waiting at a gate.
    pub(crate) fn new(workers: usize) -> Self {
        Self {
            words: (0..workers.div_ceil(BITS)).map(|_| AtomicU64::new(0)).collect(),
            parks: Box::new([]),
        }
    }

    /// The sleep state of workers that keep the timers, the frame-critical pool's, with a
    /// timed wait for each. A worker whose wait cannot be made, as on Windows before version
    /// 1803, waits with `std::thread::park_timeout` instead, which is logged once.
    pub(crate) fn with_timed_parks(workers: usize) -> Self {
        let mut failed = 0u32;
        let mut first_error = None;
        let parks = (0..workers)
            .map(|_| {
                TimedPark::new().unwrap_or_else(|error| {
                    failed += 1;
                    first_error.get_or_insert(error);
                    TimedPark::coarse()
                })
            })
            .collect();
        if let Some(error) = first_error {
            tracing::warn!(
                target: TARGET,
                %error,
                failed,
                workers,
                "the keeper's timed wait could not be made; those workers wait with a std \
                 park_timeout, in whole milliseconds on Windows",
            );
        }
        Self { parks, ..Self::new(workers) }
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
    /// A waker may already have claimed the bit. Its wake then leaves a stale signal, which
    /// is harmless, and its notification is not lost: this worker is awake and about to look
    /// for work, which is all a notification asks for.
    pub(crate) fn cancel(&self, worker: usize) {
        let (word, bit) = self.slot(worker);
        word.fetch_and(!bit, Ordering::SeqCst);
    }

    /// Parks `worker`, which must have announced, until a waker claims it or `deadline`
    /// passes. Returns `false` only on a timeout that no waker raced. Only `worker` calls
    /// it, and with a deadline only where it has a timed wait ([`Sleep::with_timed_parks`]).
    ///
    /// A timed park is a loop of shorter waits ([`TimedPark::wait`]), each for
    /// [`TIMER_STEP`] of the time left, with a spin over the last [`TIMER_SPIN`]. Every
    /// return from a wait, early or on time, goes through the same check of the bit and the
    /// clock, so a claim is never missed and the deadline is never passed by more than the
    /// clock's resolution. Nothing about the protocol depends on how many waits it takes.
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
                    let left = deadline - now;
                    if left <= TIMER_SPIN {
                        core::hint::spin_loop();
                    } else {
                        self.parks
                            .get(worker)
                            .expect("only a worker with a timed wait parks with a deadline")
                            .wait(left.mul_f64(TIMER_STEP));
                    }
                }
            }
        }
        true
    }

    /// Wakes `worker`, which the caller has just claimed ([`Sleep::notify_one`],
    /// [`Sleep::announce_entry`], [`Sleep::notify`] returning `true`, [`Sleep::notify_all`]):
    /// ends its timed wait if it has one, then calls `unpark`, which unparks its thread.
    ///
    /// Every waker of a claimed worker calls this, the pool's and the loom models' alike,
    /// and nothing else ends a timed wait. A waker that only unparked would leave the keeper
    /// on Windows in its wait until the timer ends it, up to 0.6 of the time to its deadline
    /// later. A worker that is awake or in an untimed park keeps the signal and takes it for
    /// a spurious wake in its next timed wait. Either way out of the wait orders the claim
    /// before the worker's next look at its bit.
    pub(crate) fn wake_claimed(&self, worker: usize, unpark: impl FnOnce()) {
        if let Some(timed) = self.parks.get(worker) {
            timed.wake();
        }
        unpark();
    }

    /// Claims one sleeping worker, if there is one. The caller made work visible before the
    /// call and wakes the returned worker after it ([`Sleep::wake_claimed`]).
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

    /// What a worker does after it queued an entry: claim a sleeper for it, unless the
    /// worker `kept` the entry for itself. A worker at the end of its job that releases
    /// several entries onto its own deque takes one of them as its next job; a sleeper woken
    /// for that one would find it gone. The caller wakes the returned worker.
    #[must_use]
    pub(crate) fn announce_entry(&self, kept: bool) -> Option<usize> {
        if kept { None } else { self.notify_one() }
    }

    /// Claims `worker` if it is asleep. The caller changed something that worker waits for
    /// before the call, and wakes it if this returns `true`.
    #[must_use]
    pub(crate) fn notify(&self, worker: usize) -> bool {
        fence(Ordering::SeqCst);
        let (word, bit) = self.slot(worker);
        word.load(Ordering::SeqCst) & bit != 0 && word.fetch_and(!bit, Ordering::SeqCst) & bit != 0
    }

    /// How many workers are announced: asleep, or about to be, and not yet claimed.
    #[cfg(all(test, not(feature = "loom")))]
    pub(crate) fn announced(&self) -> u32 {
        self.words.iter().map(|word| word.load(Ordering::SeqCst).count_ones()).sum()
    }

    /// Claims every sleeping worker and passes each to `wake`.
    pub(crate) fn notify_all(&self, mut wake: impl FnMut(usize)) {
        fence(Ordering::SeqCst);
        for (index, word) in self.words.iter().enumerate() {
            if word.load(Ordering::SeqCst) == 0 {
                continue;
            }
            let mut bits = word.swap(0, Ordering::SeqCst);
            while bits != 0 {
                wake(index * BITS + bits.trailing_zeros() as usize);
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

    /// A wake that finds its worker in no timed wait leaves a signal behind: on Windows the
    /// event stays set, and everywhere the park token. The next wait returns at once, and the
    /// park takes that for what it is: its bit is still set and its deadline still ahead, so
    /// it waits again and returns at the deadline, not before.
    #[test]
    fn a_stale_wake_does_not_end_a_timed_park_early() {
        let sleep = Sleep::with_timed_parks(1);
        sleep.wake_claimed(0, || std::thread::current().unpark());
        sleep.announce(0);
        let deadline = Instant::now() + Duration::from_millis(20);
        assert!(!sleep.park(0, Some(deadline)), "nothing claimed the worker");
        assert!(Instant::now() >= deadline);
    }

    /// A claim ends a timed park whose deadline is an hour away. Its first wait is 36
    /// minutes long, so on Windows, where an unpark does not end that wait, this fails
    /// unless the claim's wake does.
    #[test]
    fn a_claim_ends_a_timed_park_an_hour_from_its_deadline() {
        use std::sync::Arc;
        use std::sync::mpsc::channel;

        let sleep = Arc::new(Sleep::with_timed_parks(1));
        let (parked, outcome) = channel();
        let sleeper = {
            let sleep = sleep.clone();
            std::thread::spawn(move || {
                sleep.announce(0);
                let hour = Instant::now() + Duration::from_secs(3600);
                let _ = parked.send(sleep.park(0, Some(hour)));
            })
        };
        // Claimed once it has announced, whether or not it has reached its wait yet.
        while sleep.announced() == 0 {
            std::thread::yield_now();
        }
        assert!(sleep.notify(0));
        sleep.wake_claimed(0, || sleeper.thread().unpark());
        let limit = Duration::from_secs(if cfg!(miri) { 600 } else { 60 });
        assert_eq!(outcome.recv_timeout(limit), Ok(true), "the claim did not end the wait");
        sleeper.join().unwrap();
    }
}
