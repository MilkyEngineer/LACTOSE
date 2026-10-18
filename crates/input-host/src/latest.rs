//! The latest raw sample for the late-latched camera (architecture section 5.3: "rotation
//! from the latest input"): a place the input task publishes every frame and the latch
//! reads immediately before submit, outside the boundary and outside the tick.
//!
//! Milestone 2's presentation crate reads it; nothing here is simulation input.

use core::sync::atomic::{AtomicU32, AtomicU64};

use lactose_input::Actions;

use crate::raw::RawSample;
use crate::seqlock::{self, Native, WORDS};

/// The most recently published [`RawSample`], shared between the thread that samples
/// devices and the frame-critical worker that latches the camera. `publish` and `read`
/// never lock; a reader waits only while a publish is in flight (six stores), which a
/// descheduled publisher can stretch to a scheduling quantum. The slot is a sequence-locked copy of the sample's
/// six words (the shape `net-api`'s interpolation buffer uses). `publish`: compare-exchange
/// (`Acquire`) the sequence from even to odd, then `fence(Release)`, then the words
/// `Relaxed`, then store the next even value (`Release`); so a publisher may wait for
/// another publisher but never for a reader, two concurrent publishers cannot interleave
/// their words, and the fence orders every word write after the sequence was taken.
/// `read`: load the sequence `Acquire`, the words `Relaxed`, `fence(Acquire)`, reload the
/// sequence, and retry on mismatch or odd; so it returns a sample some `publish` wrote,
/// never a mix of two. With several publishers the last to finish wins.
pub struct LatestSample {
    sequence: AtomicU64,
    words: [AtomicU32; WORDS],
}

impl LatestSample {
    /// A slot holding [`RawSample::IDLE`] until the first `publish`.
    #[must_use]
    pub const fn new() -> Self {
        // All-zero words are `RawSample::IDLE`.
        Self { sequence: AtomicU64::new(0), words: [const { AtomicU32::new(0) }; WORDS] }
    }

    /// Makes `sample` the latest. Returns once it is visible to the next `read`.
    pub fn publish(&self, sample: RawSample) {
        seqlock::publish::<Native>(&self.sequence, &self.words, to_words(sample));
    }

    /// The latest published sample, or `IDLE` before the first `publish`. Never a mix of
    /// two publishes.
    #[must_use]
    pub fn read(&self) -> RawSample {
        from_words(seqlock::read::<Native>(&self.sequence, &self.words))
    }
}

/// The sample's fields as bits, in declaration order: a float's bits travel unchanged, NaN
/// payloads included.
fn to_words(sample: RawSample) -> [u32; WORDS] {
    [
        sample.movement_x.to_bits(),
        sample.movement_y.to_bits(),
        sample.yaw.to_bits(),
        sample.pitch.to_bits(),
        sample.held.bits(),
        sample.latched.bits(),
    ]
}

fn from_words(words: [u32; WORDS]) -> RawSample {
    let [movement_x, movement_y, yaw, pitch, held, latched] = words;
    RawSample {
        movement_x: f32::from_bits(movement_x),
        movement_y: f32::from_bits(movement_y),
        yaw: f32::from_bits(yaw),
        pitch: f32::from_bits(pitch),
        held: Actions::from_bits(held),
        latched: Actions::from_bits(latched),
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
