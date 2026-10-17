//! The physics around a rollback, with the physics test backend recording its calls:
//! every restore is the world's then the physics', a correction tells the sync which rows
//! it wrote, the tolerance for a dynamic body's rows (D124), `Teleported` is never
//! compared or written, a proxy is swept along its history, and a slot whose physics
//! snapshot failed restores the world alone.

#![forbid(unsafe_code)]

mod common;

use std::sync::{Arc, Mutex};

use common::fake::{Call, CallLog, Fake};
use common::*;
use lactose_core::{StableKey, Tick};
use lactose_ecs::LayoutConfig;
use lactose_input::PlayerInput;
use lactose_math::{Scalar, Vec3};
use lactose_net_api::StagedFrame;
use lactose_physics_api::{PhysicsBackend, RigidBody, Teleported, Transform, Velocity};
use lactose_predict::Absorbed;

fn body(n: u64) -> StableKey {
    key(n)
}

/// A transform at `(x, y, 0)`.
fn placed(x: f64, y: f64) -> Transform {
    Transform::at(Vec3::new(Scalar::from_f64(x), Scalar::from_f64(y), Scalar::ZERO))
}

/// A difference as `Absorbed` stores it.
fn bits(difference: f64) -> u64 {
    #[allow(clippy::useless_conversion, reason = "`ScalarBits` is `u32` under the 32-bit scalar")]
    let bits = u64::from(Scalar::from_f64(difference).to_canonical_bits());
    bits
}

/// Stages a claimed body's exact rows, whole as the server sends them: `transform`, at
/// rest, `body`, a sphere.
fn whole(frame: &mut StagedFrame, key: StableKey, transform: &Transform, body: &RigidBody) {
    exact(frame, key, transform);
    exact(frame, key, &Velocity::ZERO);
    exact(frame, key, body);
    exact(frame, key, &sphere());
}

/// The join frame with two claimed floating bodies at 0 and 10, the first teleported at
/// tick 2 on the server, and a proxy body at -10.
fn bodies_frame(tick: Tick) -> StagedFrame {
    let mut frame = join_frame(tick);
    let parts =
        |x: i32| vec![data(transform(x)), data(Velocity::ZERO), data(floating()), data(sphere())];
    let mut first = parts(0);
    first.push(data(Teleported(Tick(2))));
    frame.spawns.insert(net(5), claimed(REGION, first));
    frame.spawns.insert(net(6), claimed(REGION, parts(10)));
    frame.spawns.insert(net(7), remote(REGION, 4, parts(-10)));
    frame
}

fn rig_with(backend: Box<dyn PhysicsBackend>) -> Rig {
    let mut rig = Rig::new(simulation(LayoutConfig::default(), Some(backend)), 1);
    rig.push(bodies_frame(JOIN));
    rig.step(PlayerInput::IDLE);
    rig.reconcile();
    rig
}

fn physics_rig() -> (Rig, CallLog) {
    let calls = CallLog::default();
    let rig = rig_with(Box::new(Fake::new(calls.clone()).without_contacts()));
    (rig, calls)
}

fn transform_of(rig: &Rig, key: StableKey) -> Transform {
    *rig.world().get::<Transform>(key).expect("a body row")
}

/// The calls that are neither steps, snapshots nor restores: what the sync pushed.
fn pushed(calls: &[Call]) -> Vec<Call> {
    calls
        .iter()
        .filter(|call| !matches!(call, Call::Step { .. } | Call::Snapshot | Call::Restore))
        .cloned()
        .collect()
}

fn count(calls: &[Call], wanted: &Call) -> usize {
    calls.iter().filter(|call| *call == wanted).count()
}

