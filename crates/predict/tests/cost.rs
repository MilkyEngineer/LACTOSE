//! What a slot and a rollback cost (`README.md`, "Proof for milestone 1"): the bytes and
//! the time of `World::snapshot` and `physics_api::snapshot` for fifty claimed bodies plus
//! 0 to 10,000 statics with the physics test backend, the record's bytes beside them, and
//! the time of a re-simulated tick. Asserted on what does not depend on the machine (the
//! bytes grow with the statics, the record does not); the times are measured by an ignored
//! test, run by hand, and reported, never asserted.

#![forbid(unsafe_code)]

mod common;

use common::fake::{CallLog, Fake};
use common::*;
use lactose_core::{StableKey, Tick};
use lactose_ecs::LayoutConfig;
use lactose_net_api::StagedFrame;
use lactose_physics_api::{RigidBody, Velocity};

const CLAIMED: u64 = 50;
/// The first static's key: after the claimed bodies and the join frame's three.
const FIRST_STATIC: u64 = 1_000;

fn body(n: u64) -> StableKey {
    key(100 + n)
}

/// A joined prediction world with the test backend, fifty claimed floating bodies in a row
/// and `statics` fixed proxies.
fn world_with(statics: u32) -> Rig {
    let backend = Box::new(Fake::new(CallLog::default()).without_contacts());
    let mut rig = Rig::new(simulation(LayoutConfig::default(), Some(backend)), 1);
    let mut frame = join_frame(JOIN);
    for n in 0..CLAIMED {
        let x = i32::try_from(n).unwrap() * 3;
        let parts =
            vec![data(transform(x)), data(Velocity::ZERO), data(floating()), data(sphere())];
        frame.spawns.insert(net(100 + n), claimed(REGION, parts));
    }
    for index in 0..statics {
        let x = -10 - i32::try_from(index).unwrap();
        let parts = vec![data(transform(x)), data(RigidBody::fixed()), data(sphere())];
        frame.spawns.insert(net(FIRST_STATIC + u64::from(index)), remote(REGION, index + 1, parts));
    }
    rig.push(frame);
    rig.step(lactose_input::PlayerInput::IDLE);
    assert!(rig.reconcile().corrections.is_empty());
    rig
}

#[test]
fn the_slot_bytes_grow_with_the_statics_and_the_record_does_not() {
    let mut sizes = Vec::new();
    for statics in [0, 1_000] {
        let mut rig = world_with(statics);
        rig.step(walking(12));
        let slot = rig.p().ring().slot_bytes(Tick(12)).unwrap();
        assert!(slot.physics > 0);
        sizes.push(slot);
    }
    assert!(sizes[1].world > sizes[0].world);
    assert!(sizes[1].physics > sizes[0].physics);
    assert_eq!(sizes[1].record, sizes[0].record, "the record holds the claimed rows alone");
}

/// The bytes and times of a slot's parts and of a rollback, printed for the report. Run
/// with `cargo nextest run -p lactose-predict --run-ignored only --cargo-profile release
/// --no-capture -E 'test(cost_)'`.
#[test]
#[ignore = "a measurement, run by hand: timings are reported, not asserted"]
#[expect(clippy::disallowed_types, reason = "test code measuring the ring, outside the step")]
fn cost_of_a_slot_and_a_rollback() {
    use std::time::Instant;
    const ROUNDS: u32 = 200;
    const ROLLBACKS: u32 = 20;
    const DEPTH: u32 = 6;
    for statics in [0, 1_000, 10_000] {
        let mut rig = world_with(statics);
        rig.step_to(40, walking);
        let world = rig.world();
        let start = Instant::now();
        let mut world_bytes = 0;
        for _ in 0..ROUNDS {
            world_bytes = std::hint::black_box(world.snapshot()).size_in_bytes();
        }
        let world_us = start.elapsed().as_secs_f64() * 1e6 / f64::from(ROUNDS);
        let start = Instant::now();
        let mut physics_bytes = 0;
        for _ in 0..ROUNDS {
            physics_bytes =
                std::hint::black_box(lactose_physics_api::snapshot(world).unwrap()).size_in_bytes();
        }
        let physics_us = start.elapsed().as_secs_f64() * 1e6 / f64::from(ROUNDS);
        let slot = rig.p().ring().slot_bytes(Tick(40)).unwrap();

        // A predicted tick, frames none: the step, the slot's snapshots and its record.
        let start = Instant::now();
        rig.step_to(40 + ROUNDS, walking);
        let step_us = start.elapsed().as_secs_f64() * 1e6 / f64::from(ROUNDS);

        // A correction `DEPTH` ticks back, each frame moving the first body elsewhere.
        let mut reconcile_us = 0.0;
        let mut resimulated = 0;
        for round in 0..ROLLBACKS {
            let present = rig.tick().0;
            let mut frame = StagedFrame::new(Tick(present - DEPTH));
            let x = 1_000 + i32::try_from(round).unwrap();
            exact(&mut frame, body(0), &transform(x));
            rig.push(frame);
            rig.step(walking(present + 1));
            let start = Instant::now();
            let reconciled = rig.reconcile();
            reconcile_us += start.elapsed().as_secs_f64() * 1e6;
            assert_eq!(reconciled.corrections.len(), 1);
            resimulated += reconciled.resimulated;
        }
        let per_tick = reconcile_us / f64::from(resimulated);
        println!(
            "statics {statics:>6}: world snapshot {world_bytes:>9} B {world_us:>9.1} us; \
             physics snapshot {physics_bytes:>8} B {physics_us:>8.1} us; record {} B; \
             slot {} + {} + {} B; predicted tick {step_us:>9.1} us; rollback of {} ticks \
             {:>9.1} us, {per_tick:>8.1} us per re-simulated tick",
            slot.record,
            slot.world,
            slot.physics,
            slot.record,
            resimulated / ROLLBACKS,
            reconcile_us / f64::from(ROLLBACKS),
        );
    }
}
