//! [`Transform`]: translation, rotation and scale.

use bytemuck::{Pod, Zeroable};

use crate::{Mat4, Quat, Scalar, Vec3};

/// A placement in space: scale, then rotation, then translation.
///
/// `rotation` must have unit length. The default value is all zeros, which is not a valid
/// transform; use [`Transform::IDENTITY`].
///
/// Layout: `repr(C)`, `translation` (three scalars), `rotation` (four), `scale` (three), ten
/// scalars in all, no padding.
#[derive(Clone, Copy, PartialEq, Debug, Default, Pod, Zeroable)]
#[repr(C)]
pub struct Transform {
    /// Where the local origin lands.
    pub translation: Vec3,
    /// The orientation, a unit quaternion.
    pub rotation: Quat,
    /// The scale along each local axis, applied before the rotation.
    pub scale: Vec3,
}

impl Transform {
    /// The transform that changes nothing.
    pub const IDENTITY: Self = Self::new(Vec3::ZERO, Quat::IDENTITY, Vec3::ONE);

    /// A transform from its parts.
    #[must_use]
    pub const fn new(translation: Vec3, rotation: Quat, scale: Vec3) -> Self {
        Self { translation, rotation, scale }
    }

    /// The unrotated, unscaled transform at `translation`.
    #[must_use]
    pub const fn at(translation: Vec3) -> Self {
        Self::new(translation, Quat::IDENTITY, Vec3::ONE)
    }

    /// A transform from `[translation.xyz, rotation.xyzw, scale.xyz]`.
    #[must_use]
    pub const fn from_array(s: [Scalar; 10]) -> Self {
        Self::new(
            Vec3::new(s[0], s[1], s[2]),
            Quat::new(s[3], s[4], s[5], s[6]),
            Vec3::new(s[7], s[8], s[9]),
        )
    }

    /// The scalars as `[translation.xyz, rotation.xyzw, scale.xyz]`.
    #[must_use]
    pub const fn to_array(self) -> [Scalar; 10] {
        let (t, r, s) = (self.translation, self.rotation, self.scale);
        [t.x, t.y, t.z, r.x, r.y, r.z, r.w, s.x, s.y, s.z]
    }

    /// The image of a local point: `rotation * (point * scale) + translation`.
    #[must_use]
    pub fn transform_point(self, point: Vec3) -> Vec3 {
        self.rotation * (point * self.scale) + self.translation
    }

    /// The image of a local direction, ignoring translation: `rotation * (vector * scale)`.
    #[must_use]
    pub fn transform_vector(self, vector: Vec3) -> Vec3 {
        self.rotation * (vector * self.scale)
    }

    /// The equivalent affine matrix.
    #[must_use]
    pub fn to_mat4(self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// Places `child` in the space of `self`, which must have a uniform scale (all three
    /// components equal; checked in debug builds).
    ///
    /// The translation is `self.transform_point(child.translation)`, the rotation is
    /// `self.rotation * child.rotation` and the scale is the componentwise product. Under a
    /// uniform parent scale that is the transform whose matrix is
    /// `self.to_mat4() * child.to_mat4()`.
    ///
    /// There is deliberately no general composition and no `*` operator: under a non-uniform
    /// parent scale the true composition has shear, which this type cannot hold, and a
    /// componentwise answer would be a silently wrong placement. A hierarchy with
    /// non-uniform scale composes through [`Mat4`]: multiply the [`to_mat4`](Self::to_mat4)
    /// matrices.
    #[must_use]
    pub fn compose_uniform(self, child: Self) -> Self {
        debug_assert!(
            self.scale.x == self.scale.y && self.scale.y == self.scale.z,
            "compose_uniform needs a uniform parent scale, got {:?}",
            self.scale
        );
        Self {
            translation: self.transform_point(child.translation),
            rotation: self.rotation * child.rotation,
            scale: self.scale * child.scale,
        }
    }
}
