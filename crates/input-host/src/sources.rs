//! Input sources (architecture section 4: gamepad and keyboard sources), milestone 5. The
//! one shape every source shares is here; no source is, and nothing in this crate touches
//! a device or a clock yet.

use crate::raw::RawSample;

/// A device that contributes to the frame's [`RawSample`]. The input task calls
/// [`RawSample::begin_frame`] and then `poll` on every source, so `raw` arrives with
/// `held` empty, the movement zero and the angles as the previous frame left them; a
/// source adds what it holds with [`RawSample::hold`], offers its movement with
/// [`RawSample::offer_movement`] (the larger magnitude wins across sources) and turns the
/// camera with [`RawSample::turn`] (deltas, wrapped and clamped there). It never assigns
/// the fields directly, so sources combine whatever their poll order. A source reads no
/// clock and produces nothing the simulation sees except through
/// [`Quantiser::next`](crate::Quantiser::next). Milestone 5 adds the sources and the action
/// map they consult.
pub trait Source: Send + 'static {
    /// Folds this frame's device state into `raw`.
    fn poll(&mut self, raw: &mut RawSample);
}
