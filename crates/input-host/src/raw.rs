//! The raw sample on the host side and its quantisation (architecture section 9.2:
//! "inputs are quantised on the client before simulation, so the client simulates exactly
//! the value the server receives").
//!
//! Floats live here and never cross into a boundary crate; the only way into the
//! simulation is [`Quantiser::next`].

use core::f64::consts::{FRAC_PI_2, PI, TAU};

use lactose_input::{AXIS_SCALE, Actions, Axis2, Pitch, PlayerInput, UNITS_PER_TURN, Yaw};

/// What the sources produced for one frame, before quantisation: movement in the player's
/// frame, the camera's absolute yaw and pitch in radians, the actions held, and the
/// actions held at any poll since the last tick. Floats, outside the boundary.
///
/// A frame starts with [`begin_frame`](Self::begin_frame), which empties `held` and the
/// movement and keeps the angles, which are absolute and accumulate, and `latched`, which
/// accumulates until a tick; each source then folds its state in through
/// [`hold`](Self::hold), [`offer_movement`](Self::offer_movement) and [`turn`](Self::turn),
/// so sources combine instead of overwriting each other in poll order.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct RawSample {
    /// Movement to the right, in `[-1, 1]`; a keyboard composes it from keys, a gamepad
    /// reads its stick. Together with `movement_y` it is limited to the unit disc at
    /// quantisation.
    pub movement_x: f32,
    /// Movement forward, as `movement_x`.
    pub movement_y: f32,
    /// The absolute camera yaw in radians, kept in `[-PI, PI)` by [`turn`](Self::turn)
    /// (which counts a non-finite yaw as zero),
    /// so that an accumulated yaw never grows coarser than a sample unit (past 1024 rad
    /// an `f32` is, and at 8192 rad a mouse delta vanishes). Zero faces `-Z`; positive
    /// turns left.
    pub yaw: f32,
    /// The absolute camera pitch in radians, positive upward, kept within
    /// `[-PI/2, PI/2]` by [`turn`](Self::turn) (which counts a non-finite pitch as zero).
    pub pitch: f32,
    /// The actions held this frame: the union of every source's.
    pub held: Actions,
    /// The actions held at any poll since the last tick: `hold` adds to it and
    /// [`Quantiser::next`] takes and clears it, so a tap pressed and released between two
    /// polls still reaches a sample as one tick of held. `begin_frame` keeps it.
    pub latched: Actions,
}

impl RawSample {
    /// No movement, facing `-Z`, level, nothing held or latched; also `Default`.
    pub const IDLE: Self = Self {
        movement_x: 0.0,
        movement_y: 0.0,
        yaw: 0.0,
        pitch: 0.0,
        held: Actions::EMPTY,
        latched: Actions::EMPTY,
    };

    /// Starts a frame: `held` becomes empty and the movement zero; `yaw`, `pitch` and
    /// `latched` keep their values, since the angles are absolute and the latch
    /// accumulates until a tick. The input task calls it before polling the sources, so a
    /// key released on one source cannot be held over by a stale value.
    pub fn begin_frame(&mut self) {
        self.held = Actions::EMPTY;
        self.movement_x = 0.0;
        self.movement_y = 0.0;
    }

    /// Adds `actions` to `held` and to `latched`: `held.union(actions)` and
    /// `latched.union(actions)`. How a source reports what it holds, so two sources
    /// holding the same action agree and neither clears the other's.
    pub fn hold(&mut self, actions: Actions) {
        self.held = self.held.union(actions);
        self.latched = self.latched.union(actions);
    }

    /// Offers a movement vector: it replaces the current one only if its squared length is
    /// larger, so of a stick and the keys the larger movement wins and a centred stick does
    /// not cancel the keys. A non-finite component (NaN or infinite) counts as zero, in the
    /// offer and in the current movement, and is stored as zero if the offer wins.
    pub fn offer_movement(&mut self, x: f32, y: f32) {
        let (x, y) = (finite_or_zero(x), finite_or_zero(y));
        let current =
            squared_length(finite_or_zero(self.movement_x), finite_or_zero(self.movement_y));
        if squared_length(x, y) > current {
            self.movement_x = x;
            self.movement_y = y;
        }
    }

