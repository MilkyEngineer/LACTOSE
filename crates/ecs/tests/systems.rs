//! The systems of architecture section 6.1, written with `#[system]` and `#[finalize]` in a
//! crate that forbids `unsafe`, run for several ticks in a server world and a prediction
//! world.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::{EntityRef, PredictionKey, StableKey};
use lactose_ecs::{LayoutConfig, World, WorldKind};
use lactose_math::Scalar;

/// The bomb `net(1)` throws at tick 1: its key is predicted in both worlds.
fn bomb(world: &World) -> StableKey {
    let bombs: Vec<StableKey> =
        world.entities_in(REGION).filter(|&key| world.get::<Bomb>(key).is_some()).collect();
    assert_eq!(bombs.len(), 1, "one bomb is thrown");
    bombs[0]
}

#[test]
fn the_example_systems_run_on_the_server() {
    let executor = executor();
    let mut schedule = game_schedule(WorldKind::Server);
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    populate(&mut world);
    schedule.prepare(&world).unwrap();
    let (player, npc) = (net(1), net(2));

    world = tick(&mut schedule, world, &executor);
    let bomb = bomb(&world);
    let StableKey::Predicted(PredictionKey { player: owner, input_tick, .. }) = bomb else {
        panic!("a spawn from a claimed entity with an input tick is predicted: {bomb:?}");
    };
    assert_eq!((owner, input_tick), (PLAYER, t(1)));
    let StableKey::Predicted(predicted) = bomb else { unreachable!() };
    assert_eq!(world.alias(predicted), Some(lactose_core::NetId::from_bits(3).unwrap()));
    assert_eq!(world.get::<lactose_ecs::ClaimedBy>(bomb).unwrap().player, PLAYER);
    assert_eq!(world.get::<ThrownBy>(bomb).unwrap().thrower, EntityRef::new(player));
    assert_eq!(world.referrers(player).map(|r| r.source).collect::<Vec<_>>(), [bomb]);
    assert_eq!(world.get::<HeldItem>(player).unwrap().kind, ItemKind::None);
    assert_eq!(world.resource::<ImpulseCount>().unwrap().0, 1, "the physics stage read it");

    assert_eq!(world.get::<Health>(npc), Some(&Health(90)));
    assert_eq!(world.get::<Health>(player), Some(&Health(100)));
    assert_eq!(world.get::<Velocity>(npc).unwrap().y, Scalar::from_i32(-2));
    assert_eq!(world.get::<Position>(player), Some(&pos(1, 98)));
    assert_eq!(world.resource::<TotalMass>().unwrap().0, Scalar::from_i32(8));
    let events = world.take_present_events();
    assert_eq!(events.emitted.len(), 1);
    assert_eq!(events.emitted[0].key.origin, player);
    assert_eq!(events.emitted[0].get::<AudioRequest>(), Some(AudioRequest::FOOTSTEP));

    world = tick(&mut schedule, world, &executor);
    world = tick(&mut schedule, world, &executor);
    assert_eq!(bomb, self::bomb(&world), "one throw only");
    assert_eq!(world.get::<Health>(npc), Some(&Health(70)));
    assert_eq!(world.get::<Velocity>(npc).unwrap().y, Scalar::from_i32(-6));
    assert_eq!(world.get::<Position>(npc), Some(&pos(5, 88)));
    assert_eq!(world.tick(), t(3));
    assert!(world.take_present_events().emitted.is_empty());
}

#[test]
fn the_prediction_world_runs_kernels_over_claimed_entities_only() {
    let executor = executor();
    let mut server = world(WorldKind::Server, LayoutConfig::default());
    let mut client = world(WorldKind::Prediction, LayoutConfig::default());
    populate(&mut server);
    populate(&mut client);
    let mut server_schedule = game_schedule(WorldKind::Server);
    let mut client_schedule = game_schedule(WorldKind::Prediction);
    assert!(
        !client_schedule.order().iter().any(|system| system.name() == "game_sim::total_mass"),
        "a system whose worlds exclude the kind is skipped"
    );
    let (player, npc) = (net(1), net(2));

    for _ in 0..3 {
        server = tick(&mut server_schedule, server, &executor);
        client = tick(&mut client_schedule, client, &executor);
    }
    let bomb = bomb(&client);
    assert_eq!(bomb, self::bomb(&server), "both machines derive the same key");
    let StableKey::Predicted(predicted) = bomb else { panic!("predicted") };
    assert_eq!(client.alias(predicted), None, "only the server binds a NetId");
    assert!(server.alias(predicted).is_some());
    assert_eq!(client.get::<lactose_ecs::ClaimedBy>(bomb).unwrap().player, PLAYER);

    // The claimed player moved and fell like on the server; the unclaimed proxy did not.
    assert_eq!(client.get::<Position>(player), server.get::<Position>(player));
    assert_eq!(client.get::<Velocity>(npc), Some(&vel(0, 0)));
    assert_eq!(client.get::<Health>(npc), Some(&Health(100)), "damage applies to claimed rows");
    assert_eq!(client.get::<AnimState>(npc), Some(&AnimState { phase: 3 }));
    assert_eq!(client.get::<AnimState>(player), Some(&AnimState { phase: 10 }));
    let footsteps = client.take_present_events();
    assert_eq!(footsteps.emitted.len(), 1);
    assert_eq!(footsteps.emitted[0].key.origin, player);
}
