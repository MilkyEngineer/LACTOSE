//! The physics around a rollback, with the physics test backend recording its calls:
//! every restore is the world's then the physics', a correction tells the sync which rows
//! it wrote, `Teleported` is never compared, a proxy is swept along its history, and a slot
//! whose physics snapshot failed restores the world alone.

#![forbid(unsafe_code)]

mod common;

use std::sync::{Arc, Mutex};

use common::fake::{Call, CallLog, Fake};
use common::*;
use lactose_core::{StableKey, Tick};
use lactose_ecs::LayoutConfig;
use lactose_input::PlayerInput;
use lactose_net_api::StagedFrame;
use lactose_physics_api::{PhysicsBackend, Teleported, Transform, Velocity};

fn body(n: u64) -> StableKey {
    key(n)
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
    exact(&mut frame, body(5), &transform(3));
    exact(&mut frame, body(6), &transform(10));
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
fn a_corrected_transform_and_the_servers_unchanged_teleported_make_no_second_correction() {
    let (mut rig, _calls) = physics_rig();
    rig.step_to(20, walking);
    let mut frame = StagedFrame::new(Tick(15));
    exact(&mut frame, body(5), &transform(3));
    exact(&mut frame, body(5), &Teleported(Tick(2)));
    rig.push(frame);
    rig.step(walking(21));
    let first = rig.reconcile();
    assert_eq!(first.corrections.len(), 1);
    assert_eq!(first.corrections[0].divergence.fields, 1, "the transform alone differed");
    // The placement the correction caused is a teleport on the client at 16, which the
    // server never made.
    assert_eq!(rig.world().get::<Teleported>(body(5)), Some(&Teleported(Tick(16))));
    let mut frame = StagedFrame::new(Tick(17));
    exact(&mut frame, body(5), &transform(3));
    exact(&mut frame, body(5), &Teleported(Tick(2)));
    exact(&mut frame, body(5), &Velocity::ZERO);
    rig.push(frame);
    rig.step(walking(22));
    let second = rig.reconcile();
    assert!(second.corrections.is_empty(), "{:?}", summaries(&second));
    // And a correction never writes it.
    assert_eq!(rig.world().get::<Teleported>(body(5)), Some(&Teleported(Tick(16))));
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
    exact(&mut frame, body(5), &transform(4));
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
