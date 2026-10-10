//! Structural changes through the direct methods and through commands: spawn, get, insert,
//! remove, despawn, region moves and ownership migration, each in the structural log with
//! its tick.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::{EntityRef, ErrorKind, StableKey};
use lactose_ecs::{
    Commands, LayoutConfig, LogEntry, OwnedBy, RegionConfig, RegionMode, Schedule,
    StructuralChange, World, WorldKind, system,
};

fn entries(world: &World) -> Vec<LogEntry> {
    let mut cursor = world.structural_log().cursor_at_start();
    world.structural_log().since(&mut cursor).unwrap().copied().collect()
}

fn other_region(world: &mut World) {
    world
        .create_region(RegionConfig { id: OTHER_REGION, seed: 7, mode: RegionMode::Simulated })
        .unwrap();
}

#[test]
fn direct_methods_change_structure_and_log_it() {
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    other_region(&mut world);
    world.set_tick(t(5)).unwrap();
    let owner = net(1);
    let item = net(2);

    world.spawn(owner, REGION, (pos(1, 2), vel(3, 4))).unwrap();
    assert_eq!(world.get::<Position>(owner), Some(&pos(1, 2)));
    assert_eq!(world.get::<Health>(owner), None);
    assert!(world.spawn(owner, REGION, (pos(0, 0),)).is_err(), "a live key spawns once");

    world.insert(owner, Health(10)).unwrap();
    assert_eq!(world.get::<Health>(owner), Some(&Health(10)));
    world.insert(owner, Health(9)).unwrap();
    assert_eq!(world.remove::<Velocity>(owner).unwrap(), Some(vel(3, 4)));
    assert_eq!(world.remove::<Velocity>(owner).unwrap(), None);

    world.spawn(item, REGION, (Counter(1), OwnedBy { owner: EntityRef::new(owner) })).unwrap();
    assert_eq!(world.owned_by(owner).collect::<Vec<_>>(), [item]);

    world.move_to_region(owner, OTHER_REGION).unwrap();
    assert_eq!(world.region_of(owner), Some(OTHER_REGION));
    assert_eq!(world.region_of(item), Some(OTHER_REGION), "owned entities migrate");
    assert_eq!(world.entities_in(OTHER_REGION).collect::<Vec<_>>(), [owner, item]);
    assert_eq!(world.region(REGION).unwrap().entity_count, 0);

    world.despawn(owner).unwrap();
    assert!(!world.contains(owner) && !world.contains(item), "owned entities go too");
    assert_eq!(world.despawn(owner).unwrap_err().kind(), ErrorKind::NotFound);

    let tick = t(5);
    let log = [
        StructuralChange::Spawned { key: owner, region: REGION },
        StructuralChange::Added {
            key: owner,
            component: <Health as lactose_reflect::Reflect>::KEY,
        },
        StructuralChange::Removed {
            key: owner,
            component: <Velocity as lactose_reflect::Reflect>::KEY,
        },
        StructuralChange::Spawned { key: item, region: REGION },
        StructuralChange::Moved { key: owner, from: REGION, to: OTHER_REGION },
        StructuralChange::Moved { key: item, from: REGION, to: OTHER_REGION },
        StructuralChange::Despawned { key: owner, region: OTHER_REGION },
        StructuralChange::Despawned { key: item, region: OTHER_REGION },
    ]
    .map(|change| LogEntry { tick, change });
    assert_eq!(entries(&world), log);
}

/// Drives the commands of one tick from an exclusive system, which knows the tick.
#[system(name = "test::structure_script", worlds = [server], exclusive)]
fn script(world: &mut World, cmd: Commands) {
    let (owner, item) = (net(1), net(2));
    match world.tick().0 {
        1 => {
            let owner = cmd.spawn_in(REGION, (pos(1, 1), vel(0, 0)));
            let item = cmd.spawn_in(REGION, (Counter(0),));
            cmd.set(item, OwnedBy { owner: EntityRef::new(owner) });
        }
        2 => {
            cmd.set(owner, Health(5));
            cmd.remove::<Velocity>(owner);
            cmd.set(item, Counter(7));
        }
        3 => cmd.move_to_region(owner, OTHER_REGION),
        4 => {
            // The item moves back on its own, after its owner took it along.
            cmd.move_to_region(owner, REGION);
            cmd.move_to_region(item, OTHER_REGION);
        }
        5 => cmd.despawn(owner),
        _ => {}
    }
}

