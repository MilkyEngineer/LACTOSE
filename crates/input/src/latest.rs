//! The latest raw sample for the late-latched camera (architecture section 5.3: "rotation
//! from the latest input"): a place the input task publishes every frame and the latch
//! reads immediately before submit, outside the boundary and outside the tick.
//!
//! Host-only. Milestone 2's presentation crate reads it; nothing here is simulation input.

use core::sync::atomic::AtomicU32;

use crate::raw::RawSample;

/// The most recently published [`RawSample`], shared between the thread that samples
/// devices and the frame-critical worker that latches the camera. `publish` and `read`
/// never block each other and never lock: the implementation is a sequence-locked copy
/// (the shape `net-api`'s interpolation buffer uses), so a reader that observes a torn
/// write retries, and a publisher is never delayed by readers. One publisher is the
/// expected use; with several, the last to finish wins.
pub struct LatestSample {
    sequence: AtomicU32,
    words: [AtomicU32; 5],
}

impl LatestSample {
    /// A slot holding [`RawSample::IDLE`] until the first `publish`.
    #[must_use]
    pub const fn new() -> Self {
        Self { sequence: AtomicU32::new(0), words: [const { AtomicU32::new(0) }; 5] }
    }

    /// Makes `sample` the latest. Returns once it is visible to the next `read`.
    pub fn publish(&self, sample: RawSample) {
        todo!()
    }

    /// The latest published sample, or `IDLE` before the first `publish`. Never a mix of
    /// two publishes.
    #[must_use]
    pub fn read(&self) -> RawSample {
        todo!()
    }
}

impl Default for LatestSample {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for LatestSample {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LatestSample").finish_non_exhaustive()
    }
}
