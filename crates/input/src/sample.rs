//! The quantised per-tick sample (architecture sections 5.3, 6.1 and 9.2): the component a
//! simulation kernel reads, the integer encodings every value crosses the boundary in, and
//! the byte image the wire carries and the journal stores.
//!
//! Every stored value is an integer, and every `Scalar`, vector or rotation derived from
//! one is computed through `lactose_math` in the order the documentation states, so one
//! sample gives the same bits on every target. No float enters from the host side:
//! quantisation is `lactose-input-host`'s, outside the boundary.

use lactose_core::{Error, ErrorKind, Result, Tick};
use lactose_math::{Quat, Scalar, Vec2, Vec3};
use lactose_reflect::{component, value};
use lactose_sim::PlayerSample;

/// The size in bytes of a sample's image: the size of [`PlayerInput`], twenty.
pub const SAMPLE_SIZE: usize = size_of::<PlayerInput>();

/// The unit of [`Yaw`] and [`Pitch`]: one turn is this many units, so a unit is 1/65536 of
/// a turn, about 0.0055 degrees.
pub const UNITS_PER_TURN: u32 = 1 << 16;

/// The scale of an [`Axis2`] component: a raw component in `[-1, 1]` is multiplied by this
/// and rounded, so `AXIS_SCALE` decodes to one.
pub const AXIS_SCALE: i16 = i16::MAX;

/// A movement vector quantised to sixteen bits per axis, `x` to the player's right and `y`
/// forward, each in `[-AXIS_SCALE, AXIS_SCALE]`. A valid sample's vector lies within the
/// disc `x² + y² <= AXIS_SCALE² + 2·AXIS_SCALE`, the unit disc plus the quantiser's
/// rounding ([`PlayerInput::validate`]), so a decoded vector exceeds unit length only by
/// rounding; `i16::MIN` is invalid in either component.
#[value]
#[derive(PartialEq, Eq, Hash, Debug, Default)]
pub struct Axis2 {
    /// Right.
    #[field(id = 1)]
    pub x: i16,
    /// Forward.
    #[field(id = 2)]
    pub y: i16,
}

impl Axis2 {
    /// No movement.
    pub const ZERO: Self = Self { x: 0, y: 0 };

    /// The vector in `[-1, 1]` per component: each component is
    /// `Scalar::from_i32(i32::from(c)) / Scalar::from_i32(i32::from(AXIS_SCALE))`, one true
    /// division per component.
    #[must_use]
    pub fn to_vec2(self) -> Vec2 {
        let scale = Scalar::from_i32(i32::from(AXIS_SCALE));
        Vec2::new(
            Scalar::from_i32(i32::from(self.x)) / scale,
            Scalar::from_i32(i32::from(self.y)) / scale,
        )
    }
}

/// The absolute camera yaw, in [`UNITS_PER_TURN`]ths of a turn, wrapping: zero faces the
/// `-Z` axis and a quarter turn (16384 units) faces `-X`, the rotation
/// `Quat::from_rotation_y` makes (the world frame of the crate `README.md`).
#[value]
#[derive(PartialEq, Eq, Hash, Debug, Default)]
pub struct Yaw(
    /// The units, wrapping at a full turn.
    #[field(id = 1)]
    pub u16,
);

impl Yaw {
    /// Facing `-Z`.
    pub const ZERO: Self = Self(0);

    /// The yaw in radians, in `[0, TAU)`:
    /// `Scalar::from_u32(units) / Scalar::from_u32(UNITS_PER_TURN) * Scalar::TAU`, where the
    /// division by a power of two is exact and the product rounds once.
    #[must_use]
    pub fn radians(self) -> Scalar {
        Scalar::from_u32(u32::from(self.0)) / Scalar::from_u32(UNITS_PER_TURN) * Scalar::TAU
    }

    /// The yaw as a rotation about `+Y`: `Quat::from_rotation_y(self.radians())`, which
    /// turns `-Vec3::Z` into the horizontal part of [`PlayerInput::aim`] (within rounding;
    /// `aim` is the stated formula, not this product). What a facing kernel or the sim
    /// skeleton composes with.
    #[must_use]
    pub fn rotation(self) -> Quat {
        Quat::from_rotation_y(self.radians())
    }
}