#[test]
fn a_correction_restores_both_worlds_and_pushes_exactly_the_written_rows() {
    let (mut rig, calls) = physics_rig();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(15));
    whole(&mut frame, body(5), &transform(3), &floating());
    whole(&mut frame, body(6), &transform(10), &floating());
    rig.push(frame);
    rig.step(walking(21));
    let _ = calls.take();
    let reconciled = rig.reconcile();
    assert_eq!(summaries(&reconciled).len(), 1);
    assert_eq!(reconciled.corrections[0].entities, vec![body(5)]);
    let during = calls.take();
    // One restore, of slot 15; a snapshot for the corrected slot and each re-simulated one.
    assert_eq!(count(&during, &Call::Restore), 1);
    assert_eq!(count(&during, &Call::Snapshot), 1 + 6);
    assert_eq!(during.iter().filter(|call| matches!(call, Call::Step { .. })).count(), 6);
    // What the sync pushed after the correction: the corrected body's rows (the five
    // columns `corrected` names, an equal value included, which the backend ignores),
    // nothing of the body the frame agreed with or of the rest.
    let pushed = pushed(&during);
    assert!(pushed.contains(&Call::SetTransform(body(5), transform(3))), "{pushed:?}");
    assert!(pushed.iter().all(|call| call.key() == Some(body(5))), "{pushed:?}");
    assert_eq!(transform_of(&rig, body(5)), transform(3));
    assert_eq!(transform_of(&rig, body(6)), transform(10));
}

#[test]
fn a_teleported_difference_is_not_a_mismatch_and_is_not_written() {
    let (mut rig, _calls) = physics_rig();
    rig.step_to(20, walking);
    // The server's `Teleported` alone differs from the client's, which has none: no fact.
    let mut frame = StagedFrame::new(Tick(13));
    whole(&mut frame, body(6), &transform(10), &floating());
    exact(&mut frame, body(6), &Teleported(Tick(12)));
    rig.push(frame);
    rig.step(walking(21));
    assert!(rig.reconcile().corrections.is_empty());
    assert_eq!(rig.world().get::<Teleported>(body(6)), None);

    let mut frame = StagedFrame::new(Tick(15));
    whole(&mut frame, body(5), &transform(3), &floating());
    exact(&mut frame, body(5), &Teleported(Tick(2)));
    rig.push(frame);
    rig.step(walking(22));
    let first = rig.reconcile();
    assert_eq!(first.corrections.len(), 1);
    assert_eq!(first.corrections[0].divergence.fields, 1, "the transform alone differed");
    // The placement the correction caused is a teleport on the client at 16, which the
    // server never made.
    assert_eq!(rig.world().get::<Teleported>(body(5)), Some(&Teleported(Tick(16))));
    let mut frame = StagedFrame::new(Tick(17));
    whole(&mut frame, body(5), &transform(3), &floating());
    exact(&mut frame, body(5), &Teleported(Tick(2)));
    rig.push(frame);
    rig.step(walking(23));
    let second = rig.reconcile();
    assert!(second.corrections.is_empty(), "{:?}", summaries(&second));
    // And a correction never writes it.
    assert_eq!(rig.world().get::<Teleported>(body(5)), Some(&Teleported(Tick(16))));
}

#[test]
fn a_dynamic_bodys_transform_within_the_tolerance_makes_no_correction_and_is_counted() {
    let (mut rig, calls) = physics_rig();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(15));
    whole(&mut frame, body(5), &placed(0.0015, 0.0), &floating());
    whole(&mut frame, body(6), &transform(10), &floating());
    rig.push(frame);
    let mut frame = StagedFrame::new(Tick(16));
    whole(&mut frame, body(5), &placed(0.0, -0.001), &floating());
    rig.push(frame);
    rig.step(walking(21));
    let _ = calls.take();
    let reconciled = rig.reconcile();
    assert!(reconciled.corrections.is_empty(), "{:?}", summaries(&reconciled));
    assert_eq!(pushed(&calls.take()), vec![]);
    // Two facts absorbed, one translation field each; the larger difference kept.
    let absorbed = Absorbed { fields: 2, translation: bits(0.0015), ..Absorbed::default() };
    assert_eq!(rig.p().stats().absorbed, absorbed);
    assert_eq!(transform_of(&rig, body(5)), transform(0));
}

