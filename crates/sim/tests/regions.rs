//! Regions (README, "Regions"): the policy under `Anchored` with a moving anchor, pinned
//! regions, late tables, clamps and hysteresis; the refusals of the frame's region entries,
//! each of which changes nothing; unloads under `Directed` on and off the server.

#![forbid(unsafe_code)]

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::*;
use lactose_core::{ErrorKind, PlayerId};
use lactose_ecs::{Executor, RegionId, RegionMode, WorldKind};
use lactose_sim::{RegionExport, RegionPolicy, RegionRequest, SessionEvent, Simulation, TickInput};

/// Steps once with the driver's bundles, and lets the driver observe.
fn drive(
    executor: &Executor,
    driver: &mut Driver,
    simulation: Simulation,
    mut frame: TickInput,
) -> Simulation {
    driver.fill(&mut frame);
    let mut simulation = step(executor, simulation, frame);
    driver.observe(&mut simulation);
    simulation
}

/// Moves the player's entity between steps: an unjournaled edit, for the test only.
fn move_anchor(simulation: &mut Simulation, to: RegionId) {
    let key = player_entity(simulation.world_mut(), PlayerId(1)).unwrap();
    let cell = cell_of(to);
    let walker = Walker { x: cell.x * CELL + CELL / 2, y: cell.y * CELL + CELL / 2 };
    simulation.world_mut().insert(key, walker).unwrap();
    simulation.world_mut().move_to_region(key, to).unwrap();
}

/// The grid regions within `radius` of `centre`.
fn ring(centre: RegionId, radius: u32) -> BTreeSet<RegionId> {
    atlas().within(cell_of(centre), radius).collect()
}

fn loads(requests: &[RegionRequest]) -> BTreeSet<RegionId> {
    requests
        .iter()
        .filter_map(|request| match *request {
            RegionRequest::Load { region } => Some(region),
            _ => None,
        })
        .collect()
}

/// The policy's invariants after a sync point, for an anchor at `anchor`.
fn check(simulation: &Simulation, anchor: RegionId, policy: &RegionPolicy) {
    let modes = loaded(simulation);
    for (&region, &mode) in &modes {
        let distance = cell_of(region).distance(cell_of(anchor));
        assert!(distance <= policy.release_radius, "{region:?} loaded outside the release radius");
        let active = distance <= policy.activation_radius
            && simulation.world().region_assets(region).is_some();
        let expected = if active { RegionMode::Simulated } else { RegionMode::Frozen };
        assert_eq!(mode, expected, "{region:?}");
    }
    for region in ring(anchor, policy.streaming_radius) {
        assert!(
            modes.contains_key(&region) || simulation.lifecycle().is_requested(region),
            "{region:?} is within the streaming radius and neither loaded nor requested"
        );
    }
}