/// The absolute camera pitch, in [`UNITS_PER_TURN`]ths of a turn, positive looking up,
/// within `[-LIMIT, LIMIT]`: a quarter turn each way. A sample whose pitch lies outside
/// that range is invalid ([`PlayerInput::validate`]).
#[value]
#[derive(PartialEq, Eq, Hash, Debug, Default)]
pub struct Pitch(
    /// The units, within `[-LIMIT, LIMIT]`.
    #[field(id = 1)]
    pub i16,
);

impl Pitch {
    /// A quarter turn, the largest magnitude a valid pitch has: 16384 units.
    pub const LIMIT: i16 = (UNITS_PER_TURN / 4) as i16;
    /// Level.
    pub const ZERO: Self = Self(0);

    /// Whether the pitch is within `[-LIMIT, LIMIT]`.
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.0 >= -Self::LIMIT && self.0 <= Self::LIMIT
    }

    /// The pitch in radians, in `[-PI/2, PI/2]` for a valid pitch:
    /// `Scalar::from_i32(units) / Scalar::from_u32(UNITS_PER_TURN) * Scalar::TAU`, the
    /// division exact and the product rounded once.
    #[must_use]
    pub fn radians(self) -> Scalar {
        Scalar::from_i32(i32::from(self.0)) / Scalar::from_u32(UNITS_PER_TURN) * Scalar::TAU
    }
}

/// A named action bit of [`Actions`]: the engine's actions, the set milestone 1 names. The
/// discriminant is the bit. Every named bit is below [`Actions::GAME_FIRST`]; a later
/// milestone may name more of the engine's bits below it, which adds variants (hence
/// `non_exhaustive`) and moves no bit. The bits from [`Actions::GAME_FIRST`] up belong to
/// the game, mapped by milestone 5's action map and read by index through
/// [`Actions::contains_bit`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
#[non_exhaustive]
pub enum Action {
    /// Jump.
    Jump = 0,
    /// Throw the held item (the `throw_bomb` kernel of architecture section 6.1).
    Throw = 1,
    /// Interact with what is in front.
    Interact = 2,
    /// Sprint while held.
    Sprint = 3,
    /// Crouch while held.
    Crouch = 4,
}

impl Action {
    /// How many low bits are named: the bits below this are the variants above, in order.
    /// It grows when a variant is added, never past [`Actions::GAME_FIRST`]; a game's bit is
    /// numbered from `GAME_FIRST`, never from this.
    pub const NAMED: u8 = 5;

    /// The bit this action occupies.
    #[must_use]
    pub const fn bit(self) -> u8 {
        self as u8
    }
}

/// A set of actions as a 32-bit mask: bit `n` set means action `n` is in the set. The
/// definition of the type, so the methods are final.
#[value]
#[derive(PartialEq, Eq, Hash, Debug, Default)]
pub struct Actions(
    /// The mask.
    #[field(id = 1)]
    pub u32,
);

impl Actions {
    /// No action.
    pub const EMPTY: Self = Self(0);
    /// The number of bits, the engine's and the game's together.
    pub const BITS: u8 = 32;
    /// The first of the game's bits, which never changes: the bits below it are the
    /// engine's, named by [`Action`] or reserved for it, and the game's bit `k` is
    /// `GAME_FIRST + k`, read with [`contains_bit`](Self::contains_bit) and set with
    /// [`with_bit`](Self::with_bit). Part of the sample's encoding.
    pub const GAME_FIRST: u8 = 16;

    /// The set with exactly the bits of `bits`.
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// The mask.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether the set is empty.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether `action` is in the set.
    #[must_use]
    pub const fn contains(self, action: Action) -> bool {
        self.contains_bit(action.bit())
    }

    /// Whether bit `bit` is set. A bit at or above [`BITS`](Self::BITS) is never set.
    #[must_use]
    pub const fn contains_bit(self, bit: u8) -> bool {
        match 1u32.checked_shl(bit as u32) {
            Some(mask) => self.0 & mask != 0,
            None => false,
        }
    }

