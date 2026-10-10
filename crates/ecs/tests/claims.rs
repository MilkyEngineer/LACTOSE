//! The claim index behind `World::claimed`: every path that gives a row `ClaimedBy` or
//! takes it away keeps the index current (spawn, insert, remove, despawn, ownership
//! despawn, commands, predicted spawns, a region unload, a snapshot and restore), the
//! keys come out in ascending origin-key order, and the result is the same under every
//! layout.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::{EntityRef, PlayerId, StableKey};
use lactose_ecs::{
    ClaimedBy, Commands, LayoutConfig, OwnedBy, RegionConfig, RegionMode, Schedule, StableKeyOf,
    World, WorldKind, system,
};

const OTHER: PlayerId = PlayerId(4);

fn claimed(world: &World, player: PlayerId) -> Vec<StableKey> {
    world.claimed(player).collect()
}

/// Claims every entity with positive health for the player its health names, and releases
/// the rest: the command path into the index.
#[system(name = "test::claim_by_health", worlds = [server, prediction])]
fn claim_by_health(cmd: Commands, key: StableKeyOf, hp: &Health) {
    if hp.0 > 0 {
        cmd.set(*key, ClaimedBy { player: PlayerId(hp.0 as u32) });
    } else {
        cmd.remove::<ClaimedBy>(*key);
    }
}

fn claim_schedule(kind: WorldKind) -> Schedule {
    let mut builder = Schedule::builder(kind);
    builder.stage("claims", &[]).add(&CLAIM_BY_HEALTH);
    builder.build().unwrap()
}

#[test]
fn direct_methods_keep_the_index_in_key_order() {
    for layout in layouts() {
        let mut world = world(WorldKind::Server, layout);
        let (a, b, c) = (net(1), net(2), net(3));
        // Spawned out of key order, with the claim in the bundle or inserted after.
        world.spawn(c, REGION, (pos(0, 0), ClaimedBy { player: PLAYER })).unwrap();
        world.spawn(a, REGION, (pos(0, 0),)).unwrap();
        world.spawn(b, REGION, (pos(0, 0), ClaimedBy { player: OTHER })).unwrap();
        world.insert(a, ClaimedBy { player: PLAYER }).unwrap();
        assert_eq!(claimed(&world, PLAYER), [a, c]);
        assert_eq!(claimed(&world, OTHER), [b]);
        assert_eq!(claimed(&world, PlayerId(9)), []);

        // Re-claiming moves the entity between players; an equal rewrite changes nothing.
        world.insert(c, ClaimedBy { player: OTHER }).unwrap();
        world.insert(a, ClaimedBy { player: PLAYER }).unwrap();
        assert_eq!(claimed(&world, PLAYER), [a]);
        assert_eq!(claimed(&world, OTHER), [b, c]);

        // Removing the component and despawning each release the claim.
        assert!(world.remove::<ClaimedBy>(b).unwrap().is_some());
        world.despawn(c).unwrap();
        assert_eq!(claimed(&world, OTHER), []);
        assert_eq!(claimed(&world, PLAYER), [a]);
        assert_eq!(world.get::<ClaimedBy>(a).map(|claim| claim.player), Some(PLAYER));
    }
}

#[test]
fn owned_entities_leave_the_index_with_their_owner() {
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    let (owner, item) = (net(1), net(2));
    world.spawn(owner, REGION, (pos(0, 0), ClaimedBy { player: PLAYER })).unwrap();
    world
        .spawn(
            item,
            REGION,
            (Counter(1), OwnedBy { owner: EntityRef::new(owner) }, ClaimedBy { player: PLAYER }),
        )
        .unwrap();
    assert_eq!(claimed(&world, PLAYER), [owner, item]);
    world.despawn(owner).unwrap();
    assert!(!world.contains(item), "owned entities go with their owner");
    assert_eq!(claimed(&world, PLAYER), []);
}

#[test]
fn commands_and_predicted_spawns_keep_the_index() {
    let executor = executor();
    for layout in layouts() {
        // Commands: `set` claims, `remove` releases, in one apply.
        let mut world = world(WorldKind::Server, layout);
        let (a, b, c) = (net(1), net(2), net(3));
        world.spawn(a, REGION, (Health(3),)).unwrap();
        world.spawn(b, REGION, (Health(4),)).unwrap();
        world.spawn(c, REGION, (Health(0), ClaimedBy { player: PLAYER })).unwrap();
        let world = tick(&mut claim_schedule(WorldKind::Server), world, &executor);
        assert_eq!(claimed(&world, PLAYER), [a]);
        assert_eq!(claimed(&world, OTHER), [b]);
        assert!(world.get::<ClaimedBy>(c).is_none(), "released by the command");

        // A predicted spawn inherits the claim, on the server and in the prediction world.
        for kind in [WorldKind::Server, WorldKind::Prediction] {
            let mut world = world_with(kind, layout, registry());
            populate(&mut world);
            let world = tick(&mut game_schedule(kind), world, &executor);
            let claims = claimed(&world, PLAYER);
            assert_eq!(claims.len(), 2, "the player and the bomb: {claims:?}");
            assert_eq!(claims[0], net(1), "Net keys order before Predicted ones");
            assert!(matches!(claims[1], StableKey::Predicted(_)), "{claims:?}");
            assert!(claims.windows(2).all(|pair| pair[0] < pair[1]));
        }
    }
}

#[test]
fn an_unload_releases_and_a_restore_puts_the_index_back() {
    let mut world = world(WorldKind::Prediction, LayoutConfig::default());
    world
        .create_region(RegionConfig { id: OTHER_REGION, seed: 7, mode: RegionMode::Simulated })
        .unwrap();
    let (a, b) = (net(1), net(2));
    world.spawn(a, REGION, (pos(0, 0), ClaimedBy { player: PLAYER })).unwrap();
    world.spawn(b, OTHER_REGION, (pos(0, 0), ClaimedBy { player: PLAYER })).unwrap();
    assert_eq!(claimed(&world, PLAYER), [a, b]);

    let snapshot = world.snapshot();
    world.unload_region(OTHER_REGION).unwrap();
    assert_eq!(claimed(&world, PLAYER), [a], "an unload releases the region's claims");
    world.insert(a, ClaimedBy { player: OTHER }).unwrap();
    world.spawn(net(3), REGION, (pos(0, 0), ClaimedBy { player: PLAYER })).unwrap();
    assert_eq!(claimed(&world, PLAYER), [net(3)]);
    assert_eq!(claimed(&world, OTHER), [a]);

    world.restore(&snapshot).unwrap();
    assert_eq!(claimed(&world, PLAYER), [a, b], "the snapshot carries the index");
    assert_eq!(claimed(&world, OTHER), []);
    assert_eq!(world.get::<ClaimedBy>(b).map(|claim| claim.player), Some(PLAYER));
}
