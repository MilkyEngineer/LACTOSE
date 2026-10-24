//! The validity run against the test backend (`README.md`, "Validity"): the fake applies
//! this crate's rules and nothing of its own, so the run that holds
//! `lactose-physics-rapier` to them holds the fake too, and a test written against the
//! fake sees what a real backend does with a refused value. And the run against backends
//! that break the rules, one way each: every one of them fails it.

#![forbid(unsafe_code)]

mod common;

use common::fake::{CallLog, Fake};
use lactose_core::{Result, StableKey};
use lactose_math::{Quat, Scalar, Vec3};
use lactose_physics_api::conformance::validity;
use lactose_physics_api::{
    BodyDesc, BodyHandle, Collider, Invalid, Joint, JointHandle, JointKind, PhysicsBackend,
    PhysicsCommand, QueryFilter, Ray, RayHit, RigidBody, Rule, Shape, ShapeHit, StepInput,
    StepOutput, Transform, Velocity,
};

#[test]
fn the_fake_holds_to_the_validity_rules_under_every_pool_size() {
    for threads in [1, 4] {
        let fake = Fake::new(CallLog::default()).with_threads(threads);
        validity(Box::new(fake)).unwrap_or_else(|error| panic!("{threads} threads: {error}"));
    }
}

/// One way a backend breaks the rules. A backend that skips a check is the fake with the
/// value it would have let through made valid first (`fix`), so the fake accepts what the
/// deviant would.
#[derive(Clone, Copy)]
enum Deviation {
    /// Skips a check of `RigidBody::validate`, at `create_body` and `set_body`.
    Body(fn(&mut RigidBody)),
    /// Skips a check of `Collider::validate`, at `create_body` and `set_collider`.
    Collider(fn(&mut Collider)),
    /// Skips a check of `Joint::validate`.
    Joint(fn(&mut Joint)),
    /// Skips a check of `PhysicsCommand::validate`.
    Command(fn(&mut PhysicsCommand)),
    /// Refuses a collider the rules accept.
    OwnCollider(fn(&Collider) -> bool),
    /// Refuses a joint the rules accept.
    OwnJoint(fn(&Joint) -> bool),
    /// Checks a description's velocity, collider, body and transform, in that order.
    ReversedDescription,
    /// Checks a collider's coefficients for sign before its shape.
    BoundsBeforeShape,
    /// Checks a joint's value before the same-body rule.
    ValueBeforeSameBody,
    /// Steps a `dt` or gravity the step-input rule refuses, as a valid one.
    AnyStepInput,
    /// Answers a ray whose `max_distance` is not finite, as one of a hundred.
    AnyReach,
    /// Drops every command of a step that holds one the rules refuse.
    WholeStepDropped,
}

struct Deviant {
    inner: Fake,
    deviation: Deviation,
}

fn finite_or(value: &mut Scalar, valid: Scalar) {
    if !value.is_finite() {
        *value = valid;
    }
}

impl Deviant {
    fn body(&self, body: &RigidBody) -> RigidBody {
        let mut body = *body;
        if let Deviation::Body(fix) = self.deviation {
            fix(&mut body);
        }
        body
    }

    fn collider(&self, collider: &Collider) -> Result<Collider> {
        let mut collider = *collider;
        match self.deviation {
            Deviation::Collider(fix) => fix(&mut collider),
            Deviation::OwnCollider(refuses) if refuses(&collider) => {
                return Err(Invalid { field: "Collider::friction", rule: Rule::Finite }.into());
            }
            Deviation::BoundsBeforeShape => {
                let coefficients = [
                    ("Collider::friction", collider.friction),
                    ("Collider::restitution", collider.restitution),
                    ("Collider::density", collider.density),
                    ("Collider::impact_threshold", collider.impact_threshold),
                ];
                for (field, value) in coefficients {
                    if value.is_nan() || value < Scalar::ZERO {
                        return Err(Invalid { field, rule: Rule::NonNegative }.into());
                    }
                }
            }
            _ => {}
        }
        Ok(collider)
    }
}

