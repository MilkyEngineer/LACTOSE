//! The raw sample on the host side and its quantisation (architecture section 9.2:
//! "inputs are quantised on the client before simulation, so the client simulates exactly
//! the value the server receives").
//!
//! Host-only: floats live here and never cross into a boundary crate. A boundary crate
//! names nothing in this module.

use crate::sample::{Actions, PlayerInput};

/// What the sources produced for one frame, before quantisation: movement in the player's
/// frame, the camera's absolute yaw and pitch in radians, and the actions held. Floats,
/// outside the boundary; the only way into the simulation is [`quantise`](Self::quantise).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct RawSample {
    /// Movement to the right, in `[-1, 1]`; keyboard composes it from keys, a gamepad reads
    /// its stick. Together with `movement_y` it is limited to the unit disc at quantisation.
    pub movement_x: f32,
    /// Movement forward, as `movement_x`.
    pub movement_y: f32,
    /// The absolute camera yaw in radians, any value: a mouse source adds its delta, a
    /// gamepad source integrates its stick. Wrapped into a turn at quantisation.
    pub yaw: f32,
    /// The absolute camera pitch in radians, positive upward. Clamped to a quarter turn
    /// each way at quantisation.
    pub pitch: f32,
    /// The actions held this frame.
    pub held: Actions,
}

impl RawSample {
    /// No movement, facing `-Z`, level, nothing held; also `Default`.
    pub const IDLE: Self =
        Self { movement_x: 0.0, movement_y: 0.0, yaw: 0.0, pitch: 0.0, held: Actions::EMPTY };

    /// The quantised sample for a tick, given the `held` set of the previous tick's sample
    /// (so the edges are the client's to compute, once, and travel in the sample):
    ///
    /// - `movement`: the vector `(movement_x, movement_y)` scaled down to the unit disc if
    ///   longer (a NaN component counts as zero), then each component times
    ///   [`AXIS_SCALE`](crate::AXIS_SCALE), rounded to nearest, ties away from zero, as an
    ///   `i16` (saturating);
    /// - `yaw`: `yaw` reduced into `[0, TAU)`, divided by `TAU`, times
    ///   [`UNITS_PER_TURN`](crate::UNITS_PER_TURN), rounded to nearest, modulo
    ///   `UNITS_PER_TURN`; a non-finite yaw quantises to zero;
    /// - `pitch`: `pitch` clamped to `[-PI/2, PI/2]`, divided by `TAU`, times
    ///   `UNITS_PER_TURN`, rounded to nearest, clamped to
    ///   [`Pitch::LIMIT`](crate::Pitch::LIMIT); a NaN pitch quantises to zero;
    /// - `held`: `held`; `pressed`: `held.without(previous_held)`.
    ///
    /// The result always passes [`PlayerInput::validate`]. Host floats: this is the one
    /// place a float becomes an integer, and the integer is what both machines simulate.
    #[must_use]
    pub fn quantise(&self, previous_held: Actions) -> PlayerInput {
        todo!()
    }
}
