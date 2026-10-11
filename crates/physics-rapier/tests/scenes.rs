//! `physics-api`'s own test scenes, through its sync, step and write-back, with this
//! backend in place of the test one (the harness of `physics-api`'s test support, by path).
//! Rapier resolves overlaps and integrates as the test backend does not, so these assert on
//! structure: who touched whom and in which order, which rows the write-back wrote, signs;
//! and on numbers only where they are exact: a teleported pose, a velocity nothing acts on.

#![forbid(unsafe_code)]

#[path = "../../physics-api/tests/common/mod.rs"]
mod common;

use common::*;
use lactose_core::{EntityRef, StableKey};
use lactose_ecs::{OwnedBy, WorldKind};
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::{
    Collider, CollisionGroups, ContactPhase, Joint, JointHandle, JointKind, PhysicsCommand,
    QueryFilter, Ray, Shape, Teleported, Transform, Velocity,
};

fn rapier(threads: usize) -> Box<dyn lactose_physics_api::PhysicsBackend> {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
    lactose_physics_rapier::backend(pool)
}

fn harness(threads: usize) -> Harness {
    Options::new(WorldKind::Server).build_with(rapier(threads), CallLog::default())
}

fn at_x(x: f64) -> Transform {
    Transform::at(Vec3::new(Scalar::from_f64(x), Scalar::ZERO, Scalar::ZERO))
}

type Seen = Vec<(u32, StableKey, StableKey, ContactPhase)>;

/// `physics-api`'s event scene: A with `events` at the origin, B sliding away from it
/// while touching, C resting on it, a sensor overlapping D.
fn contacts(threads: usize) -> (Seen, Seen) {
    let mut h = harness(threads);
    let (a, b, c, sensor, d) = (key(1), key(2), key(3), key(4), key(5));
    let reporting = Collider::new(Shape::sphere(Scalar::HALF)).with_events(Scalar::HALF);
    let sliding = Velocity { linear: v(6, 0, 0), angular: Vec3::ZERO };
    h.spawn(a, (Transform::IDENTITY, Velocity::ZERO, floating(), reporting));
    h.spawn(b, (at_x(0.65), sliding, floating(), sphere()));
    let above = Transform::at(Vec3::new(Scalar::ZERO, Scalar::from_f64(0.6), Scalar::ZERO));
    h.spawn(c, (above, Velocity::ZERO, floating(), sphere()));
    h.spawn(sensor, (at_x(10.0), Collider::new(Shape::sphere(Scalar::HALF)).sensor()));
    h.spawn(d, (at_x(10.5), Velocity::ZERO, floating(), sphere()));
    h = h.run(6);
    let recorded = h.recorded();
    let contacts =
        recorded.contacts.iter().map(|(t, e)| (*t, e.target, e.other, e.phase)).collect();
    let triggers =
        recorded.triggers.iter().map(|(t, e)| (*t, e.target, e.other, e.phase)).collect();
    (contacts, triggers)
}

#[test]
fn events_reach_the_right_targets_sorted_and_once() {
    use ContactPhase::{Impact, Started};
    let (a, b, c, sensor, d) = (key(1), key(2), key(3), key(4), key(5));
    let (contacts_seen, triggers_seen) = contacts(1);
    // Each tick's events are emitted ascending by (target, other, phase), each once.
    for tick in 1..=6 {
        let events: Vec<_> =
            contacts_seen.iter().filter(|e| e.0 == tick).map(|e| (e.1, e.2, e.3)).collect();
        let mut sorted = events.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(events, sorted, "tick {tick}");
    }
    // A has `events`: both sides of A's pairs start at the first tick; B and C touch too,
    // but neither has `events`, so nothing is said of them.
    let first: Vec<_> =
        contacts_seen.iter().filter(|e| e.0 == 1 && e.3 == Started).map(|e| (e.1, e.2)).collect();
    assert_eq!(first, [(a, b), (a, c), (b, a), (c, a)]);
    assert!(!contacts_seen.iter().any(|e| (e.1, e.2) == (b, c) || (e.1, e.2) == (c, b)));
    // Impacts only to A, whose collider has `events` (Rapier separates the overlapping
    // spawns by position correction, so whether an impulse passes the threshold of one half
    // is its own; `events.rs` drops bodies hard enough to see one).
    assert!(contacts_seen.iter().filter(|e| e.3 == Impact).all(|e| e.1 == a));
    // The sensor overlaps D from the first step.
    assert_eq!(
        triggers_seen.iter().filter(|e| e.0 == 1).map(|e| (e.1, e.2, e.3)).collect::<Vec<_>>(),
        [(sensor, d, Started), (d, sensor, Started)]
    );
    // The emission does not depend on the solver's pool.
    assert_eq!(contacts(4), (contacts_seen, triggers_seen));
}

