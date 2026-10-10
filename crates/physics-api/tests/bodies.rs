//! Bodies follow rows (`README.md`, "Bodies"): spawn, `cmd.set` and removal of `RigidBody`
//! and `Collider`, despawn, region moves, mode changes, unload and load reach the backend
//! as `create_body`, `set_body`, `set_collider`, `remove_body` and `set_frozen` in the
//! stated order, with the handle row present exactly meanwhile; the same calls under every
//! layout and worker count; and the prediction world's effective bodies.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_ecs::{ClaimedBy, LayoutConfig, RegionConfig, RegionId, RegionMode, WorldKind};
use lactose_math::Scalar;
use lactose_physics_api::{BodyHandle, BodyType, Collider, RigidBody, Shape, Transform, Velocity};
use lactose_sim::harness::{ChunkCapacity, layout_mode};

const A: u64 = 10;
const B: u64 = 11;
const C: u64 = 12;
const D: u64 = 13;
const THIRD_REGION: RegionId = RegionId(3);

/// The scene of the first test: what the game does, tick by tick, and what the backend
/// must have been told after each tick.
fn lifecycle(options: Options) -> (Vec<Vec<Call>>, Vec<Vec<bool>>) {
    let mut h = options.build();
    let frozen = RegionConfig { id: OTHER_REGION, seed: 2, mode: RegionMode::Frozen };
    h.world_mut().create_region(frozen).unwrap();
    h.spawn_body(key(A), at(0, 0, 0));
    h.spawn(key(B), (at(5, 0, 0), Collider::new(Shape::sphere(Scalar::ONE))));
    h.spawn(key(C), (at(9, 0, 0), Velocity::ZERO, floating(), sphere()));
    let bigger = Collider::new(Shape::sphere(Scalar::TWO));
    h.script()
        .at(2, When::Simulation, key(A), Action::SetBody(key(B), floating()))
        .at(3, When::Simulation, key(A), Action::RemoveBody(key(B)))
        .at(4, When::Simulation, key(C), Action::SetCollider(key(B), bigger))
        .at(5, When::Simulation, key(C), Action::Despawn(key(A)))
        .at(6, When::Simulation, key(C), Action::Move(key(C), OTHER_REGION));
    let mut calls = Vec::new();
    let mut handles = Vec::new();
    for tick in 1..=9 {
        match tick {
            7 => h.world_mut().set_region_mode(OTHER_REGION, RegionMode::Simulated).unwrap(),
            8 => h.world_mut().unload_region(OTHER_REGION).unwrap(),
            9 => {
                let third = RegionConfig { id: THIRD_REGION, seed: 3, mode: RegionMode::Simulated };
                h.world_mut().create_region(third).unwrap();
                h.world_mut()
                    .spawn(key(D), THIRD_REGION, (at(0, 9, 0), floating(), sphere()))
                    .unwrap();
            }
            _ => {}
        }
        h = h.run(1);
        calls.push(sync_calls(&h.take()));
        handles.push([A, B, C, D].map(|n| h.world().get::<BodyHandle>(key(n)).is_some()).to_vec());
        // The handle row is present exactly while the bookkeeping holds the body.
        for n in [A, B, C, D] {
            assert_eq!(
                h.world().get::<BodyHandle>(key(n)).is_some(),
                h.physics().is_body(key(n)),
                "tick {tick}, key {n}"
            );
        }
    }
    (calls, handles)
}

