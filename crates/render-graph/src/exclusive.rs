//! A `Sync` cell for a value that is only `Send`.
//!
//! An encoder is `Send` and not `Sync`, and so is a recorder. The graph and a recorded
//! frame hold encoders, a declaration holds recorders, and all three are promised
//! `Send + Sync` (`README.md`, "Concurrency"). The value is reached through `&mut` or by
//! value alone, which already excludes every other thread, so no lock is ever taken:
//! `Mutex::get_mut` and `Mutex::into_inner` take none, and no guard exists to poison the
//! mutex. `std::sync::Exclusive` is this type and is not stable on the pinned toolchain;
//! this module goes when it is.

use std::sync::{Mutex, PoisonError};

/// A value reached through `&mut` alone, and `Sync` whatever it holds.
pub(crate) struct Exclusive<T>(Mutex<T>);

impl<T> Exclusive<T> {
    /// Wraps `value`.
    pub(crate) const fn new(value: T) -> Self {
        Self(Mutex::new(value))
    }

    /// The value, with no lock taken.
    pub(crate) fn get_mut(&mut self) -> &mut T {
        self.0.get_mut().unwrap_or_else(PoisonError::into_inner)
    }

    /// The value, with no lock taken.
    pub(crate) fn into_inner(self) -> T {
        self.0.into_inner().unwrap_or_else(PoisonError::into_inner)
    }
}
