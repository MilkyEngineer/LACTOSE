//! [`Mat3`] and [`Mat4`], column-major matrices acting on column vectors.

use core::ops::{Add, Mul, Sub};

use bytemuck::{Pod, Zeroable};

use crate::{Quat, Scalar, Vec3, Vec4};

/// A 3×3 matrix stored as three columns. `m * v` is `x_axis * v.x + y_axis * v.y +
/// z_axis * v.z`, and `a * b` applies `b` first.
///
/// Layout: `repr(C)`, the columns in order, nine scalars, no padding.
#[derive(Clone, Copy, PartialEq, Debug, Default, Pod, Zeroable)]
#[repr(C)]
pub struct Mat3 {
    /// The first column, the image of the x axis.
    pub x_axis: Vec3,
    /// The second column, the image of the y axis.
    pub y_axis: Vec3,
    /// The third column, the image of the z axis.
    pub z_axis: Vec3,
}

impl Mat3 {
    /// All elements zero.
    pub const ZERO: Self = Self::from_cols(Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
    /// The identity.
    pub const IDENTITY: Self = Self::from_cols(Vec3::X, Vec3::Y, Vec3::Z);

    /// A matrix from its columns.
    #[must_use]
    pub const fn from_cols(x_axis: Vec3, y_axis: Vec3, z_axis: Vec3) -> Self {
        Self { x_axis, y_axis, z_axis }
    }

    /// A matrix from its elements in column-major order.
    #[must_use]
    pub const fn from_array(m: [Scalar; 9]) -> Self {
        Self::from_cols(
            Vec3::new(m[0], m[1], m[2]),
            Vec3::new(m[3], m[4], m[5]),
            Vec3::new(m[6], m[7], m[8]),
        )
    }

    /// The elements in column-major order.
    #[must_use]
    pub const fn to_array(self) -> [Scalar; 9] {
        let (x, y, z) = (self.x_axis, self.y_axis, self.z_axis);
        [x.x, x.y, x.z, y.x, y.y, y.z, z.x, z.y, z.z]
    }

    /// The matrix that scales each axis by the matching component.
    #[must_use]
    pub const fn from_diagonal(diagonal: Vec3) -> Self {
        let zero = Scalar::ZERO;
        Self::from_cols(
            Vec3::new(diagonal.x, zero, zero),
            Vec3::new(zero, diagonal.y, zero),
            Vec3::new(zero, zero, diagonal.z),
        )
    }

    /// The rotation matrix of a unit quaternion.
    #[must_use]
    pub fn from_quat(rotation: Quat) -> Self {
        let Quat { x, y, z, w } = rotation;
        let (x2, y2, z2) = (x + x, y + y, z + z);
        let (xx, xy, xz) = (x * x2, x * y2, x * z2);
        let (yy, yz, zz) = (y * y2, y * z2, z * z2);
        let (wx, wy, wz) = (w * x2, w * y2, w * z2);
        let one = Scalar::ONE;
        Self::from_cols(
            Vec3::new(one - (yy + zz), xy + wz, xz - wy),
            Vec3::new(xy - wz, one - (xx + zz), yz + wx),
            Vec3::new(xz + wy, yz - wx, one - (xx + yy)),
        )
    }

    /// Rows and columns exchanged.
    #[must_use]
    pub const fn transpose(self) -> Self {
        let (x, y, z) = (self.x_axis, self.y_axis, self.z_axis);
        Self::from_cols(
            Vec3::new(x.x, y.x, z.x),
            Vec3::new(x.y, y.y, z.y),
            Vec3::new(x.z, y.z, z.z),
        )
    }

    /// The determinant, `x_axis · (y_axis × z_axis)`.
    #[must_use]
    pub fn determinant(self) -> Scalar {
        self.x_axis.dot(self.y_axis.cross(self.z_axis))
    }

    /// The inverse, or `None` when the [determinant](Self::determinant) is zero or not finite.
    ///
    /// It is the transposed cofactor matrix with every element divided by the determinant.
    #[must_use]
    pub fn inverse(self) -> Option<Self> {
        let cofactor_x = self.y_axis.cross(self.z_axis);
        let cofactor_y = self.z_axis.cross(self.x_axis);
        let cofactor_z = self.x_axis.cross(self.y_axis);
        let determinant = self.x_axis.dot(cofactor_x);
        if determinant == Scalar::ZERO || !determinant.is_finite() {
            return None;
        }
        let cofactors = Self::from_cols(
            cofactor_x / determinant,
            cofactor_y / determinant,
            cofactor_z / determinant,
        );
        Some(cofactors.transpose())
    }
}

/// `(x_axis * v.x + y_axis * v.y) + z_axis * v.z`.
impl Mul<Vec3> for Mat3 {
    type Output = Vec3;
    fn mul(self, rhs: Vec3) -> Vec3 {
        (self.x_axis * rhs.x + self.y_axis * rhs.y) + self.z_axis * rhs.z
    }
}

/// The matrix product: each column of the result is `self` times the matching column of
/// `rhs`.
impl Mul for Mat3 {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        Self::from_cols(self * rhs.x_axis, self * rhs.y_axis, self * rhs.z_axis)
    }
}

