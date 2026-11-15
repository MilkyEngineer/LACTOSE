//! Binds and stored references (review of the implementation, S2 and M3): a reference
//! stored under a `Net` key goes to the origin key when the key is bound, also when the bind
//! arrived between a snapshot and its restore; a bind may not redirect a live entity's key.
//! And the `NetId` counter (D142): it moves past every runtime `NetId` a world is given.

#![forbid(unsafe_code)]

mod common;

use core::num::NonZeroU64;

use common::*;
use lactose_asset_api::AssetSet;
use lactose_bundle::{Baseline, BaselineRow, ComponentData, Overlay, Record};
use lactose_core::{EntityRef, NetId, PredictionKey, StableKey};
use lactose_ecs::{
    LayoutConfig, OwnedBy, PredictedOrigin, RegionConfig, RegionId, RegionMode, World, WorldKind,
};
use lactose_reflect::{Reflect, Registry};

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

// ---- The counter (D142) ----

fn id(bits: u64) -> NetId {
    NetId::from_bits(bits).unwrap()
}

fn counter(bits: u64) -> NonZeroU64 {
    NonZeroU64::new(bits).unwrap()
}

fn data<T: Reflect>(value: T) -> ComponentData {
    ComponentData { component: T::KEY, bytes: bytemuck::bytes_of(&value).to_vec() }
}

fn origin(key: PredictionKey) -> PredictedOrigin {
    PredictedOrigin { player: key.player, input_tick: key.input_tick, ordinal: key.ordinal }
}

fn predicted(ordinal: u32) -> PredictionKey {
    PredictionKey { player: PLAYER, input_tick: t(9), ordinal }
}

/// A document of `region`: its one cooked row in the baseline, and `records` over it.
fn document(registry: &Registry, region: RegionId, records: Vec<Record>) -> (Baseline, Overlay) {
    let rows = [BaselineRow { id: NetId::cooked(region.0), components: vec![data(pos(0, 0))] }];
    let baseline = Baseline::from_rows(AssetSet::new(), schemas(registry), rows).unwrap();
    let overlay = Overlay::new(baseline.content_hash(), schemas(registry), records).unwrap();
    (baseline, overlay)
}

fn load(world: &mut World, region: RegionId, records: Vec<Record>) -> lactose_core::Result<()> {
    world.create_region(RegionConfig { id: region, seed: 7, mode: RegionMode::Simulated })?;
    let (baseline, overlay) = document(&registry(), region, records);
    let loaded = world.load_region(region, &baseline, Some(&overlay));
    if loaded.is_err() {
        world.unload_region(region).unwrap();
    }
    loaded
}

