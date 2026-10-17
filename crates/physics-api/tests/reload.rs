//! An empty solver is a fresh solver (`README.md`, "Bodies"; D124): a sync whose log
//! despawned every body the solver held, the pending entries a restore carried included,
//! removes them all, restores the backend's bytes from `setup` and creates from the rows by
//! a full diff; a sync whose removals empty the solver restores those bytes and makes the
//! next sync a full diff; a `Spawned` entry for a key the bookkeeping holds re-inserts its
//! handle rows. Each region is reloaded through `lactose-sim`'s frame (`World::load_region`
//! of its export), and the reloaded world is compared with a fresh process loading the same
//! bundles: the calls after `setup`, the backend's bytes, the world hash and the handles.
//! The same holds when `set_tick` made the cursor stale before the full frame (a client's
//! re-join), where the full diff reads the log retained since the epoch began, and for a
//! rollback to a slot taken then.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::{StableKey, Tick};
use lactose_ecs::{RegionConfig, RegionId, RegionMode, WorldKind};
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::{
    BodyHandle, Collider, Joint, JointHandle, JointKind, RigidBody, Shape, Transform, Velocity,
    restore, snapshot,
};
use lactose_sim::RegionBundle;

/// A fixed collider and a frozen body in `OTHER_REGION`, which is `Frozen`.
const STATIC: u64 = 1;
const FROZEN: u64 = 10;
/// A moving body in `REGION`, touching the frozen one, and a joint between the two.
const MOVING: u64 = 20;
const JOINT: u64 = 30;
const BODIES: [u64; 3] = [STATIC, FROZEN, MOVING];
const T0: u32 = 3;

fn scene() -> Harness {
    let mut h = Harness::new(WorldKind::Server);
    let frozen = RegionConfig { id: OTHER_REGION, seed: 2, mode: RegionMode::Frozen };
    h.world_mut().create_region(frozen).unwrap();
    let world = h.world_mut();
    let floor = Collider::new(Shape::sphere(Scalar::ONE));
    world.spawn(key(STATIC), OTHER_REGION, (at(0, -2, 0), floor)).unwrap();
    world
        .spawn(key(FROZEN), OTHER_REGION, (at(1, 0, 0), Velocity::ZERO, floating(), sphere()))
        .unwrap();
    let sliding = Velocity { linear: v(0, 0, 1), angular: Vec3::ZERO };
    let reporting = sphere().with_events(Scalar::ZERO);
    h.spawn(key(MOVING), (Transform::IDENTITY, sliding, floating(), reporting));
    h.spawn(key(JOINT), (Joint::new(JointKind::Spherical, key(FROZEN), key(MOVING)),));
    h
}

fn bundles(h: &Harness, regions: &[RegionId]) -> Vec<(RegionId, RegionBundle)> {
    regions.iter().map(|&region| (region, bundle(h.world(), region))).collect()
}

/// A fresh process at `tick`: nothing loaded yet (the harness's empty region goes, so the
/// bundles bring it), and the backend's `setup` snapshot already taken.
fn fresh(tick: u32) -> Harness {
    let mut options = Options::new(WorldKind::Server);
    options.tick = Tick(tick);
    let mut h = options.build();
    h.world_mut().unload_region(REGION).unwrap();
    assert_eq!(h.calls.take(), [Call::Snapshot]);
    h
}

fn removed(keys: &[u64]) -> Vec<Call> {
    keys.iter().map(|&n| Call::RemoveBody(key(n))).collect()
}

fn created(calls: &[Call]) -> Vec<StableKey> {
    calls
        .iter()
        .filter_map(|call| match call {
            Call::CreateBody { key, .. } | Call::CreateJoint { key, .. } => Some(*key),
            _ => None,
        })
        .collect()
}

/// Every body's and the joint's handle row, read off the world.
fn handles(h: &Harness) -> (Vec<Option<BodyHandle>>, Option<JointHandle>) {
    let bodies = BODIES.iter().map(|&n| h.world().get::<BodyHandle>(key(n)).copied()).collect();
    (bodies, h.world().get::<JointHandle>(key(JOINT)).copied())
}

/// Runs both worlds `ticks` ticks and requires the same world hash, backend bytes and
/// handle rows after every one.
fn run_equal(mut a: Harness, mut b: Harness, ticks: u32) {
    for _ in 0..ticks {
        assert_eq!(a.tick(), b.tick());
        assert_eq!(a.simulation.hash().unwrap(), b.simulation.hash().unwrap(), "tick {}", a.tick());
        let (ours, theirs) = (snapshot(a.world()).unwrap(), snapshot(b.world()).unwrap());
        assert_eq!(ours.hash(), theirs.hash(), "the backend's bytes at tick {}", a.tick());
        assert_eq!(handles(&a), handles(&b));
        a = a.run(1);
        b = b.run(1);
    }
}