impl Mul<Scalar> for Mat3 {
    type Output = Self;
    fn mul(self, rhs: Scalar) -> Self {
        Self::from_cols(self.x_axis * rhs, self.y_axis * rhs, self.z_axis * rhs)
    }
}

impl Add for Mat3 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::from_cols(
            self.x_axis + rhs.x_axis,
            self.y_axis + rhs.y_axis,
            self.z_axis + rhs.z_axis,
        )
    }
}

impl Sub for Mat3 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::from_cols(
            self.x_axis - rhs.x_axis,
            self.y_axis - rhs.y_axis,
            self.z_axis - rhs.z_axis,
        )
    }
}

/// A 4×4 matrix stored as four columns. `m * v` is `x_axis * v.x + y_axis * v.y +
/// z_axis * v.z + w_axis * v.w`, and `a * b` applies `b` first. An affine transform keeps its
/// translation in `w_axis`.
///
/// Layout: `repr(C)`, the columns in order, sixteen scalars, no padding.
#[derive(Clone, Copy, PartialEq, Debug, Default, Pod, Zeroable)]
#[repr(C)]
pub struct Mat4 {
    /// The first column.
    pub x_axis: Vec4,
    /// The second column.
    pub y_axis: Vec4,
    /// The third column.
    pub z_axis: Vec4,
    /// The fourth column, the translation of an affine transform.
    pub w_axis: Vec4,
}

impl Mat4 {
    /// All elements zero.
    pub const ZERO: Self = Self::from_cols(Vec4::ZERO, Vec4::ZERO, Vec4::ZERO, Vec4::ZERO);
    /// The identity.
    pub const IDENTITY: Self = Self::from_cols(
        Vec3::X.extend(Scalar::ZERO),
        Vec3::Y.extend(Scalar::ZERO),
        Vec3::Z.extend(Scalar::ZERO),
        Vec3::ZERO.extend(Scalar::ONE),
    );

    /// A matrix from its columns.
    #[must_use]
    pub const fn from_cols(x_axis: Vec4, y_axis: Vec4, z_axis: Vec4, w_axis: Vec4) -> Self {
        Self { x_axis, y_axis, z_axis, w_axis }
    }

    /// A matrix from its elements in column-major order.
    #[must_use]
    pub const fn from_array(m: [Scalar; 16]) -> Self {
        Self::from_cols(
            Vec4::new(m[0], m[1], m[2], m[3]),
            Vec4::new(m[4], m[5], m[6], m[7]),
            Vec4::new(m[8], m[9], m[10], m[11]),
            Vec4::new(m[12], m[13], m[14], m[15]),
        )
    }

    /// The elements in column-major order.
    #[must_use]
    pub const fn to_array(self) -> [Scalar; 16] {
        let (x, y, z, w) = (self.x_axis, self.y_axis, self.z_axis, self.w_axis);
        [x.x, x.y, x.z, x.w, y.x, y.y, y.z, y.w, z.x, z.y, z.z, z.w, w.x, w.y, w.z, w.w]
    }