    /// Adds a rotation delta, radians: `yaw + delta_yaw` wrapped into `[-PI, PI)` (summed in
    /// `f64`; a sum already in range is only rounded), and `pitch + delta_pitch` clamped to
    /// `[-PI/2, PI/2]`. A non-finite delta counts as zero, each of the two on its own, and so
    /// does a non-finite current yaw or pitch written through the public field, so both
    /// angles are in range after any call. How a mouse or a stick turns the
    /// camera; the camera rig may call it too.
    pub fn turn(&mut self, delta_yaw: f32, delta_pitch: f32) {
        let yaw = f64::from(finite_or_zero(self.yaw));
        let delta_yaw = f64::from(finite_or_zero(delta_yaw));
        self.yaw = wrap_yaw(yaw + delta_yaw);
        let limit = core::f32::consts::FRAC_PI_2;
        let delta_pitch = finite_or_zero(delta_pitch);
        self.pitch = (finite_or_zero(self.pitch) + delta_pitch).clamp(-limit, limit);
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

    /// The quantised sample for the next tick, from `raw`, which it takes mutably to
    /// consume the latch:
    ///
    /// - `movement`: the vector `(movement_x, movement_y)` scaled down to the unit disc if
    ///   longer (a non-finite component, NaN or infinite, counts as zero), then each
    ///   component times [`AXIS_SCALE`](lactose_input::AXIS_SCALE), rounded to nearest, ties
    ///   away from zero, as an `i16` (saturating);
    /// - `yaw`: `yaw` divided by `TAU`, its fraction of a turn (one rounding, in `[0, 1]`) times
    ///   [`UNITS_PER_TURN`](lactose_input::UNITS_PER_TURN), rounded to nearest, modulo
    ///   `UNITS_PER_TURN` (1.0 is taken modulo a turn); a
    ///   non-finite yaw quantises to zero;
    /// - `pitch`: `pitch` clamped to `[-PI/2, PI/2]`, divided by `TAU`, times
    ///   `UNITS_PER_TURN`, rounded to nearest, clamped to
    ///   [`Pitch::LIMIT`](lactose_input::Pitch::LIMIT); a NaN pitch quantises to zero;
    /// - `held`: `raw.held.union(raw.latched)`, and `raw.latched` becomes empty, so a tap
    ///   pressed and released between two polls is one tick of held: pressed at this tick,
    ///   released at the next; `pressed`: `held.without(previous_held)`; `released`:
    ///   `previous_held.without(held)`; then `previous_held` becomes `held`.
    ///
    /// Each value is computed in `f64` from the exact widening of the `f32`, with IEEE basic
    /// operations and exact roundings only, so one raw sample gives the same integers on
    /// every target. The result always passes [`PlayerInput::validate`]. This is the one
    /// place a float becomes an integer, and the integer is what both machines simulate.
    pub fn next(&mut self, raw: &mut RawSample) -> PlayerInput {
        let held = raw.held.union(raw.latched);
        raw.latched = Actions::EMPTY;
        let pressed = held.without(self.previous_held);
        let released = self.previous_held.without(held);
        self.previous_held = held;
        PlayerInput {
            movement: quantise_movement(raw.movement_x, raw.movement_y),
            yaw: quantise_yaw(raw.yaw),
            pitch: quantise_pitch(raw.pitch),
            held,
            pressed,
            released,
        }
    }
}

// Every float step below is an IEEE basic operation (`+`, `-`, `*`, `/`, `sqrt`) or an exact
// one (`floor`, `round`, a widening or saturating cast), never a library function, and Rust
// never contracts a multiply and an add: so a raw sample quantises to the same integers on
// every target, which is what lets the client simulate the value the server receives.
// `f64` holds every `f32` exactly, and the square of one, so nothing here overflows.

fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

fn squared_length(x: f32, y: f32) -> f64 {
    let (x, y) = (f64::from(x), f64::from(y));
    x * x + y * y
}

/// `yaw` wrapped into `[-PI, PI)` as an `f32`, the bounds being the `f32` PI. A sum already
/// in range is only rounded, so a zero delta never moves the yaw; NaN stays NaN.
fn wrap_yaw(yaw: f64) -> f32 {
    let pi = f64::from(core::f32::consts::PI);
    let wrapped = if (-pi..pi).contains(&yaw) {
        yaw
    } else {
        let turns = (yaw + PI) / TAU;
        (turns - turns.floor()) * TAU - PI
    };
    // Either branch can round up to the `f32` PI. A turn back from the `f64` sum, rounded
    // once more, is the `f32` -PI, which is in range.
    let narrow = wrapped as f32;
    if narrow >= core::f32::consts::PI { (wrapped - TAU) as f32 } else { narrow }
}

fn quantise_movement(x: f32, y: f32) -> Axis2 {
    let (x, y) = (finite_or_zero(x), finite_or_zero(y));
    let squared = squared_length(x, y);
    let (x, y) = (f64::from(x), f64::from(y));
    let (x, y) = if squared > 1.0 {
        let length = squared.sqrt();
        (x / length, y / length)
    } else {
        (x, y)
    };
    // Within `[-1, 1]` up to one rounding, so the product rounds to at most `AXIS_SCALE`;
    // the cast saturates regardless.
    let scale = f64::from(AXIS_SCALE);
    Axis2 { x: (x * scale).round() as i16, y: (y * scale).round() as i16 }
}

fn quantise_yaw(yaw: f32) -> Yaw {
    if !yaw.is_finite() {
        return Yaw::ZERO;
    }
    // The fraction of a turn: the subtraction is one rounding, in `[0, 1]`; 1.0 is taken
    // modulo a turn.
    let turns = f64::from(yaw) / TAU;
    let fraction = turns - turns.floor();
    let units = (fraction * f64::from(UNITS_PER_TURN)).round() as u32 % UNITS_PER_TURN;
    Yaw(units as u16)
}

fn quantise_pitch(pitch: f32) -> Pitch {
    if pitch.is_nan() {
        return Pitch::ZERO;
    }
    let radians = f64::from(pitch).clamp(-FRAC_PI_2, FRAC_PI_2);
    let units = (radians / TAU * f64::from(UNITS_PER_TURN)).round();
    let limit = f64::from(Pitch::LIMIT);
    Pitch(units.clamp(-limit, limit) as i16)
}
