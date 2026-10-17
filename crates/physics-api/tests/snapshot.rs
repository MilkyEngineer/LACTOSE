//! Snapshot and restore (`README.md`, "Snapshot and restore"): the conformance run with the
//! test backend under several pool sizes, layouts and worker counts; probe B (the pending
//! work at the snapshot point carried across the restore, so the re-simulated syncs make
//! the first run's calls); probe C (a correction named through `corrected` is pushed); the
//! predict loop; a world restored alone and the 2^30 rebase diff without clearing (probe
//! D); and the refusals.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::{ErrorKind, NetId, StableKey, Tick};
use lactose_ecs::{
    ClaimedBy, Executor, LayoutConfig, OwnedBy, RegionConfig, RegionMode, WorldKind,
};
use lactose_math::Scalar;
use lactose_physics_api::conformance::{
    BODIES, SNAPSHOT_TICK, TICKS, empty_restore, snapshot_restore,
};
use lactose_physics_api::{
    BodyType, Joint, JointKind, PhysicsBackend, PhysicsCommand, PhysicsConfig, RigidBody,
    Transform, Velocity, corrected, restore, snapshot,
};
use lactose_sim::harness::{ChunkCapacity, layout_mode};
use lactose_sim::{SimulationBuilder, TypeTable};

fn impulse(body: StableKey, x: i32) -> PhysicsCommand {
    PhysicsCommand::ApplyImpulse { body, impulse: v(x, 0, 0) }
}

/// The calls between two markers of the log, exclusive.
fn between(calls: &[Call], from: usize, to: usize) -> Vec<Call> {
    calls[from + 1..to].to_vec()
}

#[test]
fn the_conformance_run_passes_with_the_test_backend() {
    let run = |threads: u32, layout: LayoutConfig, workers: u32| {
        let calls = CallLog::default();
        let backend = Box::new(Fake::new(calls.clone()).with_threads(threads));
        let executor =
            Executor::builder().frame_workers(workers).background_workers(1).build().unwrap();
        let report =
            snapshot_restore(&executor, backend, PhysicsConfig::default(), layout).unwrap();
        (report, calls.all())
    };
    let (reference, calls) = run(1, LayoutConfig::default(), 1);
    assert_eq!(reference.first.len(), TICKS as usize);
    assert_eq!(reference.resumed, reference.first[SNAPSHOT_TICK as usize..]);
    assert!(reference.snapshot_bytes > 0);
    // The test backend's bytes are a function of its state, which the resumed run reaches
    // again exactly.
    assert_eq!(reference.physics_hash.0, reference.physics_hash.1);

    // The resumed run told the backend exactly what the first run did after the snapshot.
    // The first marker is `setup`'s snapshot of the empty world, before any other call.
    let markers: Vec<usize> = calls
        .iter()
        .enumerate()
        .filter(|(_, call)| matches!(call, Call::Snapshot | Call::Restore))
        .map(|(index, _)| index)
        .collect();
    let [0, taken, first_end, restored, resumed_end] = markers[..] else { panic!("{markers:?}") };
    assert_eq!(calls[restored], Call::Restore);
    let first = between(&calls, taken, first_end);
    let resumed = between(&calls, restored, resumed_end);
    assert_eq!(first, resumed);

    // And the scene carried what it claims: the snapshot tick's post-stage spawn, despawn,
    // `cmd.set` and command reach the next tick's sync and step, then a velocity write and
    // a command, a bone going limp, a freeze and a thaw.
    let net = |n: u64| StableKey::Net(NetId::cooked(n));
    let sphere = |index: u64| net(100 + index);
    let step = |tick: u32| {
        first
            .iter()
            .position(|call| matches!(call, Call::Step { tick: t, .. } if t.0 == tick))
            .unwrap()
    };
    let tick_4 = &first[..step(4)];
    assert!(
        tick_4
            .iter()
            .any(|call| matches!(call, Call::CreateBody { key: StableKey::Predicted(_), .. }))
    );
    assert!(tick_4.contains(&Call::RemoveBody(sphere(1))));
    assert!(
        tick_4.iter().any(|call| matches!(call, Call::SetTransform(key, _) if *key == sphere(2)))
    );
    let Call::Step { commands, .. } = &first[step(4)] else { unreachable!() };
    assert!(commands.iter().any(|command| command.body() == sphere(3)));
    let tick_5 = &first[step(4) + 1..step(5)];
    assert!(
        tick_5.iter().any(|call| matches!(call, Call::SetVelocity(key, _) if *key == sphere(4)))
    );
    let Call::Step { commands, .. } = &first[step(5)] else { unreachable!() };
    assert!(commands.iter().any(|command| command.body() == sphere(5)));
    let tick_6 = &first[step(5) + 1..step(6)];
    assert!(tick_6.iter().any(
        |call| matches!(call, Call::SetBody(key, Some(body)) if *key == net(2) && body.body_type == BodyType::Dynamic)
    ));
    let frozen = |tick: u32, value: bool| {
        first[step(tick - 1) + 1..step(tick)]
            .iter()
            .filter(|call| matches!(call, Call::SetFrozen(_, v) if *v == value))
            .count()
    };
    // Every body of the region: the floor, the stack less one, two bones, the proxy and
    // the spawn.
    let bodies = 1 + (BODIES as usize - 1) + 2 + 1 + 1;
    assert_eq!((frozen(7, true), frozen(8, false)), (bodies, bodies));

    // Under every pool size, two layouts and two worker counts, the same report.
    for threads in [1, 4] {
        for layout in [LayoutConfig::default(), layout_mode(ChunkCapacity::Rows(7), 9).unwrap()] {
            for workers in [1, 4] {
                assert_eq!(
                    run(threads, layout, workers).0,
                    reference,
                    "{threads} threads, {layout:?}, {workers} workers"
                );
            }
        }
    }
}

