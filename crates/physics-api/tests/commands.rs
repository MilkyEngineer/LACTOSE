//! Commands (`README.md`, "Commands" and "Keys"): the step applies every command recorded
//! since the previous step in command order, the same under every layout, across the tick
//! boundary for one recorded in `Stage::Post` or in `Stage::Physics` after the step; a
//! command for a non-body or a dropped spawn's placeholder is dropped; a bound predicted
//! body is named by either key.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::{NetId, StableKey, Tick};
use lactose_ecs::WorldKind;
use lactose_math::Scalar;
use lactose_physics_api::{PhysicsCommand, QueryFilter, Ray, Transform};
use lactose_sim::harness::{ChunkCapacity, layout_mode};

fn impulse(body: StableKey, x: i32) -> PhysicsCommand {
    PhysicsCommand::ApplyImpulse { body, impulse: v(x, 0, 0) }
}

fn ordered_steps(options: Options) -> Vec<(Tick, Vec<PhysicsCommand>)> {
    let mut h = options.build();
    let [k1, k2, k3, k4] = [key(1), key(2), key(3), key(4)];
    for (n, body) in [k1, k2, k3, k4].into_iter().enumerate() {
        h.spawn_body(body, at(n as i32 * 3, 0, 0));
    }
    let torque = PhysicsCommand::ApplyTorqueImpulse { body: k2, torque: v(0, 1, 0) };
    let force = PhysicsCommand::ApplyForce { body: k3, force: v(0, 0, 1) };
    let teleport = PhysicsCommand::Teleport { body: k4, transform: at(0, 9, 0) };
    h.script()
        // Recorded by three kernels in `Stage::Simulation`, in the opposite of key order.
        .at(2, When::Simulation, k3, Action::Command(impulse(k1, 3)))
        .at(2, When::Simulation, k1, Action::Command(impulse(k1, 1)))
        .at(2, When::Simulation, k1, Action::Command(impulse(k2, 2)))
        // After the step: the next tick's.
        .at(2, When::AfterStep, k1, Action::Command(torque))
        // In `Stage::Post`: the next tick's, after the physics stage's.
        .at(2, When::Post, k4, Action::Command(force))
        .at(2, When::Post, k2, Action::Command(teleport))
        .at(3, When::Simulation, k2, Action::Command(impulse(k4, 4)));
    h = h.run(3);
    steps(&h.calls.all())
}

#[test]
fn the_step_applies_commands_in_command_order_across_the_tick_boundary() {
    let [k1, k2, k3, k4] = [key(1), key(2), key(3), key(4)];
    let expected = vec![
        (Tick(1), vec![]),
        (Tick(2), vec![impulse(k1, 1), impulse(k2, 2), impulse(k1, 3)]),
        (
            Tick(3),
            vec![
                PhysicsCommand::ApplyTorqueImpulse { body: k2, torque: v(0, 1, 0) },
                PhysicsCommand::Teleport { body: k4, transform: at(0, 9, 0) },
                PhysicsCommand::ApplyForce { body: k3, force: v(0, 0, 1) },
                impulse(k4, 4),
            ],
        ),
    ];
    assert_eq!(ordered_steps(Options::new(WorldKind::Server)), expected);
    for capacity in
        [ChunkCapacity::Rows(1), ChunkCapacity::Rows(7), ChunkCapacity::Bytes(16 * 1024)]
    {
        for seed in [3, 4] {
            for workers in [1, 4] {
                let mut options = Options::new(WorldKind::Server);
                options.layout = layout_mode(capacity, seed).unwrap();
                options.workers = workers;
                assert_eq!(
                    ordered_steps(options),
                    expected,
                    "{capacity:?}, seed {seed}, {workers} workers"
                );
            }
        }
    }
}