#[test]
fn commands_change_structure_at_the_sync_point_and_log_it() {
    let executor = executor();
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    other_region(&mut world);
    let mut schedule = Schedule::builder(WorldKind::Server);
    schedule.stage("script", &[]).add(&SCRIPT);
    let mut schedule = schedule.build().unwrap();
    let (owner, item) = (net(1), net(2));

    world = tick(&mut schedule, world, &executor);
    assert_eq!(world.get::<Position>(owner), Some(&pos(1, 1)));
    assert_eq!(world.owned_by(owner).collect::<Vec<_>>(), [item]);
    assert_eq!(world.referrers(owner).map(|r| r.source).collect::<Vec<_>>(), [item]);
    world = tick(&mut schedule, world, &executor);
    assert_eq!(world.get::<Health>(owner), Some(&Health(5)));
    assert_eq!(world.get::<Velocity>(owner), None);
    assert_eq!(world.get::<Counter>(item), Some(&Counter(7)));
    world = tick(&mut schedule, world, &executor);
    assert_eq!(world.region_of(item), Some(OTHER_REGION));
    world = tick(&mut schedule, world, &executor);
    assert_eq!(world.region_of(owner), Some(REGION));
    assert_eq!(world.region_of(item), Some(OTHER_REGION));
    world = tick(&mut schedule, world, &executor);
    assert!(!world.contains(owner) && !world.contains(item));

    use StructuralChange::*;
    let (health, velocity, owned_by) = (
        <Health as lactose_reflect::Reflect>::KEY,
        <Velocity as lactose_reflect::Reflect>::KEY,
        <OwnedBy as lactose_reflect::Reflect>::KEY,
    );
    let expected = [
        (1, Spawned { key: owner, region: REGION }),
        (1, Spawned { key: item, region: REGION }),
        (1, Added { key: item, component: owned_by }),
        (2, Added { key: owner, component: health }),
        (2, Removed { key: owner, component: velocity }),
        (3, Moved { key: owner, from: REGION, to: OTHER_REGION }),
        (3, Moved { key: item, from: REGION, to: OTHER_REGION }),
        (4, Moved { key: owner, from: OTHER_REGION, to: REGION }),
        (4, Moved { key: item, from: OTHER_REGION, to: REGION }),
        (4, Moved { key: item, from: REGION, to: OTHER_REGION }),
        (5, Despawned { key: owner, region: REGION }),
        (5, Despawned { key: item, region: OTHER_REGION }),
    ]
    .map(|(tick, change)| LogEntry { tick: t(tick), change });
    assert_eq!(entries(&world), expected);
}

/// Commands that cannot apply are dropped, one by one, and the rest of the buffer applies.
#[system(name = "test::bad_commands", worlds = [server], exclusive)]
fn bad_commands(world: &mut World, cmd: Commands) {
    if world.tick().0 == 1 {
        cmd.despawn(net(99));
        cmd.move_to_region(net(1), lactose_ecs::RegionId(404));
        cmd.spawn((pos(0, 0),));
        let ghost = cmd.spawn_in(lactose_ecs::RegionId(404), (pos(0, 0),));
        cmd.set(ghost, Health(1));
        let real = cmd.spawn_in(REGION, (ThrownBy { thrower: EntityRef::new(net(99)) },));
        cmd.set(real, Health(2));
    }
}

#[test]
fn commands_that_cannot_apply_are_dropped_and_dangling_references_emptied() {
    let executor = executor();
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    world.spawn(net(1), REGION, (pos(0, 0),)).unwrap();
    let mut schedule = Schedule::builder(WorldKind::Server);
    schedule.stage("script", &[]).add(&BAD_COMMANDS);
    let mut schedule = schedule.build().unwrap();
    world = tick(&mut schedule, world, &executor);
    let spawned: Vec<StableKey> = world.entities_in(REGION).collect();
    assert_eq!(spawned, [net(1), net(2)], "only the spawn into a loaded region applied");
    assert_eq!(world.get::<Health>(net(2)), Some(&Health(2)));
    assert_eq!(world.get::<ThrownBy>(net(2)).unwrap().thrower, EntityRef::NONE);
    assert_eq!(world.region_of(net(1)), Some(REGION));
}

#[test]
fn spawn_rejects_placeholders_unloaded_regions_and_the_worlds_components() {
    let mut world = world(WorldKind::Server, LayoutConfig::default());
    let err = |result: lactose_core::Result<()>| result.unwrap_err().kind();
    assert_eq!(
        err(world.spawn(StableKey::Placeholder(0), REGION, (pos(0, 0),))),
        ErrorKind::InvalidData
    );
    assert_eq!(err(world.spawn(net(1), OTHER_REGION, (pos(0, 0),))), ErrorKind::NotFound);
    let in_region = lactose_ecs::InRegion { region: REGION };
    assert_eq!(err(world.spawn(net(1), REGION, (in_region,))), ErrorKind::InvalidData);
    assert_eq!(err(world.spawn(net(1), REGION, (pos(0, 0), pos(1, 1)))), ErrorKind::InvalidData);
    assert!(!world.contains(net(1)));
}
