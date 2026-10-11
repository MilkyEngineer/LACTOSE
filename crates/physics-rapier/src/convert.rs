//! Between the `physics-api` values and Rapier's: keys and their `user_data` code, vectors,
//! rotations and poses, shapes, and the validation every value passes before it reaches the
//! solver.
//!
//! Every conversion is exact in both directions while `Scalar` is 64-bit: `to_f64` and
//! `from_f64` are identities on every finite value (`from_f64` canonicalises a NaN, which
//! validation keeps out of the solver).

use lactose_core::{NetId, PlayerId, PredictionKey, StableKey, Tick};
use lactose_math::{Quat, Scalar, Vec3};
use lactose_physics_api::{
    Collider, Joint, JointKind, RigidBody, Shape, ShapeKind, Transform, Velocity, locked_axes,
};
use rapier3d_f64::geometry::SharedShape;
use rapier3d_f64::math::{Pose, Rotation, Vector};

// ---- Keys ----

const TAG_SHIFT: u32 = 96;
const TAG_NET: u128 = 1;
const TAG_PREDICTED: u128 = 2;
const TAG_PLACEHOLDER: u128 = 3;

/// The key as the `u128` Rapier carries in `user_data`, and the snapshot in its maps. The
/// codes ascend as the keys do (network keys, then prediction keys by player, input tick and
/// ordinal, then placeholders), and none is zero, Rapier's default `user_data`.
pub(crate) const fn key_code(key: StableKey) -> u128 {
    match key {
        StableKey::Net(id) => TAG_NET << TAG_SHIFT | id.to_bits() as u128,
        StableKey::Predicted(key) => {
            TAG_PREDICTED << TAG_SHIFT
                | (key.player.0 as u128) << 64
                | (key.input_tick.0 as u128) << 32
                | key.ordinal as u128
        }
        StableKey::Placeholder(index) => TAG_PLACEHOLDER << TAG_SHIFT | index as u128,
    }
}

/// The key a [`key_code`] encodes, or `None` for a code no key has.
pub(crate) fn key_from_code(code: u128) -> Option<StableKey> {
    let word = |shift: u32| (code >> shift) as u32;
    match code >> TAG_SHIFT {
        TAG_NET => {
            if code >> 64 & 0xffff_ffff != 0 {
                return None;
            }
            NetId::from_bits(code as u64).map(StableKey::Net)
        }
        TAG_PREDICTED => Some(StableKey::Predicted(PredictionKey {
            player: PlayerId(word(64)),
            input_tick: Tick(word(32)),
            ordinal: word(0),
        })),
        TAG_PLACEHOLDER if code >> 32 & 0xffff_ffff_ffff_ffff == 0 => {
            Some(StableKey::Placeholder(word(0)))
        }
        _ => None,
    }
}

// ---- Vectors, rotations, poses ----

pub(crate) fn vector(v: Vec3) -> Vector {
    Vector::new(v.x.to_f64(), v.y.to_f64(), v.z.to_f64())
}

pub(crate) fn vec3(v: Vector) -> Vec3 {
    Vec3::new(Scalar::from_f64(v.x), Scalar::from_f64(v.y), Scalar::from_f64(v.z))
}

pub(crate) fn rotation(q: Quat) -> Rotation {
    Rotation::from_xyzw(q.x.to_f64(), q.y.to_f64(), q.z.to_f64(), q.w.to_f64())
}

pub(crate) fn quat(r: Rotation) -> Quat {
    Quat::new(
        Scalar::from_f64(r.x),
        Scalar::from_f64(r.y),
        Scalar::from_f64(r.z),
        Scalar::from_f64(r.w),
    )
}

pub(crate) fn pose(transform: &Transform) -> Pose {
    Pose::from_parts(vector(transform.translation), rotation(transform.rotation))
}

pub(crate) fn transform(pose: &Pose) -> Transform {
    Transform::new(vec3(pose.translation), quat(pose.rotation))
}

