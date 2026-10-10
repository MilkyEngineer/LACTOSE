//! Region modes and per-region systems: kernels run over `Simulated` regions only, a
//! per-region system runs once per such region that holds its region resources, and its
//! RNG streams are the world's.

#![forbid(unsafe_code)]

mod common;

use std::sync::Mutex;

use common::*;
use lactose_ecs::{
    Commands, Ctx, LayoutConfig, RegionConfig, RegionId, RegionMode, RegionRes, Res, Schedule,
    WorldKind, system,
};

const FROZEN: RegionId = RegionId(30);

/// Per region: the region, its weather and its first draw.
type Reports = Mutex<Vec<(RegionId, u32, u64)>>;

#[system(name = "test::weather_report", worlds = [server])]
fn weather_report(ctx: Ctx, weather: RegionRes<Weather>, reports: Res<Reports>, cmd: Commands) {
    let draw = ctx.rng(net(1)).next_u64();
    reports.lock().unwrap().push((ctx.region(), weather.0, draw));
    // A per-region spawn goes to the invocation's region.
    cmd.spawn((Counter(u64::from(weather.0)),));
}

#[system(name = "test::grow", worlds = [server])]
fn grow(counter: lactose_ecs::Mut<Counter>) {
    counter.0 += 1;
}

#[test]
fn only_simulated_regions_run_and_a_missing_region_resource_skips_one() {
    let executor = executor();
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    let config = |id, mode| RegionConfig { id, seed: id.0 * 3, mode };
    world.create_region(config(OTHER_REGION, RegionMode::Simulated)).unwrap();
    world.create_region(config(FROZEN, RegionMode::Frozen)).unwrap();
    world.insert_region_resource(REGION, Weather(1)).unwrap();
    world.insert_region_resource(FROZEN, Weather(3)).unwrap();
    world.spawn(net(1), REGION, (Counter(0),)).unwrap();
    world.spawn(net(2), FROZEN, (Counter(0),)).unwrap();
    world.insert_service::<Reports>(Box::new(Mutex::new(Vec::new())));
    let mut builder = Schedule::builder(WorldKind::Server);
    builder.stage("regions", &[]).add(&WEATHER_REPORT).add(&GROW);
    let mut schedule = builder.build().unwrap();

    world = tick(&mut schedule, world, &executor);
    let reports = std::mem::take(world.service_mut::<Reports>().unwrap().get_mut().unwrap());
    let expected_draw = world.rng(REGION, WEATHER_REPORT.key(), net(1)).unwrap().next_u64();
    assert_eq!(
        reports,
        [(REGION, 1, expected_draw)],
        "one region simulated and holding the resource"
    );
    assert_eq!(world.get::<Counter>(net(1)), Some(&Counter(1)));
    assert_eq!(world.get::<Counter>(net(2)), Some(&Counter(0)), "frozen rows are not run");
    let spawned: Vec<_> = world.entities_in(REGION).collect();
    assert_eq!(spawned.len(), 2, "the per-region spawn landed in its region");

    world.set_region_mode(FROZEN, RegionMode::Simulated).unwrap();
    world = tick(&mut schedule, world, &executor);
    let regions: Vec<RegionId> =
        world.service::<Reports>().unwrap().lock().unwrap().iter().map(|r| r.0).collect();
    assert_eq!(regions, [REGION, FROZEN], "per-region invocations in ascending order");
    assert_eq!(world.get::<Counter>(net(2)), Some(&Counter(1)));
    assert_eq!(world.region(FROZEN).unwrap().mode, RegionMode::Simulated);
}

#[test]
fn region_mode_is_the_mode_region_reports() {
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    world.create_region(RegionConfig { id: FROZEN, seed: 1, mode: RegionMode::Frozen }).unwrap();
    world.spawn(net(1), FROZEN, (Counter(0),)).unwrap();
    for region in [REGION, FROZEN] {
        assert_eq!(world.region_mode(region), world.region(region).map(|info| info.mode));
    }
    assert_eq!(world.region_mode(FROZEN), Some(RegionMode::Frozen));
    world.set_region_mode(FROZEN, RegionMode::Replicated).unwrap();
    assert_eq!(world.region_mode(FROZEN), Some(RegionMode::Replicated));
    world.unload_region(FROZEN).unwrap();
    assert_eq!(world.region_mode(FROZEN), None, "an unloaded region has no mode");
    assert_eq!(world.region_mode(RegionId(999)), None);
}

/// A region resource with a reference (review of the implementation, M5).
#[lactose_reflect::resource(key = "test::boss", persistent)]
#[derive(Debug)]
pub struct Boss {
    /// Whom the region's boss hunts.
    #[field(id = 1)]
    pub target: lactose_core::EntityRef,
}

#[test]
fn region_resources_are_validated_and_hold_origin_keys() {
    use lactose_core::{EntityRef, NetId, PredictionKey, StableKey};
    let mut registry = base_registry();
    registry.register::<Boss>().unwrap();
    let registry = std::sync::Arc::new(registry);
    let mut world = world_with(WorldKind::Prediction, LayoutConfig::default(), registry);
    let placeholder = Boss { target: EntityRef::new(StableKey::Placeholder(1)) };
    let error = world.insert_region_resource(REGION, placeholder).unwrap_err();
    assert_eq!(error.kind(), lactose_core::ErrorKind::InvalidData);
    assert!(world.region_resource::<Boss>(REGION).is_none());

    let predicted = PredictionKey { player: PLAYER, input_tick: t(2), ordinal: 5 };
    let id = NetId::from_bits(40).unwrap();
    world.bind(predicted, id).unwrap();
    let boss = Boss { target: EntityRef::new(StableKey::Net(id)) };
    world.insert_region_resource(REGION, boss).unwrap();
    let stored = world.region_resource::<Boss>(REGION).unwrap().target;
    assert_eq!(stored, EntityRef::new(StableKey::Predicted(predicted)), "stored as the origin key");

    // An update that leaves an invalid value is refused and keeps the old one.
    let error = world
        .update_region_resource::<Boss>(REGION, |boss| boss.target = placeholder.target)
        .unwrap_err();
    assert_eq!(error.kind(), lactose_core::ErrorKind::InvalidData);
    assert_eq!(world.region_resource::<Boss>(REGION).unwrap().target, stored);

    // A valid update naming the bound `Net` key is stored under the origin key.
    world
        .update_region_resource::<Boss>(REGION, |boss| {
            boss.target = EntityRef::new(StableKey::Net(id))
        })
        .unwrap();
    assert_eq!(world.region_resource::<Boss>(REGION).unwrap().target, stored);
    let error = world.update_region_resource::<Boss>(RegionId(77), |_| {}).unwrap_err();
    assert_eq!(error.kind(), lactose_core::ErrorKind::NotFound);
}