#[test]
fn one_leaf_beyond_the_tolerance_rolls_back_and_writes_the_absorbed_facts_too() {
    let (mut rig, calls) = physics_rig();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(15));
    whole(&mut frame, body(5), &placed(0.001, 0.0), &floating());
    whole(&mut frame, body(6), &placed(10.0, 0.003), &floating());
    rig.push(frame);
    rig.step(walking(21));
    let _ = calls.take();
    let reconciled = rig.reconcile();
    // The body beyond the tolerance alone differed, in one field; both were written.
    assert_eq!(summaries(&reconciled).len(), 1);
    let correction = &reconciled.corrections[0];
    assert_eq!((correction.tick, correction.divergence.entities), (Tick(15), 1));
    assert_eq!(correction.divergence.fields, 1);
    assert_eq!(correction.entities, vec![body(5), body(6)]);
    let pushed = pushed(&calls.take());
    assert!(pushed.contains(&Call::SetTransform(body(5), placed(0.001, 0.0))), "{pushed:?}");
    assert!(pushed.contains(&Call::SetTransform(body(6), placed(10.0, 0.003))), "{pushed:?}");
    assert_eq!(transform_of(&rig, body(5)), placed(0.001, 0.0));
    assert_eq!(transform_of(&rig, body(6)), placed(10.0, 0.003));
    // Counted once, at the comparison before the rollback.
    let absorbed = Absorbed { fields: 1, translation: bits(0.001), ..Absorbed::default() };
    assert_eq!(rig.p().stats().absorbed, absorbed);
}

#[test]
fn a_kinematic_or_fixed_bodys_rows_agree_by_bytes_only() {
    let (mut rig, _calls) = physics_rig();
    rig.world_mut().insert(body(5), RigidBody::kinematic()).unwrap();
    rig.world_mut().insert(body(6), RigidBody::fixed()).unwrap();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(15));
    whole(&mut frame, body(5), &placed(0.001, 0.0), &RigidBody::kinematic());
    whole(&mut frame, body(6), &placed(10.0, 0.001), &RigidBody::fixed());
    rig.push(frame);
    rig.step(walking(21));
    let reconciled = rig.reconcile();
    assert_eq!(summaries(&reconciled).len(), 1);
    assert_eq!(reconciled.corrections[0].entities, vec![body(5), body(6)]);
    assert_eq!(reconciled.corrections[0].divergence.fields, 2);
    assert_eq!(rig.p().stats().absorbed, Absorbed::default());
}

#[test]
fn a_body_dynamic_at_the_tick_before_and_kinematic_at_its_tick_takes_the_tolerance() {
    let (mut rig, _calls) = physics_rig();
    rig.step_to(14, walking);
    // Slot 14 holds it dynamic; 15 and later kinematic.
    rig.world_mut().insert(body(5), RigidBody::kinematic()).unwrap();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(15));
    whole(&mut frame, body(5), &placed(0.001, 0.0), &RigidBody::kinematic());
    rig.push(frame);
    // Kinematic at 15 and 16: by bytes.
    let mut frame = StagedFrame::new(Tick(16));
    whole(&mut frame, body(5), &placed(0.001, 0.0), &RigidBody::kinematic());
    rig.push(frame);
    rig.step(walking(21));
    let reconciled = rig.reconcile();
    assert_eq!(summaries(&reconciled).len(), 1);
    assert_eq!(reconciled.corrections[0].tick, Tick(16));
    let absorbed = Absorbed { fields: 1, translation: bits(0.001), ..Absorbed::default() };
    assert_eq!(rig.p().stats().absorbed, absorbed);
}