    /// The affine transform that applies `linear` and then adds `translation`.
    #[must_use]
    pub const fn from_mat3_translation(linear: Mat3, translation: Vec3) -> Self {
        Self::from_cols(
            linear.x_axis.extend(Scalar::ZERO),
            linear.y_axis.extend(Scalar::ZERO),
            linear.z_axis.extend(Scalar::ZERO),
            translation.extend(Scalar::ONE),
        )
    }

    /// The affine transform that scales, then rotates by a unit quaternion, then translates.
    #[must_use]
    pub fn from_scale_rotation_translation(scale: Vec3, rotation: Quat, translation: Vec3) -> Self {
        let rotation = Mat3::from_quat(rotation);
        let linear = Mat3::from_cols(
            rotation.x_axis * scale.x,
            rotation.y_axis * scale.y,
            rotation.z_axis * scale.z,
        );
        Self::from_mat3_translation(linear, translation)
    }

    /// Rows and columns exchanged.
    #[must_use]
    pub const fn transpose(self) -> Self {
        let (x, y, z, w) = (self.x_axis, self.y_axis, self.z_axis, self.w_axis);
        Self::from_cols(
            Vec4::new(x.x, y.x, z.x, w.x),
            Vec4::new(x.y, y.y, z.y, w.y),
            Vec4::new(x.z, y.z, z.z, w.z),
            Vec4::new(x.w, y.w, z.w, w.w),
        )
    }

    /// The determinant, by cofactor expansion along the first column over the 2×2 minors of
    /// the last two columns.
    #[must_use]
    pub fn determinant(self) -> Scalar {
        let Vec4 { x: m00, y: m01, z: m02, w: m03 } = self.x_axis;
        let Vec4 { x: m10, y: m11, z: m12, w: m13 } = self.y_axis;
        let Vec4 { x: m20, y: m21, z: m22, w: m23 } = self.z_axis;
        let Vec4 { x: m30, y: m31, z: m32, w: m33 } = self.w_axis;

        let a2323 = m22 * m33 - m23 * m32;
        let a1323 = m21 * m33 - m23 * m31;
        let a1223 = m21 * m32 - m22 * m31;
        let a0323 = m20 * m33 - m23 * m30;
        let a0223 = m20 * m32 - m22 * m30;
        let a0123 = m20 * m31 - m21 * m30;

        let c0 = (m11 * a2323 - m12 * a1323) + m13 * a1223;
        let c1 = (m10 * a2323 - m12 * a0323) + m13 * a0223;
        let c2 = (m10 * a1323 - m11 * a0323) + m13 * a0123;
        let c3 = (m10 * a1223 - m11 * a0223) + m12 * a0123;
        ((m00 * c0 - m01 * c1) + m02 * c2) - m03 * c3
    }