#[test]
fn queries_answer_through_physics_with_ties_to_the_lowest_key() {
    let mut h = harness(2);
    let (near, tied_high, tied_low, sensor, other_group) = (key(1), key(9), key(4), key(2), key(3));
    h.spawn(tied_high, (at(0, 0, 10), sphere()));
    h.spawn(tied_low, (at(0, 0, 10), sphere()));
    h.spawn(sensor, (at(0, 0, 4), sphere().sensor()));
    h.spawn(other_group, (at(0, 0, 6), sphere().with_groups(CollisionGroups::new(2, u32::MAX))));
    h.spawn(near, (at(0, 0, 20), sphere()));
    h = h.run(1);
    let physics = h.physics();
    assert_eq!(physics.body_count(), 5);
    let ray = Ray::new(v(0, 0, 0), v(0, 0, 1), Scalar::from_i32(100));
    let only_one = QueryFilter { groups: CollisionGroups::new(1, 1), ..QueryFilter::ALL };
    let hit = physics.cast_ray(&ray, &only_one).unwrap();
    assert_eq!(hit.body, tied_low);
    assert!((hit.distance - Scalar::from_f64(9.5)).abs() < Scalar::from_f64(1e-12));
    let without = QueryFilter { exclude: Some(tied_low), ..only_one };
    assert_eq!(physics.cast_ray(&ray, &without).unwrap().body, tied_high);
    assert_eq!(physics.cast_ray(&ray, &QueryFilter::ALL).unwrap().body, other_group);
    let sensors = QueryFilter { sensors: true, ..QueryFilter::ALL };
    assert_eq!(physics.cast_ray(&ray, &sensors).unwrap().body, sensor);
    let mut hits = Vec::new();
    physics.overlap(
        &Shape::sphere(Scalar::from_i32(30)),
        &Transform::IDENTITY,
        &QueryFilter::ALL,
        &mut hits,
    );
    assert_eq!(hits, [near, other_group, tied_low, tied_high]);
}

#[test]
fn the_write_back_writes_what_rapier_stepped() {
    let mut h = harness(1);
    let (pushed, teleported, resting) = (key(1), key(2), key(3));
    h.spawn_body(pushed, at(0, 0, 0));
    h.spawn_body(teleported, at(5, 0, 0));
    h.spawn_body(resting, at(10, 0, 0));
    let target = Transform::at(v(5, 7, -3));
    h.script()
        .at(
            2,
            When::Simulation,
            pushed,
            Action::Command(PhysicsCommand::ApplyImpulse { body: pushed, impulse: v(1, 0, 0) }),
        )
        .at(
            2,
            When::Simulation,
            teleported,
            Action::Command(PhysicsCommand::Teleport { body: teleported, transform: target }),
        );
    h = h.run(3);
    let velocity = *h.world().get::<Velocity>(pushed).unwrap();
    assert!(velocity.linear.x > Scalar::ZERO, "{velocity:?}");
    assert_eq!(
        (velocity.linear.y, velocity.linear.z, velocity.angular),
        (Scalar::ZERO, Scalar::ZERO, Vec3::ZERO)
    );
    assert!(h.world().get::<Transform>(pushed).unwrap().translation.x > Scalar::ZERO);
    // Placed exactly, no gravity and nothing else acting on it, and flagged with the tick
    // whose step applied the command.
    assert_eq!(*h.world().get::<Transform>(teleported).unwrap(), target);
    assert_eq!(h.world().get::<Teleported>(teleported).map(|t| t.0.0), Some(2));
    // Untouched: where it was spawned, at rest.
    assert_eq!(*h.world().get::<Transform>(resting).unwrap(), at(10, 0, 0));
    assert_eq!(*h.world().get::<Velocity>(resting).unwrap(), Velocity::ZERO);
}

#[test]
fn a_joint_waits_for_its_bodies_and_is_in_the_solver_while_both_exist() {
    let mut h = harness(1);
    let (a, b, joint, controller) = (key(1), key(2), key(3), key(4));
    let has_handle = |h: &Harness| h.world().get::<JointHandle>(joint).is_some();
    h.spawn_body(controller, at(0, -9, 0));
    let hinge = Joint::new(JointKind::Revolute, a, b)
        .with_anchors(v(1, 0, 0), v(-1, 0, 0))
        .with_axis(v(0, 0, 1))
        .with_limits(-Scalar::HALF, Scalar::HALF);
    h.spawn(joint, (hinge, OwnedBy { owner: EntityRef::new(a) }));
    h = h.run(1);
    assert!(!has_handle(&h));
    h.spawn_body(a, at(0, 0, 0));
    h = h.run(1);
    assert!(!has_handle(&h));
    h.spawn_body(b, at(2, 0, 0));
    h = h.run(1);
    assert!(has_handle(&h));
    // Pushed apart at A, B follows: the joint holds.
    let push = PhysicsCommand::ApplyImpulse { body: a, impulse: v(0, 2, 0) };
    h.script().at(4, When::Simulation, controller, Action::Command(push));
    h = h.run(10);
    let ya = h.world().get::<Transform>(a).unwrap().translation.y;
    let yb = h.world().get::<Transform>(b).unwrap().translation.y;
    assert!(ya > Scalar::ZERO && yb > Scalar::ZERO, "{ya:?} {yb:?}");
    // B stops being a body: the joint leaves the solver and waits.
    h.script().at(14, When::Simulation, controller, Action::RemoveBody(b));
    h.world_mut().remove::<Collider>(b).unwrap();
    h = h.run(1);
    assert!(!has_handle(&h));
    // Despawning the owner takes the joint row with it; nothing is left waiting.
    h.script().at(15, When::Simulation, controller, Action::Despawn(a));
    h = h.run(2);
    assert!(!h.world().contains(joint));
    assert_eq!(h.physics().body_count(), 1);
}