impl PhysicsBackend for Deviant {
    fn create_body(&mut self, key: StableKey, desc: &BodyDesc<'_>) -> Result<BodyHandle> {
        if let Deviation::ReversedDescription = self.deviation {
            if let Some(velocity) = desc.velocity {
                velocity.validate()?;
            }
            if let Some(collider) = desc.collider {
                collider.validate()?;
            }
            if let Some(body) = &desc.body {
                body.validate()?;
            }
            desc.transform.validate()?;
        }
        let body = desc.body.map(|body| self.body(&body));
        let collider = desc.collider.map(|collider| self.collider(collider)).transpose()?;
        let desc = BodyDesc { body, collider: collider.as_ref(), ..*desc };
        self.inner.create_body(key, &desc)
    }
    fn remove_body(&mut self, key: StableKey) {
        self.inner.remove_body(key);
    }
    fn set_body(&mut self, key: StableKey, body: Option<&RigidBody>) {
        let body = body.map(|body| self.body(body));
        self.inner.set_body(key, body.as_ref());
    }
    fn set_collider(&mut self, key: StableKey, collider: Option<&Collider>) {
        if let Ok(collider) = collider.map(|collider| self.collider(collider)).transpose() {
            self.inner.set_collider(key, collider.as_ref());
        }
    }
    fn set_transform(&mut self, key: StableKey, transform: &Transform) {
        self.inner.set_transform(key, transform);
    }
    fn set_velocity(&mut self, key: StableKey, velocity: &Velocity) {
        self.inner.set_velocity(key, velocity);
    }
    fn set_frozen(&mut self, key: StableKey, frozen: bool) {
        self.inner.set_frozen(key, frozen);
    }
    fn create_joint(
        &mut self,
        key: StableKey,
        joint: &Joint,
        body_a: StableKey,
        body_b: StableKey,
    ) -> Result<JointHandle> {
        let mut joint = *joint;
        match self.deviation {
            Deviation::Joint(fix) => fix(&mut joint),
            Deviation::OwnJoint(refuses) if refuses(&joint) => {
                return Err(Invalid { field: "Joint::anchor_a", rule: Rule::Finite }.into());
            }
            Deviation::ValueBeforeSameBody => joint.validate()?,
            _ => {}
        }
        self.inner.create_joint(key, &joint, body_a, body_b)
    }
    fn remove_joint(&mut self, key: StableKey) {
        self.inner.remove_joint(key);
    }
    fn step(&mut self, input: &StepInput<'_>, output: &mut StepOutput) {
        let mut commands = input.commands.to_vec();
        let mut input = *input;
        match self.deviation {
            Deviation::Command(fix) => commands.iter_mut().for_each(fix),
            Deviation::AnyStepInput => {
                finite_or(&mut input.dt, Scalar::from_f64(1.0 / 60.0));
                if input.dt <= Scalar::ZERO {
                    input.dt = Scalar::from_f64(1.0 / 60.0);
                }
                if !input.gravity.is_finite() {
                    input.gravity = Vec3::ZERO;
                }
            }
            Deviation::WholeStepDropped
                if commands.iter().any(|command| command.validate().is_err()) =>
            {
                commands.clear();
            }
            _ => {}
        }
        self.inner.step(&StepInput { commands: &commands, ..input }, output);
    }
    fn snapshot(&self, out: &mut Vec<u8>) -> Result<()> {
        self.inner.snapshot(out)
    }
    fn restore(&mut self, bytes: &[u8]) -> Result<()> {
        self.inner.restore(bytes)
    }
    fn cast_ray(&self, ray: &Ray, filter: &QueryFilter) -> Option<RayHit> {
        let mut ray = *ray;
        if let Deviation::AnyReach = self.deviation {
            finite_or(&mut ray.max_distance, Scalar::from_i32(100));
        }
        self.inner.cast_ray(&ray, filter)
    }
    fn cast_shape(
        &self,
        shape: &Shape,
        pose: &Transform,
        direction: Vec3,
        max_distance: Scalar,
        filter: &QueryFilter,
    ) -> Option<ShapeHit> {
        self.inner.cast_shape(shape, pose, direction, max_distance, filter)
    }
    fn overlap(
        &self,
        shape: &Shape,
        pose: &Transform,
        filter: &QueryFilter,
        hits: &mut Vec<StableKey>,
    ) {
        self.inner.overlap(shape, pose, filter, hits);
    }
}