/// The scene of probe B: a spawner, a body to despawn and one to move, all claimed.
fn probe_b(input_ticks: bool) -> Harness {
    let mut h = Harness::new(WorldKind::Prediction);
    let (spawner, doomed, moved) = (key(1), key(2), key(3));
    h.spawn_body(spawner, at(0, 0, 0));
    h.spawn_body(doomed, at(3, 0, 0));
    h.spawn_body(moved, at(6, 0, 0));
    h.script().input_ticks = input_ticks;
    h.script()
        .at(2, When::Post, spawner, Action::Spawn(at(0, 9, 0)))
        .at(2, When::Post, spawner, Action::Despawn(doomed))
        .at(2, When::Post, spawner, Action::SetTransform(moved, at(6, 9, 0)))
        .at(2, When::Post, spawner, Action::Command(impulse(moved, 1)))
        .at(2, When::AfterStep, spawner, Action::Command(impulse(spawner, 2)))
        .at(3, When::Simulation, spawner, Action::Command(impulse(spawner, 3)));
    h
}

/// Probe B: a `Stage::Post` move, spawn, despawn and impulse and a `Stage::Physics`
/// command at the snapshot tick, with an input tick set every tick. The re-simulated
/// syncs make the first run's calls and the rows end equal.
#[test]
fn the_restore_carries_the_pending_work_of_the_snapshot_point() {
    let mut h = probe_b(true).run(2);
    let world_snapshot = h.world().snapshot();
    let physics_snapshot = snapshot(h.world()).unwrap();
    h.take();
    h = h.run(2);
    let first = h.take();
    let first_rows = rows(h.world());
    // What the first run's sync at tick 3 saw: the spawn, the despawn, the move, and the
    // two commands ahead of the tick's own.
    let syncs = sync_calls(&first);
    assert!(
        syncs
            .iter()
            .any(|call| matches!(call, Call::CreateBody { key: StableKey::Predicted(_), .. }))
    );
    assert!(syncs.contains(&Call::RemoveBody(key(2))));
    assert!(syncs.contains(&Call::SetTransform(key(3), at(6, 9, 0))));
    // The step after the snapshot point: the physics stage's command, the post stage's
    // (the spawn's own impulse, then the moved body's), then the tick's own.
    let commands = &steps(&first)[0].1;
    assert_eq!(commands.len(), 4);
    assert_eq!(
        (commands[0], commands[2], commands[3]),
        (impulse(key(1), 2), impulse(key(3), 1), impulse(key(1), 3))
    );
    assert!(matches!(commands[1].body(), StableKey::Predicted(_)));

    h.world_mut().restore(&world_snapshot).unwrap();
    restore(h.world_mut(), &physics_snapshot).unwrap();
    h.take();
    h = h.run(2);
    assert_eq!(h.take(), first);
    assert_eq!(rows(h.world()), first_rows);
}

