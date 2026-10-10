//! Quantised wire values (architecture section 9.3): a position relative to its region's
//! origin and a rotation, as integers, with the two conversions written through
//! `lactose_math` so that the server and every client derive the same bits from the same
//! integers.
//!
//! Host-only: the packer quantises and the network thread dequantises before it stages a
//! row, so no boundary crate names these.

use bytemuck::{Pod, Zeroable};
use lactose_math::{Quat, Vec3};

/// Position units per metre on the wire: a power of two, so that dequantising is an exact
/// division and, under the 64-bit scalar, quantising a value that was dequantised gives
/// the integer back for every integer (under a 32-bit scalar, for integers within 2^24
/// units, sixteen kilometres, of the region origin). About one millimetre; an `i32` axis
/// then spans about two thousand kilometres either side of the region origin.
pub const POSITION_UNITS_PER_METRE: i32 = 1024;

/// The integer a unit quaternion component of one maps to.
pub const ROTATION_SCALE: i16 = i16::MAX;

/// A position relative to the origin of the region the entity resides in, in
/// [`POSITION_UNITS_PER_METRE`]. Twelve bytes of plain data; what the replication stream
/// carries for every entity the receiving player has not claimed (claimed entities travel
/// at full precision, architecture section 9.4).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Pod, Zeroable)]
#[repr(C)]
pub struct QuantisedPosition {
    /// Along x.
    pub x: i32,
    /// Along y.
    pub y: i32,
    /// Along z.
    pub z: i32,
}

impl QuantisedPosition {
    /// The region origin itself.
    pub const ZERO: Self = Self { x: 0, y: 0, z: 0 };

    /// Quantises `relative`, a simulation position with the region origin already
    /// subtracted: each coordinate times [`POSITION_UNITS_PER_METRE`] through
    /// `Scalar::round` (nearest, ties away from zero) and `Scalar::to_i32`, which saturates
    /// and maps a NaN to zero.
    #[must_use]
    pub fn quantise(relative: Vec3) -> Self {
        todo!()
    }

    /// The position back as a simulation vector relative to the region origin:
    /// `Scalar::from_i32(c) / Scalar::from_i32(POSITION_UNITS_PER_METRE)` per coordinate,
    /// an exact division by a power of two, so the same bits on every target, and
    /// `quantise(dequantise(q)) == q` for every `q` under the 64-bit scalar.
    #[must_use]
    pub fn dequantise(self) -> Vec3 {
        todo!()
    }
}

/// A unit quaternion with each component in [`ROTATION_SCALE`]ths. Eight bytes of plain
/// data. The sign is canonical (`Quat::canonical_sign`) before quantising, so `q` and `-q`
/// quantise alike.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct QuantisedRotation {
    /// The x component.
    pub x: i16,
    /// The y component.
    pub y: i16,
    /// The z component.
    pub z: i16,
    /// The w component.
    pub w: i16,
}

impl QuantisedRotation {
    /// No rotation.
    pub const IDENTITY: Self = Self { x: 0, y: 0, z: 0, w: ROTATION_SCALE };

    /// Quantises `rotation`, normalised and with its sign made canonical first: each
    /// component times [`ROTATION_SCALE`] through `Scalar::round` and `Scalar::to_i32`,
    /// then narrowed to `i16` (the product of a unit component never exceeds it). A zero
    /// or NaN quaternion, whose normalisation is NaN, quantises to the all-zero value;
    /// nothing else does. Rotations do not round-trip exactly: `quantise(dequantise(q))`
    /// differs from `q` by one unit in one component for about one percent of values,
    /// and the value is stable after one trip.
    #[must_use]
    pub fn quantise(rotation: Quat) -> Self {
        todo!()
    }

    /// The rotation back: each component as `Scalar::from_i32(c) /
    /// Scalar::from_i32(ROTATION_SCALE)` (one true division each), then `Quat::normalize`,
    /// so the result has unit length up to rounding. The all-zero value dequantises to
    /// `Quat::IDENTITY`.
    #[must_use]
    pub fn dequantise(self) -> Quat {
        todo!()
    }
}

impl Default for QuantisedRotation {
    fn default() -> Self {
        Self::IDENTITY
    }
}
