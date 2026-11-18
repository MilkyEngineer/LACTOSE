//! A `Sync` cell for a value that is only `Send`.
//!
//! An encoder is `Send` and not `Sync`, and so is a recorder. The graph and a recorded
//! frame hold encoders, a declaration holds recorders, and all three are promised
//! `Send + Sync` (`README.md`, "Concurrency"). The value is reached through `&mut` or by
//! value alone, which already excludes every other thread, so no lock is ever taken:
//! `Mutex::get_mut` and `Mutex::into_inner` take none, and no guard exists to poison the
//! mutex. The cell is a `Mutex<T>`, so it is `Sync` where `T` is `Send`, which every value
//! it holds is. `std::sync::SyncView` (unstable `exclusive_wrapper`) does this job, and is
//! `Sync` for any `T`; it is not stable on the pinned toolchain, and this module goes when
//! it is.

use std::sync::{Mutex, PoisonError};

/// A value reached through `&mut` alone, and `Sync` for any `Send` value it holds.
pub(crate) struct SyncView<T>(Mutex<T>);

impl<T> SyncView<T> {
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