#[test]
fn a_body_with_no_slot_before_its_tick_is_judged_at_its_tick_alone() {
    let (mut rig, _calls) = physics_rig();
    rig.step_to(14, walking);
    // Slot 11 is the ring's restart: there is no slot 10.
    assert_eq!(rig.p().ring().oldest(), Some(Tick(11)));
    let mut frame = StagedFrame::new(Tick(11));
    whole(&mut frame, body(5), &placed(0.001, 0.0), &floating());
    // A kinematic body the record holds dynamic: the frame's type at 11 alone decides, so
    // by bytes.
    whole(&mut frame, body(6), &placed(10.0, 0.001), &RigidBody::kinematic());
    rig.push(frame);
    rig.step(walking(15));
    let reconciled = rig.reconcile();
    assert_eq!(summaries(&reconciled).len(), 1);
    assert_eq!(reconciled.corrections[0].tick, Tick(11));
    assert_eq!(reconciled.corrections[0].entities, vec![body(5), body(6)]);
    // The second body's transform and two fields of its `RigidBody` (the type, and the
    // gravity scale `floating` zeroed) differ.
    assert_eq!(
        reconciled.corrections[0].divergence,
        lactose_predict::Divergence { entities: 1, fields: 3 }
    );
    let absorbed = Absorbed { fields: 1, translation: bits(0.001), ..Absorbed::default() };
    assert_eq!(rig.p().stats().absorbed, absorbed);
}

#[test]
fn a_nan_leaf_is_never_within_the_tolerance() {
    let (mut rig, _calls) = physics_rig();
    rig.step_to(20, walking);
    let nan = Transform::at(Vec3::new(Scalar::NAN, Scalar::ZERO, Scalar::ZERO));
    let mut frame = StagedFrame::new(Tick(15));
    whole(&mut frame, body(5), &nan, &floating());
    rig.push(frame);
    rig.step(walking(21));
    let reconciled = rig.reconcile();
    assert_eq!(summaries(&reconciled).len(), 1);
    assert_eq!(rig.p().stats().absorbed, Absorbed::default());
}

#[test]
fn a_proxy_is_swept_along_the_rows_the_hook_writes() {
    let (mut rig, calls) = physics_rig();
    rig.step_to(12, walking);
    let _ = calls.take();
    let mut frame = StagedFrame::new(Tick(12));
    dequantised(&mut frame, body(7), &transform(-8));
    rig.push(frame);
    rig.step(walking(13));
    let during = calls.take();
    assert_eq!(pushed(&during), vec![Call::SetTransform(body(7), transform(-8))]);
    assert_eq!(transform_of(&rig, body(7)), transform(-8));
    assert!(rig.reconcile().corrections.is_empty());
}

#[test]
fn a_slot_without_physics_restores_the_world_alone_and_is_counted() {
    let calls = CallLog::default();
    let fail = Arc::new(Mutex::new(false));
    let flaky = Flaky { inner: Fake::new(calls.clone()).without_contacts(), fail: fail.clone() };
    let mut rig = rig_with(Box::new(flaky));
    rig.step_to(13, walking);
    *fail.lock().unwrap() = true;
    rig.step(walking(14));
    *fail.lock().unwrap() = false;
    rig.step_to(20, walking);
    assert_eq!(rig.p().stats().slots_without_physics, 1);
    let ring = rig.p().ring();
    assert_eq!(ring.slot_bytes(Tick(14)).unwrap().physics, 0);
    assert!(ring.slot_bytes(Tick(15)).unwrap().physics > 0);

    let mut frame = StagedFrame::new(Tick(14));
    whole(&mut frame, body(5), &transform(4), &floating());
    rig.push(frame);
    rig.step(walking(21));
    let _ = calls.take();
    let reconciled = rig.reconcile();
    assert_eq!(reconciled.corrections[0].tick, Tick(14));
    let during = calls.take();
    assert_eq!(count(&during, &Call::Restore), 0);
    // The re-take of 14 keeps no physics part; the re-simulated slots have theirs.
    let ring = rig.p().ring();
    assert_eq!(ring.slot_bytes(Tick(14)).unwrap().physics, 0);
    assert!(ring.slot_bytes(Tick(15)).unwrap().physics > 0);
    assert_eq!(rig.p().stats().slots_without_physics, 1);
    assert_eq!(transform_of(&rig, body(5)), transform(4));
}