    /// The inverse, or `None` when the [determinant](Self::determinant) is zero or not finite.
    ///
    /// It is the adjugate with every element divided by the determinant.
    #[must_use]
    pub fn inverse(self) -> Option<Self> {
        let determinant = self.determinant();
        if determinant == Scalar::ZERO || !determinant.is_finite() {
            return None;
        }

        let Vec4 { x: m00, y: m01, z: m02, w: m03 } = self.x_axis;
        let Vec4 { x: m10, y: m11, z: m12, w: m13 } = self.y_axis;
        let Vec4 { x: m20, y: m21, z: m22, w: m23 } = self.z_axis;
        let Vec4 { x: m30, y: m31, z: m32, w: m33 } = self.w_axis;

        let coef00 = m22 * m33 - m32 * m23;
        let coef02 = m12 * m33 - m32 * m13;
        let coef03 = m12 * m23 - m22 * m13;

        let coef04 = m21 * m33 - m31 * m23;
        let coef06 = m11 * m33 - m31 * m13;
        let coef07 = m11 * m23 - m21 * m13;

        let coef08 = m21 * m32 - m31 * m22;
        let coef10 = m11 * m32 - m31 * m12;
        let coef11 = m11 * m22 - m21 * m12;

        let coef12 = m20 * m33 - m30 * m23;
        let coef14 = m10 * m33 - m30 * m13;
        let coef15 = m10 * m23 - m20 * m13;

        let coef16 = m20 * m32 - m30 * m22;
        let coef18 = m10 * m32 - m30 * m12;
        let coef19 = m10 * m22 - m20 * m12;

        let coef20 = m20 * m31 - m30 * m21;
        let coef22 = m10 * m31 - m30 * m11;
        let coef23 = m10 * m21 - m20 * m11;

        let fac0 = Vec4::new(coef00, coef00, coef02, coef03);
        let fac1 = Vec4::new(coef04, coef04, coef06, coef07);
        let fac2 = Vec4::new(coef08, coef08, coef10, coef11);
        let fac3 = Vec4::new(coef12, coef12, coef14, coef15);
        let fac4 = Vec4::new(coef16, coef16, coef18, coef19);
        let fac5 = Vec4::new(coef20, coef20, coef22, coef23);

        let vec0 = Vec4::new(m10, m00, m00, m00);
        let vec1 = Vec4::new(m11, m01, m01, m01);
        let vec2 = Vec4::new(m12, m02, m02, m02);
        let vec3 = Vec4::new(m13, m03, m03, m03);

        let adj0 = (vec1 * fac0 - vec2 * fac1) + vec3 * fac2;
        let adj1 = (vec0 * fac0 - vec2 * fac3) + vec3 * fac4;
        let adj2 = (vec0 * fac1 - vec1 * fac3) + vec3 * fac5;
        let adj3 = (vec0 * fac2 - vec1 * fac4) + vec2 * fac5;

        // The cofactor signs alternate; multiplying by ±1 is exact.
        let (p, n) = (Scalar::ONE, Scalar::NEG_ONE);
        let sign_even = Vec4::new(p, n, p, n);
        let sign_odd = Vec4::new(n, p, n, p);

        Some(Self::from_cols(
            adj0 * sign_even / determinant,
            adj1 * sign_odd / determinant,
            adj2 * sign_even / determinant,
            adj3 * sign_odd / determinant,
        ))
    }

    /// The image of a point under an affine transform: `((x_axis * p.x + y_axis * p.y) +
    /// z_axis * p.z) + w_axis`, without a perspective divide.
    #[must_use]
    pub fn transform_point3(self, point: Vec3) -> Vec3 {
        let sum =
            ((self.x_axis * point.x + self.y_axis * point.y) + self.z_axis * point.z) + self.w_axis;
        sum.truncate()
    }

    /// The image of a direction under an affine transform, ignoring translation:
    /// `(x_axis * v.x + y_axis * v.y) + z_axis * v.z`.
    #[must_use]
    pub fn transform_vector3(self, vector: Vec3) -> Vec3 {
        let sum = (self.x_axis * vector.x + self.y_axis * vector.y) + self.z_axis * vector.z;
        sum.truncate()
    }
}

/// `((x_axis * v.x + y_axis * v.y) + z_axis * v.z) + w_axis * v.w`.
impl Mul<Vec4> for Mat4 {
    type Output = Vec4;
    fn mul(self, rhs: Vec4) -> Vec4 {
        ((self.x_axis * rhs.x + self.y_axis * rhs.y) + self.z_axis * rhs.z) + self.w_axis * rhs.w
    }
}

/// The matrix product: each column of the result is `self` times the matching column of
/// `rhs`.
impl Mul for Mat4 {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        Self::from_cols(self * rhs.x_axis, self * rhs.y_axis, self * rhs.z_axis, self * rhs.w_axis)
    }
}

impl Mul<Scalar> for Mat4 {
    type Output = Self;
    fn mul(self, rhs: Scalar) -> Self {
        Self::from_cols(self.x_axis * rhs, self.y_axis * rhs, self.z_axis * rhs, self.w_axis * rhs)
    }
}
