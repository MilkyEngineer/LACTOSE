//! [`Quat`], a rotation quaternion.

use core::ops::{Mul, MulAssign, Neg};

use bytemuck::{Pod, Zeroable};

use crate::{Mat3, Scalar, Vec3};

/// A quaternion `w + xi + yj + zk`, used as a rotation when it has unit length.
///
/// Rotations are right-handed: a positive angle turns counter-clockwise when the axis points
/// at the viewer. `a * b` applies `b` first, then `a`. Operations that assume unit length say
/// so; nothing renormalises behind the caller's back, because that would hide drift and cost
/// a square root on every product.
///
/// Layout: `repr(C)`, fields `x`, `y`, `z`, `w` in order, no padding. The default value is all
/// zeros, which is not a rotation; use [`Quat::IDENTITY`].
#[derive(Clone, Copy, PartialEq, Debug, Default, Pod, Zeroable)]
#[repr(C)]
pub struct Quat {
    /// The i coefficient.
    pub x: Scalar,
    /// The j coefficient.
    pub y: Scalar,
    /// The k coefficient.
    pub z: Scalar,
    /// The real part.
    pub w: Scalar,
}

impl Quat {
    /// The rotation that changes nothing.
    pub const IDENTITY: Self = Self::new(Scalar::ZERO, Scalar::ZERO, Scalar::ZERO, Scalar::ONE);

    /// Dot products above this are close enough for [`slerp`](Self::slerp) to interpolate
    /// linearly; beyond it the division by the sine of the angle loses accuracy.
    const SLERP_LINEAR_THRESHOLD: Scalar = Scalar::from_f64(0.9995);

    /// A quaternion from its coefficients.
    #[must_use]
    pub const fn new(x: Scalar, y: Scalar, z: Scalar, w: Scalar) -> Self {
        Self { x, y, z, w }
    }

    /// A quaternion from `[x, y, z, w]`.
    #[must_use]
    pub const fn from_array(components: [Scalar; 4]) -> Self {
        let [x, y, z, w] = components;
        Self { x, y, z, w }
    }

    /// The coefficients as `[x, y, z, w]`.
    #[must_use]
    pub const fn to_array(self) -> [Scalar; 4] {
        [self.x, self.y, self.z, self.w]
    }

    /// The rotation by `angle` radians about `axis`, which must have unit length.
    #[must_use]
    pub fn from_axis_angle(axis: Vec3, angle: Scalar) -> Self {
        let (sin, cos) = (angle * Scalar::HALF).sin_cos();
        Self::new(axis.x * sin, axis.y * sin, axis.z * sin, cos)
    }

    /// The rotation by `angle` radians about the x axis.
    #[must_use]
    pub fn from_rotation_x(angle: Scalar) -> Self {
        let (sin, cos) = (angle * Scalar::HALF).sin_cos();
        Self::new(sin, Scalar::ZERO, Scalar::ZERO, cos)
    }

    /// The rotation by `angle` radians about the y axis.
    #[must_use]
    pub fn from_rotation_y(angle: Scalar) -> Self {
        let (sin, cos) = (angle * Scalar::HALF).sin_cos();
        Self::new(Scalar::ZERO, sin, Scalar::ZERO, cos)
    }

    /// The rotation by `angle` radians about the z axis.
    #[must_use]
    pub fn from_rotation_z(angle: Scalar) -> Self {
        let (sin, cos) = (angle * Scalar::HALF).sin_cos();
        Self::new(Scalar::ZERO, Scalar::ZERO, sin, cos)
    }

    /// The rotation a rotation matrix performs. `matrix` must be orthonormal with determinant
    /// one. Of the two quaternions for a rotation, the one returned is the
    /// [`canonical_sign`](Self::canonical_sign) one.
    #[must_use]
    pub fn from_mat3(matrix: Mat3) -> Self {
        Self::from_mat3_either_sign(matrix).canonical_sign()
    }

