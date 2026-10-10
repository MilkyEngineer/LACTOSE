//! Joints (`README.md`, "Joints"): a joint row spawned before its bodies waits and is
//! created when they exist; a body's removal removes it and it waits again; an emptied
//! reference parks it until its row changes; a change of its value recreates it; the
//! handle row is present exactly while it is in the solver; a joint the backend refuses
//! waits for its row to change.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::{EntityRef, StableKey};
use lactose_ecs::{OwnedBy, WorldKind};
use lactose_math::Scalar;
use lactose_physics_api::{Collider, Joint, JointHandle, JointKind, RigidBody};

const A: u64 = 1;
const B: u64 = 2;
const J: u64 = 3;
const CONTROLLER: u64 = 4;

fn joint() -> Joint {
    Joint::new(JointKind::Spherical, key(A), key(B))
}

fn created(calls: &[Call]) -> Vec<StableKey> {
    calls
        .iter()
        .filter_map(|call| match call {
            Call::CreateJoint { key, .. } => Some(*key),
            _ => None,
        })
        .collect()
}

fn has_handle(h: &Harness) -> bool {
    h.world().get::<JointHandle>(key(J)).is_some()
}

#[test]
fn a_joint_waits_for_its_bodies_and_follows_them() {
    let mut h = Harness::new(WorldKind::Server);
    h.spawn_body(key(CONTROLLER), at(0, -9, 0));
    // The joint first: its references name rows that do not exist yet, which a direct
    // spawn keeps.
    h.spawn(key(J), (joint(), OwnedBy { owner: EntityRef::new(key(A)) }));
    h = h.run(1);
    assert_eq!(created(&h.take()), []);
    assert!(!has_handle(&h));
    // A body arrives: still waiting for the other.
    h.spawn_body(key(A), at(0, 0, 0));
    h = h.run(1);
    assert_eq!(created(&h.take()), []);
    h.spawn_body(key(B), at(1, 0, 0));
    h = h.run(1);
    assert_eq!(
        sync_calls(&h.take()).last(),
        Some(&Call::CreateJoint { key: key(J), body_a: key(A), body_b: key(B) })
    );
    assert!(has_handle(&h));

    // B stops being a body: the backend removed the joint with it; the joint waits.
    h.script().at(4, When::Simulation, key(CONTROLLER), Action::RemoveBody(key(B))).at(
        4,
        When::Post,
        key(CONTROLLER),
        Action::SetCollider(key(B), sphere()),
    );
    h.world_mut().remove::<Collider>(key(B)).unwrap();
    h = h.run(1);
    assert_eq!(sync_calls(&h.take()), [Call::RemoveBody(key(B))]);
    assert!(!has_handle(&h));
    // B is a fixed collider again (its collider written in `Stage::Post`): the joint is
    // recreated at the next sync, to a fixed body.
    h = h.run(1);
    let calls = sync_calls(&h.take());
    assert!(
        matches!(calls[0], Call::CreateBody { key: k, body: None, .. } if k == key(B)),
        "{calls:?}"
    );
    assert_eq!(created(&calls), [key(J)]);
    assert!(has_handle(&h));

    // A change of the joint's value removes and recreates it.
    let limited = joint().with_limits(-Scalar::ONE, Scalar::ONE);
    h.script().at(6, When::Simulation, key(CONTROLLER), Action::SetJoint(key(J), limited));
    h = h.run(1);
    assert_eq!(
        sync_calls(&h.take()),
        [
            Call::RemoveJoint(key(J)),
            Call::CreateJoint { key: key(J), body_a: key(A), body_b: key(B) }
        ]
    );
    // The same value written again changes nothing.
    h.script().at(7, When::Simulation, key(CONTROLLER), Action::SetJoint(key(J), limited));
    h = h.run(1);
    assert_eq!(sync_calls(&h.take()), []);

    // Despawning the owner takes the joint row with it.
    h.script().at(8, When::Simulation, key(CONTROLLER), Action::Despawn(key(A)));
    h = h.run(1);
    let calls = sync_calls(&h.take());
    assert!(calls.contains(&Call::RemoveBody(key(A))));
    assert!(!h.world().contains(key(J)));
    // Either the joint row went first (`remove_joint`) or the body did (the backend took
    // the joint with it): in log order, the same on every machine, and nothing waits.
    h = h.run(2);
    assert_eq!(sync_calls(&h.take()), []);
}

