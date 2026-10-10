//! The quantised per-tick sample (architecture sections 5.3, 6.1 and 9.2): the component a
//! simulation kernel reads, the integer encodings every value crosses the boundary in, and
//! the byte image the wire carries and the journal stores.
//!
//! What boundary code may use. Deterministic by construction: every stored value is an
//! integer, and every `Scalar` or vector derived from one is computed through
//! `lactose_math` in the order the documentation states, so one sample gives the same bits
//! on every target. No float enters from the host side: quantisation is
//! [`RawSample::quantise`](crate::raw::RawSample::quantise), outside the boundary.

use lactose_core::{Result, Tick};
use lactose_math::{Scalar, Vec2, Vec3};
use lactose_reflect::{component, value};
use lactose_sim::PlayerSample;

/// The size in bytes of a sample's image: the size of [`PlayerInput`], sixteen.
pub const SAMPLE_SIZE: usize = size_of::<PlayerInput>();

/// The unit of [`Yaw`] and [`Pitch`]: one turn is this many units, so a unit is 1/65536 of
/// a turn, about 0.0055 degrees.
pub const UNITS_PER_TURN: u32 = 1 << 16;

/// The scale of an [`Axis2`] component: a raw component in `[-1, 1]` is multiplied by this
/// and rounded, so `AXIS_SCALE` decodes to one.
pub const AXIS_SCALE: i16 = i16::MAX;

/// A movement vector quantised to sixteen bits per axis, `x` to the player's right and `y`
/// forward, each in `[-AXIS_SCALE, AXIS_SCALE]`; the quantiser limits the raw vector to the
/// unit disc, so a decoded vector exceeds unit length only by rounding (at most one unit per
/// axis). `i16::MIN` is accepted and decodes to just below minus one.
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
        todo!()
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
        todo!()
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
        todo!()
    }

    /// The pitch in radians, in `[-PI/2, PI/2]` for a valid pitch:
    /// `Scalar::from_i32(units) / Scalar::from_u32(UNITS_PER_TURN) * Scalar::TAU`, the
    /// division exact and the product rounded once.
    #[must_use]
    pub fn radians(self) -> Scalar {
        todo!()
    }
}

/// A named action bit of [`Actions`]: the fixed set milestone 1 names. The discriminant is
/// the bit. Bits from [`Action::NAMED`] up to [`Actions::BITS`] belong to the game, mapped
/// by milestone 5's action map and read by index through [`Actions::contains_bit`]; a
/// later milestone may name more of them, which adds variants and changes no encoding.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
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
    /// The number of bits, named and generic together.
    pub const BITS: u8 = 32;

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

    /// The actions of `self` that are not in `other`: how the quantiser derives the
    /// `pressed` edges, `held.without(previous_held)`.
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
/// Sixteen bytes, no padding, every field an integer; the layout is the byte image
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
    /// history. A substitute for a lost sample carries none ([`substitute`](Self::substitute)).
    #[field(id = 5)]
    pub pressed: Actions,
}

impl PlayerInput {
    /// No movement, facing `-Z`, level, nothing held or pressed: the all-zero value the ECS
    /// leaves in the column when no sample is written, and `Default`.
    pub const IDLE: Self = Self {
        movement: Axis2::ZERO,
        yaw: Yaw::ZERO,
        pitch: Pitch::ZERO,
        held: Actions::EMPTY,
        pressed: Actions::EMPTY,
    };

    /// Checks the one range the layout cannot: the pitch is within
    /// [`Pitch::LIMIT`]. Fails with `InvalidData`. Every other field accepts every bit
    /// pattern of its width.
    pub fn validate(&self) -> Result<()> {
        todo!()
    }

    /// The byte image: the sixteen memory bytes of the value, which on every supported
    /// target is each field little-endian at its `repr(C)` offset (`movement.x` at 0,
    /// `movement.y` at 2, `yaw` at 4, `pitch` at 6, `held` at 8, `pressed` at 12). This is
    /// what [`PlayerSample::sample`] holds, the journal stores verbatim and the wire
    /// carries; its version is the component's schema version, which the protocol hash of
    /// architecture section 9.1 covers, so no version byte is in the image.
    #[must_use]
    pub fn to_sample_bytes(&self) -> [u8; SAMPLE_SIZE] {
        todo!()
    }

    /// The value whose image `bytes` is: exactly [`SAMPLE_SIZE`] bytes, read through the
    /// component's `CheckedBitPattern` ([`lactose_reflect::decode`]) and then
    /// [`validate`](Self::validate)d. Fails with `InvalidData` otherwise, so an applier
    /// drops a malformed sample and the tick runs, and `net` can refuse one before it
    /// reaches a frame.
    pub fn from_sample_bytes(bytes: &[u8]) -> Result<Self> {
        todo!()
    }

    /// The frame entry for this sample at `input_tick`: `PlayerSample { input_tick,
    /// substituted: false, sample: to_sample_bytes().to_vec() }`. What `lactose-predict`
    /// puts in the prediction world's frame for the local player and what a test puts in
    /// a server frame; `net` sets `substituted` on the entry it builds from
    /// [`substitute`](Self::substitute).
    #[must_use]
    pub fn to_player_sample(&self, input_tick: Tick) -> PlayerSample {
        todo!()
    }

    /// The sample the server applies for a tick whose sample is still missing after
    /// redundancy (architecture section 9.2): this sample, the previous one applied,
    /// continued, with `pressed` cleared, so a held action stays held and no edge repeats.
    /// A player with no previous sample gets [`IDLE`](Self::IDLE), which `substitute` of
    /// `IDLE` is. A pure function of its input; `net` sets `substituted` on the frame
    /// entry so the journal records the substitution.
    #[must_use]
    pub fn substitute(&self) -> Self {
        todo!()
    }

    /// The unit direction the camera looks along, in the world frame of the crate
    /// `README.md` (right-handed, `+Y` up): with `(sin_y, cos_y) = yaw.radians().sin_cos()`
    /// and `(sin_p, cos_p) = pitch.radians().sin_cos()`, the vector
    /// `(-(cos_p * sin_y), sin_p, -(cos_p * cos_y))`, each product rounded once. At the
    /// idle sample it is `-Vec3::Z`. The `aim` of the architecture section 6.1 example.
    #[must_use]
    pub fn aim(&self) -> Vec3 {
        todo!()
    }

    /// The movement vector in the world frame, on the horizontal plane: `movement.y`
    /// along the yaw's forward `(-sin_y, 0, -cos_y)` and `movement.x` along its right
    /// `(cos_y, 0, -sin_y)`, computed as `(m.x * cos_y - m.y * sin_y, 0,
    /// -(m.x * sin_y + m.y * cos_y))` from `m = movement.to_vec2()`, each operation in
    /// that order. Its length is at most one up to rounding. What a movement kernel reads
    /// (architecture section 5.3: the sample's yaw gives the movement direction).
    #[must_use]
    pub fn world_movement(&self) -> Vec3 {
        todo!()
    }
}