#[test]
fn a_moving_anchor_streams_activates_releases_and_cancels() {
    let executor = executor();
    let policy = RegionPolicy { activation_radius: 0, streaming_radius: 1, release_radius: 2 };
    let simulation =
        game(WorldKind::Server).regions(anchored(false)).policy(policy).build().unwrap();
    let mut driver = Driver::default();
    let west = region_at(0, 2);

    // A join for an unloaded region: held, and its region counts as the anchor.
    let mut simulation = drive(&executor, &mut driver, simulation, join(1, west));
    let requested = std::mem::take(&mut driver.requests);
    assert_eq!(loads(&requested), ring(west, 1));
    let in_order: Vec<RegionId> = requested
        .iter()
        .map(|request| match *request {
            RegionRequest::Load { region } => region,
            _ => unreachable!(),
        })
        .collect();
    assert!(in_order.is_sorted(), "requests ascend by region");

    // The bundles arrive, the join applies, the policy decides the modes.
    simulation = drive(&executor, &mut driver, simulation, TickInput::default());
    check(&simulation, west, &policy);
    assert_eq!(loaded(&simulation).len(), 6);

    // East one region at a time: the streaming ring follows.
    for x in 1..=3 {
        let at = region_at(x, 2);
        move_anchor(&mut simulation, at);
        simulation = drive(&executor, &mut driver, simulation, TickInput::default());
        simulation = drive(&executor, &mut driver, simulation, TickInput::default());
        check(&simulation, at, &policy);
    }
    // At x = 3 the west column left the release radius: exported, unloaded, released.
    let west_column: BTreeSet<RegionId> = (1..=3).map(|y| region_at(0, y)).collect();
    let released: BTreeSet<RegionId> = driver
        .requests
        .iter()
        .filter_map(|request| match *request {
            RegionRequest::Release { region } => Some(region),
            _ => None,
        })
        .collect();
    assert_eq!(released, west_column);
    assert_eq!(driver.stored.keys().copied().collect::<BTreeSet<_>>(), west_column);
    assert!(loaded(&simulation).keys().all(|region| !west_column.contains(region)));

    // Hysteresis: jittering across the border between x = 2 and x = 3 loads and unloads
    // nothing.
    driver.requests.clear();
    let before = loaded(&simulation).keys().copied().collect::<BTreeSet<_>>();
    for x in [2, 3, 2, 3, 2, 3] {
        move_anchor(&mut simulation, region_at(x, 2));
        simulation = drive(&executor, &mut driver, simulation, TickInput::default());
        check(&simulation, region_at(x, 2), &policy);
    }
    assert!(driver.requests.is_empty(), "{:?}", driver.requests);
    assert_eq!(loaded(&simulation).keys().copied().collect::<BTreeSet<_>>(), before);

    // A request whose region leaves the release radius before its bundle arrives is
    // cancelled, and its bundle refused if it still comes. From x = 1 the release radius
    // reaches x = 3: the east column goes, and the west column is wanted again.
    let east_column: BTreeSet<RegionId> = (1..=3).map(|y| region_at(4, y)).collect();
    move_anchor(&mut simulation, region_at(1, 2));
    simulation = step(&executor, simulation, TickInput::default());
    let requests = simulation.lifecycle_mut().take_requests();
    let released: BTreeSet<RegionId> = requests
        .iter()
        .filter_map(|request| match *request {
            RegionRequest::Release { region } => Some(region),
            _ => None,
        })
        .collect();
    assert_eq!(released, east_column);
    assert_eq!(loads(&requests), west_column);
    // Back east before the west bundles arrive: their requests are cancelled.
    move_anchor(&mut simulation, region_at(3, 2));
    simulation = step(&executor, simulation, TickInput::default());
    let requests = simulation.lifecycle_mut().take_requests();
    let cancelled: BTreeSet<RegionId> = requests
        .iter()
        .filter_map(|request| match *request {
            RegionRequest::Cancel { region } => Some(region),
            _ => None,
        })
        .collect();
    assert_eq!(cancelled, west_column);
    assert_eq!(loads(&requests), east_column);
    assert!(west_column.iter().all(|&region| !simulation.lifecycle().is_requested(region)));
    let mut late = TickInput::default();
    late.loaded.insert(region_at(0, 2), fresh(region_at(0, 2)));
    simulation = step(&executor, simulation, late);
    let failures = simulation.lifecycle_mut().take_load_failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].error.kind(), ErrorKind::InvalidData);
    assert!(!loaded(&simulation).contains_key(&region_at(0, 2)));
}

#[test]
fn the_exports_of_an_unload_equal_a_save_of_those_regions() {
    let executor = executor();
    let policy = RegionPolicy { activation_radius: 0, streaming_radius: 1, release_radius: 1 };
    let simulation =
        game(WorldKind::Server).regions(anchored(false)).policy(policy).build().unwrap();
    let mut driver = Driver::default();
    let mut simulation = drive(&executor, &mut driver, simulation, join(1, region_at(1, 2)));
    for _ in 0..30 {
        simulation = drive(&executor, &mut driver, simulation, TickInput::default());
    }
    // A save at the sync point before the anchor moves two regions east.
    let (saved, save) = executor.block_on(simulation.save_point(&executor)).unwrap();
    simulation = saved;
    let west_column: BTreeSet<RegionId> = (1..=3).map(|y| region_at(0, y)).collect();
    move_anchor(&mut simulation, region_at(2, 2));
    let mut frame = TickInput::default();
    driver.fill(&mut frame);
    simulation = step(&executor, simulation, frame);
    let exports = simulation.lifecycle_mut().take_exports();
    let unloaded: BTreeSet<RegionId> = exports.iter().map(|export| export.region).collect();
    assert_eq!(unloaded, west_column);
    let saved: BTreeMap<RegionId, &RegionExport> =
        save.regions.iter().map(|export| (export.region, export)).collect();
    for export in &exports {
        assert_eq!(&export, saved.get(&export.region).unwrap(), "{:?}", export.region);
    }
}

