//! [`Vec2`], [`Vec3`] and [`Vec4`].
//!
//! Every sum of more than two terms is written out left to right, because the order of
//! floating-point additions is part of the result. Nothing here uses fused multiply-add.

use core::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

use bytemuck::{Pod, Zeroable};

use crate::Scalar;

/// A two-component vector.
///
/// Layout: `repr(C)`, fields `x`, `y` in order, no padding.
#[derive(Clone, Copy, PartialEq, Debug, Default, Pod, Zeroable)]
#[repr(C)]
pub struct Vec2 {
    /// The x component.
    pub x: Scalar,
    /// The y component.
    pub y: Scalar,
}

/// A three-component vector.
///
/// Layout: `repr(C)`, fields `x`, `y`, `z` in order, no padding.
#[derive(Clone, Copy, PartialEq, Debug, Default, Pod, Zeroable)]
#[repr(C)]
pub struct Vec3 {
    /// The x component.
    pub x: Scalar,
    /// The y component.
    pub y: Scalar,
    /// The z component.
    pub z: Scalar,
}

/// A four-component vector.
///
/// Layout: `repr(C)`, fields `x`, `y`, `z`, `w` in order, no padding.
#[derive(Clone, Copy, PartialEq, Debug, Default, Pod, Zeroable)]
#[repr(C)]
pub struct Vec4 {
    /// The x component.
    pub x: Scalar,
    /// The y component.
    pub y: Scalar,
    /// The z component.
    pub z: Scalar,
    /// The w component.
    pub w: Scalar,
}

/// The parts of a vector type that are componentwise, where no evaluation order arises.
macro_rules! componentwise {
    ($ty:ident, $count:literal, $($field:ident),+) => {
        impl $ty {
            /// All components zero.
            pub const ZERO: Self = Self::splat(Scalar::ZERO);
            /// All components one.
            pub const ONE: Self = Self::splat(Scalar::ONE);

            /// A vector from its components.
            #[must_use]
            pub const fn new($($field: Scalar),+) -> Self {
                Self { $($field),+ }
            }

            /// A vector with every component equal to `value`.
            #[must_use]
            pub const fn splat(value: Scalar) -> Self {
                Self { $($field: value),+ }
            }

            /// A vector from its components in declaration order.
            #[must_use]
            pub const fn from_array(components: [Scalar; $count]) -> Self {
                let [$($field),+] = components;
                Self { $($field),+ }
            }

            /// The components in declaration order.
            #[must_use]
            pub const fn to_array(self) -> [Scalar; $count] {
                [$(self.$field),+]
            }

            /// The squared length, `self.dot(self)`.
            #[must_use]
            pub fn length_squared(self) -> Scalar {
                self.dot(self)
            }

            /// The length, `sqrt(length_squared)`. The squares overflow to infinity for
            /// components beyond about `1e154`.
            #[must_use]
            pub fn length(self) -> Scalar {
                self.length_squared().sqrt()
            }

            /// The distance to `other`, `(self - other).length()`.
            #[must_use]
            pub fn distance(self, other: Self) -> Scalar {
                (self - other).length()
            }

            /// `self / self.length()`, which has unit length only when the length is positive
            /// and finite. A zero length, including one whose squares underflowed, gives NaN
            /// or infinite components. An infinite length, including one whose squares
            /// overflowed (components beyond about `1e154`), gives zeros for the finite
            /// components and NaN for infinite ones. Use
            /// [`try_normalize`](Self::try_normalize) where either can happen.
            #[must_use]
            pub fn normalize(self) -> Self {
                self / self.length()
            }

            /// `self / self.length()`, or `None` when the computed length is zero or not
            /// finite. That includes finite non-zero vectors whose squared components
            /// underflow to zero or overflow to infinity.
            #[must_use]
            pub fn try_normalize(self) -> Option<Self> {
                let length = self.length();
                if length > Scalar::ZERO && length.is_finite() {
                    Some(self / length)
                } else {
                    None
                }
            }

            /// `self + (end - self) * t` per component. Not clamped.
            #[must_use]
            pub fn lerp(self, end: Self, t: Scalar) -> Self {
                self + (end - self) * t
            }

            /// The componentwise [`Scalar::min`].
            #[must_use]
            pub const fn min(self, other: Self) -> Self {
                Self { $($field: self.$field.min(other.$field)),+ }
            }

            /// The componentwise [`Scalar::max`].
            #[must_use]
            pub const fn max(self, other: Self) -> Self {
                Self { $($field: self.$field.max(other.$field)),+ }
            }

            /// The componentwise [`Scalar::clamp`].
            #[must_use]
            pub const fn clamp(self, lo: Self, hi: Self) -> Self {
                Self { $($field: self.$field.clamp(lo.$field, hi.$field)),+ }
            }

            /// The componentwise absolute value.
            #[must_use]
            pub const fn abs(self) -> Self {
                Self { $($field: self.$field.abs()),+ }
            }

            /// Whether every component is finite.
            #[must_use]
            pub const fn is_finite(self) -> bool {
                $(self.$field.is_finite())&&+
            }
        }

        impl Add for $ty {
            type Output = Self;
            fn add(self, rhs: Self) -> Self {
                Self { $($field: self.$field + rhs.$field),+ }
            }
        }

        impl Sub for $ty {
            type Output = Self;
            fn sub(self, rhs: Self) -> Self {
                Self { $($field: self.$field - rhs.$field),+ }
            }
        }

        impl Neg for $ty {
            type Output = Self;
            fn neg(self) -> Self {
                Self { $($field: -self.$field),+ }
            }
        }

        /// Componentwise product.
        impl Mul for $ty {
            type Output = Self;
            fn mul(self, rhs: Self) -> Self {
                Self { $($field: self.$field * rhs.$field),+ }
            }
        }

        impl Mul<Scalar> for $ty {
            type Output = Self;
            fn mul(self, rhs: Scalar) -> Self {
                Self { $($field: self.$field * rhs),+ }
            }
        }

        impl Mul<$ty> for Scalar {
            type Output = $ty;
            fn mul(self, rhs: $ty) -> $ty {
                $ty { $($field: self * rhs.$field),+ }
            }
        }

        /// Each component divided by `rhs`; a true division per component, never a
        /// multiplication by the reciprocal.
        impl Div<Scalar> for $ty {
            type Output = Self;
            fn div(self, rhs: Scalar) -> Self {
                Self { $($field: self.$field / rhs),+ }
            }
        }

        impl AddAssign for $ty {
            fn add_assign(&mut self, rhs: Self) {
                *self = *self + rhs;
            }
        }

        impl SubAssign for $ty {
            fn sub_assign(&mut self, rhs: Self) {
                *self = *self - rhs;
            }
        }

        impl MulAssign<Scalar> for $ty {
            fn mul_assign(&mut self, rhs: Scalar) {
                *self = *self * rhs;
            }
        }

        impl DivAssign<Scalar> for $ty {
            fn div_assign(&mut self, rhs: Scalar) {
                *self = *self / rhs;
            }
        }
    };
}

