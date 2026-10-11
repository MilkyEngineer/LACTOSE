//! The trait's calls driven directly, without a world: what a step reports for falling,
//! pushed, teleported, swept, frozen and sleeping bodies; that equal writes change nothing;
//! validation; joints. Rapier integrates differently from the test backend, so these assert
//! on structure (signs, which bodies, which events) and on numbers only where they are
//! exact: a placed pose, a velocity nothing acts on, a resting pose across steps.

#![forbid(unsafe_code)]

mod support;

use lactose_core::ErrorKind;
use lactose_math::{Quat, Scalar, Vec3};
use lactose_physics_api::{
    BodyDesc, Collider, Joint, JointKind, PhysicsBackend, PhysicsCommand, QueryFilter, Ray,
    RigidBody, Shape, StepInput, StepOutput, Transform, Velocity,
};
use support::*;

/// Steps until `key` is no longer reported, at most `limit` ticks from `tick`; returns the
/// tick after which it was not reported and its last reported state.
fn until_asleep(
    backend: &mut dyn PhysicsBackend,
    key: lactose_core::StableKey,
    mut tick: u32,
    limit: u32,
) -> (u32, lactose_physics_api::BodyState) {
    let mut last = None;
    for _ in 0..limit {
        tick += 1;
        match state(&step(backend, tick, &[]), key) {
            Some(reported) => last = Some(reported),
            None => return (tick, last.expect("reported at least once")),
        }
    }
    panic!("{key:?} still awake after {limit} ticks");
}

/// The distance from ten units above `x` straight down to the first collider.
fn height_below(backend: &dyn PhysicsBackend, x: f64) -> Scalar {
    let ray = Ray::new(v(x, 10.0, 0.0), v(0.0, -1.0, 0.0), s(100.0));
    backend.cast_ray(&ray, &QueryFilter::ALL).unwrap().distance
}

#[test]
fn a_falling_body_is_reported_until_it_rests_and_then_holds_its_pose_exactly() {
    let mut b = backend(1);
    let (ground, falling) = (key(1), key(2));
    create(&mut *b, ground, None, Some(floor()), Transform::IDENTITY);
    create(&mut *b, falling, Some(RigidBody::dynamic()), Some(ball()), at(0.0, 2.0, 0.0));
    let first = step(&mut *b, 1, &[]);
    assert_eq!(first.bodies.len(), 1, "the fixed floor is never reported");
    let fell = state(&first, falling).unwrap();
    assert!(fell.velocity.linear.y < Scalar::ZERO && fell.transform.translation.y < s(2.0));
    assert_eq!((fell.velocity.linear.x, fell.velocity.linear.z), (Scalar::ZERO, Scalar::ZERO));
    assert!(!fell.teleported);

    let (tick, last) = until_asleep(&mut *b, falling, 1, 1_000);
    // The last report is the body put to sleep: at rest on the floor, its velocity zero.
    assert!(still(last.velocity), "{last:?}");
    let y = last.transform.translation.y;
    assert!((y - s(0.5)).abs() < s(0.05), "rests on the floor: {y:?}");
    // Asleep, it is not reported and does not move by a bit.
    let resting = height_below(&*b, 0.0);
    for later in tick + 1..tick + 30 {
        assert!(step(&mut *b, later, &[]).bodies.is_empty());
        assert_eq!(height_below(&*b, 0.0).to_canonical_bits(), resting.to_canonical_bits());
    }
}

#[test]
fn an_impulse_moves_a_floating_body_along_it_and_mass_comes_from_the_body_or_the_density() {
    let mut b = backend(1);
    let (light, heavy) = (key(1), key(2));
    let mut two = floating();
    two.mass = Scalar::TWO;
    create(&mut *b, light, Some(floating()), Some(ball()), at(0.0, 0.0, 0.0));
    create(&mut *b, heavy, Some(two), Some(ball()), at(5.0, 0.0, 0.0));
    let push = |body| PhysicsCommand::ApplyImpulse { body, impulse: v(1.0, 0.0, 0.0) };
    let out = step(&mut *b, 1, &[push(light), push(heavy)]);
    let (light, heavy) = (state(&out, light).unwrap(), state(&out, heavy).unwrap());
    // A unit-density ball of radius one half weighs pi / 6.
    let expected = 6.0 / std::f64::consts::PI;
    assert!((light.velocity.linear.x - s(expected)).abs() < s(1e-9), "{light:?}");
    // `RigidBody::mass` is the collider's mass when it is positive.
    assert!((heavy.velocity.linear.x - Scalar::HALF).abs() < s(1e-12), "{heavy:?}");
    for pushed in [light, heavy] {
        assert_eq!(pushed.velocity.linear.y, Scalar::ZERO);
        assert_eq!(pushed.velocity.linear.z, Scalar::ZERO);
        assert_eq!(pushed.velocity.angular, Vec3::ZERO);
        assert!(pushed.transform.translation.x > Scalar::ZERO);
    }
}