#[test]
fn a_same_frame_reload_of_every_region_makes_a_fresh_solver() {
    let mut h = scene().run(T0);
    assert_eq!(h.physics().body_count(), 3);
    let all = [REGION, OTHER_REGION];
    let loaded = bundles(&h, &all);
    h.calls.take();
    h = h.run_with(frame(all, loaded.clone()));
    let calls = h.calls.take();

    // The fresh process loads the same bundles at the same tick.
    let mut other = fresh(T0);
    other = other.run_with(frame([], loaded));
    let fresh_calls = other.calls.take();
    assert_eq!(created(&fresh_calls), [key(STATIC), key(FROZEN), key(MOVING), key(JOINT)]);
    assert!(matches!(fresh_calls[1], Call::CreateBody { frozen: true, .. }));

    // Every held body removed in key order, the bytes of `setup` restored, then exactly
    // the fresh process's calls: a full diff in key order and the step.
    let mut expected = removed(&BODIES);
    expected.push(Call::Restore);
    expected.extend(fresh_calls);
    assert_eq!(calls, expected);

    // Every row is a body again, with the fresh process's handles.
    assert!(handles(&h).0.iter().all(Option::is_some) && handles(&h).1.is_some());
    run_equal(h, other, 20);
}

#[test]
fn a_same_frame_reload_of_one_region_keeps_the_solver_and_the_handle_rows() {
    let mut h = scene().run(T0);
    let before = handles(&h);
    assert!(before.0.iter().all(Option::is_some) && before.1.is_some());
    let loaded = bundles(&h, &[REGION]);
    h.calls.take();
    h = h.run_with(frame([REGION], loaded));
    let calls = h.calls.take();
    // The solver kept every body and the joint: nothing created, removed or restored.
    assert!(
        !calls.iter().any(|call| matches!(
            call,
            Call::CreateBody { .. }
                | Call::RemoveBody(_)
                | Call::CreateJoint { .. }
                | Call::RemoveJoint(_)
                | Call::Restore
        )),
        "{calls:?}"
    );
    assert_eq!(h.physics().body_count(), 3);
    // The reloaded rows have their handle rows back, the values the bookkeeping holds.
    assert_eq!(handles(&h), before);
    // And they go on following the solver: the moving body's removal takes both rows.
    h.world_mut().remove::<RigidBody>(key(MOVING)).unwrap();
    h.world_mut().remove::<Collider>(key(MOVING)).unwrap();
    h = h.run(1);
    assert_eq!(handles(&h).0[2], None);
    assert_eq!(handles(&h).1, None);
}

#[test]
fn a_two_frame_reload_makes_a_fresh_solver_and_creates_in_key_order() {
    let mut h = scene().run(T0);
    let all = [REGION, OTHER_REGION];
    let loaded = bundles(&h, &all);
    h.calls.take();
    // The unload: every body removed, the bytes restored, nothing left to step.
    h = h.run_with(frame(all, []));
    let mut expected = removed(&BODIES);
    expected.extend([Call::Restore, Call::Step { tick: Tick(T0 + 1), commands: Vec::new() }]);
    assert_eq!(h.calls.take(), expected);
    assert_eq!(h.physics().body_count(), 0);
    assert_eq!(handles(&h), (vec![None; 3], None));
    // The load, a frame later: what a fresh process loading at that tick is told.
    h = h.run_with(frame([], loaded.clone()));
    let mut other = fresh(T0 + 1);
    other = other.run_with(frame([], loaded));
    let calls = h.calls.take();
    assert_eq!(calls, other.calls.take());
    assert_eq!(created(&calls), [key(STATIC), key(FROZEN), key(MOVING), key(JOINT)]);
    run_equal(h, other, 20);
}

#[test]
fn removals_that_empty_the_solver_make_the_next_sync_a_full_diff() {
    let (a, b) = (key(5), key(6));
    let mut h = Harness::new(WorldKind::Server);
    h.spawn(a, (at(0, 0, 0), Velocity::ZERO, floating(), sphere()));
    h.spawn(b, (at(3, 0, 0), Velocity::ZERO, floating(), sphere()));
    h = h.run(2);
    h.calls.take();
    // No row is despawned, but neither is a body any more: the solver is emptied.
    for body in [a, b] {
        h.world_mut().remove::<Transform>(body).unwrap();
    }
    h = h.run(1);
    let calls = sync_calls(&h.calls.take());
    assert_eq!(calls, [Call::RemoveBody(a), Call::RemoveBody(b), Call::Restore]);
    // Back in the opposite order: the log would create `b` first, the full diff creates
    // in key order, as a fresh process's first sync does.
    for (body, x) in [(b, 3), (a, 0)] {
        h.world_mut().insert(body, at(x, 0, 0)).unwrap();
    }
    h = h.run(1);
    assert_eq!(created(&h.take()), [a, b]);
    // And the sync after it reads the log again: an unrelated spawn is created alone.
    h.spawn(key(7), (at(6, 0, 0), Velocity::ZERO, floating(), sphere()));
    h = h.run(1);
    assert_eq!(sync_calls(&h.take()).len(), 1);
}