#[test]
fn bodies_follow_rows_in_the_stated_order() {
    let (calls, handles) = lifecycle(Options::new(WorldKind::Server));
    let created = |call: &Call| matches!(call, Call::CreateBody { .. });
    // Tick 1: the first sync diffs every row and creates every body, in key order, each
    // from the rows as they are, the region's mode as `frozen`.
    assert_eq!(calls[0].len(), 3);
    assert!(calls[0].iter().all(created));
    assert_eq!(
        calls[0].iter().map(|call| call.key().unwrap()).collect::<Vec<_>>(),
        [key(A), key(B), key(C)]
    );
    let Call::CreateBody { body, collider, frozen, .. } = &calls[0][1] else { unreachable!() };
    assert_eq!(
        (*body, collider.is_some(), *frozen),
        (None, true, false),
        "a collider alone is fixed"
    );
    // Tick 2: `cmd.set` of a `RigidBody` on a fixed collider makes it a body; the handle stays.
    assert_eq!(calls[1], [Call::SetBody(key(B), Some(floating()))]);
    // Tick 3: its removal leaves a fixed collider.
    assert_eq!(calls[2], [Call::SetBody(key(B), None)]);
    // Tick 4: a collider written through `cmd.set`.
    assert_eq!(
        calls[3],
        [Call::SetCollider(key(B), Some(Collider::new(Shape::sphere(Scalar::TWO))))]
    );
    // Tick 5: a despawn removes the body; the row and its handle are gone.
    assert_eq!(calls[4], [Call::RemoveBody(key(A))]);
    // Tick 6: a body moved into a frozen region is frozen.
    assert_eq!(calls[5], [Call::SetFrozen(key(C), true)]);
    // Tick 7: its region simulated again, it is thawed and takes the row's velocity.
    assert_eq!(
        calls[6],
        [Call::SetFrozen(key(C), false), Call::SetVelocity(key(C), Velocity::ZERO)]
    );
    // Tick 8: the region unloads; the body goes with its row.
    assert_eq!(calls[7], [Call::RemoveBody(key(C))]);
    // Tick 9: a region loads with a body in it.
    assert_eq!(calls[8].len(), 1);
    assert!(created(&calls[8][0]) && calls[8][0].key() == Some(key(D)));
    assert_eq!(
        handles,
        [
            [true, true, true, false],
            [true, true, true, false],
            [true, true, true, false],
            [true, true, true, false],
            [false, true, true, false],
            [false, true, true, false],
            [false, true, true, false],
            [false, true, false, false],
            [false, true, false, true],
        ]
    );
}

#[test]
fn the_calls_are_the_same_under_every_layout_and_worker_count() {
    let reference = lifecycle(Options::new(WorldKind::Server));
    let capacities = [
        ChunkCapacity::Rows(1),
        ChunkCapacity::Rows(7),
        ChunkCapacity::Rows(64),
        ChunkCapacity::Bytes(16 * 1024),
        ChunkCapacity::Production,
    ];
    for capacity in capacities {
        for seed in [1, 2, 0x5eed] {
            for workers in [1, 4] {
                let mut options = Options::new(WorldKind::Server);
                options.layout = layout_mode(capacity, seed).unwrap();
                options.workers = workers;
                assert_eq!(
                    lifecycle(options),
                    reference,
                    "{capacity:?}, seed {seed}, {workers} workers"
                );
            }
        }
    }
    let mut options = Options::new(WorldKind::Server);
    options.layout = LayoutConfig::default();
    options.threads = 4;
    assert_eq!(lifecycle(options), reference, "the backend's pool size");
}