    /// The set with `action` added.
    #[must_use]
    pub const fn with(self, action: Action) -> Self {
        self.with_bit(action.bit())
    }

    /// The set with bit `bit` added. A bit at or above [`BITS`](Self::BITS) is ignored.
    #[must_use]
    pub const fn with_bit(self, bit: u8) -> Self {
        match 1u32.checked_shl(bit as u32) {
            Some(mask) => Self(self.0 | mask),
            None => self,
        }
    }

    /// The actions in either set: how the sources' held sets combine.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// The actions in both sets.
    #[must_use]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// The actions of `self` that are not in `other`: how the quantiser derives the edges,
    /// `held.without(previous_held)` for `pressed` and `previous_held.without(held)` for
    /// `released`.
    #[must_use]
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

/// One player's input for one tick, as the simulation reads it (architecture sections 5.3,
/// 6.1 and 9.2): `transient`, so the ECS zeroes it at every input sync point and
/// [`PlayerInputApplier`](crate::PlayerInputApplier) writes it there for every player whose
/// sample the frame carries. A player without a sample this tick reads as
/// [`IDLE`](Self::IDLE), which is also `Default`.
///
/// Twenty bytes, no padding, every field an integer; the layout is the byte image
/// ([`to_sample_bytes`](Self::to_sample_bytes)). A kernel takes `&PlayerInput` and reads
/// the fields, or the derived directions [`aim`](Self::aim) and
/// [`world_movement`](Self::world_movement), which are computed from the integers through
/// `lactose_math` when called and never stored.
///
/// ```
/// use lactose_ecs::{Commands, Ctx, StableKeyOf, system};
/// use lactose_input::{Action, PlayerInput};
///
/// #[system(name = "doc::throw", worlds = [server, prediction])]
/// fn throw(ctx: Ctx, cmd: Commands, key: StableKeyOf, input: &PlayerInput) {
///     if input.pressed.contains(Action::Throw) {
///         let direction = input.aim();
///         // cmd.spawn(..) a projectile along `direction` ...
///     }
/// }
/// ```
#[component(key = "input::player_input", transient)]
#[derive(PartialEq, Eq, Hash, Debug, Default)]
pub struct PlayerInput {
    /// The movement stick or keys, in the player's own frame.
    #[field(id = 1)]
    pub movement: Axis2,
    /// The absolute camera yaw the sample was produced with.
    #[field(id = 2)]
    pub yaw: Yaw,
    /// The absolute camera pitch the sample was produced with.
    #[field(id = 3)]
    pub pitch: Pitch,
    /// The actions held during the tick.
    #[field(id = 4)]
    pub held: Actions,
    /// The actions that became held since the previous sample: the edges, computed by the
    /// quantiser on the client and carried in the sample, so the server keeps no button
    /// history. A subset of `held`. A substitute for a lost sample carries none
    /// ([`substitute`](Self::substitute)).
    #[field(id = 5)]
    pub pressed: Actions,
    /// The actions that stopped being held since the previous sample, computed and carried
    /// like `pressed`; disjoint from `held`. For a bow drawn and loosed, a charged attack, a
    /// hold-to-aim throw. A substitute carries none.
    #[field(id = 6)]
    pub released: Actions,
}

impl PlayerInput {
    /// No movement, facing `-Z`, level, nothing held, pressed or released: the all-zero
    /// value the ECS leaves in the column when no sample is written, and `Default`.
    pub const IDLE: Self = Self {
        movement: Axis2::ZERO,
        yaw: Yaw::ZERO,
        pitch: Pitch::ZERO,
        held: Actions::EMPTY,
        pressed: Actions::EMPTY,
        released: Actions::EMPTY,
    };

