//! The atomics [`Staging`](crate::staging::Staging) shares between the writers and the
//! renderer: the standard library's, or loom's in the crate's own unit tests built with its
//! `loom` feature, so that the models in `models` run the shipped functions and not a copy
//! of them. The feature changes nothing the crate ships: only `cfg(test)` builds see loom.

#[cfg(all(test, feature = "loom"))]
pub(crate) use loom::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(all(test, feature = "loom")))]
pub(crate) use std::sync::atomic::{AtomicU64, Ordering};

/// What stands for a shadow entry's words as one location under loom, so that the models
/// can see what atomics hide: every word is an atomic, and loom models neither load
/// buffering nor a read-modify-write that reads a store older than the latest, so a read of
/// an entry by one use's `prepare` and a write of it by the next use's writer never conflict
/// in loom unless something checks that one happens before the other. The loom cell checks
/// that, and fails the model with "Causality violation" where nothing orders the two: what
/// orders them is `open`'s release and `begin`'s acquire. The std `Mutex` makes the cell
/// `Sync` without `unsafe`, and loom does not see it, so it orders nothing the models rest
/// on.
#[cfg(all(test, feature = "loom"))]
pub(crate) struct Access(std::sync::Mutex<loom::cell::UnsafeCell<()>>);

#[cfg(all(test, feature = "loom"))]
impl Access {
    pub(crate) fn new() -> Self {
        Self(std::sync::Mutex::new(loom::cell::UnsafeCell::new(())))
    }

    /// A read of the entry.
    pub(crate) fn with(&self) {
        self.0.lock().expect("the marker's lock is never poisoned").with(|_| ());
    }

    /// A write of the entry.
    pub(crate) fn with_mut(&self) {
        self.0.lock().expect("the marker's lock is never poisoned").with_mut(|_| ());
    }
}

/// Outside the loom models, the marker is nothing.
#[cfg(not(all(test, feature = "loom")))]
pub(crate) struct Access;

#[cfg(not(all(test, feature = "loom")))]
impl Access {
    pub(crate) const fn new() -> Self {
        Self
    }

    #[inline(always)]
    pub(crate) const fn with(&self) {}

    #[inline(always)]
    pub(crate) const fn with_mut(&self) {}
}