/// Probe C: a correction written without recording is invisible to the change stamps;
/// `corrected` names it, and the next sync pushes it.
#[test]
fn a_correction_named_through_corrected_is_pushed() {
    for name_it in [false, true] {
        let mut h = probe_b(true).run(2);
        let world_snapshot = h.world().snapshot();
        let physics_snapshot = snapshot(h.world()).unwrap();
        h = h.run(1);
        h.world_mut().restore(&world_snapshot).unwrap();
        restore(h.world_mut(), &physics_snapshot).unwrap();
        // The authoritative state differs: written as `lactose-predict` writes a correction.
        let fixed = at(-5, 0, 0);
        for column in h.world_mut().columns_mut::<Transform>() {
            column.write(|keys, values, _dirty| {
                for (key_of_row, value) in keys.iter().zip(values.iter_mut()) {
                    if *key_of_row == key(1) {
                        *value = fixed;
                    }
                }
            });
        }
        if name_it {
            // A key that is not a body is ignored.
            corrected(h.world_mut(), &[key(1), key(500)]).unwrap();
        }
        h.take();
        h = h.run(1);
        let pushed = sync_calls(&h.take()).contains(&Call::SetTransform(key(1), fixed));
        assert_eq!(pushed, name_it);
    }
}

/// The predict loop: snapshot at tick 3 beside `World::snapshot`, ten ticks with
/// post-stage spawns, despawns, moves, commands and a claim transition; `World::restore`,
/// `restore`, `corrected`; the re-run tells the backend what the first run did and ends
/// with the same rows.
#[test]
fn the_predict_loop_re_simulates_exactly() {
    let mut h = Harness::new(WorldKind::Prediction);
    let bodies: Vec<StableKey> = (1..=6).map(key).collect();
    for (n, &body) in bodies.iter().enumerate() {
        h.spawn_body(body, at(n as i32 * 3, 0, 0));
    }
    let proxy = key(20);
    h.spawn(
        proxy,
        (at(0, 20, 0), lactose_physics_api::Velocity::ZERO, RigidBody::dynamic(), sphere()),
    );
    h.script().input_ticks = true;
    let sliding = lactose_physics_api::Velocity { linear: v(2, 0, 0), angular: v(0, 0, 0) };
    h.script()
        .at(2, When::Simulation, bodies[0], Action::WriteVelocity(sliding))
        .at(4, When::Post, bodies[0], Action::Spawn(at(0, 30, 0)))
        .at(5, When::Post, bodies[1], Action::Despawn(bodies[2]))
        .at(5, When::Post, bodies[1], Action::SetTransform(bodies[3], at(9, 9, 9)))
        .at(6, When::Simulation, bodies[1], Action::Command(impulse(bodies[4], 4)))
        .at(6, When::AfterStep, bodies[1], Action::Command(impulse(bodies[1], 1)))
        .at(7, When::Post, bodies[1], Action::Claim(proxy))
        .at(8, When::Post, bodies[1], Action::Release(bodies[5]))
        .at(9, When::Post, bodies[0], Action::Spawn(at(5, 30, 0)));
    h = h.run(3);
    let world_snapshot = h.world().snapshot();
    let physics_snapshot = snapshot(h.world()).unwrap();
    h.take();
    h = h.run(7);
    let first = h.take();
    let first_rows = rows(h.world());
    assert_eq!(h.tick(), 10);
    assert!(
        sync_calls(&first).contains(&Call::SetBody(proxy, Some(RigidBody::dynamic()))),
        "the claim"
    );

    h.world_mut().restore(&world_snapshot).unwrap();
    restore(h.world_mut(), &physics_snapshot).unwrap();
    // Nothing was mispredicted: the corrections write what the rows already hold, and the
    // sync is told about the sliding body anyway.
    corrected(h.world_mut(), &[bodies[0]]).unwrap();
    h.take();
    h = h.run(7);
    let mut again = h.take();
    // A corrected key's five columns read as changed: its body and collider are pushed
    // with the values the backend holds, which it ignores; its transform and velocity
    // equal what the write-back wrote and are not. Everything else is the first run's.
    let body = *h.world().get::<RigidBody>(bodies[0]).unwrap();
    let extra =
        [Call::SetBody(bodies[0], Some(body)), Call::SetCollider(bodies[0], Some(sphere()))];
    let at_extra = again.iter().position(|call| *call == extra[0]).unwrap();
    assert_eq!(again[at_extra..at_extra + 2], extra);
    assert!(at_extra < again.iter().position(|call| matches!(call, Call::Step { .. })).unwrap());
    again.drain(at_extra..at_extra + 2);
    assert_eq!(again, first);
    assert_eq!(rows(h.world()), first_rows);
}