/// Review S1: the tick's commands run after the rows are applied, so an impulse in the
/// tick a body is created or its mass changes sees the mass the rows give, with or without
/// a collider.
#[test]
fn an_impulse_in_the_tick_of_a_creation_or_a_mass_change_sees_the_new_mass() {
    let push = PhysicsCommand::ApplyImpulse { body: key(10), impulse: v(4.0, 0.0, 0.0) };
    let dv = |out: &StepOutput| state(out, key(10)).unwrap().velocity.linear.x;
    let close = |got: Scalar, want: f64| (got - s(want)).abs() < s(1e-9);
    let weighing = |mass: f64| {
        let mut body = floating();
        body.mass = s(mass);
        body
    };
    // Created without a collider and pushed in the same tick: a mass of zero is one.
    for (mass, expected) in [(2.0, 2.0), (0.0, 4.0)] {
        let mut b = backend(1);
        create(&mut *b, key(10), Some(weighing(mass)), None, at(0.0, 5.0, 0.0));
        let got = dv(&step(&mut *b, 1, &[push]));
        assert!(close(got, expected), "mass {mass}, no collider: {got:?}");
    }
    // The mass doubled and pushed in the same tick: half the change a mass of two gives.
    for collider in [None, Some(ball())] {
        let mut b = backend(1);
        create(&mut *b, key(10), Some(weighing(2.0)), collider, at(0.0, 5.0, 0.0));
        step(&mut *b, 1, &[]);
        let before = dv(&step(&mut *b, 2, &[push]));
        assert!(close(before, 2.0), "mass 2, collider {}: {before:?}", collider.is_some());
        b.set_body(key(10), Some(&weighing(4.0)));
        let after = dv(&step(&mut *b, 3, &[push]));
        assert!(close(after - before, 1.0), "mass 4, collider {}: {after:?}", collider.is_some());
    }
}

#[test]
fn a_force_lasts_one_step_and_a_push_on_a_body_that_is_not_dynamic_is_dropped() {
    let mut b = backend(1);
    let (pushed, kinematic, fixed) = (key(1), key(2), key(3));
    create(&mut *b, pushed, Some(floating()), Some(ball()), at(0.0, 0.0, 0.0));
    create(&mut *b, kinematic, Some(RigidBody::kinematic()), Some(ball()), at(5.0, 0.0, 0.0));
    create(&mut *b, fixed, None, Some(ball()), at(10.0, 0.0, 0.0));
    let force = PhysicsCommand::ApplyForce { body: pushed, force: v(60.0, 0.0, 0.0) };
    let ignored = [
        PhysicsCommand::ApplyImpulse { body: kinematic, impulse: v(1.0, 0.0, 0.0) },
        PhysicsCommand::ApplyImpulse { body: fixed, impulse: v(1.0, 0.0, 0.0) },
        PhysicsCommand::ApplyImpulse { body: key(99), impulse: v(1.0, 0.0, 0.0) },
        PhysicsCommand::ApplyImpulse { body: pushed, impulse: v(f64::NAN, 0.0, 0.0) },
    ];
    let mut commands = vec![force];
    commands.extend(ignored);
    let first = step(&mut *b, 1, &commands);
    let gained = state(&first, pushed).unwrap().velocity.linear.x;
    assert!(gained > Scalar::ZERO);
    assert_eq!(first.bodies.len(), 1, "{first:?}");
    // The next step without the command keeps the velocity: the force is gone.
    let second = step(&mut *b, 2, &[]);
    assert_eq!(state(&second, pushed).unwrap().velocity.linear.x, gained);
}

