//! The count of a device's holders: the device and its encoders. It is a type of its own,
//! with no driver, so that Miri runs its ordering.

use std::sync::atomic::{self, AtomicUsize, Ordering};

/// How many hold a device, from one at its making.
pub(crate) struct Holders(AtomicUsize);

impl Holders {
    pub(crate) const fn new() -> Self {
        Self(AtomicUsize::new(1))
    }

    /// Counts a new holder. Relaxed: a new holder is made from one that lives, as
    /// `Arc::clone` is; the decrement orders what each holder did before the last one's
    /// teardown.
    pub(crate) fn hold(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    /// Counts a holder's drop, and whether it was the last. Release, and Acquire at the last,
    /// as `Arc`'s drop: everything another holder did before its drop, its queue calls
    /// among them, happens before what the last does after this returns `true`.
    pub(crate) fn let_go(&self) -> bool {
        if self.0.fetch_sub(1, Ordering::Release) != 1 {
            return false;
        }
        atomic::fence(Ordering::Acquire);
        true
    }
}

#[cfg(test)]
mod tests {
    use std::cell::UnsafeCell;
    use std::sync::Arc;
    use std::thread;

    use super::Holders;

    /// One holder's call, in plain memory: written by its holder before its drop, read by
    /// the last holder after its own.
    struct Call(UnsafeCell<u64>);

    // SAFETY: each cell is written by one thread and read by another only after `let_go`
    // returned `true` there, which the count orders after the write; the test exists to
    // have Miri's race detector check that order.
    unsafe impl Sync for Call {}

    /// Two holders each write their own call and let go; the last reads the other's. With
    /// the decrement weakened to `Relaxed`, or no fence, nothing orders the read after the
    /// write, and Miri reports a data race. Run under Miri with many seeds by the nightly
    /// job.
    #[test]
    fn the_last_holder_sees_every_other_holders_calls() {
        const ROUNDS: usize = if cfg!(miri) { 4 } else { 10_000 };
        for _ in 0..ROUNDS {
            let holders = Arc::new(Holders::new());
            holders.hold();
            let calls = Arc::new([Call(UnsafeCell::new(0)), Call(UnsafeCell::new(0))]);
            let holder = |i: usize| {
                let (holders, calls) = (Arc::clone(&holders), Arc::clone(&calls));
                thread::spawn(move || {
                    // SAFETY: this holder's own cell, which no thread reads before its drop.
                    unsafe { *calls[i].0.get() = 1 };
                    if holders.let_go() {
                        // SAFETY: the other holder let go first, by the count, which orders
                        // its write before this read.
                        assert_eq!(unsafe { *calls[1 - i].0.get() }, 1, "a call not seen");
                    }
                })
            };
            let (one, two) = (holder(0), holder(1));
            one.join().unwrap();
            two.join().unwrap();
        }
    }
}
