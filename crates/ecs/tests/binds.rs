//! Binds and stored references (review of the implementation, S2 and M3): a reference
//! stored under a `Net` key goes to the origin key when the key is bound, also when the bind
//! arrived between a snapshot and its restore; a bind may not redirect a live entity's key.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::{EntityRef, NetId, PredictionKey, StableKey};
use lactose_ecs::{LayoutConfig, OwnedBy, WorldKind};

fn key() -> PredictionKey {
    PredictionKey { player: PLAYER, input_tick: t(4), ordinal: 0xABCD }
}

fn net_77() -> NetId {
    NetId::from_bits(77).unwrap()
}

#[test]
fn a_bind_between_snapshot_and_restore_leaves_storage_and_index_on_the_origin_key() {
    let mut world = world(WorldKind::Prediction, LayoutConfig::default());
    // A row refers to the server's key of an entity whose bind has not arrived yet.
    let thrown_by = ThrownBy { thrower: EntityRef::new(StableKey::Net(net_77())) };
    world.spawn(net(10), REGION, (thrown_by,)).unwrap();
    let snapshot = world.snapshot();
    world.bind(key(), net_77()).unwrap();
    let origin = Some(StableKey::Predicted(key()));
    assert_eq!(world.get::<ThrownBy>(net(10)).unwrap().thrower.get(), origin);

    world.restore(&snapshot).unwrap();
    assert_eq!(world.alias(key()), Some(net_77()), "the bind survives the restore");
    assert_eq!(world.get::<ThrownBy>(net(10)).unwrap().thrower.get(), origin);
    let by_origin: Vec<_> =
        world.referrers(StableKey::Predicted(key())).map(|r| r.source).collect();
    assert_eq!(by_origin, [net(10)]);
}

#[test]
fn an_owner_bound_between_snapshot_and_restore_still_takes_its_items_along() {
    let mut world = world(WorldKind::Prediction, LayoutConfig::default());
    world.spawn(StableKey::Predicted(key()), REGION, (pos(0, 0),)).unwrap();
    let owned = OwnedBy { owner: EntityRef::new(StableKey::Net(net_77())) };
    world.spawn(net(10), REGION, (pos(1, 1), owned)).unwrap();
    let snapshot = world.snapshot();
    world.bind(key(), net_77()).unwrap();
    world.restore(&snapshot).unwrap();
    world.despawn(StableKey::Predicted(key())).unwrap();
    assert!(!world.contains(net(10)), "the owned item outlived its owner's despawn");
}

#[test]
fn an_owner_bound_without_a_restore_takes_its_items_along() {
    let mut world = world(WorldKind::Prediction, LayoutConfig::default());
    world.spawn(StableKey::Predicted(key()), REGION, (pos(0, 0),)).unwrap();
    let owned = OwnedBy { owner: EntityRef::new(StableKey::Net(net_77())) };
    world.spawn(net(10), REGION, (pos(1, 1), owned)).unwrap();
    world.bind(key(), net_77()).unwrap();
    world.despawn(StableKey::Predicted(key())).unwrap();
    assert!(!world.contains(net(10)));
}

#[test]
fn a_bind_to_a_net_key_that_names_a_live_entity_is_refused() {
    let mut world = world(WorldKind::Prediction, LayoutConfig::default());
    world.spawn(StableKey::Net(net_77()), REGION, (pos(0, 0),)).unwrap();
    let thrown_by = ThrownBy { thrower: EntityRef::new(StableKey::Net(net_77())) };
    world.spawn(net(10), REGION, (thrown_by,)).unwrap();
    assert_eq!(
        world.bind(key(), net_77()).unwrap_err().kind(),
        lactose_core::ErrorKind::InvalidData
    );
    assert_eq!(world.get::<ThrownBy>(net(10)).unwrap().thrower, thrown_by.thrower, "untouched");
    assert_eq!(world.alias(key()), None);
}