#[test]
fn a_command_for_a_non_body_is_dropped_and_one_for_a_fixed_body_reaches_the_backend() {
    let mut h = Harness::new(WorldKind::Server);
    let (body, scenery, plain, missing) = (key(1), key(2), key(3), key(4));
    h.spawn_body(body, at(0, 0, 0));
    h.spawn(scenery, (at(5, 0, 0), sphere()));
    h.spawn(plain, (at(9, 0, 0),));
    let teleport = PhysicsCommand::Teleport { body: plain, transform: Transform::IDENTITY };
    h.script()
        .at(2, When::Simulation, body, Action::Command(impulse(plain, 1)))
        .at(2, When::Simulation, body, Action::Command(teleport))
        .at(2, When::Simulation, body, Action::Command(impulse(missing, 1)))
        .at(2, When::Simulation, body, Action::Command(impulse(scenery, 1)))
        .at(2, When::Simulation, body, Action::Command(impulse(body, 1)));
    h = h.run(2);
    let steps = steps(&h.take());
    // The backend drops the push on the fixed collider; this crate drops what names no body.
    assert_eq!(steps[1], (Tick(2), vec![impulse(scenery, 1), impulse(body, 1)]));
    assert_eq!(*h.world().get::<Transform>(scenery).unwrap(), at(5, 0, 0));
}

/// Probe B without an input tick: in a prediction world a spawn from a claimed body is
/// predicted only when the player has an input tick; otherwise it is dropped, and a
/// command naming its placeholder names nothing and is dropped.
#[test]
fn a_prediction_spawn_needs_an_input_tick_and_a_dropped_spawns_command_is_dropped() {
    for input_ticks in [false, true] {
        let mut h = Harness::new(WorldKind::Prediction);
        let source = key(1);
        h.spawn_body(source, at(0, 0, 0));
        h.script().input_ticks = input_ticks;
        h.script().at(2, When::Simulation, source, Action::Spawn(at(0, 5, 0)));
        h = h.run(1);
        h.take();
        h = h.run(1);
        let calls = h.take();
        let created: Vec<StableKey> = calls
            .iter()
            .filter_map(|call| match call {
                Call::CreateBody { key, .. } => Some(*key),
                _ => None,
            })
            .collect();
        let commands = steps(&calls).remove(0).1;
        if input_ticks {
            assert_eq!(created.len(), 1);
            assert!(matches!(created[0], StableKey::Predicted(_)));
            assert_eq!(commands, [impulse(created[0], 1)], "the placeholder resolved at apply");
        } else {
            assert_eq!(created, [], "the spawn was dropped");
            assert_eq!(commands, [], "its command was dropped with it");
        }
    }
}

#[test]
fn a_bound_predicted_body_is_named_by_either_key_from_the_physics_sync_point() {
    let mut h = Harness::new(WorldKind::Prediction);
    let source = key(1);
    h.spawn_body(source, at(0, 0, 0));
    h.script().input_ticks = true;
    h.script().at(2, When::Simulation, source, Action::Spawn(at(0, 5, 0)));
    h = h.run(2);
    let predicted = *h
        .take()
        .iter()
        .find_map(|call| match call {
            Call::CreateBody { key: key @ StableKey::Predicted(_), .. } => Some(key),
            _ => None,
        })
        .unwrap();
    let StableKey::Predicted(prediction) = predicted else { unreachable!() };
    let net = NetId::cooked(77);
    h.world_mut().bind(prediction, net).unwrap();
    // Bound between ticks: the physics sync point has not seen the bind yet.
    assert!(h.physics().is_body(predicted));
    assert!(!h.physics().is_body(StableKey::Net(net)));
    h.script().at(3, When::Simulation, source, Action::Command(impulse(StableKey::Net(net), 2)));
    h = h.run(1);
    assert!(h.physics().is_body(StableKey::Net(net)));
    // The command named the body by its `Net` key; the step got the origin key.
    assert_eq!(steps(&h.take())[0].1, [impulse(predicted, 2)]);
    // A filter excluding the `Net` key excludes the body.
    let pose = *h.world().get::<Transform>(predicted).unwrap();
    let from = pose.translation + v(0, 10, 0);
    let ray = Ray::new(from, -v(0, 1, 0), Scalar::from_i32(20));
    assert_eq!(h.physics().cast_ray(&ray, &QueryFilter::ALL).map(|hit| hit.body), Some(predicted));
    let excluded = h.physics().cast_ray(&ray, &QueryFilter::excluding(StableKey::Net(net)));
    assert_ne!(excluded.map(|hit| hit.body), Some(predicted));
}