    /// The same quaternion or its negation, whichever has its first non-zero coefficient in
    /// the order `w, x, y, z` positive. `q` and `-q` are the same rotation, and this picks
    /// one of the two by a rule that depends only on the values: `w > 0`, or `w == 0` and
    /// `x > 0`, and so on. Negation is exact, so nothing is rounded. A quaternion with a NaN
    /// in the deciding coefficient is returned unchanged.
    ///
    /// With `w >= 0` guaranteed, a unit quaternion can be sent as `x, y, z` alone.
    #[must_use]
    pub fn canonical_sign(self) -> Self {
        let zero = Scalar::ZERO;
        let negative = self.w < zero
            || (self.w == zero
                && (self.x < zero
                    || (self.x == zero && (self.y < zero || (self.y == zero && self.z < zero)))));
        if negative { -self } else { self }
    }

    fn from_mat3_either_sign(matrix: Mat3) -> Self {
        let Vec3 { x: m00, y: m01, z: m02 } = matrix.x_axis;
        let Vec3 { x: m10, y: m11, z: m12 } = matrix.y_axis;
        let Vec3 { x: m20, y: m21, z: m22 } = matrix.z_axis;
        // Each branch divides by the largest coefficient, so the square root's argument is
        // at least one.
        if m22 <= Scalar::ZERO {
            let difference = m11 - m00;
            let one_minus_m22 = Scalar::ONE - m22;
            if difference <= Scalar::ZERO {
                let four_x_squared = one_minus_m22 - difference;
                let scale = Scalar::HALF / four_x_squared.sqrt();
                Self::new(
                    four_x_squared * scale,
                    (m01 + m10) * scale,
                    (m02 + m20) * scale,
                    (m12 - m21) * scale,
                )
            } else {
                let four_y_squared = one_minus_m22 + difference;
                let scale = Scalar::HALF / four_y_squared.sqrt();
                Self::new(
                    (m01 + m10) * scale,
                    four_y_squared * scale,
                    (m12 + m21) * scale,
                    (m20 - m02) * scale,
                )
            }
        } else {
            let sum = m11 + m00;
            let one_plus_m22 = Scalar::ONE + m22;
            if sum <= Scalar::ZERO {
                let four_z_squared = one_plus_m22 - sum;
                let scale = Scalar::HALF / four_z_squared.sqrt();
                Self::new(
                    (m02 + m20) * scale,
                    (m12 + m21) * scale,
                    four_z_squared * scale,
                    (m01 - m10) * scale,
                )
            } else {
                let four_w_squared = one_plus_m22 + sum;
                let scale = Scalar::HALF / four_w_squared.sqrt();
                Self::new(
                    (m12 - m21) * scale,
                    (m20 - m02) * scale,
                    (m01 - m10) * scale,
                    four_w_squared * scale,
                )
            }
        }
    }

    /// The dot product, `((x*x' + y*y') + z*z') + w*w'`.
    #[must_use]
    pub fn dot(self, other: Self) -> Scalar {
        ((self.x * other.x + self.y * other.y) + self.z * other.z) + self.w * other.w
    }

    /// The squared length, `self.dot(self)`.
    #[must_use]
    pub fn length_squared(self) -> Scalar {
        self.dot(self)
    }

    /// The length.
    #[must_use]
    pub fn length(self) -> Scalar {
        self.length_squared().sqrt()
    }

    /// Each coefficient divided by the length. A unit quaternion only when the length is
    /// positive and finite: a zero length gives NaN or infinite coefficients, and an infinite
    /// one (coefficients beyond about `1e154`) gives zeros.
    #[must_use]
    pub fn normalize(self) -> Self {
        let length = self.length();
        Self::new(self.x / length, self.y / length, self.z / length, self.w / length)
    }

    /// The conjugate, which is the inverse rotation when `self` has unit length.
    #[must_use]
    pub fn conjugate(self) -> Self {
        Self::new(-self.x, -self.y, -self.z, self.w)
    }