/// Limits out of order are an invalid value (`PhysicsBackend`, "Validation"): the backend
/// refuses the joint, which has no handle until its row changes to a valid one.
#[test]
fn a_joint_with_limits_out_of_order_is_refused_until_its_row_changes() {
    let mut h = Harness::new(WorldKind::Server);
    h.spawn_body(key(CONTROLLER), at(0, -9, 0));
    h.spawn_body(key(A), at(0, 0, 0));
    h.spawn_body(key(B), at(1, 0, 0));
    h.spawn(key(J), (joint().with_limits(Scalar::ONE, -Scalar::ONE),));
    h = h.run(1);
    assert_eq!(created(&h.take()), [key(J)]);
    assert!(!has_handle(&h));
    // Not retried while the row stays as it is.
    h = h.run(1);
    assert_eq!(created(&h.take()), []);
    let ordered = joint().with_limits(-Scalar::ONE, Scalar::ONE);
    h.script().at(3, When::Simulation, key(CONTROLLER), Action::SetJoint(key(J), ordered));
    h = h.run(1);
    assert_eq!(created(&h.take()), [key(J)]);
    assert!(has_handle(&h));
}

#[test]
fn an_emptied_reference_parks_the_joint_until_its_row_changes() {
    let mut h = Harness::new(WorldKind::Server);
    h.spawn_body(key(A), at(0, 0, 0));
    h.spawn_body(key(B), at(1, 0, 0));
    h.spawn(key(J), (joint(),));
    h = h.run(1);
    assert_eq!(created(&h.take()), [key(J)]);
    // A reference to a missing entity is written empty at apply.
    let dangling = Joint::new(JointKind::Spherical, key(99), key(B));
    h.script().at(2, When::Simulation, key(A), Action::SetJoint(key(J), dangling));
    h = h.run(1);
    assert!(h.world().get::<Joint>(key(J)).unwrap().body_a.is_none());
    assert_eq!(sync_calls(&h.take()), [Call::RemoveJoint(key(J))]);
    assert!(!has_handle(&h));
    // Parked: not retried, even though both bodies are there.
    h = h.run(3);
    assert_eq!(sync_calls(&h.take()), []);
    // Its row changes: created again.
    h.script().at(6, When::Simulation, key(A), Action::SetJoint(key(J), joint()));
    h = h.run(1);
    assert_eq!(created(&h.take()), [key(J)]);
    assert!(has_handle(&h));
}

#[test]
fn a_despawned_body_leaves_the_joint_waiting_for_its_key() {
    let mut h = Harness::new(WorldKind::Server);
    h.spawn_body(key(A), at(0, 0, 0));
    h.spawn_body(key(B), at(1, 0, 0));
    // Owned by neither body, so it outlives them.
    h.spawn(key(J), (joint(),));
    h = h.run(1);
    h.take();
    h.world_mut().despawn(key(B)).unwrap();
    h = h.run(1);
    assert_eq!(sync_calls(&h.take()), [Call::RemoveBody(key(B))]);
    assert!(!has_handle(&h));
    // The ECS empties no reference on a despawn: the joint still names B and waits for a
    // body of that key, which a reload or a re-simulation brings back.
    assert_eq!(h.world().get::<Joint>(key(J)).unwrap().body_b.get(), Some(key(B)));
    h = h.run(2);
    assert_eq!(sync_calls(&h.take()), []);
    h.spawn(key(B), (at(1, 0, 0), RigidBody::kinematic(), sphere()));
    h = h.run(1);
    assert_eq!(created(&h.take()), [key(J)]);
    assert!(has_handle(&h));
}