#[test]
fn a_teleport_places_the_body_exactly_and_is_flagged_for_that_step_only() {
    let mut b = backend(2);
    let body = key(1);
    create(&mut *b, body, Some(floating()), Some(ball()), at(0.0, 0.0, 0.0));
    step(&mut *b, 1, &[]);
    let target = Transform::new(v(3.0, 4.0, 5.0), Quat::IDENTITY);
    let out = step(&mut *b, 2, &[PhysicsCommand::Teleport { body, transform: target }]);
    let placed = state(&out, body).unwrap();
    assert!(placed.teleported);
    assert_eq!(placed.transform, target);
    let after = state(&step(&mut *b, 3, &[]), body).unwrap();
    assert!(!after.teleported);
    assert_eq!(after.transform, target);

    // A transform write places a dynamic body the same way.
    let written = at(-2.0, 1.0, 0.0);
    b.set_transform(body, &written);
    let out = step(&mut *b, 4, &[]);
    assert_eq!(state(&out, body).map(|s| (s.transform, s.teleported)), Some((written, true)));
}

#[test]
fn a_kinematic_body_sweeps_to_its_pose_reports_its_stop_once_and_ignores_velocity_writes() {
    let mut b = backend(1);
    let body = key(1);
    create(&mut *b, body, Some(RigidBody::kinematic()), Some(ball()), at(0.0, 0.0, 0.0));
    assert!(step(&mut *b, 1, &[]).bodies.is_empty(), "an unmoved kinematic body is not reported");
    let target = at(1.0, 0.0, 0.0);
    b.set_transform(body, &target);
    let swept = state(&step(&mut *b, 2, &[]), body).unwrap();
    assert_eq!(swept.transform, target);
    assert!(!swept.teleported);
    assert!((swept.velocity.linear.x - s(60.0)).abs() < s(1e-9), "{swept:?}");
    let stopped = state(&step(&mut *b, 3, &[]), body).unwrap();
    assert_eq!((stopped.transform, stopped.velocity), (target, Velocity::ZERO));
    assert!(step(&mut *b, 4, &[]).bodies.is_empty());
    b.set_velocity(body, &Velocity { linear: v(5.0, 0.0, 0.0), angular: Vec3::ZERO });
    assert!(step(&mut *b, 5, &[]).bodies.is_empty());
}

#[test]
fn equal_writes_leave_a_sleeping_body_asleep_and_change_no_byte() {
    let scene = || {
        let mut b = backend(1);
        create(&mut *b, key(1), Some(floating()), Some(ball()), at(0.0, 0.0, 0.0));
        let (tick, last) = until_asleep(&mut *b, key(1), 0, 1_000);
        (b, tick, last)
    };
    let (mut written, tick, last) = scene();
    let (mut control, control_tick, _) = scene();
    assert_eq!(tick, control_tick);
    written.set_transform(key(1), &last.transform);
    written.set_velocity(key(1), &Velocity::ZERO);
    written.set_body(key(1), Some(&floating()));
    written.set_collider(key(1), Some(&ball()));
    written.set_frozen(key(1), false);
    assert!(step(&mut *written, tick + 1, &[]).bodies.is_empty());
    step(&mut *control, tick + 1, &[]);
    assert_eq!(snapshot(&*written), snapshot(&*control));

    // A write of another value wakes it, and nothing acts on the velocity it is given.
    let moving = Velocity { linear: v(1.0, 0.0, 0.0), angular: Vec3::ZERO };
    written.set_velocity(key(1), &moving);
    let woken = state(&step(&mut *written, tick + 2, &[]), key(1)).unwrap();
    assert_eq!(woken.velocity, moving);
}

#[test]
fn a_frozen_body_is_a_fixed_collider_and_thaws_with_its_velocity() {
    let mut b = backend(1);
    let body = key(1);
    let moving = Velocity { linear: v(1.0, 0.0, 0.0), angular: Vec3::ZERO };
    let desc = BodyDesc {
        body: Some(floating()),
        collider: Some(&ball()),
        transform: &at(0.0, 0.0, 0.0),
        velocity: Some(&moving),
        frozen: false,
    };
    b.create_body(body, &desc).unwrap();
    let before = state(&step(&mut *b, 1, &[]), body).unwrap();
    assert_eq!(before.velocity, moving);
    b.set_frozen(body, true);
    for tick in 2..5 {
        assert!(step(&mut *b, tick, &[]).bodies.is_empty());
    }
    // Still a collider where it stopped.
    let ray = Ray::new(
        v(before.transform.translation.x.to_f64(), 10.0, 0.0),
        v(0.0, -1.0, 0.0),
        s(100.0),
    );
    assert_eq!(b.cast_ray(&ray, &QueryFilter::ALL).map(|hit| hit.body), Some(body));
    b.set_frozen(body, false);
    let thawed = state(&step(&mut *b, 5, &[]), body).unwrap();
    assert_eq!(thawed.velocity, moving);
    assert!(thawed.transform.translation.x > before.transform.translation.x);
}

