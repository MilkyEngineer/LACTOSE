//! Timed driver waits held to the host's clock. A driver may return `VK_TIMEOUT` before the
//! time it was given has passed by `Instant`, which the spec allows (the timeout "is adjusted
//! to the closest value allowed by the implementation-dependent timeout accuracy"): AMD's
//! Windows driver 26.11.2 did so from `vkWaitSemaphores` on a point only the host signals, in
//! 11 of 50 waits of 30 ms, the earliest by 2.97 ms. The contract's timed waits end unreached
//! only once their time has passed, so each is made again for what is left.
//!
//! A driver whose call returned `VK_TIMEOUT` in less than a tenth of the time it was given,
//! as one may for a time below its accuracy, is not called again at once: the host first
//! sleeps for what is left, at most [`PAUSE`], so such a driver costs a call a millisecond
//! rather than a core for the rest of the wait.

use core::time::Duration;
use std::time::Instant;

use ash::prelude::VkResult;
use ash::vk;

/// The most the host sleeps between two calls of a driver that returned at once.
const PAUSE: Duration = Duration::from_millis(1);

#[cfg(test)]
thread_local! {
    /// How many of this thread's next driver waits are given half the time left: a driver
    /// that returns `VK_TIMEOUT` early, for the device tests ([`return_early`]).
    static EARLY: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

/// Plays a driver that returns `VK_TIMEOUT` early on this thread: each of its next `calls`
/// driver waits made through [`until`] is given half the time left, and so ends at half its
/// time. A device test sets it before a timed wait and asserts [`early_left`] is 0 after, so
/// a call site that waits without [`until`] fails it, as one that waits once does by the
/// time it took.
#[cfg(test)]
pub(crate) fn return_early(calls: u32) {
    EARLY.set(calls);
}

/// How many of the early returns [`return_early`] asked for are yet to be played.
#[cfg(test)]
pub(crate) fn early_left() -> u32 {
    EARLY.get()
}

/// When a wait of `timeout` begun now ends: `None` for one too long to have an end the clock
/// can hold, which waits with no timeout.
pub(crate) fn deadline(timeout: Duration) -> Option<Instant> {
    Instant::now().checked_add(timeout)
}

/// Calls `wait`, a driver's timed wait given the nanoseconds it may take, until it returns
/// anything but `VK_TIMEOUT`, or returns `VK_TIMEOUT` once `deadline` has passed by
/// `Instant`. A deadline already passed is one call given zero, a poll; with no deadline
/// each call is given `u64::MAX`, which the driver takes as no timeout.
pub(crate) fn until<T>(
    deadline: Option<Instant>,
    mut wait: impl FnMut(u64) -> VkResult<T>,
) -> VkResult<T> {
    loop {
        let nanos = given(deadline);
        let called = Instant::now();
        match wait(nanos) {
            Err(vk::Result::TIMEOUT)
                if deadline.is_none_or(|deadline| Instant::now() < deadline) =>
            {
                if called.elapsed() < Duration::from_nanos(nanos / 10) {
                    let left = deadline.map_or(PAUSE, |deadline| {
                        deadline.saturating_duration_since(Instant::now())
                    });
                    std::thread::sleep(left.min(PAUSE));
                }
            }
            other => return other,
        }
    }
}

/// The nanoseconds left until `deadline`, `u64::MAX` for none.
fn given(deadline: Option<Instant>) -> u64 {
    let nanos = deadline.map_or(u64::MAX, |deadline| {
        let left = deadline.saturating_duration_since(Instant::now());
        u64::try_from(left.as_nanos()).unwrap_or(u64::MAX)
    });
    #[cfg(test)]
    let nanos = match EARLY.get() {
        0 => nanos,
        early => {
            EARLY.set(early - 1);
            nanos / 2
        }
    };
    nanos
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A driver that returns `VK_TIMEOUT` at once, `early` times, then `last`: the times
    /// it was given, in order, and what `until` returned.
    fn driver(
        deadline: Option<Instant>,
        early: usize,
        last: VkResult<u32>,
    ) -> (Vec<u64>, VkResult<u32>) {
        let mut given = Vec::new();
        let result = until(deadline, |nanos| {
            given.push(nanos);
            if given.len() <= early { Err(vk::Result::TIMEOUT) } else { last }
        });
        (given, result)
    }

    #[test]
    fn a_timeout_returned_early_is_waited_again_for_what_is_left() {
        let timeout = Duration::from_secs(60);
        let (given, result) = driver(deadline(timeout), 2, Ok(7));
        assert_eq!(result, Ok(7));
        assert_eq!(given.len(), 3, "{given:?}");
        let most = u64::try_from(timeout.as_nanos()).unwrap();
        assert!(given.is_sorted_by(|a, b| a >= b) && given[0] <= most, "{given:?}");
        // What is left is given, not the whole again.
        assert!(given[2] < most, "{given:?}");
    }

    #[test]
    fn a_wait_that_times_out_returns_only_once_its_time_has_passed() {
        // Each call returns at half the time it was given, as the AMD driver's early returns.
        let timeout = Duration::from_millis(30);
        let start = Instant::now();
        let result: VkResult<()> = until(deadline(timeout), |nanos| {
            std::thread::sleep(Duration::from_nanos(nanos / 2));
            Err(vk::Result::TIMEOUT)
        });
        assert_eq!(result, Err(vk::Result::TIMEOUT));
        assert!(start.elapsed() >= timeout, "returned after {:?}", start.elapsed());
    }

    #[test]
    fn a_driver_that_returns_at_once_is_called_again_after_a_pause() {
        // A driver whose wait never blocks: without the pause it is called hundreds of
        // thousands of times in 30 ms. A loaded host only lengthens the sleeps.
        let timeout = Duration::from_millis(30);
        let start = Instant::now();
        let (given, result) = driver(deadline(timeout), usize::MAX, Ok(0));
        assert_eq!(result, Err(vk::Result::TIMEOUT));
        assert!(start.elapsed() >= timeout, "returned after {:?}", start.elapsed());
        // A call a millisecond at most, besides the first and the poll at the deadline.
        assert!((2..=32).contains(&given.len()), "{} calls", given.len());
    }

    #[test]
    fn an_early_return_played_halves_the_time_given_and_is_used_up() {
        return_early(2);
        let timeout = Duration::from_secs(60);
        let (given, result) = driver(deadline(timeout), 3, Ok(1));
        assert_eq!((given.len(), result, early_left()), (4, Ok(1), 0));
        let most = u64::try_from(timeout.as_nanos()).unwrap();
        assert!(given[0] <= most / 2 && given[1] <= most / 2, "{given:?}");
        assert!(given[2] > most / 2, "{given:?}");
    }

    #[test]
    fn a_zero_timeout_is_one_poll() {
        let (given, result) = driver(deadline(Duration::ZERO), usize::MAX, Ok(0));
        assert_eq!((given, result), (vec![0], Err(vk::Result::TIMEOUT)));
    }

    #[test]
    fn a_wait_with_no_end_is_given_none() {
        assert_eq!(deadline(Duration::MAX), None);
        let (given, result) = driver(None, 1, Ok(3));
        assert_eq!((given, result), (vec![u64::MAX; 2], Ok(3)));
    }

    #[test]
    fn any_other_result_returns_at_once() {
        let soon = deadline(Duration::from_secs(60));
        for last in [Err(vk::Result::ERROR_DEVICE_LOST), Err(vk::Result::NOT_READY), Ok(1)] {
            let (given, result) = driver(soon, 0, last);
            assert_eq!((given.len(), result), (1, last));
        }
    }
}