#[test]
fn a_prediction_world_hands_the_effective_body() {
    let mut h = Harness::new(WorldKind::Prediction);
    let proxy = key(1);
    let claimed = key(2);
    let scenery = key(3);
    // No `Velocity` on the claimed row: the first write-back adds one.
    h.spawn(proxy, (at(0, 0, 0), RigidBody::dynamic(), sphere()));
    let claim = ClaimedBy { player: PLAYER };
    h.spawn(claimed, (at(3, 0, 0), RigidBody::dynamic(), sphere(), claim));
    h.spawn(scenery, (at(6, 0, 0), sphere()));
    h = h.run(1);
    let calls = sync_calls(&h.take());
    let bodies: Vec<_> = calls
        .iter()
        .map(|call| match call {
            Call::CreateBody { key, body, .. } => (*key, body.map(|body| body.body_type)),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        bodies,
        [(proxy, Some(BodyType::Kinematic)), (claimed, Some(BodyType::Dynamic)), (scenery, None),]
    );

    // A claim is a change of the effective body; so is a release.
    h.world_mut().insert(proxy, ClaimedBy { player: PLAYER }).unwrap();
    h.world_mut().remove::<ClaimedBy>(claimed).unwrap();
    h = h.run(1);
    let mut kinematic = RigidBody::dynamic();
    kinematic.body_type = BodyType::Kinematic;
    assert_eq!(
        sync_calls(&h.take()),
        [Call::SetBody(proxy, Some(RigidBody::dynamic())), Call::SetBody(claimed, Some(kinematic))]
    );

    // A body change of a proxy is handed as kinematic, whatever the row says.
    let mut heavy = RigidBody::dynamic();
    heavy.mass = Scalar::TWO;
    h.world_mut().insert(claimed, heavy).unwrap();
    h = h.run(1);
    let mut heavy_kinematic = heavy;
    heavy_kinematic.body_type = BodyType::Kinematic;
    assert_eq!(sync_calls(&h.take()), [Call::SetBody(claimed, Some(heavy_kinematic))]);

    // In a server world the same rows are what they say.
    let mut server = Harness::new(WorldKind::Server);
    server.spawn(proxy, (at(0, 0, 0), RigidBody::dynamic(), sphere()));
    server = server.run(1);
    let calls = sync_calls(&server.take());
    assert!(
        matches!(&calls[..], [Call::CreateBody { body: Some(body), .. }] if body.body_type == BodyType::Dynamic)
    );
}

#[test]
fn a_refused_body_is_retried_when_its_row_changes_and_a_row_without_transform_is_skipped() {
    let mut h = Harness::new(WorldKind::Server);
    // A negative radius: the backend refuses it.
    h.spawn(key(1), (at(0, 0, 0), floating(), Collider::new(Shape::sphere(-Scalar::ONE))));
    // A body without a transform is not a body.
    h.spawn(key(2), (Velocity::ZERO, floating(), sphere()));
    h = h.run(1);
    let calls = sync_calls(&h.take());
    assert_eq!(calls.len(), 1);
    assert!(matches!(calls[0], Call::CreateBody { key: k, .. } if k == key(1)));
    assert!(!h.physics().is_body(key(1)) && h.world().get::<BodyHandle>(key(1)).is_none());
    // Nothing changed: no retry.
    h = h.run(2);
    assert_eq!(sync_calls(&h.take()), []);
    // The row changes: retried, and now accepted.
    h.world_mut().insert(key(1), sphere()).unwrap();
    h.world_mut().insert(key(2), Transform::IDENTITY).unwrap();
    h = h.run(1);
    let created: Vec<_> = sync_calls(&h.take()).iter().filter_map(Call::key).collect();
    // The log first (the transform added), then the changed rows (the collider written).
    assert_eq!(created, [key(2), key(1)]);
    assert!(h.physics().is_body(key(1)) && h.physics().is_body(key(2)));
    assert_eq!(h.physics().body_count(), 2);
}

#[test]
fn a_region_mode_change_freezes_and_thaws_every_body_of_the_region() {
    let mut h = Harness::new(WorldKind::Server);
    let moving = Velocity { linear: v(1, 0, 0), angular: v(0, 0, 0) };
    h.spawn(key(1), (at(0, 0, 0), moving, floating(), sphere()));
    h.spawn(key(2), (at(5, 0, 0), sphere()));
    h = h.run(1);
    h.take();
    h.world_mut().set_region_mode(REGION, RegionMode::Frozen).unwrap();
    h = h.run(1);
    let calls = h.take();
    assert_eq!(sync_calls(&calls), [Call::SetFrozen(key(1), true), Call::SetFrozen(key(2), true)]);
    // Frozen, nothing moves and nothing is written back.
    let before = *h.world().get::<Transform>(key(1)).unwrap();
    h = h.run(2);
    assert_eq!(*h.world().get::<Transform>(key(1)).unwrap(), before);
    h.take();
    h.world_mut().set_region_mode(REGION, RegionMode::Simulated).unwrap();
    h = h.run(1);
    assert_eq!(
        sync_calls(&h.take()),
        [
            Call::SetFrozen(key(1), false),
            Call::SetVelocity(key(1), moving),
            Call::SetFrozen(key(2), false)
        ]
    );
    // A mode change made between ticks reaches the sync of the next tick once.
    h = h.run(1);
    assert_eq!(sync_calls(&h.take()), []);
}

/// The log entries a tick-1 post stage makes after the sync's cursor (a spawn, the
/// write-back's added `Velocity`) are read by the tick-2 sync, which is incremental:
/// `lactose-sim` once trimmed the log to the end of its first step, which made the second
/// sync a full diff that pushed every body.
#[test]
fn the_second_sync_reads_what_the_first_tick_logged_after_it() {
    let mut h = Harness::new(WorldKind::Server);
    let source = key(1);
    h.spawn(source, (at(0, 0, 0), RigidBody::dynamic(), sphere()));
    h.spawn(key(2), (at(5, 0, 0), sphere()));
    h.script().at(1, When::Post, source, Action::Spawn(at(0, 9, 0)));
    h = h.run(1);
    h.take();
    assert!(h.world().get::<Velocity>(source).is_some(), "the write-back added one");
    h = h.run(1);
    let calls = sync_calls(&h.take());
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(matches!(calls[0], Call::CreateBody { .. }));
    assert!(![Some(source), Some(key(2))].contains(&calls[0].key()), "the spawned body");
}