#[test]
fn invalid_values_are_refused_and_change_nothing() {
    let mut b = backend(1);
    let body = key(1);
    let nan = at(f64::NAN, 0.0, 0.0);
    let refused = |b: &mut dyn PhysicsBackend,
                   transform: &Transform,
                   collider: Option<&Collider>| {
        let desc =
            BodyDesc { body: Some(floating()), collider, transform, velocity: None, frozen: false };
        b.create_body(body, &desc).map(|_| ()).map_err(|error| error.kind())
    };
    assert_eq!(refused(&mut *b, &nan, Some(&ball())), Err(ErrorKind::InvalidData));
    let unnormalised = Transform::new(Vec3::ZERO, Quat::new(s(1.0), s(1.0), s(0.0), s(0.0)));
    assert_eq!(refused(&mut *b, &unnormalised, Some(&ball())), Err(ErrorKind::InvalidData));
    let flat = Collider::new(Shape::sphere(Scalar::ZERO));
    assert_eq!(refused(&mut *b, &Transform::IDENTITY, Some(&flat)), Err(ErrorKind::InvalidData));
    let empty = BodyDesc {
        body: None,
        collider: None,
        transform: &Transform::IDENTITY,
        velocity: None,
        frozen: false,
    };
    assert_eq!(
        b.create_body(body, &empty).map_err(|e| e.kind()).err(),
        Some(ErrorKind::InvalidData)
    );
    let empty_bytes = snapshot(&*b);
    // Nothing was held: the key is free.
    assert_eq!(refused(&mut *b, &Transform::IDENTITY, Some(&ball())), Ok(()));
    assert_eq!(refused(&mut *b, &Transform::IDENTITY, Some(&ball())), Err(ErrorKind::InvalidData));
    assert_ne!(snapshot(&*b), empty_bytes);

    step(&mut *b, 1, &[]);
    let bytes = snapshot(&*b);
    b.set_transform(body, &nan);
    b.set_velocity(body, &Velocity { linear: v(f64::INFINITY, 0.0, 0.0), angular: Vec3::ZERO });
    b.set_collider(body, Some(&flat));
    let mut negative = floating();
    negative.linear_damping = -Scalar::ONE;
    b.set_body(body, Some(&negative));
    assert_eq!(snapshot(&*b), bytes);
    // A step of no length or an invalid gravity does nothing.
    let mut output = StepOutput::default();
    for (dt, gravity) in [(Scalar::ZERO, GRAVITY), (s(1.0 / 60.0), v(f64::NAN, 0.0, 0.0))] {
        let input = StepInput { tick: lactose_core::Tick(2), dt, gravity, commands: &[] };
        b.step(&input, &mut output);
    }
    assert!(output.bodies.is_empty());
    assert_eq!(snapshot(&*b), bytes);
}

#[test]
fn joints_wait_for_nothing_and_hold_their_bodies_together() {
    let mut b = backend(2);
    let (a, c, joint) = (key(1), key(2), key(3));
    create(&mut *b, a, Some(floating()), Some(ball()), at(0.0, 0.0, 0.0));
    create(&mut *b, c, Some(floating()), Some(ball()), at(2.0, 0.0, 0.0));
    let fixed =
        Joint::new(JointKind::Fixed, a, c).with_anchors(v(1.0, 0.0, 0.0), v(-1.0, 0.0, 0.0));
    let missing = b.create_joint(joint, &fixed, a, key(9)).unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::NotFound);
    assert_eq!(b.create_joint(joint, &fixed, a, a).unwrap_err().kind(), ErrorKind::InvalidData);
    b.create_joint(joint, &fixed, a, c).unwrap();
    assert_eq!(b.create_joint(joint, &fixed, a, c).unwrap_err().kind(), ErrorKind::InvalidData);
    let push = PhysicsCommand::ApplyImpulse { body: a, impulse: v(0.0, 1.0, 0.0) };
    let mut out = step(&mut *b, 1, &[push]);
    for tick in 2..30 {
        out = step(&mut *b, tick, &[]);
    }
    let (moved_a, moved_c) = (state(&out, a).unwrap(), state(&out, c).unwrap());
    assert!(moved_c.transform.translation.y > Scalar::ZERO, "{moved_c:?}");
    let gap = moved_c.transform.translation - moved_a.transform.translation;
    assert!((gap.length_squared() - s(4.0)).abs() < s(1e-3), "{gap:?}");

    // Removing a body removes its joints: the other body is free.
    b.remove_body(c);
    b.remove_joint(joint);
    step(&mut *b, 30, &[]);
}

