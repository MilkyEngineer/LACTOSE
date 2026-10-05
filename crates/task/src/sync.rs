//! The primitives the scope latch and the sleep-and-wake protocol are built from.
//!
//! With the `loom` feature these are loom's model-checked versions, so `latch` and `sleep`
//! and the modules built on them (`gate`, `quiesce`)
//! compile unchanged against either. Nothing else in the crate goes through this module: the
//! rest of the executor is not modelled, and with the feature on it must not be run, because a
//! loom object panics outside `loom::model`.

pub(crate) use core::sync::atomic::Ordering;

#[cfg(not(feature = "loom"))]
mod imp {
    pub(crate) use core::sync::atomic::{AtomicU64, AtomicUsize, fence};
    pub(crate) use std::sync::Mutex;
    pub(crate) use std::thread::{park, park_timeout};
}

#[cfg(feature = "loom")]
mod imp {
    use core::time::Duration;

    pub(crate) use loom::sync::Mutex;
    pub(crate) use loom::sync::atomic::{AtomicU64, AtomicUsize, fence};
    pub(crate) use loom::thread::park;

    /// loom has no clock. A timed park is modelled as an untimed one, which only removes
    /// wake-ups, so a model that passes does not depend on a timeout for liveness.
    pub(crate) fn park_timeout(_timeout: Duration) {
        park();
    }
}

pub(crate) use imp::*;
