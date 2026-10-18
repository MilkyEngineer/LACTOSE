//! Quantised wire values (architecture section 9.3): a position relative to its region's
//! origin and a rotation, as integers, with the two conversions written through
//! `lactose_math` so that the server and every client derive the same bits from the same
//! integers.
//!
//! Host-only: the packer quantises and the network thread dequantises before it stages a
//! row, so no boundary crate names these.

use bytemuck::{Pod, Zeroable};
use lactose_math::{Quat, Scalar, Vec3};

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
        let axis = |c: Scalar| (c * Scalar::from_i32(POSITION_UNITS_PER_METRE)).round().to_i32();
        Self { x: axis(relative.x), y: axis(relative.y), z: axis(relative.z) }
    }

    /// The position back as a simulation vector relative to the region origin:
    /// `Scalar::from_i32(c) / Scalar::from_i32(POSITION_UNITS_PER_METRE)` per coordinate,
    /// an exact division by a power of two, so the same bits on every target, and
    /// `quantise(dequantise(q)) == q` for every `q` under the 64-bit scalar.
    #[must_use]
    pub fn dequantise(self) -> Vec3 {
        let axis = |q: i32| Scalar::from_i32(q) / Scalar::from_i32(POSITION_UNITS_PER_METRE);
        Vec3::new(axis(self.x), axis(self.y), axis(self.z))
    }
}

/// A unit quaternion with each component in [`ROTATION_SCALE`]ths. Eight bytes of plain
/// data. The sign is canonical (the rule of `Quat::canonical_sign`, applied to the
/// integers), so `q` and `-q` quantise alike.
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

    /// What a quaternion that is no rotation quantises to.
    const NOT_A_ROTATION: Self = Self { x: 0, y: 0, z: 0, w: 0 };

    /// Quantises `rotation`: divided by its largest component magnitude and normalised,
    /// each component times [`ROTATION_SCALE`] through `Scalar::round` and
    /// `Scalar::to_i32`, clamped to `±ROTATION_SCALE`, then the sign of the four integers
    /// made canonical by the rule of `Quat::canonical_sign`. Quantising is odd-symmetric,
    /// so `q` and `-q` quantise alike; the sign is decided on the integers because a
    /// component that rounds to zero would otherwise leave it to a smaller one. A zero
    /// quaternion or one with a NaN or infinite component quantises to the all-zero
    /// value; nothing else does, since the largest component of a unit quaternion is at
    /// least one half. Rotations do not round-trip exactly (see the crate README for the
    /// measured rates).
    #[must_use]
    pub fn quantise(rotation: Quat) -> Self {
        if !rotation.is_finite() {
            return Self::NOT_A_ROTATION;
        }
        let [x, y, z, w] = rotation.to_array();
        let largest = x.abs().max(y.abs()).max(z.abs()).max(w.abs());
        if largest == Scalar::ZERO {
            return Self::NOT_A_ROTATION;
        }
        // Dividing by the largest magnitude first keeps the squared length in `[1, 4]`, so
        // no finite non-zero quaternion overflows or underflows in `normalize`.
        let scaled = Quat::new(x / largest, y / largest, z / largest, w / largest).normalize();
        let scale = i32::from(ROTATION_SCALE);
        let component = |c: Scalar| {
            let q = (c * Scalar::from_i32(scale)).round().to_i32().clamp(-scale, scale);
            i16::try_from(q).expect("clamped to the scale, which is an i16")
        };
        let quantised = Self {
            x: component(scaled.x),
            y: component(scaled.y),
            z: component(scaled.z),
            w: component(scaled.w),
        };
        let Self { x, y, z, w } = quantised;
        let negative = w < 0 || (w == 0 && (x < 0 || (x == 0 && (y < 0 || (y == 0 && z < 0)))));
        if negative { Self { x: -x, y: -y, z: -z, w: -w } } else { quantised }
    }

    /// The rotation back: each component as `Scalar::from_i32(c) /
    /// Scalar::from_i32(ROTATION_SCALE)` (one true division each), then `Quat::normalize`,
    /// so the result has unit length up to rounding. The all-zero value dequantises to
    /// `Quat::IDENTITY`.
    #[must_use]
    pub fn dequantise(self) -> Quat {
        if self == Self::NOT_A_ROTATION {
            return Quat::IDENTITY;
        }
        let scale = Scalar::from_i32(i32::from(ROTATION_SCALE));
        let component = |c: i16| Scalar::from_i32(i32::from(c)) / scale;
        Quat::new(component(self.x), component(self.y), component(self.z), component(self.w))
            .normalize()
    }
}

impl Default for QuantisedRotation {
    fn default() -> Self {
        Self::IDENTITY
    }
}