#[test]
fn a_pinned_region_is_simulated_with_no_anchor_and_never_released() {
    let executor = executor();
    let simulation = game(WorldKind::Server).regions(anchored(true)).build().unwrap();
    let mut driver = Driver::default();
    let mut simulation = drive(&executor, &mut driver, simulation, TickInput::default());
    assert_eq!(std::mem::take(&mut driver.requests), [RegionRequest::Load { region: PINNED }]);
    for _ in 0..50 {
        simulation = drive(&executor, &mut driver, simulation, TickInput::default());
        assert_eq!(loaded(&simulation), BTreeMap::from([(PINNED, RegionMode::Simulated)]));
    }
    assert!(driver.requests.is_empty());
    // World-global state lives in the pinned region's resources, simulated every tick.
    let score = simulation.world().region_resource::<Score>(PINNED).unwrap().0;
    assert_eq!(score, 50 * PINNED.0);
}

#[test]
fn a_region_simulates_only_once_its_table_has_entered() {
    let executor = executor();
    let simulation = game(WorldKind::Server).regions(anchored(false)).build().unwrap();
    let centre = region_at(2, 2);
    let mut driver = Driver::default();
    driver.late_tables.insert(centre);
    let mut simulation = drive(&executor, &mut driver, simulation, join(1, centre));
    simulation = drive(&executor, &mut driver, simulation, TickInput::default());
    assert_eq!(loaded(&simulation)[&centre], RegionMode::Frozen, "loaded, no table yet");
    assert!(simulation.world().region_assets(centre).is_none());
    simulation = drive(&executor, &mut driver, simulation, TickInput::default());
    assert_eq!(loaded(&simulation)[&centre], RegionMode::Simulated);
    // A later table is a hot reload: it replaces the one in force, and the mode stays.
    let mut frame = TickInput::default();
    frame.assets.insert(centre, table(&[(1, 9)]));
    simulation = step(&executor, simulation, frame);
    assert_eq!(simulation.world().region_assets(centre).unwrap().table().len(), 1);
    assert_eq!(loaded(&simulation)[&centre], RegionMode::Simulated);
}

#[test]
fn the_radii_are_clamped_at_run_time() {
    let executor = executor();
    let simulation = game(WorldKind::Server).regions(anchored(false)).build().unwrap();
    let mut driver = Driver::default();
    let centre = region_at(2, 2);
    let mut simulation = drive(&executor, &mut driver, simulation, join(1, centre));
    simulation = drive(&executor, &mut driver, simulation, TickInput::default());
    // The ladder turns the activation radius past the streaming radius, and a bad edit
    // shrinks the release radius below it.
    let turned = RegionPolicy { activation_radius: 9, streaming_radius: 1, release_radius: 0 };
    simulation.world_mut().insert_resource(turned);
    for _ in 0..3 {
        simulation = drive(&executor, &mut driver, simulation, TickInput::default());
    }
    let clamped = RegionPolicy { activation_radius: 0, streaming_radius: 1, release_radius: 1 };
    check(&simulation, centre, &clamped);
    assert_eq!(loaded(&simulation).len(), ring(centre, 1).len());
}

#[test]
fn with_no_anchor_and_no_pinned_region_everything_unloads() {
    let executor = executor();
    let simulation = game(WorldKind::Server).regions(anchored(false)).build().unwrap();
    let mut driver = Driver::default();
    let centre = region_at(2, 2);
    let mut simulation = drive(&executor, &mut driver, simulation, join(1, centre));
    simulation = drive(&executor, &mut driver, simulation, TickInput::default());
    let count = loaded(&simulation).len();
    assert!(count > 1);
    let mut frame = TickInput::default();
    frame.sessions.insert(PlayerId(1), SessionEvent::Leave);
    driver.requests.clear();
    simulation = drive(&executor, &mut driver, simulation, frame);
    assert!(loaded(&simulation).is_empty());
    assert_eq!(driver.stored.len(), count);
    assert!(driver.requests.iter().all(|request| matches!(request, RegionRequest::Release { .. })));
    // The player's row was kept, unclaimed, in its region's export.
    let export = &driver.stored[&centre];
    let player = data(Player { id: PlayerId(1) });
    assert!(export.rows.rows.iter().any(|row| row.components.contains(&player)));
}

#[test]
fn a_streaming_radius_of_zero_is_clamped_to_one() {
    let executor = executor();
    let simulation = game(WorldKind::Server).regions(anchored(false)).build().unwrap();
    let mut driver = Driver::default();
    let centre = region_at(2, 2);
    let mut simulation = drive(&executor, &mut driver, simulation, join(1, centre));
    simulation = drive(&executor, &mut driver, simulation, TickInput::default());
    let zero = RegionPolicy { activation_radius: 0, streaming_radius: 0, release_radius: 0 };
    simulation.world_mut().insert_resource(zero);
    for _ in 0..3 {
        simulation = drive(&executor, &mut driver, simulation, TickInput::default());
    }
    let clamped = RegionPolicy { activation_radius: 0, streaming_radius: 1, release_radius: 1 };
    check(&simulation, centre, &clamped);
    assert_eq!(loaded(&simulation).len(), ring(centre, 1).len(), "the ring stays loaded");
}