    /// Checks the invariants the layout cannot, each one a kernel is told it may rely on;
    /// fails with `InvalidData` on the first that does not hold:
    ///
    /// - the pitch is within [`Pitch::LIMIT`];
    /// - neither movement component is `i16::MIN`, and in 64-bit integers
    ///   `x² + y² <= AXIS_SCALE² + 2·AXIS_SCALE`, a bound the quantiser's half-unit rounding
    ///   of a vector in the unit disc never exceeds;
    /// - `pressed.without(held).is_empty()`: an edge names a held action;
    /// - `released.intersection(held).is_empty()`: a release names an action not held.
    ///
    /// Every other field accepts every bit pattern of its width. The quantiser's output
    /// always passes; a forged image that does not is refused before it reaches a row.
    pub fn validate(&self) -> Result<()> {
        if !self.pitch.is_valid() {
            return Err(invalid(format!(
                "player input pitch {} is outside the quarter-turn limit {}",
                self.pitch.0,
                Pitch::LIMIT
            )));
        }
        let Axis2 { x, y } = self.movement;
        if x == i16::MIN || y == i16::MIN {
            return Err(invalid(format!("player input movement ({x}, {y}) holds `i16::MIN`")));
        }
        // In 64 bits nothing overflows: each square is below 2^30.
        let (x, y, scale) = (i64::from(x), i64::from(y), i64::from(AXIS_SCALE));
        if x * x + y * y > scale * scale + 2 * scale {
            return Err(invalid(format!(
                "player input movement ({x}, {y}) lies outside the unit disc and its rounding"
            )));
        }
        if !self.pressed.without(self.held).is_empty() {
            return Err(invalid(format!(
                "player input presses {:#x}, which it does not hold ({:#x})",
                self.pressed.bits(),
                self.held.bits()
            )));
        }
        if !self.released.intersection(self.held).is_empty() {
            return Err(invalid(format!(
                "player input releases {:#x}, which it holds ({:#x})",
                self.released.bits(),
                self.held.bits()
            )));
        }
        Ok(())
    }

    /// The byte image: the twenty memory bytes of the value, which on every supported
    /// target is each field little-endian at its `repr(C)` offset (`movement.x` at 0,
    /// `movement.y` at 2, `yaw` at 4, `pitch` at 6, `held` at 8, `pressed` at 12,
    /// `released` at 16). This is what [`PlayerSample::sample`] holds, the journal stores
    /// verbatim and the wire carries; its version is the component's schema version, which
    /// the protocol hash of architecture section 9.1 covers as a boundary component type,
    /// so no version byte is in the image.
    #[must_use]
    pub fn to_sample_bytes(&self) -> [u8; SAMPLE_SIZE] {
        let mut image = [0; SAMPLE_SIZE];
        image.copy_from_slice(lactose_reflect::bytemuck::bytes_of(self));
        image
    }

    /// The value whose image `bytes` is: exactly [`SAMPLE_SIZE`] bytes, read through the
    /// component's `CheckedBitPattern` ([`lactose_reflect::decode`]) and then
    /// [`validate`](Self::validate)d. Fails with `InvalidData` otherwise. `net` must run it
    /// on every received sample and treat a failure as a lost sample (substitute), so that
    /// a malformed image never reaches a frame; the applier runs it again and drops what
    /// fails, and the tick runs.
    pub fn from_sample_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != SAMPLE_SIZE {
            return Err(invalid(format!(
                "a player input image is {SAMPLE_SIZE} bytes, not {}",
                bytes.len()
            )));
        }
        let input: Self = lactose_reflect::decode(bytes)?;
        input.validate()?;
        Ok(input)
    }

    /// The frame entry for this sample at `input_tick`: `PlayerSample { input_tick,
    /// substituted: false, sample: to_sample_bytes().to_vec() }`. What `lactose-predict`
    /// puts in the prediction world's frame for the local player and what a test puts in
    /// a server frame; `net` sets `substituted` on the entry it builds from
    /// [`substitute`](Self::substitute).
    #[must_use]
    pub fn to_player_sample(&self, input_tick: Tick) -> PlayerSample {
        PlayerSample { input_tick, substituted: false, sample: self.to_sample_bytes().to_vec() }
    }

    /// The sample the server applies for a tick whose sample is still missing after
    /// redundancy (architecture section 9.2, "replaced by the previous input", refined):
    /// this sample, the previous one applied, continued, with `pressed` and `released`
    /// cleared, so a held action stays held and no edge repeats. `IDLE.substitute()` is
    /// `IDLE`, the substitute for a player with no previous sample. A pure function; `net`
    /// sets `substituted` on the frame entry so the journal records the substitution.
    #[must_use]
    pub fn substitute(&self) -> Self {
        Self { pressed: Actions::EMPTY, released: Actions::EMPTY, ..*self }
    }

