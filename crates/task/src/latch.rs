//! The scope latch: how the opener of a fork-join scope learns that every child has finished.
//!
//! Modelled under loom (`models.rs`).

use crate::sync::{AtomicUsize, Ordering, park};

/// Counts the units of child work a scope still waits for.
///
/// The latch lives in the opener's stack frame. The opener leaves that frame as soon as it
/// observes zero, so the decrement that reaches zero is the last access any other thread may
/// make: [`Latch::count_down`] touches nothing afterwards, and its caller wakes the opener
/// through a handle that does not live in the frame.
pub(crate) struct Latch {
    pending: AtomicUsize,
    /// Stands for the stack frame in the loom model: every access reads it and
    /// [`Latch::retire`] writes it, so loom reports any access that can race the frame's end.
    #[cfg(feature = "loom")]
    frame: loom::cell::UnsafeCell<()>,
}

impl Latch {
    pub(crate) fn new() -> Self {
        Self {
            pending: AtomicUsize::new(0),
            #[cfg(feature = "loom")]
            frame: loom::cell::UnsafeCell::new(()),
        }
    }

    fn pending(&self) -> &AtomicUsize {
        #[cfg(feature = "loom")]
        self.frame.with(|_| ());
        &self.pending
    }

    /// Adds `units` of outstanding work. Called by the opener, and only before it waits.
    pub(crate) fn add(&self, units: usize) {
        // Relaxed: the count is only ever compared with zero by the opener itself, and a
        // child learns of its own units through the deque entry that carries them.
        self.pending().fetch_add(units, Ordering::Relaxed);
    }

    /// Marks `units` as finished. Returns whether that was the last outstanding work, in
    /// which case the caller must unpark the opener and must not touch the latch again.
    #[must_use]
    pub(crate) fn count_down(&self, units: usize) -> bool {
        // Release: everything the child wrote through its borrows is visible to the opener
        // once it reads zero.
        self.pending().fetch_sub(units, Ordering::Release) == units
    }

    /// Whether no work is outstanding. Opener only.
    pub(crate) fn is_set(&self) -> bool {
        self.pending().load(Ordering::Acquire) == 0
    }

    /// Parks the calling thread, which must be the opener, until something unparks it. The
    /// opener calls this in a loop that checks [`Latch::is_set`] first.
    ///
    /// A park token left by an unrelated unpark only costs one more trip round that loop,
    /// and a token set between the check and the park makes the park return at once, so no
    /// wake-up is lost.
    pub(crate) fn park(&self) {
        park();
    }

    /// Ends the modelled stack frame.
    #[cfg(all(test, feature = "loom"))]
    pub(crate) fn retire(&self) {
        self.frame.with_mut(|_| ());
    }
}