/// Whether two vectors hold the same bits, so a write of `-0` over `+0` is a change.
pub(crate) fn same_vector(a: Vector, b: Vector) -> bool {
    same_scalars(&[a.x, a.y, a.z], &[b.x, b.y, b.z])
}

/// Whether two poses hold the same bits.
pub(crate) fn same_pose(a: &Pose, b: &Pose) -> bool {
    same_vector(a.translation, b.translation)
        && same_scalars(
            &[a.rotation.x, a.rotation.y, a.rotation.z, a.rotation.w],
            &[b.rotation.x, b.rotation.y, b.rotation.z, b.rotation.w],
        )
}

fn same_scalars(a: &[f64], b: &[f64]) -> bool {
    a.iter().zip(b).all(|(&a, &b)| {
        Scalar::from_f64(a).to_canonical_bits() == Scalar::from_f64(b).to_canonical_bits()
    })
}

/// Whether two plain-data values hold the same bytes: the comparison every `set_*` makes
/// before it touches the solver.
pub(crate) fn same<T: bytemuck::NoUninit>(a: &T, b: &T) -> bool {
    bytemuck::bytes_of(a) == bytemuck::bytes_of(b)
}

// ---- Shapes ----

/// The solver's shape for a valid `shape`. A capsule is built along y from its half-height,
/// so [`shape_of`] reads back the same value.
pub(crate) fn shared_shape(shape: &Shape) -> SharedShape {
    match shape.kind {
        ShapeKind::Sphere => SharedShape::ball(shape.radius.to_f64()),
        ShapeKind::Capsule => {
            SharedShape::capsule_y(shape.half_extents.y.to_f64(), shape.radius.to_f64())
        }
        ShapeKind::Cuboid => {
            let half = shape.half_extents;
            SharedShape::cuboid(half.x.to_f64(), half.y.to_f64(), half.z.to_f64())
        }
    }
}

/// The value a shape [`shared_shape`] built was built from, exactly; `None` for any other
/// shape.
pub(crate) fn shape_of(shape: &SharedShape) -> Option<Shape> {
    if let Some(ball) = shape.as_ball() {
        return Some(Shape::sphere(Scalar::from_f64(ball.radius)));
    }
    if let Some(capsule) = shape.as_capsule() {
        let built = SharedShape::capsule_y(capsule.segment.b.y, capsule.radius);
        let same_segment = built.as_capsule().is_some_and(|rebuilt| {
            same_vector(rebuilt.segment.a, capsule.segment.a)
                && same_vector(rebuilt.segment.b, capsule.segment.b)
        });
        return same_segment.then(|| {
            Shape::capsule(Scalar::from_f64(capsule.segment.b.y), Scalar::from_f64(capsule.radius))
        });
    }
    if let Some(cuboid) = shape.as_cuboid() {
        return Some(Shape::cuboid(vec3(cuboid.half_extents)));
    }
    None
}

// ---- Validation ----

/// What `Transform` must be: a finite translation and a unit quaternion, to within the
/// tolerance the test backend uses.
pub(crate) fn valid_transform(transform: &Transform) -> bool {
    let unit = (transform.rotation.length_squared() - Scalar::ONE).abs() < Scalar::from_f64(1e-6);
    transform.translation.is_finite() && transform.rotation.is_finite() && unit
}

pub(crate) fn valid_velocity(velocity: &Velocity) -> bool {
    velocity.linear.is_finite() && velocity.angular.is_finite()
}

fn non_negative(value: Scalar) -> bool {
    value.is_finite() && value >= Scalar::ZERO
}

fn positive(value: Scalar) -> bool {
    value.is_finite() && value > Scalar::ZERO
}

pub(crate) fn valid_shape(shape: &Shape) -> bool {
    match shape.kind {
        ShapeKind::Sphere => positive(shape.radius),
        ShapeKind::Capsule => positive(shape.radius) && non_negative(shape.half_extents.y),
        ShapeKind::Cuboid => shape.half_extents.to_array().into_iter().all(positive),
    }
}