#[test]
fn a_world_restored_without_a_physics_restore_is_diffed_without_clearing() {
    let mut h = Harness::new(WorldKind::Server);
    let (kept, doomed) = (key(1), key(2));
    h.spawn_body(kept, at(0, 0, 0));
    h.spawn_body(doomed, at(3, 0, 0));
    h.script().at(3, When::Simulation, kept, Action::Despawn(doomed)).at(
        3,
        When::Simulation,
        kept,
        Action::Spawn(at(0, 9, 0)),
    );
    h = h.run(2);
    let world_snapshot = h.world().snapshot();
    h = h.run(2);
    let spawned = h
        .world()
        .columns::<Transform>()
        .flat_map(|c| c.keys().to_vec())
        .find(|&k| k != kept)
        .unwrap();
    h.take();
    h.world_mut().restore(&world_snapshot).unwrap();
    // A rollback without a physics restore, and a game that does not repeat itself.
    h.script().actions.clear();
    h = h.run(1);
    let calls = sync_calls(&h.take());
    // Every row against the bookkeeping: the body spawned after the snapshot is gone, the
    // one despawned is back, the one in both is pushed and never removed.
    assert!(calls.contains(&Call::RemoveBody(spawned)));
    assert!(
        calls.iter().any(|call| matches!(call, Call::CreateBody { key, .. } if *key == doomed))
    );
    assert!(!calls.contains(&Call::RemoveBody(kept)));
    let pushed: Vec<_> = calls.iter().filter(|call| call.key() == Some(kept)).collect();
    assert!(matches!(
        pushed[..],
        [Call::SetBody(..), Call::SetCollider(..), Call::SetTransform(..), Call::SetVelocity(..)]
    ));
    // And it keeps running, incrementally again.
    h = h.run(2);
    assert_eq!(sync_calls(&h.take()), []);
}

/// Probe D: the 2^30 stamp rebase starts an epoch; the sync diffs without clearing.
#[test]
fn the_stamp_rebase_is_diffed_without_clearing() {
    let mut options = Options::new(WorldKind::Server);
    options.tick = Tick((1 << 30) - 3);
    let mut h = options.build();
    h.spawn_body(key(1), at(0, 0, 0));
    h.spawn(key(2), (at(5, 0, 0), sphere()));
    h = h.run(1);
    h.take();
    let mut per_tick = Vec::new();
    for _ in 0..4 {
        h = h.run(1);
        per_tick.push((h.tick(), sync_calls(&h.take())));
    }
    for (tick, calls) in &per_tick {
        if *tick == 1 << 30 {
            assert!(!calls.is_empty());
            assert!(
                calls.iter().all(|call| matches!(
                    call,
                    Call::SetBody(..)
                        | Call::SetCollider(..)
                        | Call::SetTransform(..)
                        | Call::SetVelocity(..)
                )),
                "{calls:?}"
            );
        } else {
            assert_eq!(calls, &[], "tick {tick}");
        }
    }
    assert!(per_tick.iter().any(|(tick, _)| *tick == 1 << 30));
}

#[test]
fn a_snapshot_taken_before_the_first_sync_restores_into_a_full_diff() {
    let mut h = Harness::new(WorldKind::Prediction);
    h.spawn_body(key(1), at(0, 0, 0));
    let world_snapshot = h.world().snapshot();
    let physics_snapshot = snapshot(h.world()).unwrap();
    h = h.run(2);
    h.world_mut().restore(&world_snapshot).unwrap();
    restore(h.world_mut(), &physics_snapshot).unwrap();
    h.take();
    h = h.run(1);
    assert!(matches!(&sync_calls(&h.take())[..], [Call::CreateBody { .. }]));
}