#[test]
fn a_region_replaced_in_one_frame_is_not_released() {
    let executor = executor();
    let region = region_at(1, 1);
    let mut frame = TickInput::default();
    frame.loaded.insert(region, fresh(region));
    let mut simulation = step(&executor, game(WorldKind::Server).build().unwrap(), frame);
    // Unloaded and loaded again in one frame: exported, replaced, not released.
    let mut resync = TickInput::default();
    resync.unloaded.insert(region);
    resync.loaded.insert(region, fresh(region));
    simulation = step(&executor, simulation, resync);
    assert!(simulation.lifecycle_mut().take_requests().is_empty());
    assert_eq!(simulation.lifecycle_mut().take_exports().len(), 1);
    assert!(loaded(&simulation).contains_key(&region));
    // A refused reload leaves the unload a release.
    let mut refused = TickInput::default();
    refused.unloaded.insert(region);
    let mut bad = fresh(region);
    bad.transient_resources.push(<Weather as lactose_reflect::Reflect>::KEY);
    refused.loaded.insert(region, bad);
    simulation = step(&executor, simulation, refused);
    assert_eq!(simulation.lifecycle_mut().take_requests(), [RegionRequest::Release { region }]);
    assert!(!loaded(&simulation).contains_key(&region));
}

/// A square atlas of `side` × `side` regions.
fn square(side: i32) -> lactose_sim::RegionAtlas {
    let mut atlas = lactose_sim::RegionAtlas::new();
    for y in 0..side {
        for x in 0..side {
            let region = RegionId(1 + (y * side + x) as u64);
            atlas.insert(region, lactose_sim::RegionCell { x, y }).unwrap();
        }
    }
    atlas
}

#[test]
fn within_walks_the_cells_or_scans_and_agrees_with_the_definition() {
    let atlas = square(40);
    let brute = |centre: lactose_sim::RegionCell, radius: u32| -> Vec<RegionId> {
        (0..40 * 40_u64)
            .map(|n| RegionId(1 + n))
            .filter(|&region| atlas.cell(region).unwrap().distance(centre) <= radius)
            .collect()
    };
    let cells = [(0, 0), (39, 39), (20, 7), (-3, 5), (45, 45), (i32::MIN, i32::MAX)];
    for (x, y) in cells {
        for radius in [0, 1, 2, 5, 19, 20, 40, u32::MAX] {
            let centre = lactose_sim::RegionCell { x, y };
            let within: Vec<RegionId> = atlas.within(centre, radius).collect();
            assert_eq!(within, brute(centre, radius), "({x}, {y}) radius {radius}");
        }
    }
}

#[test]
fn the_policy_on_a_large_atlas_requests_the_union_of_the_rings() {
    // 65,536 regions and 64 held joins whose regions never load: each is an anchor every
    // step, and the policy's work follows the rings, not the atlas.
    let executor = executor();
    let side = 256;
    let atlas = square(side);
    let control =
        lactose_sim::RegionControl::Anchored { atlas: atlas.clone(), pinned: BTreeSet::new() };
    let simulation = lactose_sim::SimulationBuilder::new(WorldKind::Server, types())
        .regions(control)
        .build()
        .unwrap();
    let mut frame = TickInput::default();
    let mut expected = BTreeSet::new();
    for p in 0..64_i32 {
        let (x, y) = ((p * 7) % side, (p * 13) % side);
        let region = RegionId(1 + (y * side + x) as u64);
        frame.sessions.insert(PlayerId(p as u32 + 1), SessionEvent::Join { region });
        for dy in -2..=2 {
            for dx in -2..=2 {
                let (cx, cy) = (x + dx, y + dy);
                if (0..side).contains(&cx) && (0..side).contains(&cy) {
                    expected.insert(RegionId(1 + (cy * side + cx) as u64));
                }
            }
        }
    }
    let mut simulation = step(&executor, simulation, frame);
    let requests = simulation.lifecycle_mut().take_requests();
    assert_eq!(loads(&requests), expected);
    assert_eq!(requests.len(), expected.len(), "each region requested once");
    for _ in 0..5 {
        simulation = step(&executor, simulation, TickInput::default());
        assert!(simulation.lifecycle_mut().take_requests().is_empty());
    }
    assert!(expected.iter().all(|&region| simulation.lifecycle().is_requested(region)));
}
