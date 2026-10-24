//! What a value must be for a backend to accept it (architecture sections 8.2 and 16;
//! `README.md`, "Validity"): the one statement of the rules, as predicates every backend
//! applies and every crate that authors a body calls before the row exists, so that no copy
//! of the rules can drift from another (D135).
//!
//! Each `validate` reads the fields `README.md` lists for its value and nothing else, checks
//! them in declaration order, every scalar finite first and then the bounds, and names the
//! first field that breaks a rule. Every comparison is an IEEE comparison on `Scalar`s, so
//! the answer is the same on every host and at either scalar width.

use core::fmt;

use lactose_core::{Error, ErrorKind};
use lactose_math::{Quat, Scalar, Vec3};

use crate::backend::BodyDesc;
use crate::command::PhysicsCommand;
use crate::components::{
    Collider, Joint, JointKind, RigidBody, Shape, ShapeKind, Transform, Velocity, locked_axes,
};

/// How far the squared length of a rotation may be from one, exclusive: a quaternion `q`
/// is unit when `(q.length_squared() - 1).abs() < UNIT_TOLERANCE`. A normalised
/// quaternion is within a few units in the last place of one at either scalar width, and
/// what a crate composes down a chain of them it normalises before it writes a row
/// (`lactose-anim`'s bone rows); the tolerance is not a licence to skip that.
pub const UNIT_TOLERANCE: Scalar = Scalar::from_f64(1e-6);

/// The rule a field breaks: what the field must be and is not.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Rule {
    /// Every scalar of the field is neither infinite nor NaN.
    Finite,
    /// A unit quaternion: its squared length within [`UNIT_TOLERANCE`] of one.
    Unit,
    /// Zero or positive; `-0.0` counts as zero.
    NonNegative,
    /// Strictly positive.
    Positive,
    /// A direction: a finite vector whose computed length is positive and finite
    /// (`Vec3::try_normalize` succeeds), so the zero vector and one whose squares underflow
    /// or overflow are not.
    Direction,
    /// Only bits of [`locked_axes::TRANSLATION`] and [`locked_axes::ROTATION`] set.
    KnownBits,
    /// At most the upper limit: `limit_min <= limit_max`.
    Ordered,
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Rule::Finite => "finite",
            Rule::Unit => "a unit quaternion",
            Rule::NonNegative => "zero or positive",
            Rule::Positive => "positive",
            Rule::Direction => "a direction with a positive finite length",
            Rule::KnownBits => "within the known `locked_axes` bits",
            Rule::Ordered => "at most `limit_max`",
        })
    }
}