/// Every way a runtime `NetId` enters a world moves the counter one past it, in every kind
/// and layout; nothing else moves it.
#[test]
fn a_runtime_net_id_given_directly_moves_the_counter_past_it() {
    for kind in [WorldKind::Server, WorldKind::Client, WorldKind::Prediction] {
        for layout in layouts() {
            let mut world = world(kind, layout);
            let at = |world: &World| world.next_net_id().get();
            assert_eq!(at(&world), 1, "{kind:?}");

            // A direct spawn, above the counter and then below it.
            world.spawn(net(5), REGION, (pos(0, 0),)).unwrap();
            assert_eq!(at(&world), 6, "{kind:?}: a spawn above the counter");
            world.spawn(net(3), REGION, (pos(0, 0),)).unwrap();
            assert_eq!(at(&world), 6, "{kind:?}: a spawn below it");
            world.spawn(StableKey::Net(NetId::cooked(0x77)), REGION, (pos(0, 0),)).unwrap();
            assert_eq!(at(&world), 6, "{kind:?}: a cooked spawn");
            // Refused: the region is not loaded, and the key is live.
            world.spawn(net(50), RegionId(99), (pos(0, 0),)).unwrap_err();
            world.spawn(net(5), REGION, (pos(0, 0),)).unwrap_err();
            assert_eq!(at(&world), 6, "{kind:?}: refused spawns");

            // An erased spawn, then a refused one (bytes of another size).
            world.spawn_erased(net(8), REGION, &[data(pos(1, 1))]).unwrap();
            assert_eq!(at(&world), 9, "{kind:?}: an erased spawn");
            let short = ComponentData { component: Position::KEY, bytes: vec![0; 3] };
            world.spawn_erased(net(60), REGION, &[short]).unwrap_err();
            assert_eq!(at(&world), 9, "{kind:?}: a refused erased spawn");

            // A bind, then refused ones: a live `NetId`, a key bound to another.
            world.bind(predicted(1), id(12)).unwrap();
            assert_eq!(at(&world), 13, "{kind:?}: a bind");
            world.bind(predicted(2), id(5)).unwrap_err();
            world.bind(predicted(1), id(70)).unwrap_err();
            world.bind(predicted(3), NetId::cooked(0x99)).unwrap();
            assert_eq!(at(&world), 13, "{kind:?}: refused and cooked binds");

            // A load: a document's runtime row, then the bind of a row with an origin.
            let row = Record::Spawn { id: id(20), components: vec![data(pos(2, 2))] };
            load(&mut world, RegionId(30), vec![row]).unwrap();
            assert_eq!(at(&world), 21, "{kind:?}: a loaded row");
            let components = vec![data(pos(3, 3)), data(origin(predicted(4)))];
            load(&mut world, RegionId(31), vec![Record::Spawn { id: id(25), components }]).unwrap();
            assert_eq!(world.alias(predicted(4)), Some(id(25)));
            assert!(world.contains(StableKey::Predicted(predicted(4))));
            assert_eq!(at(&world), 26, "{kind:?}: a loaded bind");
            // Refused whole: its second row is live.
            let records = vec![
                Record::Spawn { id: id(5), components: vec![data(pos(4, 4))] },
                Record::Spawn { id: id(40), components: vec![data(pos(4, 4))] },
            ];
            load(&mut world, RegionId(32), records).unwrap_err();
            assert!(!world.contains(net(40)));
            assert_eq!(at(&world), 26, "{kind:?}: a refused load");

            // The next allocation is the counter's value, and no ID in use.
            let next = world.allocate_net_id().unwrap();
            assert_eq!(next, id(26), "{kind:?}");
            assert_eq!(at(&world), 27);

            // An ID given and gone is not handed out again: despawned, or unloaded.
            world.spawn(net(100), REGION, (pos(5, 5),)).unwrap();
            world.despawn(net(100)).unwrap();
            assert_eq!(world.allocate_net_id().unwrap(), id(101), "{kind:?}: despawned");
            let row = Record::Spawn { id: id(200), components: vec![data(pos(6, 6))] };
            load(&mut world, RegionId(33), vec![row]).unwrap();
            world.unload_region(RegionId(33)).unwrap();
            assert_eq!(world.allocate_net_id().unwrap(), id(201), "{kind:?}: unloaded");

            if kind == WorldKind::Server {
                let below = world.next_net_id();
                for region in world.regions().collect::<Vec<_>>() {
                    for row in world.export_region(region).unwrap().rows {
                        assert!(
                            row.id.is_cooked() || row.id.to_bits() < below.get(),
                            "{:?} is not below the counter {below}",
                            row.id
                        );
                    }
                }
            }
        }
    }
}

/// A restore puts the counter back as snapshotted; the binds stay.
#[test]
fn a_restore_puts_the_counter_back_and_keeps_the_binds() {
    let mut world = world(WorldKind::Prediction, LayoutConfig::default());
    let snapshot = world.snapshot();
    world.bind(key(), id(40)).unwrap();
    assert_eq!(world.next_net_id(), counter(41), "the bind moved the counter");
    world.restore(&snapshot).unwrap();
    assert_eq!(world.next_net_id(), counter(1), "the restore put it back");
    assert_eq!(world.alias(key()), Some(id(40)), "the bind stays");
}
