//! The shutdown flag, and the race between queueing work and shutdown's sweep of the queues.
//!
//! Modelled under loom (`models.rs`), so everything here goes through `sync`.
//!
//! A submitter queues an entry and then reads the flag; shutdown sets the flag and then reads
//! the queues to sweep them. Each side writes one location and then reads the other's: a
//! store-buffering pattern, so each has a `SeqCst` fence between its write and its read, and one
//! of the two always sees the other's write. Either the sweep finds the entry, or the submitter
//! sees the flag and sweeps the queues itself. With either fence gone both could read the old
//! values of the model's queue, and the entry would stay queued, its future unresolved, until its
//! queue was dropped; `crossbeam-deque`'s injector and the stand-in deque order the pair on their
//! own, by a `SeqCst` push and a fence or a lock in the steal, so against them the fences are
//! margin.

use crate::sync::{AtomicBool, Ordering, fence};

/// Whether the executor has begun to shut down.
pub(crate) struct ShutdownFlag {
    set: AtomicBool,
    /// The one change a weakened loom model makes.
    #[cfg(all(test, feature = "loom"))]
    pub(crate) weakened: Option<Weakened>,
    /// Sets the flag between [`ShutdownFlag::queue`]'s push and its read, as a shutdown that
    /// lands in that gap does, for the tests of the sweep after queueing. Each executor has
    /// its own, so a test that turns it on reaches no other test's executor.
    #[cfg(all(test, not(feature = "loom")))]
    pub(crate) sets_after_queueing: core::sync::atomic::AtomicBool,
}

/// The weakened variants the models fail. Each changes one thing in this file.
#[cfg(all(test, feature = "loom"))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Weakened {
    /// [`ShutdownFlag::queue`] never reads the flag after queueing.
    NoCheck,
    /// [`ShutdownFlag::queue`] reads the flag without its fence.
    QueueFence,
    /// [`ShutdownFlag::before_sweep`] issues no fence.
    SweepFence,
}

impl ShutdownFlag {
    pub(crate) fn new() -> Self {
        Self {
            set: AtomicBool::new(false),
            #[cfg(all(test, feature = "loom"))]
            weakened: None,
            #[cfg(all(test, not(feature = "loom")))]
            sets_after_queueing: core::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Whether shutdown has begun. Nothing is published through the flag, so a reader that
    /// must not miss a sweep goes through [`ShutdownFlag::queue`].
    pub(crate) fn is_set(&self) -> bool {
        self.set.load(Ordering::Acquire)
    }

    /// Begins shutdown. The caller then sweeps the queues, after [`ShutdownFlag::before_sweep`].
    pub(crate) fn set(&self) {
        self.set.store(true, Ordering::SeqCst);
    }

    /// Queues `entry` with `push`, or hands it to `refuse` if shutdown has begun; then, if
    /// shutdown began meanwhile, sweeps the queues with `sweep`, which may find the entry or
    /// find that shutdown's own sweep already took it.
    pub(crate) fn queue<E>(
        &self,
        entry: E,
        refuse: impl FnOnce(E),
        push: impl FnOnce(E),
        sweep: impl FnOnce(),
    ) {
        if self.is_set() {
            refuse(entry);
        } else {
            push(entry);
            #[cfg(all(test, not(feature = "loom")))]
            if self.sets_after_queueing.load(Ordering::SeqCst) {
                self.set();
            }
            if self.seen_after_queueing() {
                sweep();
            }
        }
    }

    /// The submitter's read: the queueing comes before it, with a fence between them, the
    /// other half of [`ShutdownFlag::before_sweep`]'s pair.
    pub(crate) fn seen_after_queueing(&self) -> bool {
        #[cfg(all(test, feature = "loom"))]
        match self.weakened {
            Some(Weakened::NoCheck) => return false,
            Some(Weakened::QueueFence) => return self.set.load(Ordering::SeqCst),
            _ => {}
        }
        fence(Ordering::SeqCst);
        self.set.load(Ordering::SeqCst)
    }

    /// What a sweep issues before it reads the queues: setting the flag comes before it, the
    /// other half of [`ShutdownFlag::seen_after_queueing`]'s pair.
    pub(crate) fn before_sweep(&self) {
        #[cfg(all(test, feature = "loom"))]
        if self.weakened == Some(Weakened::SweepFence) {
            return;
        }
        fence(Ordering::SeqCst);
    }
}
