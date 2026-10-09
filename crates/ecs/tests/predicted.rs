//! Predicted keys (architecture section 7): the same key on the server and the client
//! without a message, unshifted by spawns the client cannot reproduce, a `NetId` on a
//! collision, binds and aliases, and the origin key carried through a save.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_asset_api::AssetSet;
use lactose_bundle::{Baseline, Overlay, Record};
use lactose_core::{EntityRef, ErrorKind, NetId, PredictionKey, StableKey};
use lactose_ecs::{
    ClaimedBy, Commands, LayoutConfig, PredictedOrigin, RegionConfig, RegionMode, Schedule, With,
    World, WorldKind, system,
};

/// Two spawns from one invocation of a system that runs in both worlds.
#[system(name = "test::double_spawn", worlds = [server, prediction], after = debris)]
fn double_spawn(cmd: Commands, _claimed: With<ClaimedBy>, input: &PlayerInput) {
    if input.throw {
        let first = cmd.spawn((Bomb { fuse: 1 },));
        cmd.spawn((Bomb { fuse: 2 }, ThrownBy { thrower: EntityRef::new(first) }));
    }
}

/// A spawn only the server makes, applied before the predicted ones in the same tick.
#[system(name = "test::debris", worlds = [server])]
fn debris(cmd: Commands, input: &PlayerInput) {
    if input.throw {
        cmd.spawn((Bomb { fuse: 99 },));
    }
}

fn schedule(kind: WorldKind) -> Schedule {
    let mut builder = Schedule::builder(kind);
    builder.stage("input", &[]).add(&READ_INPUT);
    builder.stage("spawn", &[]).add(&DEBRIS).add(&DOUBLE_SPAWN);
    builder.build().unwrap()
}

/// The bombs of a world, by fuse.
fn bombs(world: &World) -> Vec<(u32, StableKey)> {
    let mut bombs: Vec<(u32, StableKey)> = world
        .entities_in(REGION)
        .filter_map(|key| Some((world.get::<Bomb>(key)?.fuse, key)))
        .collect();
    bombs.sort();
    bombs
}

fn predicted(key: StableKey) -> PredictionKey {
    match key {
        StableKey::Predicted(key) => key,
        other => panic!("{other:?} is not predicted"),
    }
}

/// A world with the claimed player `net(1)` throwing.
fn thrower(kind: WorldKind) -> World {
    let mut world = world(kind, LayoutConfig::default());
    populate(&mut world);
    world
}

#[test]
fn both_machines_derive_the_same_keys_and_a_server_only_spawn_shifts_nothing() {
    let executor = executor();
    let server = tick(&mut schedule(WorldKind::Server), thrower(WorldKind::Server), &executor);
    let client =
        tick(&mut schedule(WorldKind::Prediction), thrower(WorldKind::Prediction), &executor);
    let (server_bombs, client_bombs) = (bombs(&server), bombs(&client));
    assert_eq!(server_bombs.len(), 3, "two predicted spawns and the debris");
    assert!(matches!(server_bombs[2], (99, StableKey::Net(_))), "the debris is not predicted");
    assert_eq!(client_bombs, server_bombs[..2], "the predicted keys agree");
    let (first, second) = (predicted(client_bombs[0].1), predicted(client_bombs[1].1));
    assert_eq!((first.player, first.input_tick), (PLAYER, t(1)));
    assert_ne!(first.ordinal, second.ordinal, "the spawn index is part of the ordinal");
    for world in [&server, &client] {
        let origin = world.get::<PredictedOrigin>(client_bombs[0].1).unwrap();
        assert_eq!((origin.player, origin.input_tick, origin.ordinal), (PLAYER, t(1), first.ordinal));
        assert_eq!(world.get::<ClaimedBy>(client_bombs[1].1).unwrap().player, PLAYER);
        let reference = world.get::<ThrownBy>(client_bombs[1].1).unwrap().thrower;
        assert_eq!(reference, EntityRef::new(client_bombs[0].1), "the placeholder resolved");
    }
    // The server bound a NetId to each predicted spawn, from its counter.
    let aliases: Vec<Option<NetId>> =
        [first, second].iter().map(|&key| server.alias(key)).collect();
    assert!(aliases.iter().all(Option::is_some));
    assert_eq!([first, second].map(|key| client.alias(key)), [None, None]);
}

#[test]
fn a_spawn_without_an_input_tick_is_not_predicted() {
    let executor = executor();
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("spawn", &[]).add(&DEBRIS).add(&DOUBLE_SPAWN);
    let server = tick(&mut builder.build().unwrap(), thrower(WorldKind::Server), &executor);
    let bombs = bombs(&server);
    assert!(bombs.iter().all(|(_, key)| matches!(key, StableKey::Net(_))), "{bombs:?}");
    assert!(bombs.iter().all(|&(_, key)| server.get::<ClaimedBy>(key).is_none()), "no claim passes on");

    let mut builder = Schedule::builder(WorldKind::Prediction);
    builder.stage("spawn", &[]).add(&DEBRIS).add(&DOUBLE_SPAWN);
    let client = tick(&mut builder.build().unwrap(), thrower(WorldKind::Prediction), &executor);
    assert!(self::bombs(&client).is_empty(), "a client cannot assign a NetId");
}

