//! Input sources (architecture section 4: gamepad and keyboard sources), milestone 5. The
//! one shape every source shares is here; no source is, and nothing in this crate touches
//! a device or a clock yet.
//!
//! Host-only. A boundary crate names nothing in this module.

use crate::raw::RawSample;

/// A device that contributes to the frame's [`RawSample`]: the keyboard sets the movement
/// axes it composes from keys, the mouse adds its deltas to the yaw and pitch, a gamepad
/// does both. Milestone 5 adds the sources and the action map they consult; what a
/// source may not do is decided now: it reads no clock and produces nothing the simulation
/// sees except through [`RawSample::quantise`](crate::raw::RawSample::quantise).
pub trait Source: Send + 'static {
    /// Folds this frame's device state into `raw`, which arrives holding the previous
    /// frame's values so a delta source can accumulate.
    fn poll(&mut self, raw: &mut RawSample);
}