/// Review round 1 of D135 (S1, S2, M1, M2): backends that break the rules and passed the
/// validity run as it was. Each one now fails it.
#[test]
fn a_backend_that_breaks_the_rules_fails_the_validity_run() {
    let control =
        Deviant { inner: Fake::new(CallLog::default()), deviation: Deviation::Body(|_| {}) };
    validity(Box::new(control)).expect("the wrapper alone passes the run");
    let deviations: [(&str, Deviation); 25] = [
        (
            "skips the finite check on linear_damping",
            Deviation::Body(|b| {
                finite_or(&mut b.linear_damping, Scalar::ZERO);
            }),
        ),
        (
            "skips the finite check on angular_damping",
            Deviation::Body(|b| {
                finite_or(&mut b.angular_damping, Scalar::ZERO);
            }),
        ),
        (
            "skips the finite check on Shape::radius",
            Deviation::Collider(|c| {
                finite_or(&mut c.shape.radius, Scalar::ONE);
            }),
        ),
        (
            "skips the finite check on Collider::rotation",
            Deviation::Collider(|c| {
                if !c.rotation.is_finite() {
                    c.rotation = Quat::IDENTITY;
                }
            }),
        ),
        (
            "skips the finite check on friction",
            Deviation::Collider(|c| {
                finite_or(&mut c.friction, Scalar::HALF);
            }),
        ),
        (
            "skips the finite check on restitution",
            Deviation::Collider(|c| {
                finite_or(&mut c.restitution, Scalar::HALF);
            }),
        ),
        (
            "skips the finite check on density",
            Deviation::Collider(|c| {
                finite_or(&mut c.density, Scalar::HALF);
            }),
        ),
        (
            "skips the finite check on impact_threshold",
            Deviation::Collider(|c| {
                finite_or(&mut c.impact_threshold, Scalar::HALF);
            }),
        ),
        (
            "skips the positive check on a cuboid's x",
            Deviation::Collider(|c| {
                if c.shape.half_extents.x == Scalar::ZERO {
                    c.shape.half_extents.x = Scalar::ONE;
                }
            }),
        ),
        (
            "skips the positive check on a cuboid's z",
            Deviation::Collider(|c| {
                if c.shape.half_extents.z == Scalar::ZERO {
                    c.shape.half_extents.z = Scalar::ONE;
                }
            }),
        ),
        (
            "skips the finite check on anchor_b",
            Deviation::Joint(|j| {
                if !j.anchor_b.is_finite() {
                    j.anchor_b = Vec3::ZERO;
                }
            }),
        ),
        (
            "skips the finite check on a fixed joint's axis",
            Deviation::Joint(|j| {
                if j.kind == JointKind::Fixed && !j.axis.is_finite() {
                    j.axis = Vec3::ZERO;
                }
            }),
        ),
        (
            "skips the finite check on an unlimited limit_min",
            Deviation::Joint(|j| {
                if !j.limited {
                    finite_or(&mut j.limit_min, Scalar::ZERO);
                }
            }),
        ),
        (
            "refuses only the exact zero axis",
            Deviation::Joint(|j| {
                if j.axis != Vec3::ZERO && j.axis.is_finite() {
                    j.axis = Vec3::X;
                }
            }),
        ),
        (
            "takes an axis try_normalize takes, a subnormal square included",
            Deviation::Joint(|j| {
                if j.axis.is_finite() && j.axis.try_normalize().is_some() {
                    j.axis = Vec3::X;
                }
            }),
        ),
        (
            "skips the finite check on ApplyImpulseAt's impulse",
            Deviation::Command(|command| {
                if let PhysicsCommand::ApplyImpulseAt { impulse, .. } = command
                    && !impulse.is_finite()
                {
                    *impulse = Vec3::X;
                }
            }),
        ),
        (
            "refuses a friction above 1e6",
            Deviation::OwnCollider(|c| c.friction > Scalar::from_f64(1e6)),
        ),
        (
            "refuses a limited fixed or spherical joint",
            Deviation::OwnJoint(|j| {
                j.limited && matches!(j.kind, JointKind::Fixed | JointKind::Spherical)
            }),
        ),
        (
            "refuses an anchor farther than 1e6",
            Deviation::OwnJoint(|j| {
                j.anchor_a.length() > Scalar::from_f64(1e6)
                    || j.anchor_b.length() > Scalar::from_f64(1e6)
            }),
        ),
        ("checks a description in reverse", Deviation::ReversedDescription),
        ("checks a collider's coefficients before its shape", Deviation::BoundsBeforeShape),
        ("checks a joint's value before the same-body rule", Deviation::ValueBeforeSameBody),
        ("steps any dt and gravity", Deviation::AnyStepInput),
        ("answers a ray of any reach", Deviation::AnyReach),
        ("drops a whole step for one refused command", Deviation::WholeStepDropped),
    ];
    for (what, deviation) in deviations {
        let deviant = Deviant { inner: Fake::new(CallLog::default()), deviation };
        assert!(validity(Box::new(deviant)).is_err(), "a backend that {what} passes the run");
    }
}