    /// This sample, the first one delivered after one or more substitutes, with its edges
    /// recomputed against `applied`, the last substitute applied, replacing the ones the
    /// client computed against its own previous sample, which was lost: `pressed =
    /// held.without(applied.held)` and `released = applied.held.without(held)`. A press
    /// whose first sample was lost therefore fires now, and no release or re-press of an
    /// action that no applied sample held can fire, so every applied sample's edges match
    /// the applied `held` sequence; what it costs is a tap, or a release and re-press,
    /// wholly inside the gap. The result passes [`validate`](Self::validate) when `self`
    /// does. `net` applies it once, to the first delivered sample after a substitute, and
    /// the journal records the applied bytes.
    #[must_use]
    pub fn after_substitute(&self, applied: &PlayerInput) -> Self {
        Self {
            pressed: self.held.without(applied.held),
            released: applied.held.without(self.held),
            ..*self
        }
    }

    /// The unit direction the camera looks along, in the world frame of the crate
    /// `README.md` (right-handed, `+Y` up): with `(sin_y, cos_y) = yaw.radians().sin_cos()`
    /// and `(sin_p, cos_p) = pitch.radians().sin_cos()`, the vector
    /// `(-(cos_p * sin_y), sin_p, -(cos_p * cos_y))`, each product rounded once. At the
    /// idle sample it is `-Vec3::Z`. The `aim` of the architecture section 6.1 example.
    #[must_use]
    pub fn aim(&self) -> Vec3 {
        let (sin_y, cos_y) = self.yaw.radians().sin_cos();
        let (sin_p, cos_p) = self.pitch.radians().sin_cos();
        Vec3::new(-(cos_p * sin_y), sin_p, -(cos_p * cos_y))
    }

    /// The movement vector in the world frame, on the horizontal plane: `movement.y`
    /// along the yaw's forward `(-sin_y, 0, -cos_y)` and `movement.x` along its right
    /// `(cos_y, 0, -sin_y)`, computed as `(m.x * cos_y - m.y * sin_y, 0,
    /// -(m.x * sin_y + m.y * cos_y))` from `m = movement.to_vec2()`, each operation in
    /// that order. For a valid sample its length is at most one up to rounding. What a
    /// movement kernel reads (architecture section 5.3: the sample's yaw gives the
    /// movement direction).
    #[must_use]
    pub fn world_movement(&self) -> Vec3 {
        let m = self.movement.to_vec2();
        let (sin_y, cos_y) = self.yaw.radians().sin_cos();
        Vec3::new(m.x * cos_y - m.y * sin_y, Scalar::ZERO, -(m.x * sin_y + m.y * cos_y))
    }
}

fn invalid(message: String) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every named action is one of the engine's bits, below `Actions::GAME_FIRST`, and the
    /// named bits are exactly `0..Action::NAMED`. The match has no wildcard, so a new variant
    /// does not compile until it is listed here and checked.
    #[test]
    fn every_named_action_is_below_the_games_bits() {
        let index = |action: Action| match action {
            Action::Jump => 0,
            Action::Throw => 1,
            Action::Interact => 2,
            Action::Sprint => 3,
            Action::Crouch => 4,
        };
        let all = [Action::Jump, Action::Throw, Action::Interact, Action::Sprint, Action::Crouch];
        for (position, action) in all.into_iter().enumerate() {
            assert_eq!(index(action), position);
            assert_eq!(usize::from(action.bit()), position, "{action:?}");
            assert!(action.bit() < Actions::GAME_FIRST, "{action:?}");
        }
        assert_eq!(all.len(), usize::from(Action::NAMED));
        const { assert!(Action::NAMED <= Actions::GAME_FIRST) };
        // The game's first bit is readable and writable, and distinct from every named one.
        let game = Actions::EMPTY.with_bit(Actions::GAME_FIRST);
        assert!(game.contains_bit(Actions::GAME_FIRST));
        assert!(all.into_iter().all(|action| !game.contains(action)));
    }
}