pub(crate) fn valid_body(body: &RigidBody) -> bool {
    non_negative(body.mass)
        && non_negative(body.linear_damping)
        && non_negative(body.angular_damping)
        && body.gravity_scale.is_finite()
        && body.locked_axes & !(locked_axes::TRANSLATION | locked_axes::ROTATION) == 0
}

pub(crate) fn valid_collider(collider: &Collider) -> bool {
    let unit = (collider.rotation.length_squared() - Scalar::ONE).abs() < Scalar::from_f64(1e-6);
    valid_shape(&collider.shape)
        && collider.offset.is_finite()
        && collider.rotation.is_finite()
        && unit
        && non_negative(collider.friction)
        && non_negative(collider.restitution)
        && non_negative(collider.density)
        && non_negative(collider.impact_threshold)
}

/// Finite anchors, a non-zero finite axis and finite ordered limits when limited.
pub(crate) fn valid_joint(joint: &Joint) -> bool {
    let limits = !joint.limited
        || (joint.limit_min.is_finite()
            && joint.limit_max.is_finite()
            && joint.limit_min <= joint.limit_max);
    // A fixed or spherical joint never reads its axis, so a zero one is no error there.
    let axis = match joint.kind {
        JointKind::Fixed | JointKind::Spherical => true,
        JointKind::Revolute | JointKind::Prismatic => joint.axis.try_normalize().is_some(),
    };
    joint.anchor_a.is_finite()
        && joint.anchor_b.is_finite()
        && joint.axis.is_finite()
        && axis
        && limits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_codes_ascend_with_the_keys_and_round_trip() {
        let keys = [
            StableKey::Net(NetId::cooked(1)),
            StableKey::Net(NetId::cooked(u64::MAX >> 1)),
            StableKey::Predicted(PredictionKey {
                player: PlayerId(1),
                input_tick: Tick(9),
                ordinal: 3,
            }),
            StableKey::Predicted(PredictionKey {
                player: PlayerId(1),
                input_tick: Tick(9),
                ordinal: u32::MAX,
            }),
            StableKey::Predicted(PredictionKey {
                player: PlayerId(2),
                input_tick: Tick(0),
                ordinal: 0,
            }),
            StableKey::Placeholder(0),
            StableKey::Placeholder(u32::MAX),
        ];
        for pair in keys.windows(2) {
            assert!(pair[0] < pair[1]);
            assert!(key_code(pair[0]) < key_code(pair[1]));
        }
        for key in keys {
            assert_ne!(key_code(key), 0);
            assert_eq!(key_from_code(key_code(key)), Some(key));
        }
        assert_eq!(key_from_code(0), None);
        assert_eq!(key_from_code(TAG_NET << TAG_SHIFT), None);
        assert_eq!(key_from_code(4 << TAG_SHIFT), None);
    }

    #[test]
    fn shapes_read_back_exactly() {
        let shapes = [
            Shape::sphere(Scalar::from_f64(0.3)),
            Shape::capsule(Scalar::from_f64(0.7), Scalar::from_f64(0.1)),
            Shape::capsule(Scalar::ZERO, Scalar::ONE),
            Shape::cuboid(Vec3::new(Scalar::ONE, Scalar::from_f64(0.25), Scalar::from_i32(50))),
        ];
        for shape in shapes {
            assert!(valid_shape(&shape));
            let read = shape_of(&shared_shape(&shape)).expect("a shape this crate builds");
            assert!(same(&read, &shape), "{shape:?} read back as {read:?}");
        }
        let other = SharedShape::capsule_x(1.0, 0.5);
        assert_eq!(shape_of(&other), None);
    }

    #[test]
    fn poses_convert_exactly() {
        let at = Transform::new(
            Vec3::new(Scalar::from_f64(-0.0), Scalar::from_f64(1e-300), Scalar::from_f64(7.5)),
            Quat::from_rotation_y(Scalar::from_f64(0.3)),
        );
        let back = transform(&pose(&at));
        assert!(same(&back, &at));
    }
}
