//! The raw sample on the host side and its quantisation (architecture section 9.2:
//! "inputs are quantised on the client before simulation, so the client simulates exactly
//! the value the server receives").
//!
//! Floats live here and never cross into a boundary crate; the only way into the
//! simulation is [`Quantiser::next`].

use lactose_input::{Actions, PlayerInput};

/// What the sources produced for one frame, before quantisation: movement in the player's
/// frame, the camera's absolute yaw and pitch in radians, and the actions held. Floats,
/// outside the boundary.
///
/// A frame starts with [`begin_frame`](Self::begin_frame), which empties `held` and the
/// movement and keeps the angles, which are absolute and accumulate; each source then
/// folds its state in through [`hold`](Self::hold), [`offer_movement`](Self::offer_movement)
/// and [`turn`](Self::turn), so sources combine instead of overwriting each other in poll
/// order.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct RawSample {
    /// Movement to the right, in `[-1, 1]`; a keyboard composes it from keys, a gamepad
    /// reads its stick. Together with `movement_y` it is limited to the unit disc at
    /// quantisation.
    pub movement_x: f32,
    /// Movement forward, as `movement_x`.
    pub movement_y: f32,
    /// The absolute camera yaw in radians, kept in `[-PI, PI)` by [`turn`](Self::turn),
    /// so that an accumulated yaw never grows coarser than a sample unit (past 1024 rad
    /// an `f32` is, and at 8192 rad a mouse delta vanishes). Zero faces `-Z`; positive
    /// turns left.
    pub yaw: f32,
    /// The absolute camera pitch in radians, positive upward, kept within
    /// `[-PI/2, PI/2]` by [`turn`](Self::turn).
    pub pitch: f32,
    /// The actions held this frame: the union of every source's.
    pub held: Actions,
}

impl RawSample {
    /// No movement, facing `-Z`, level, nothing held; also `Default`.
    pub const IDLE: Self =
        Self { movement_x: 0.0, movement_y: 0.0, yaw: 0.0, pitch: 0.0, held: Actions::EMPTY };

    /// Starts a frame: `held` becomes empty and the movement zero; `yaw` and `pitch` keep
    /// their values, since they are absolute. The input task calls it before polling the
    /// sources, so a key released on one source cannot be held over by a stale value.
    pub fn begin_frame(&mut self) {
        todo!()
    }

    /// Adds `actions` to `held`: `held.union(actions)`. How a source reports what it holds,
    /// so two sources holding the same action agree and neither clears the other's.
    pub fn hold(&mut self, actions: Actions) {
        todo!()
    }

    /// Offers a movement vector: it replaces the current one only if its squared length is
    /// larger, so of a stick and the keys the larger movement wins and a centred stick does
    /// not cancel the keys. A NaN component counts as zero.
    pub fn offer_movement(&mut self, x: f32, y: f32) {
        todo!()
    }

    /// Adds a rotation delta, radians: `yaw + delta_yaw` wrapped into `[-PI, PI)`, and
    /// `pitch + delta_pitch` clamped to `[-PI/2, PI/2]`. A non-finite delta is ignored. How
    /// a mouse or a stick turns the camera; the camera rig may call it too.
    pub fn turn(&mut self, delta_yaw: f32, delta_pitch: f32) {
        todo!()
    }
}

/// Turns raw samples into [`PlayerInput`]s, one per tick, keeping the one piece of state
/// the edges need: the `held` set of the previous tick's sample. Called once per
/// simulation tick, never once per frame, so when the timer root runs several catch-up
/// ticks from one frame's raw sample the second and later samples carry no edges and a
/// press fires once on the client as it does on the server.
#[derive(Clone, Debug, Default)]
pub struct Quantiser {
    previous_held: Actions,
}

impl Quantiser {
    /// A quantiser whose previous sample is idle: every action held at the first tick is
    /// also pressed in it.
    #[must_use]
    pub const fn new() -> Self {
        Self { previous_held: Actions::EMPTY }
    }

    /// The quantised sample for the next tick, from `raw`:
    ///
    /// - `movement`: the vector `(movement_x, movement_y)` scaled down to the unit disc if
    ///   longer (a NaN component counts as zero), then each component times
    ///   [`AXIS_SCALE`](lactose_input::AXIS_SCALE), rounded to nearest, ties away from
    ///   zero, as an `i16` (saturating);
    /// - `yaw`: `yaw` reduced into `[0, TAU)`, divided by `TAU`, times
    ///   [`UNITS_PER_TURN`](lactose_input::UNITS_PER_TURN), rounded to nearest, modulo
    ///   `UNITS_PER_TURN`; a non-finite yaw quantises to zero;
    /// - `pitch`: `pitch` clamped to `[-PI/2, PI/2]`, divided by `TAU`, times
    ///   `UNITS_PER_TURN`, rounded to nearest, clamped to
    ///   [`Pitch::LIMIT`](lactose_input::Pitch::LIMIT); a NaN pitch quantises to zero;
    /// - `held`: `raw.held`; `pressed`: `held.without(previous_held)`; `released`:
    ///   `previous_held.without(held)`; then `previous_held` becomes `held`.
    ///
    /// The result always passes [`PlayerInput::validate`]. This is the one place a float
    /// becomes an integer, and the integer is what both machines simulate.
    pub fn next(&mut self, raw: &RawSample) -> PlayerInput {
        todo!()
    }
}