/// Why a value is refused: the first field, in the order `README.md` states, that breaks a
/// rule, and the rule. What a backend returns from a creating call (as an `InvalidData`
/// [`Error`] through `From`) and logs from a setting call or a dropped command, and what a
/// loader reports of an authored body. Two callers given one value report one `Invalid`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Invalid {
    /// The field as `Type::field`, with a component of a vector after a dot when the rule
    /// is on that component alone: `"Transform::rotation"`, `"Shape::half_extents.y"`,
    /// `"Joint::limit_min"`. The type is the one that holds the field, so a collider's
    /// shape reports as `Shape::..`, and a teleport's transform as `Transform::..`.
    pub field: &'static str,
    /// The rule it breaks.
    pub rule: Rule,
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}` is not {}", self.field, self.rule)
    }
}

impl std::error::Error for Invalid {}

impl From<Invalid> for Error {
    /// `InvalidData`, with the [`Display`](fmt::Display) text as the message.
    fn from(invalid: Invalid) -> Self {
        Error::new(ErrorKind::InvalidData, invalid.to_string())
    }
}

type Checked = Result<(), Invalid>;

const fn refuse(field: &'static str, rule: Rule) -> Checked {
    Err(Invalid { field, rule })
}

fn finite(field: &'static str, value: Scalar) -> Checked {
    if value.is_finite() { Ok(()) } else { refuse(field, Rule::Finite) }
}

fn finite_vec(field: &'static str, value: Vec3) -> Checked {
    if value.is_finite() { Ok(()) } else { refuse(field, Rule::Finite) }
}

fn finite_quat(field: &'static str, value: Quat) -> Checked {
    if value.is_finite() { Ok(()) } else { refuse(field, Rule::Finite) }
}

/// After [`finite_quat`]: the squared length within the tolerance of one.
fn unit(field: &'static str, value: Quat) -> Checked {
    if (value.length_squared() - Scalar::ONE).abs() < UNIT_TOLERANCE {
        Ok(())
    } else {
        refuse(field, Rule::Unit)
    }
}

/// After [`finite`].
fn non_negative(field: &'static str, value: Scalar) -> Checked {
    if value >= Scalar::ZERO { Ok(()) } else { refuse(field, Rule::NonNegative) }
}

/// After [`finite`].
fn positive(field: &'static str, value: Scalar) -> Checked {
    if value > Scalar::ZERO { Ok(()) } else { refuse(field, Rule::Positive) }
}

impl Transform {
    /// Whether every backend accepts the placement: `translation` finite; `rotation`
    /// finite and a unit quaternion (`README.md`, "Validity").
    pub fn validate(&self) -> Result<(), Invalid> {
        finite_vec("Transform::translation", self.translation)?;
        finite_quat("Transform::rotation", self.rotation)?;
        unit("Transform::rotation", self.rotation)
    }
}

impl Velocity {
    /// Whether every backend accepts the motion: `linear` and `angular` finite; no bound
    /// on their size.
    pub fn validate(&self) -> Result<(), Invalid> {
        finite_vec("Velocity::linear", self.linear)?;
        finite_vec("Velocity::angular", self.angular)
    }
}

impl RigidBody {
    /// Whether every backend accepts the body: the four scalars finite; `mass`,
    /// `linear_damping` and `angular_damping` zero or positive; `gravity_scale` of any
    /// sign; `locked_axes` within the known bits. `body_type` and `can_sleep` are not read:
    /// every value of theirs is accepted.
    pub fn validate(&self) -> Result<(), Invalid> {
        finite("RigidBody::mass", self.mass)?;
        finite("RigidBody::linear_damping", self.linear_damping)?;
        finite("RigidBody::angular_damping", self.angular_damping)?;
        finite("RigidBody::gravity_scale", self.gravity_scale)?;
        non_negative("RigidBody::mass", self.mass)?;
        non_negative("RigidBody::linear_damping", self.linear_damping)?;
        non_negative("RigidBody::angular_damping", self.angular_damping)?;
        if self.locked_axes & !(locked_axes::TRANSLATION | locked_axes::ROTATION) == 0 {
            Ok(())
        } else {
            refuse("RigidBody::locked_axes", Rule::KnownBits)
        }
    }
}

impl Shape {
    /// Whether every backend accepts the primitive: `half_extents` and `radius` finite,
    /// read by the kind or not; then by `kind`: a sphere's `radius` positive; a capsule's
    /// `radius` positive and its `half_extents.y` zero or positive (zero is a sphere); a
    /// cuboid's `half_extents.x`, `.y` and `.z` each positive. A capsule's `half_extents.x`
    /// and `.z` and a cuboid's `radius` are not read beyond being finite.
    pub fn validate(&self) -> Result<(), Invalid> {
        finite_vec("Shape::half_extents", self.half_extents)?;
        finite("Shape::radius", self.radius)?;
        match self.kind {
            ShapeKind::Sphere => positive("Shape::radius", self.radius),
            ShapeKind::Capsule => {
                positive("Shape::radius", self.radius)?;
                non_negative("Shape::half_extents.y", self.half_extents.y)
            }
            ShapeKind::Cuboid => {
                positive("Shape::half_extents.x", self.half_extents.x)?;
                positive("Shape::half_extents.y", self.half_extents.y)?;
                positive("Shape::half_extents.z", self.half_extents.z)
            }
        }
    }
}

impl Collider {
    /// Whether every backend accepts the collider: its `shape` by [`Shape::validate`];
    /// `offset` finite; `rotation` finite and a unit quaternion; `friction`,
    /// `restitution`, `density` and `impact_threshold` finite and zero or positive, with
    /// no upper bound. `groups`, `sensor`, `events` and `material` are not read: every
    /// value of theirs is accepted.
    pub fn validate(&self) -> Result<(), Invalid> {
        self.shape.validate()?;
        finite_vec("Collider::offset", self.offset)?;
        finite_quat("Collider::rotation", self.rotation)?;
        finite("Collider::friction", self.friction)?;
        finite("Collider::restitution", self.restitution)?;
        finite("Collider::density", self.density)?;
        finite("Collider::impact_threshold", self.impact_threshold)?;
        unit("Collider::rotation", self.rotation)?;
        non_negative("Collider::friction", self.friction)?;
        non_negative("Collider::restitution", self.restitution)?;
        non_negative("Collider::density", self.density)?;
        non_negative("Collider::impact_threshold", self.impact_threshold)
    }
}

impl Joint {
    /// Whether every backend accepts the joint's value: `anchor_a`, `anchor_b`, `axis`,
    /// `limit_min` and `limit_max` finite, whether the kind reads them or not; a revolute
    /// or prismatic joint's `axis` a direction (a fixed or spherical joint never reads it,
    /// so a zero one is accepted there); when `limited`, `limit_min` at most `limit_max`,
    /// equal allowed, whatever the kind. `body_a` and `body_b` are not read: a reference is
    /// resolved by the sync, which parks a joint whose reference is empty and waits for one
    /// whose body is not a body, and the backend refuses a joint whose two resolved ends
    /// are one body (`README.md`, "Joints").
    pub fn validate(&self) -> Result<(), Invalid> {
        finite_vec("Joint::anchor_a", self.anchor_a)?;
        finite_vec("Joint::anchor_b", self.anchor_b)?;
        finite_vec("Joint::axis", self.axis)?;
        finite("Joint::limit_min", self.limit_min)?;
        finite("Joint::limit_max", self.limit_max)?;
        let reads_axis = matches!(self.kind, JointKind::Revolute | JointKind::Prismatic);
        if reads_axis && self.axis.try_normalize().is_none() {
            return refuse("Joint::axis", Rule::Direction);
        }
        // Both limits are finite by now, so `>` is the negation of `<=`.
        if self.limited && self.limit_min > self.limit_max {
            return refuse("Joint::limit_min", Rule::Ordered);
        }
        Ok(())
    }
}

impl BodyDesc<'_> {
    /// Whether every backend accepts the row as a body: its `transform`, then its `body`,
    /// `collider` and `velocity` where present, each by its own `validate`, so that two
    /// backends name one field for one description. That at least one of `body` and
    /// `collider` is present is the sync's promise, not a rule here.
    pub fn validate(&self) -> Result<(), Invalid> {
        self.transform.validate()?;
        if let Some(body) = &self.body {
            body.validate()?;
        }
        if let Some(collider) = self.collider {
            collider.validate()?;
        }
        if let Some(velocity) = self.velocity {
            velocity.validate()?;
        }
        Ok(())
    }
}

impl PhysicsCommand {
    /// Whether every backend applies the command's value: a teleport's `transform` by
    /// [`Transform::validate`]; an impulse, point, torque or force finite, with no bound
    /// on its size. The body it names is not read: a command for a key that is not a
    /// body is dropped by the step, not refused by this.
    pub fn validate(&self) -> Result<(), Invalid> {
        match *self {
            PhysicsCommand::Teleport { transform, .. } => transform.validate(),
            PhysicsCommand::ApplyImpulse { impulse, .. } => {
                finite_vec("PhysicsCommand::impulse", impulse)
            }
            PhysicsCommand::ApplyImpulseAt { impulse, point, .. } => {
                finite_vec("PhysicsCommand::impulse", impulse)?;
                finite_vec("PhysicsCommand::point", point)
            }
            PhysicsCommand::ApplyTorqueImpulse { torque, .. }
            | PhysicsCommand::ApplyTorque { torque, .. } => {
                finite_vec("PhysicsCommand::torque", torque)
            }
            PhysicsCommand::ApplyForce { force, .. } => finite_vec("PhysicsCommand::force", force),
        }
    }
}

#[cfg(test)]
mod tests {
    use lactose_core::{NetId, StableKey};

    use super::*;

    fn s(value: f64) -> Scalar {
        Scalar::from_f64(value)
    }

    fn v(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3::new(s(x), s(y), s(z))
    }

    fn key(n: u64) -> StableKey {
        StableKey::Net(NetId::cooked(n))
    }

    fn refused(field: &'static str, rule: Rule) -> Result<(), Invalid> {
        Err(Invalid { field, rule })
    }

    /// A rotation whose squared length is `1 + off`.
    fn off_unit(off: f64) -> Quat {
        Quat::new(Scalar::ZERO, Scalar::ZERO, Scalar::ZERO, s((1.0 + off).sqrt()))
    }

    #[test]
    fn a_transform_is_finite_with_a_unit_rotation_within_the_tolerance() {
        assert_eq!(Transform::IDENTITY.validate(), Ok(()));
        assert_eq!(Transform::at(v(1e300, -0.0, 7.5)).validate(), Ok(()));
        assert_eq!(
            Transform::at(v(f64::NAN, 0.0, 0.0)).validate(),
            refused("Transform::translation", Rule::Finite)
        );
        assert_eq!(
            Transform::at(v(0.0, f64::INFINITY, 0.0)).validate(),
            refused("Transform::translation", Rule::Finite)
        );
        let rotated = |q: Quat| Transform::new(Vec3::ZERO, q);
        assert_eq!(
            rotated(Quat::new(s(f64::NAN), s(0.0), s(0.0), s(1.0))).validate(),
            refused("Transform::rotation", Rule::Finite)
        );
        assert_eq!(rotated(off_unit(0.5e-6)).validate(), Ok(()));
        assert_eq!(rotated(off_unit(-0.5e-6)).validate(), Ok(()));
        assert_eq!(rotated(off_unit(2e-6)).validate(), refused("Transform::rotation", Rule::Unit));
        assert_eq!(rotated(off_unit(-2e-6)).validate(), refused("Transform::rotation", Rule::Unit));
        let zero = Quat::new(s(0.0), s(0.0), s(0.0), s(0.0));
        assert_eq!(rotated(zero).validate(), refused("Transform::rotation", Rule::Unit));
        let minus_identity = Quat::new(s(0.0), s(0.0), s(0.0), s(-1.0));
        assert_eq!(rotated(minus_identity).validate(), Ok(()));
    }

    #[test]
    fn a_velocity_is_finite() {
        assert_eq!(Velocity::ZERO.validate(), Ok(()));
        let huge = Velocity { linear: v(1e300, 0.0, 0.0), angular: v(0.0, -1e300, 0.0) };
        assert_eq!(huge.validate(), Ok(()));
        let linear = Velocity { linear: v(f64::NAN, 0.0, 0.0), angular: Vec3::ZERO };
        assert_eq!(linear.validate(), refused("Velocity::linear", Rule::Finite));
        let angular = Velocity { linear: Vec3::ZERO, angular: v(0.0, 0.0, f64::NEG_INFINITY) };
        assert_eq!(angular.validate(), refused("Velocity::angular", Rule::Finite));
    }

    #[test]
    fn a_body_has_non_negative_mass_and_damping_a_finite_gravity_scale_and_known_bits() {
        let edit = |f: fn(&mut RigidBody)| {
            let mut body = RigidBody::dynamic();
            f(&mut body);
            body
        };
        assert_eq!(RigidBody::dynamic().validate(), Ok(()));
        assert_eq!(RigidBody::kinematic().validate(), Ok(()));
        assert_eq!(RigidBody::fixed().validate(), Ok(()));
        assert_eq!(edit(|b| b.mass = s(-0.0)).validate(), Ok(()));
        assert_eq!(edit(|b| b.gravity_scale = s(-3.0)).validate(), Ok(()));
        assert_eq!(edit(|b| b.can_sleep = false).validate(), Ok(()));
        const ALL: u8 = locked_axes::TRANSLATION | locked_axes::ROTATION;
        assert_eq!(edit(|b| b.locked_axes = ALL).validate(), Ok(()));
        for (f, field, rule) in [
            ((|b| b.mass = s(f64::NAN)) as fn(&mut RigidBody), "RigidBody::mass", Rule::Finite),
            (|b| b.mass = s(-1.0), "RigidBody::mass", Rule::NonNegative),
            (|b| b.linear_damping = s(f64::INFINITY), "RigidBody::linear_damping", Rule::Finite),
            (|b| b.linear_damping = s(-0.5), "RigidBody::linear_damping", Rule::NonNegative),
            (|b| b.angular_damping = s(-0.5), "RigidBody::angular_damping", Rule::NonNegative),
            (|b| b.gravity_scale = s(f64::NAN), "RigidBody::gravity_scale", Rule::Finite),
            (|b| b.locked_axes = 1 << 6, "RigidBody::locked_axes", Rule::KnownBits),
            (|b| b.locked_axes = 1 << 7 | ALL, "RigidBody::locked_axes", Rule::KnownBits),
            // A NaN is reported as not finite before it is reported as negative.
            (
                |b| {
                    b.mass = s(-1.0);
                    b.gravity_scale = s(f64::NAN);
                },
                "RigidBody::gravity_scale",
                Rule::Finite,
            ),
        ] {
            assert_eq!(edit(f).validate(), refused(field, rule), "{field}");
        }
    }

    #[test]
    fn a_shape_has_positive_extents_where_its_kind_reads_them_and_finite_ones_elsewhere() {
        assert_eq!(Shape::sphere(s(0.3)).validate(), Ok(()));
        assert_eq!(Shape::capsule(s(0.0), s(0.1)).validate(), Ok(()));
        assert_eq!(Shape::capsule(s(0.7), s(0.1)).validate(), Ok(()));
        assert_eq!(Shape::cuboid(v(1.0, 0.25, 50.0)).validate(), Ok(()));
        assert_eq!(Shape::sphere(s(0.0)).validate(), refused("Shape::radius", Rule::Positive));
        assert_eq!(Shape::sphere(s(-0.1)).validate(), refused("Shape::radius", Rule::Positive));
        assert_eq!(Shape::sphere(s(f64::NAN)).validate(), refused("Shape::radius", Rule::Finite));
        assert_eq!(
            Shape::capsule(s(0.1), s(0.0)).validate(),
            refused("Shape::radius", Rule::Positive)
        );
        assert_eq!(
            Shape::capsule(s(-0.1), s(0.1)).validate(),
            refused("Shape::half_extents.y", Rule::NonNegative)
        );
        assert_eq!(
            Shape::cuboid(v(0.1, 0.0, 0.1)).validate(),
            refused("Shape::half_extents.y", Rule::Positive)
        );
        assert_eq!(
            Shape::cuboid(v(0.1, 0.1, -0.1)).validate(),
            refused("Shape::half_extents.z", Rule::Positive)
        );
        assert_eq!(
            Shape::cuboid(v(f64::INFINITY, 0.1, 0.1)).validate(),
            refused("Shape::half_extents", Rule::Finite)
        );
        // Fields the kind does not read are still held finite, and nothing more.
        let mut sphere = Shape::sphere(s(0.3));
        sphere.half_extents = v(-1.0, 0.0, 1e300);
        assert_eq!(sphere.validate(), Ok(()));
        sphere.half_extents.z = s(f64::NAN);
        assert_eq!(sphere.validate(), refused("Shape::half_extents", Rule::Finite));
        let mut cuboid = Shape::cuboid(Vec3::ONE);
        cuboid.radius = s(-5.0);
        assert_eq!(cuboid.validate(), Ok(()));
        cuboid.radius = s(f64::NEG_INFINITY);
        assert_eq!(cuboid.validate(), refused("Shape::radius", Rule::Finite));
        let mut capsule = Shape::capsule(s(0.5), s(0.1));
        capsule.half_extents.x = s(-7.0);
        assert_eq!(capsule.validate(), Ok(()));
    }

    #[test]
    fn a_collider_has_a_valid_shape_a_unit_rotation_and_non_negative_coefficients() {
        let edit = |f: fn(&mut Collider)| {
            let mut collider = Collider::new(Shape::sphere(Scalar::HALF));
            f(&mut collider);
            collider
        };
        assert_eq!(edit(|_| {}).validate(), Ok(()));
        let zeroed = edit(|c| {
            c.friction = s(0.0);
            c.restitution = s(0.0);
            c.density = s(0.0);
            c.impact_threshold = s(0.0);
        });
        assert_eq!(zeroed.validate(), Ok(()));
        assert_eq!(edit(|c| c.restitution = s(7.0)).validate(), Ok(()));
        let sensor = edit(|c| *c = c.sensor().with_groups(crate::CollisionGroups::new(0, 0)));
        assert_eq!(sensor.validate(), Ok(()));
        for (f, field, rule) in [
            (
                (|c| c.shape = Shape::sphere(s(0.0))) as fn(&mut Collider),
                "Shape::radius",
                Rule::Positive,
            ),
            (|c| c.offset = v(0.0, f64::NAN, 0.0), "Collider::offset", Rule::Finite),
            (
                |c| c.rotation = Quat::new(s(0.0), s(0.0), s(0.0), s(2.0)),
                "Collider::rotation",
                Rule::Unit,
            ),
            (
                |c| c.rotation = Quat::new(s(0.0), s(f64::INFINITY), s(0.0), s(1.0)),
                "Collider::rotation",
                Rule::Finite,
            ),
            (|c| c.friction = s(-0.1), "Collider::friction", Rule::NonNegative),
            (|c| c.restitution = s(-0.1), "Collider::restitution", Rule::NonNegative),
            (|c| c.density = s(-1.0), "Collider::density", Rule::NonNegative),
            (|c| c.impact_threshold = s(-1.0), "Collider::impact_threshold", Rule::NonNegative),
            (|c| c.density = s(f64::NAN), "Collider::density", Rule::Finite),
            // Finite first: a NaN coefficient outranks a non-unit rotation.
            (
                |c| {
                    c.rotation = Quat::new(s(0.0), s(0.0), s(0.0), s(2.0));
                    c.impact_threshold = s(f64::NAN);
                },
                "Collider::impact_threshold",
                Rule::Finite,
            ),
        ] {
            assert_eq!(edit(f).validate(), refused(field, rule), "{field}");
        }
    }

    #[test]
    fn a_joint_is_finite_with_a_direction_where_read_and_ordered_limits_when_limited() {
        let joint = |kind: JointKind| Joint::new(kind, key(1), key(2));
        for kind in
            [JointKind::Fixed, JointKind::Spherical, JointKind::Revolute, JointKind::Prismatic]
        {
            assert_eq!(joint(kind).validate(), Ok(()), "{kind:?}");
            assert_eq!(joint(kind).with_limits(s(-0.5), s(0.5)).validate(), Ok(()), "{kind:?}");
            assert_eq!(joint(kind).with_limits(s(0.25), s(0.25)).validate(), Ok(()), "{kind:?}");
            assert_eq!(
                joint(kind).with_limits(s(0.5), s(-0.5)).validate(),
                refused("Joint::limit_min", Rule::Ordered),
                "{kind:?}"
            );
            // Unlimited, the limits are still held finite; the axis always is.
            let mut unlimited = joint(kind);
            (unlimited.limit_min, unlimited.limit_max) = (s(0.5), s(-0.5));
            assert_eq!(unlimited.validate(), Ok(()), "{kind:?}");
            unlimited.limit_max = s(f64::NAN);
            assert_eq!(unlimited.validate(), refused("Joint::limit_max", Rule::Finite));
            assert_eq!(
                joint(kind).with_axis(v(f64::NAN, 0.0, 0.0)).validate(),
                refused("Joint::axis", Rule::Finite),
                "{kind:?}"
            );
            assert_eq!(
                joint(kind).with_anchors(Vec3::ZERO, v(0.0, f64::INFINITY, 0.0)).validate(),
                refused("Joint::anchor_b", Rule::Finite),
                "{kind:?}"
            );
        }
        for kind in [JointKind::Fixed, JointKind::Spherical] {
            assert_eq!(joint(kind).with_axis(Vec3::ZERO).validate(), Ok(()), "{kind:?}");
            assert_eq!(joint(kind).with_axis(v(1e-200, 0.0, 0.0)).validate(), Ok(()));
        }
        for kind in [JointKind::Revolute, JointKind::Prismatic] {
            assert_eq!(
                joint(kind).with_axis(Vec3::ZERO).validate(),
                refused("Joint::axis", Rule::Direction),
                "{kind:?}"
            );
            // A length that underflows or overflows in the square is no direction either.
            assert_eq!(
                joint(kind).with_axis(v(1e-200, 0.0, 0.0)).validate(),
                refused("Joint::axis", Rule::Direction)
            );
            assert_eq!(
                joint(kind).with_axis(v(1e200, 0.0, 0.0)).validate(),
                refused("Joint::axis", Rule::Direction)
            );
            assert_eq!(joint(kind).with_axis(v(0.0, 0.0, 3.0)).validate(), Ok(()));
        }
        // The references are not read: the same key twice, or an empty one, is the
        // sync's and the backend's to judge.
        assert_eq!(Joint::new(JointKind::Fixed, key(1), key(1)).validate(), Ok(()));
    }

    #[test]
    fn a_body_description_is_checked_transform_body_collider_velocity_in_that_order() {
        let transform = Transform::at(v(0.0, f64::NAN, 0.0));
        let mut body = RigidBody::dynamic();
        body.mass = s(-1.0);
        let collider = Collider::new(Shape::sphere(s(-1.0)));
        let velocity = Velocity { linear: v(f64::INFINITY, 0.0, 0.0), angular: Vec3::ZERO };
        let desc = |transform: &Transform,
                    body: Option<RigidBody>,
                    collider: Option<&Collider>,
                    velocity: Option<&Velocity>| {
            BodyDesc { body, collider, transform, velocity, frozen: false }.validate()
        };
        assert_eq!(
            desc(&transform, Some(body), Some(&collider), Some(&velocity)),
            refused("Transform::translation", Rule::Finite)
        );
        assert_eq!(
            desc(&Transform::IDENTITY, Some(body), Some(&collider), Some(&velocity)),
            refused("RigidBody::mass", Rule::NonNegative)
        );
        assert_eq!(
            desc(&Transform::IDENTITY, None, Some(&collider), Some(&velocity)),
            refused("Shape::radius", Rule::Positive)
        );
        assert_eq!(
            desc(&Transform::IDENTITY, None, None, Some(&velocity)),
            refused("Velocity::linear", Rule::Finite)
        );
        assert_eq!(desc(&Transform::IDENTITY, None, None, None), Ok(()));
        let ball = Collider::new(Shape::sphere(Scalar::HALF));
        assert_eq!(
            desc(
                &Transform::IDENTITY,
                Some(RigidBody::fixed()),
                Some(&ball),
                Some(&Velocity::ZERO)
            ),
            Ok(())
        );
    }

    #[test]
    fn a_command_is_finite_and_a_teleport_is_a_valid_transform() {
        let body = key(1);
        let nan = v(f64::NAN, 0.0, 0.0);
        let fine = v(1.0, 2.0, 3.0);
        assert_eq!(PhysicsCommand::ApplyImpulse { body, impulse: fine }.validate(), Ok(()));
        assert_eq!(
            PhysicsCommand::ApplyImpulse { body, impulse: nan }.validate(),
            refused("PhysicsCommand::impulse", Rule::Finite)
        );
        assert_eq!(
            PhysicsCommand::ApplyImpulseAt { body, impulse: fine, point: nan }.validate(),
            refused("PhysicsCommand::point", Rule::Finite)
        );
        assert_eq!(
            PhysicsCommand::ApplyImpulseAt { body, impulse: nan, point: nan }.validate(),
            refused("PhysicsCommand::impulse", Rule::Finite)
        );
        assert_eq!(
            PhysicsCommand::ApplyTorqueImpulse { body, torque: nan }.validate(),
            refused("PhysicsCommand::torque", Rule::Finite)
        );
        assert_eq!(
            PhysicsCommand::ApplyTorque { body, torque: nan }.validate(),
            refused("PhysicsCommand::torque", Rule::Finite)
        );
        assert_eq!(
            PhysicsCommand::ApplyForce { body, force: nan }.validate(),
            refused("PhysicsCommand::force", Rule::Finite)
        );
        let placed = PhysicsCommand::Teleport { body, transform: Transform::at(fine) };
        assert_eq!(placed.validate(), Ok(()));
        let askew = Transform::new(fine, Quat::new(s(1.0), s(1.0), s(0.0), s(0.0)));
        assert_eq!(
            PhysicsCommand::Teleport { body, transform: askew }.validate(),
            refused("Transform::rotation", Rule::Unit)
        );
    }

    #[test]
    fn an_invalid_names_its_field_and_rule_and_becomes_invalid_data() {
        let invalid = Invalid { field: "Shape::half_extents.y", rule: Rule::NonNegative };
        assert_eq!(invalid.to_string(), "`Shape::half_extents.y` is not zero or positive");
        let error: Error = invalid.into();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert_eq!(error.message(), invalid.to_string());
        let rules = [
            (Rule::Finite, "finite"),
            (Rule::Unit, "a unit quaternion"),
            (Rule::NonNegative, "zero or positive"),
            (Rule::Positive, "positive"),
            (Rule::Direction, "a direction with a positive finite length"),
            (Rule::KnownBits, "within the known `locked_axes` bits"),
            (Rule::Ordered, "at most `limit_max`"),
        ];
        for (rule, text) in rules {
            assert_eq!(rule.to_string(), text);
        }
    }
}