#[test]
fn a_mismatched_snapshot_is_refused_and_changes_nothing() {
    let mut h = Harness::new(WorldKind::Prediction);
    h.spawn_body(key(1), at(0, 0, 0));
    h = h.run(2);
    let ours = snapshot(h.world()).unwrap();
    assert_eq!(ours.tick(), Tick(2));
    let other = Harness::new(WorldKind::Prediction).run(2);
    let theirs = snapshot(other.world()).unwrap();
    assert_eq!(restore(h.world_mut(), &theirs).unwrap_err().kind(), ErrorKind::InvalidData);
    // The world must be restored first: a snapshot of another tick is refused.
    h = h.run(1);
    assert_eq!(restore(h.world_mut(), &ours).unwrap_err().kind(), ErrorKind::InvalidData);
    // Nothing changed: the backend was not touched and the next sync is incremental.
    assert!(!h.calls.all().contains(&Call::Restore));
    h.take();
    h = h.run(1);
    assert_eq!(sync_calls(&h.take()), []);
    assert_eq!(ours.hash(), ours.hash());
    assert_eq!(ours.size_in_bytes(), ours.size_in_bytes());

    // A world without `Physics`.
    let mut bare = SimulationBuilder::new(WorldKind::Prediction, TypeTable::new()).build().unwrap();
    let bare = bare.world_mut();
    assert_eq!(snapshot(bare).err().unwrap().kind(), ErrorKind::NotFound);
    assert_eq!(restore(bare, &ours).unwrap_err().kind(), ErrorKind::NotFound);
    assert_eq!(corrected(bare, &[key(1)]).unwrap_err().kind(), ErrorKind::NotFound);
    // And a second `Physics` in one world.
    let backend = Box::new(Fake::new(CallLog::default()));
    let error =
        lactose_physics_api::setup(h.world_mut(), backend, PhysicsConfig::default()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    let _ = Scalar::ZERO;
}

// ---- Re-simulation from every restore point (review of the implementation, probe I1) ----

const RESIM_TICKS: u32 = 14;
const BOUND: u64 = 500;

/// Edits made between ticks, before tick `next`: the predicted body the tick-3 spawn made
/// bound to a `Net` key (the alias table keeps it across a restore), and the second region
/// frozen and thawed.
fn resim_edits(h: &mut Harness, next: u32) {
    match next {
        6 => {
            let spawned = rows(h.world())
                .into_iter()
                .find_map(|row| match row.0 {
                    StableKey::Predicted(predicted) => Some(predicted),
                    _ => None,
                })
                .expect("the tick-3 spawn");
            h.world_mut().bind(spawned, NetId::cooked(BOUND)).unwrap();
        }
        7 => h.world_mut().set_region_mode(OTHER_REGION, RegionMode::Frozen).unwrap(),
        10 => h.world_mut().set_region_mode(OTHER_REGION, RegionMode::Simulated).unwrap(),
        _ => {}
    }
}

fn resim_advance(mut h: Harness, to: u32) -> Harness {
    while h.tick() < to {
        let next = h.tick() + 1;
        resim_edits(&mut h, next);
        h = h.run(1);
    }
    h
}

/// Pending work of every kind at many ticks: post-stage spawns (one bound mid-run, one never
/// bound), a despawn that leaves a joint waiting, `cmd.set` writes, commands from three
/// stages (one naming the bound body by its `Net` key), a claim and a release, a kinematic
/// body driven every tick, a body change and a region frozen and thawed.
fn resim_scene(layout: LayoutConfig, workers: u32, threads: u32) -> Harness {
    let mut options = Options::new(WorldKind::Prediction);
    options.layout = layout;
    options.workers = workers;
    options.threads = threads;
    let mut h = options.build();
    let second = RegionConfig { id: OTHER_REGION, seed: 2, mode: RegionMode::Simulated };
    h.world_mut().create_region(second).unwrap();
    for n in 1..=6u64 {
        h.spawn_body(key(n), at(n as i32 * 3, 0, 0));
    }
    let claim = ClaimedBy { player: PLAYER };
    h.spawn(key(7), (at(0, 20, 0), Velocity::ZERO, RigidBody::dynamic(), sphere()));
    h.spawn(key(8), (at(40, 0, 0), Velocity::ZERO, RigidBody::kinematic(), sphere(), claim));
    let sliding = Velocity { linear: v(1, 0, 0), angular: v(0, 0, 0) };
    let frozen_body = (at(60, 0, 0), sliding, floating(), sphere(), claim);
    h.world_mut().spawn(key(9), OTHER_REGION, frozen_body).unwrap();
    let joint = Joint::new(JointKind::Spherical, key(3), key(4));
    h.spawn(key(30), (joint, OwnedBy { owner: key(3).into() }));
    h.script().input_ticks = true;
    let slide = Velocity { linear: v(2, 0, 0), angular: v(0, 0, 0) };
    let bound = StableKey::Net(NetId::cooked(BOUND));
    let s = h.script();
    s.at(2, When::Simulation, key(1), Action::WriteVelocity(slide))
        .at(3, When::Post, key(5), Action::Spawn(at(0, 30, 0)))
        .at(3, When::Post, key(5), Action::Command(impulse(key(1), 3)))
        .at(3, When::Post, key(5), Action::SetTransform(key(6), at(9, 9, 9)))
        .at(3, When::Post, key(5), Action::Despawn(key(4)))
        .at(4, When::AfterStep, key(1), Action::Command(impulse(key(2), 4)))
        .at(5, When::Simulation, key(2), Action::Command(impulse(key(5), 5)))
        .at(6, When::Post, key(5), Action::Claim(key(7)))
        .at(7, When::Simulation, key(2), Action::Command(impulse(bound, 7)))
        .at(8, When::Post, key(5), Action::Release(key(6)))
        .at(9, When::Simulation, key(2), Action::WriteBody(RigidBody::kinematic()))
        .at(11, When::Post, key(1), Action::Spawn(at(5, 30, 0)));
    for t in 1..=RESIM_TICKS {
        s.at(t, When::Simulation, key(8), Action::WriteTransform(at(40 + t as i32, 0, 0)));
    }
    h
}

/// The calls and rows of ticks `from + 1..=RESIM_TICKS`: straight, then re-simulated.
type Run = (Vec<Call>, Vec<Row>);

fn straight_and_resimulated(
    layout: LayoutConfig,
    workers: u32,
    threads: u32,
    from: u32,
) -> (Run, Run) {
    let mut h = resim_advance(resim_scene(layout, workers, threads), from);
    let world_snapshot = h.world().snapshot();
    let physics_snapshot = snapshot(h.world()).unwrap();
    h.take();
    h = resim_advance(h, RESIM_TICKS);
    let straight = (h.take(), rows(h.world()));
    h.world_mut().restore(&world_snapshot).unwrap();
    restore(h.world_mut(), &physics_snapshot).unwrap();
    h.take();
    h = resim_advance(h, RESIM_TICKS);
    (straight, (h.take(), rows(h.world())))
}

/// Probe I1 of the implementation review, with a predicted body bound mid-run beside one
/// never bound: every restore point, layout, worker count and pool size re-simulates to the
/// first run's calls and rows.
#[test]
fn a_re_simulation_from_every_restore_point_makes_the_first_runs_calls() {
    let layouts = [
        LayoutConfig::default(),
        layout_mode(ChunkCapacity::Rows(1), 1).unwrap(),
        layout_mode(ChunkCapacity::Rows(7), 0xC0FFEE).unwrap(),
        layout_mode(ChunkCapacity::Bytes(16 * 1024), 1).unwrap(),
    ];
    let mut reference: Option<Vec<Call>> = None;
    for layout in layouts {
        for workers in [1, 4] {
            for threads in [1, 4] {
                for from in [2, 3, 4, 5, 6, 8, 10] {
                    let (straight, again) =
                        straight_and_resimulated(layout, workers, threads, from);
                    let at = straight.0.iter().zip(&again.0).position(|(a, b)| a != b);
                    assert!(
                        straight == again,
                        "{layout:?}, {workers} workers, pool {threads}, from {from}: first \
                         difference at {at:?}"
                    );
                    if from == 2 {
                        match &reference {
                            None => reference = Some(straight.0),
                            Some(first) => assert_eq!(first, &straight.0, "{layout:?}"),
                        }
                    }
                }
            }
        }
    }
    // The command naming the bound body by its `Net` key reached the step, by origin key.
    let calls = reference.unwrap();
    let step_7 = steps(&calls).into_iter().find(|(tick, _)| tick.0 == 7).unwrap().1;
    assert!(step_7.iter().any(|command| matches!(command.body(), StableKey::Predicted(_))));
}

#[test]
fn an_emptied_test_backend_steps_as_a_new_one() {
    let executor = Executor::builder().frame_workers(1).background_workers(1).build().unwrap();
    let new_backend = || -> Box<dyn PhysicsBackend> { Box::new(Fake::new(CallLog::default())) };
    let layouts = [LayoutConfig::default(), layout_mode(ChunkCapacity::Rows(7), 5).unwrap()];
    let runs: Vec<_> = layouts
        .iter()
        .map(|&layout| {
            empty_restore(&executor, &new_backend, PhysicsConfig::default(), layout).unwrap()
        })
        .collect();
    assert_eq!(runs[0].len(), TICKS as usize);
    assert_eq!(runs[0], runs[1]);
}