componentwise!(Vec2, 2, x, y);
componentwise!(Vec3, 3, x, y, z);
componentwise!(Vec4, 4, x, y, z, w);

impl Vec2 {
    /// The unit vector along x.
    pub const X: Self = Self::new(Scalar::ONE, Scalar::ZERO);
    /// The unit vector along y.
    pub const Y: Self = Self::new(Scalar::ZERO, Scalar::ONE);

    /// The dot product, `x*x' + y*y'`.
    #[must_use]
    pub fn dot(self, other: Self) -> Scalar {
        self.x * other.x + self.y * other.y
    }

    /// The z component of the three-dimensional cross product, `x*y' - y*x'`.
    #[must_use]
    pub fn perp_dot(self, other: Self) -> Scalar {
        self.x * other.y - self.y * other.x
    }

    /// The vector with a third component appended.
    #[must_use]
    pub const fn extend(self, z: Scalar) -> Vec3 {
        Vec3::new(self.x, self.y, z)
    }
}

impl Vec3 {
    /// The unit vector along x.
    pub const X: Self = Self::new(Scalar::ONE, Scalar::ZERO, Scalar::ZERO);
    /// The unit vector along y.
    pub const Y: Self = Self::new(Scalar::ZERO, Scalar::ONE, Scalar::ZERO);
    /// The unit vector along z.
    pub const Z: Self = Self::new(Scalar::ZERO, Scalar::ZERO, Scalar::ONE);

    /// The dot product, `(x*x' + y*y') + z*z'`.
    #[must_use]
    pub fn dot(self, other: Self) -> Scalar {
        (self.x * other.x + self.y * other.y) + self.z * other.z
    }

    /// The cross product, `(y*z' - z*y', z*x' - x*z', x*y' - y*x')`.
    #[must_use]
    pub fn cross(self, other: Self) -> Self {
        Self {
            x: self.y * other.z - self.z * other.y,
            y: self.z * other.x - self.x * other.z,
            z: self.x * other.y - self.y * other.x,
        }
    }

    /// The vector with a fourth component appended.
    #[must_use]
    pub const fn extend(self, w: Scalar) -> Vec4 {
        Vec4::new(self.x, self.y, self.z, w)
    }

    /// The first two components.
    #[must_use]
    pub const fn truncate(self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }
}

impl Vec4 {
    /// The dot product, `((x*x' + y*y') + z*z') + w*w'`.
    #[must_use]
    pub fn dot(self, other: Self) -> Scalar {
        ((self.x * other.x + self.y * other.y) + self.z * other.z) + self.w * other.w
    }

    /// The first three components.
    #[must_use]
    pub const fn truncate(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }
}