/// Review M2: the axis is checked only where the joint reads it; the limits are checked in
/// order whatever the kind.
#[test]
fn a_joint_axis_is_checked_only_where_it_is_read_and_limits_must_be_in_order() {
    let mut b = backend(1);
    let (a, c) = (key(1), key(2));
    create(&mut *b, a, Some(floating()), Some(ball()), at(0.0, 0.0, 0.0));
    create(&mut *b, c, Some(floating()), Some(ball()), at(2.0, 0.0, 0.0));
    let made = |b: &mut dyn PhysicsBackend, joint: u64, kind: JointKind, axis: Vec3| {
        let row = Joint::new(kind, a, c).with_axis(axis);
        b.create_joint(key(joint), &row, a, c).map(|_| ()).map_err(|error| error.kind())
    };
    let mut joint = 10;
    for kind in [JointKind::Fixed, JointKind::Spherical] {
        joint += 1;
        assert_eq!(made(&mut *b, joint, kind, Vec3::ZERO), Ok(()), "{kind:?}");
    }
    for kind in [JointKind::Revolute, JointKind::Prismatic] {
        joint += 1;
        assert_eq!(made(&mut *b, joint, kind, Vec3::ZERO), Err(ErrorKind::InvalidData), "{kind:?}");
        assert_eq!(made(&mut *b, joint, kind, Vec3::Y), Ok(()), "{kind:?}");
    }
    for kind in [JointKind::Fixed, JointKind::Spherical, JointKind::Revolute, JointKind::Prismatic]
    {
        joint += 1;
        let reversed = Joint::new(kind, a, c).with_limits(s(0.5), s(-0.5));
        let refused = b.create_joint(key(joint), &reversed, a, c).unwrap_err();
        assert_eq!(refused.kind(), ErrorKind::InvalidData, "{kind:?}");
    }
    let mut out = step(&mut *b, 1, &[]);
    for tick in 2..10 {
        out = step(&mut *b, tick, &[]);
    }
    assert!(state(&out, a).is_some());
}

#[test]
fn a_limited_revolute_joint_stops_a_swinging_body_at_its_limit() {
    let swing = |limited: bool| {
        let mut b = backend(1);
        let (pivot, arm, joint) = (key(1), key(2), key(3));
        create(&mut *b, pivot, None, Some(ball()), at(0.0, 0.0, 0.0));
        create(&mut *b, arm, Some(RigidBody::dynamic()), Some(ball()), at(2.0, 0.0, 0.0));
        let mut hinge = Joint::new(JointKind::Revolute, pivot, arm)
            .with_anchors(Vec3::ZERO, v(-2.0, 0.0, 0.0))
            .with_axis(v(0.0, 0.0, 1.0));
        if limited {
            hinge = hinge.with_limits(s(-0.5), s(0.5));
        }
        b.create_joint(joint, &hinge, pivot, arm).unwrap();
        let mut lowest = Scalar::ZERO;
        for tick in 1..120 {
            if let Some(arm) = state(&step(&mut *b, tick, &[]), arm) {
                let y = arm.transform.translation.y;
                if y < lowest {
                    lowest = y;
                }
            }
        }
        lowest
    };
    // Two units from the pivot at an angle of at most one half radian: no lower than
    // -2 sin(0.5), about -0.96; unlimited it swings to -2.
    assert!(swing(true) > s(-1.1), "{:?}", swing(true));
    assert!(swing(false) < s(-1.9), "{:?}", swing(false));
}