#[test]
fn a_reload_across_a_restore_sees_the_despawns_the_snapshot_carried() {
    let mut h = scene().run(T0);
    let all = [REGION, OTHER_REGION];
    let loaded = bundles(&h, &all);
    // The regions go between ticks, and the snapshot taken then carries their despawns as
    // pending log entries: the sync after the restore reads them before its own log.
    for region in all {
        h.world_mut().unload_region(region).unwrap();
    }
    let world_snapshot = h.world().snapshot();
    let physics_snapshot = snapshot(h.world()).unwrap();
    h.calls.take();
    h = h.run_with(frame([], loaded.clone()));
    let first = h.calls.take();
    let mut expected = removed(&BODIES);
    expected.push(Call::Restore);
    assert_eq!(first[..4], expected[..]);
    assert_eq!(created(&first), [key(STATIC), key(FROZEN), key(MOVING), key(JOINT)]);

    h.world_mut().restore(&world_snapshot).unwrap();
    restore(h.world_mut(), &physics_snapshot).unwrap();
    assert_eq!(h.calls.take(), [Call::Restore]);
    h = h.run_with(frame([], loaded));
    assert_eq!(h.calls.take(), first);
    assert!(handles(&h).0.iter().all(Option::is_some) && handles(&h).1.is_some());
}

/// The tick a client re-joins at: `set_tick` between runs starts a new epoch, so the sync's
/// cursor is stale when the first step applies the full frame (`lactose-net-api`'s re-join,
/// `lactose-predict`'s resync).
const REJOIN: u32 = 40;

#[test]
fn set_tick_then_a_full_frame_makes_a_fresh_solver() {
    let mut h = scene().run(T0);
    let all = [REGION, OTHER_REGION];
    let loaded = bundles(&h, &all);
    h.world_mut().set_tick(Tick(REJOIN)).unwrap();
    h.calls.take();
    h = h.run_with(frame(all, loaded.clone()));
    let calls = h.calls.take();

    // A fresh client loading the same bundles at that tick.
    let mut other = fresh(REJOIN);
    other = other.run_with(frame([], loaded));
    let mut expected = removed(&BODIES);
    expected.push(Call::Restore);
    expected.extend(other.calls.take());
    assert_eq!(calls, expected);
    assert!(handles(&h).0.iter().all(Option::is_some) && handles(&h).1.is_some());
    run_equal(h, other, 20);
}

#[test]
fn a_one_region_reload_across_set_tick_keeps_the_solver_and_writes_the_handle_rows() {
    let mut h = scene().run(T0);
    let before = handles(&h);
    let loaded = bundles(&h, &[REGION]);
    h.world_mut().set_tick(Tick(REJOIN)).unwrap();
    h.calls.take();
    h = h.run_with(frame([REGION], loaded));
    let calls = h.calls.take();
    // The full diff the stale cursor asks for keeps every body and writes the handle rows
    // the reloaded rows lack.
    assert!(
        !calls.iter().any(|call| matches!(
            call,
            Call::CreateBody { .. }
                | Call::RemoveBody(_)
                | Call::CreateJoint { .. }
                | Call::Restore
        )),
        "{calls:?}"
    );
    assert_eq!(handles(&h), before);
    h = h.run(3);
    assert_eq!(handles(&h), before);
}

#[test]
fn a_rollback_to_a_slot_taken_after_set_tick_repeats_the_fresh_rebuild() {
    // `Predictor::resync`'s slot: taken after `set_tick`, before the full frame, so the
    // physics snapshot is taken with a stale cursor. Then the same with the regions
    // unloaded between `set_tick` and the slot, whose despawns only the snapshot carries.
    for unloaded_first in [false, true] {
        let mut h = scene().run(T0);
        let all = [REGION, OTHER_REGION];
        let loaded = bundles(&h, &all);
        h.world_mut().set_tick(Tick(REJOIN)).unwrap();
        let mut unloads = all.to_vec();
        if unloaded_first {
            for region in unloads.drain(..) {
                h.world_mut().unload_region(region).unwrap();
            }
        }
        let world_snapshot = h.world().snapshot();
        let physics_snapshot = snapshot(h.world()).unwrap();
        h.calls.take();
        h = h.run_with(frame(unloads.clone(), loaded.clone()));
        let first = h.calls.take();
        let mut expected = removed(&BODIES);
        expected.push(Call::Restore);
        assert_eq!(first[..4], expected[..], "unloaded first: {unloaded_first}");
        let handles_first = handles(&h);
        assert!(handles_first.0.iter().all(Option::is_some) && handles_first.1.is_some());

        h.world_mut().restore(&world_snapshot).unwrap();
        restore(h.world_mut(), &physics_snapshot).unwrap();
        assert_eq!(h.calls.take(), [Call::Restore]);
        h = h.run_with(frame(unloads, loaded));
        assert_eq!(h.calls.take(), first, "unloaded first: {unloaded_first}");
        assert_eq!(handles(&h), handles_first);
    }
}