#[test]
fn a_collision_falls_back_to_a_net_id() {
    let executor = executor();
    let taken = bombs(&tick(&mut schedule(WorldKind::Server), thrower(WorldKind::Server), &executor))[0].1;

    let mut server = thrower(WorldKind::Server);
    server.spawn(taken, REGION, (pos(9, 9),)).unwrap();
    let server = tick(&mut schedule(WorldKind::Server), server, &executor);
    let bombs = bombs(&server);
    assert_eq!(bombs.len(), 3);
    assert!(matches!(bombs[0].1, StableKey::Net(_)), "the colliding spawn took a NetId");
    assert!(matches!(bombs[1].1, StableKey::Predicted(_)), "the other one is still predicted");
    assert_eq!(server.get::<Position>(taken), Some(&pos(9, 9)), "the live entity is untouched");

    let mut client = thrower(WorldKind::Prediction);
    client.spawn(taken, REGION, (pos(9, 9),)).unwrap();
    let client = tick(&mut schedule(WorldKind::Prediction), client, &executor);
    assert_eq!(self::bombs(&client).len(), 1, "the client drops the colliding spawn");
}

#[test]
fn binds_alias_the_net_key_to_the_origin_key() {
    let mut world = world(WorldKind::Prediction, LayoutConfig::default());
    let key = PredictionKey { player: PLAYER, input_tick: t(4), ordinal: 77 };
    let id = NetId::from_bits(500).unwrap();
    // A bind before the spawn is kept and applies once the entity exists.
    world.bind(key, id).unwrap();
    assert!(!world.contains(StableKey::Net(id)));
    world.spawn(StableKey::Predicted(key), REGION, (Health(3),)).unwrap();
    assert_eq!(world.origin(StableKey::Net(id)), Some(StableKey::Predicted(key)));
    assert_eq!(world.get::<Health>(StableKey::Net(id)), Some(&Health(3)));
    assert_eq!(world.alias(key), Some(id));
    world.bind(key, id).unwrap();
    let other = NetId::from_bits(501).unwrap();
    assert_eq!(world.bind(key, other).unwrap_err().kind(), ErrorKind::InvalidData);
    let other_key = PredictionKey { ordinal: 78, ..key };
    assert_eq!(world.bind(other_key, id).unwrap_err().kind(), ErrorKind::InvalidData);

    // A reference written through the Net key is stored as the origin key.
    world.spawn(net(9), REGION, (ThrownBy { thrower: EntityRef::new(StableKey::Net(id)) },)).unwrap();
    let stored = world.get::<ThrownBy>(net(9)).unwrap().thrower;
    assert_eq!(stored, EntityRef::new(StableKey::Predicted(key)));
    assert_eq!(world.referrers(StableKey::Net(id)).count(), 1);
}

#[test]
fn a_predicted_origin_and_a_reference_to_it_survive_a_save_byte_for_byte() {
    let executor = executor();
    let mut server = tick(&mut schedule(WorldKind::Server), thrower(WorldKind::Server), &executor);
    let bombs = bombs(&server);
    let first = bombs[0].1;
    // A runtime entity that refers to the predicted one.
    let holder = StableKey::Net(server.allocate_net_id().unwrap());
    server.spawn(holder, REGION, (ThrownBy { thrower: EntityRef::new(first) },)).unwrap();
    let exported = server.export_region(REGION).unwrap();
    let first_id = server.alias(predicted(first)).unwrap();
    assert!(exported.rows.iter().any(|row| row.id == first_id));

    let registry = registry();
    let baseline = Baseline::from_rows(AssetSet::new(), schemas(&registry), []).unwrap();
    let mut records = Vec::new();
    for row in &exported.rows {
        records.push(Record::Spawn { id: row.id, components: row.components.clone() });
        if let Some(owner) = row.owner {
            records.push(Record::OwnershipChange { id: row.id, owner: Some(owner) });
        }
    }
    let overlay = Overlay::new(baseline.content_hash(), schemas(&registry), records).unwrap();

    let mut loaded = lactose_ecs::WorldBuilder::new(WorldKind::Server, registry).build().unwrap();
    loaded
        .create_region(RegionConfig { id: REGION, seed: exported.seed, mode: RegionMode::Simulated })
        .unwrap();
    loaded.load_region(REGION, &baseline, Some(&overlay)).unwrap();
    assert!(loaded.contains(first), "the entity is stored under its origin key again");
    assert_eq!(loaded.origin(StableKey::Net(first_id)), Some(first));
    assert_eq!(loaded.alias(predicted(first)), Some(first_id));
    assert_eq!(loaded.get::<ThrownBy>(holder).unwrap().thrower, EntityRef::new(first));
    assert_eq!(loaded.referrers(first).map(|r| r.source).collect::<Vec<_>>(), [holder, bombs[1].1]);
    assert_eq!(loaded.export_region(REGION).unwrap(), exported, "byte for byte");
}
