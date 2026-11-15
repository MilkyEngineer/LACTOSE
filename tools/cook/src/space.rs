//! The cooker's own transform arithmetic, and the one change of frame from an authored source
//! to the engine's world frame (architecture section 5.3).
//!
//! Poses are composed in `f64` and rounded to the build's `Scalar` once, when written. Every
//! operation here is an IEEE addition, subtraction, multiplication, division or square root,
//! each correctly rounded, and Rust contracts none of them into a fused multiply-add, so a
//! source gives the same bits on every host.

use lactose_anim::BonePose;
use lactose_math::{Quat, Scalar, Vec3};

/// A rigid transform: a translation and a rotation quaternion `[x, y, z, w]`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Pose {
    pub(crate) translation: [f64; 3],
    pub(crate) rotation: [f64; 4],
}

impl Pose {
    pub(crate) const IDENTITY: Self =
        Self { translation: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0] };

    /// `child` placed in the space of `self`, as `BonePose::compose` defines it: the
    /// translation `self.rotation * child.translation + self.translation`, the rotation
    /// `self.rotation * child.rotation`.
    pub(crate) fn compose(self, child: Self) -> Self {
        let [x, y, z] = rotate(self.rotation, child.translation);
        let [tx, ty, tz] = self.translation;
        Self {
            translation: [tx + x, ty + y, tz + z],
            rotation: multiply(self.rotation, child.rotation),
        }
    }

    /// The pose at the build's scalar width, its rotation normalised: a rotation composed
    /// through several nodes drifts from unit length, and `Skeleton::validate` holds every
    /// written rotation to `UNIT_TOLERANCE`.
    pub(crate) fn to_bone_pose(self) -> BonePose {
        let [x, y, z, w] = self.rotation;
        let length = (x * x + y * y + z * z + w * w).sqrt();
        let [tx, ty, tz] = self.translation.map(Scalar::from_f64);
        let [x, y, z, w] = [x, y, z, w].map(|c| Scalar::from_f64(c / length));
        BonePose::new(Vec3::new(tx, ty, tz), Quat::new(x, y, z, w))
    }
}

/// A pose of a source's own frame in the engine's: turned half a turn about `+Y`, so an asset
/// authored facing `+Z`, as glTF's are, faces `-Z` at yaw zero (architecture section 5.3).
///
/// It applies to what is expressed in the asset's root space and to nothing parent-local: of
/// a skeleton, the root bone's rest pose alone; of a clip, the root bone's samples and the
/// root track. Turning the root turns everything under it. The turn is exact: it negates and
/// exchanges components and rounds nothing.
pub(crate) fn to_engine_frame(pose: Pose) -> Pose {
    let [tx, ty, tz] = pose.translation;
    let [x, y, z, w] = pose.rotation;
    // `(0, 1, 0, 0) * (x, y, z, w)` and the rotation of the translation by it.
    Pose { translation: [-tx, ty, -tz], rotation: [z, w, -x, -y] }
}

/// The Hamilton product `a * b`: `b` applied first, then `a`.
fn multiply(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

/// `v` turned by the rotation `q`: `v + w t + u x t` with `t = 2 u x v`, `u` the vector part.
fn rotate(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    let [x, y, z, w] = q;
    let u = [x, y, z];
    let t = cross(u, v).map(|c| 2.0 * c);
    let c = cross(u, t);
    [v[0] + w * t[0] + c[0], v[1] + w * t[1] + c[1], v[2] + w * t[2] + c[2]]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

#[cfg(test)]
mod tests {
    use super::*;

    const HALF_TURN_Y: [f64; 4] = [0.0, 1.0, 0.0, 0.0];

    #[test]
    fn the_engine_frame_is_a_half_turn_about_y_premultiplied() {
        let pose = Pose { translation: [1.0, 2.0, 3.0], rotation: [0.5, 0.5, 0.5, 0.5] };
        let turned = Pose { translation: [0.0; 3], rotation: HALF_TURN_Y }.compose(pose);
        assert_eq!(to_engine_frame(pose), turned);
        assert_eq!(to_engine_frame(pose).translation, [-1.0, 2.0, -3.0]);
        // Twice is the identity.
        assert_eq!(to_engine_frame(to_engine_frame(pose)).rotation.map(f64::abs), pose.rotation);
    }

    #[test]
    fn composition_places_the_child_in_the_parent() {
        let parent = Pose { translation: [0.0, 1.0, 0.0], rotation: HALF_TURN_Y };
        let child = Pose { translation: [0.0, 0.0, 1.0], rotation: [0.0, 0.0, 0.0, 1.0] };
        let placed = parent.compose(child);
        assert_eq!(placed.translation, [0.0, 1.0, -1.0]);
        assert_eq!(placed.rotation, HALF_TURN_Y);
        assert_eq!(Pose::IDENTITY.compose(child), child);
    }
}