    /// The multiplicative inverse: the conjugate with each coefficient divided by the squared
    /// length. Not finite when the length is zero.
    #[must_use]
    pub fn inverse(self) -> Self {
        let length_squared = self.length_squared();
        let conjugate = self.conjugate();
        Self::new(
            conjugate.x / length_squared,
            conjugate.y / length_squared,
            conjugate.z / length_squared,
            conjugate.w / length_squared,
        )
    }

    /// The coefficientwise interpolation `self + (end - self) * t`, normalised. `end` is
    /// negated first when its dot product with `self` is negative, so the shorter arc is
    /// taken. Both must have unit length.
    #[must_use]
    pub fn nlerp(self, end: Self, t: Scalar) -> Self {
        let end = if self.dot(end) < Scalar::ZERO { -end } else { end };
        Self::lerp_unnormalized(self, end, t).normalize()
    }

    /// Interpolation along the shorter great arc at constant angular speed. Both must have
    /// unit length.
    ///
    /// `end` is negated first when its dot product with `self` is negative. When the dot
    /// product then exceeds 0.9995 the result is the normalised linear interpolation;
    /// otherwise it is `(self * sin((1 - t) * a) + end * sin(t * a)) / sin(a)` with `a` the
    /// arccosine of the dot product.
    #[must_use]
    pub fn slerp(self, end: Self, t: Scalar) -> Self {
        let dot = self.dot(end);
        let (end, dot) = if dot < Scalar::ZERO { (-end, -dot) } else { (end, dot) };
        if dot > Self::SLERP_LINEAR_THRESHOLD {
            return Self::lerp_unnormalized(self, end, t).normalize();
        }
        let angle = dot.acos();
        let from_weight = ((Scalar::ONE - t) * angle).sin();
        let to_weight = (t * angle).sin();
        let divisor = angle.sin();
        Self::new(
            (self.x * from_weight + end.x * to_weight) / divisor,
            (self.y * from_weight + end.y * to_weight) / divisor,
            (self.z * from_weight + end.z * to_weight) / divisor,
            (self.w * from_weight + end.w * to_weight) / divisor,
        )
    }

    fn lerp_unnormalized(start: Self, end: Self, t: Scalar) -> Self {
        Self::new(
            start.x + (end.x - start.x) * t,
            start.y + (end.y - start.y) * t,
            start.z + (end.z - start.z) * t,
            start.w + (end.w - start.w) * t,
        )
    }

    /// Whether every coefficient is finite.
    #[must_use]
    pub const fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite() && self.w.is_finite()
    }
}

/// The Hamilton product: `self * rhs` rotates by `rhs` first, then by `self`. Each
/// coefficient is a four-term sum evaluated left to right as written.
impl Mul for Quat {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        let (a, b) = (self, rhs);
        Self {
            x: ((a.w * b.x + a.x * b.w) + a.y * b.z) - a.z * b.y,
            y: ((a.w * b.y - a.x * b.z) + a.y * b.w) + a.z * b.x,
            z: ((a.w * b.z + a.x * b.y) - a.y * b.x) + a.z * b.w,
            w: ((a.w * b.w - a.x * b.x) - a.y * b.y) - a.z * b.z,
        }
    }
}

impl MulAssign for Quat {
    fn mul_assign(&mut self, rhs: Self) {
        *self = *self * rhs;
    }
}

/// Rotates a vector; `self` must have unit length. With `u` the vector part and
/// `t = 2 * (u × v)`, the result is `(v + t * w) + u × t`.
impl Mul<Vec3> for Quat {
    type Output = Vec3;
    fn mul(self, rhs: Vec3) -> Vec3 {
        let u = Vec3::new(self.x, self.y, self.z);
        let t = u.cross(rhs) * Scalar::TWO;
        (rhs + t * self.w) + u.cross(t)
    }
}

/// Every coefficient negated: the same rotation, the other of its two quaternions.
impl Neg for Quat {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z, -self.w)
    }
}
