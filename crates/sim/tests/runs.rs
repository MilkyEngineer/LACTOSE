//! Runs and builds (README, "Runs" and "The input sync point"): `RunError` with and without
//! the simulation, the passes, the log retention, the server's presentation drain, the
//! build refusals, `Registration::order`, and SOUR's layout mode.

#![forbid(unsafe_code)]

mod common;

use std::collections::BTreeSet;

use common::*;
use lactose_core::{ErrorKind, StableKey, Tick};
use lactose_ecs::{
    ChunkOrder, LayoutConfig, PresentEvent, PresentEvents, PresentKind, RegionConfig, RegionId,
    RegionMode, Res, ScheduleError, StableKeyOf, SystemSet, World, WorldKind, system,
};
use lactose_math::Scalar;
use lactose_reflect::component;
use lactose_sim::harness::{ChunkCapacity, layout_mode};
use lactose_sim::{
    CurrentTick, Pass, RegionAtlas, RegionCell, RegionControl, RegionPolicy, Registration,
    SimulationBuilder, Stage, TickInput, TypeTable, sets,
};

fn net(id: u64) -> StableKey {
    StableKey::Net(lactose_core::NetId::from_bits(id).unwrap())
}

/// A game simulation holding one directed, simulated region.
fn directed(builder: SimulationBuilder) -> (lactose_ecs::Executor, lactose_sim::Simulation) {
    let executor = executor();
    let region = region_at(0, 0);
    let mut frame = TickInput::default();
    frame.loaded.insert(region, fresh(region));
    let simulation = step(&executor, builder.build().unwrap(), frame);
    (executor, simulation)
}

// ---- RunError ----

#[test]
fn a_removed_resource_fails_the_next_step_with_the_simulation_inside() {
    let (executor, mut simulation) = directed(game(WorldKind::Server));
    let hash = simulation.hash().unwrap();
    simulation.world_mut().remove_resource::<Gravity>().unwrap();
    let failure =
        executor.block_on(simulation.step(&executor, TickInput::default())).err().unwrap();
    assert_eq!(failure.error.kind(), ErrorKind::InvalidData);
    let mut simulation = failure.simulation.expect("nothing ran, so the simulation comes back");
    assert_eq!(simulation.tick(), Tick(1));
    assert_eq!(simulation.hash().unwrap(), hash, "nothing ran");
    // `sim`'s own resources are named too.
    let tick = simulation.world_mut().remove_resource::<CurrentTick>().unwrap();
    simulation.world_mut().insert_resource(Gravity(Scalar::ONE));
    let failure =
        executor.block_on(simulation.step(&executor, TickInput::default())).err().unwrap();
    let mut simulation = failure.simulation.unwrap();
    simulation.world_mut().insert_resource(tick);
    let simulation = step(&executor, simulation, TickInput::default());
    assert_eq!(simulation.tick(), Tick(2));
    // `?` into a plain result drops the simulation as a plain error would.
    let error: lactose_core::Error = {
        let mut simulation = simulation;
        simulation.world_mut().remove_resource::<Gravity>();
        executor.block_on(simulation.step(&executor, TickInput::default())).err().unwrap().into()
    };
    assert_eq!(error.kind(), ErrorKind::InvalidData);
}

#[test]
fn save_point_off_the_server_hands_the_simulation_back() {
    let executor = executor();
    for kind in [WorldKind::Prediction, WorldKind::Client] {
        let simulation = game(kind).build().unwrap();
        let failure = executor.block_on(simulation.save_point(&executor)).err().unwrap();
        assert_eq!(failure.error.kind(), ErrorKind::Unsupported);
        let simulation = failure.simulation.expect("the kind check runs before anything");
        assert_eq!(simulation.kind(), kind);
    }
}

#[test]
fn after_cancelled_the_simulation_is_gone() {
    let executor = executor();
    let simulation = game(WorldKind::Server).build().unwrap();
    executor.clone().shutdown();
    let failure =
        executor.block_on(simulation.step(&executor, TickInput::default())).err().unwrap();
    assert_eq!(failure.error.kind(), ErrorKind::Cancelled);
    assert!(failure.simulation.is_none());
}

// ---- Passes ----

/// What a pass system saw: its name and the tick.
#[derive(Clone, Default)]
struct Passes(std::sync::Arc<std::sync::Mutex<Vec<(&'static str, Tick)>>>);

static EXTRACT_SET: SystemSet = SystemSet::new("test::extract");
static GATHER_A_SET: SystemSet = SystemSet::new("test::gather_a");
static GATHER_B_SET: SystemSet = SystemSet::new("test::gather_b");

#[system(name = "test::extract", worlds = [server, prediction], exclusive, in_set = EXTRACT_SET)]
fn extract(world: &mut World, seen: Res<Passes>) {
    seen.0.lock().unwrap().push(("extract", world.tick()));
}

#[system(name = "test::gather_a", worlds = [server], exclusive, in_set = GATHER_A_SET)]
fn gather_a(world: &mut World, seen: Res<Passes>) {
    seen.0.lock().unwrap().push(("gather_a", world.tick()));
}

#[system(name = "test::gather_b", worlds = [server], exclusive, in_set = GATHER_B_SET)]
fn gather_b(world: &mut World, seen: Res<Passes>, _g: Res<Gravity>) {
    seen.0.lock().unwrap().push(("gather_b", world.tick()));
}

fn passes(registration: &mut Registration) {
    registration.pass(Pass::Extract).sets(&[&EXTRACT_SET]).add(&EXTRACT);
    registration.pass(Pass::Gather).sets(&[&GATHER_A_SET]).add(&GATHER_A);
    registration.pass(Pass::Gather).sets(&[&GATHER_B_SET]).add(&GATHER_B);
}

fn passes_builder(
    kind: WorldKind,
    seen: &Passes,
    order: Option<(&'static SystemSet, &'static SystemSet)>,
) -> SimulationBuilder {
    let service = seen.clone();
    let mut builder = game(kind)
        .setup(move |world| {
            world.insert_service(Box::new(service));
            Ok(())
        })
        .systems(passes);
    if let Some((before, after)) = order {
        builder = builder.systems(move |registration| {
            registration.order(before, after);
        });
    }
    builder
}

#[test]
fn passes_run_extract_then_gather_between_ticks_in_the_order_app_gives() {
    let seen = Passes::default();
    // Two crates' exclusive systems in one pass do not name each other: unordered, the
    // build fails.
    let error = passes_builder(WorldKind::Server, &seen, None).build().err().unwrap();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    for (order, expected) in [
        ((&GATHER_A_SET, &GATHER_B_SET), ["extract", "gather_a", "gather_b"]),
        ((&GATHER_B_SET, &GATHER_A_SET), ["extract", "gather_b", "gather_a"]),
    ] {
        let seen = Passes::default();
        let (executor, simulation) =
            directed(passes_builder(WorldKind::Server, &seen, Some(order)));
        let simulation = executor.block_on(simulation.run_passes(&executor)).unwrap();
        assert_eq!(simulation.tick(), Tick(1), "a pass does not advance the tick");
        let calls = core::mem::take(&mut *seen.0.lock().unwrap());
        let names: Vec<&str> = calls.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, expected);
        assert!(calls.iter().all(|&(_, tick)| tick == Tick(1)));
    }
    // A pass none of whose systems runs in this kind is skipped, and an edge between its
    // sets orders nothing there.
    let seen = Passes::default();
    let (executor, simulation) = directed(passes_builder(
        WorldKind::Prediction,
        &seen,
        Some((&GATHER_A_SET, &GATHER_B_SET)),
    ));
    let _simulation = executor.block_on(simulation.run_passes(&executor)).unwrap();
    let names: Vec<&str> = seen.0.lock().unwrap().iter().map(|(name, _)| *name).collect();
    assert_eq!(names, ["extract"]);
}

#[test]
fn both_passes_are_prepared_before_either_runs() {
    let seen = Passes::default();
    let (executor, mut simulation) =
        directed(passes_builder(WorldKind::Server, &seen, Some((&GATHER_A_SET, &GATHER_B_SET))));
    simulation.world_mut().remove_resource::<Gravity>();
    let failure = executor.block_on(simulation.run_passes(&executor)).err().unwrap();
    assert_eq!(failure.error.kind(), ErrorKind::InvalidData);
    assert!(failure.simulation.is_some());
    assert!(seen.0.lock().unwrap().is_empty(), "extract did not run");
}

// ---- Registration::order at the sync point ----

static CRATE_A: SystemSet = SystemSet::new("test::crate_a").after(&[&sets::INPUT]);
static CRATE_B: SystemSet = SystemSet::new("test::crate_b").after(&[&sets::INPUT]);
static LATE: SystemSet = SystemSet::new("test::late");

#[system(name = "test::sync_a", worlds = [server], exclusive, in_set = CRATE_A)]
fn sync_a(_world: &mut World, seen: Res<Passes>) {
    seen.0.lock().unwrap().push(("a", Tick(0)));
}

#[system(name = "test::sync_b", worlds = [server], exclusive, in_set = CRATE_B)]
fn sync_b(_world: &mut World, seen: Res<Passes>) {
    seen.0.lock().unwrap().push(("b", Tick(0)));
}

fn crate_a(registration: &mut Registration) {
    registration.stage(Stage::Input).sets(&[&CRATE_A]).add(&SYNC_A);
}

fn crate_b(registration: &mut Registration) {
    registration.stage(Stage::Input).sets(&[&CRATE_B]).add(&SYNC_B);
    registration.stage(Stage::Post).sets(&[&LATE]);
}

fn two_crates(seen: &Passes) -> SimulationBuilder {
    let service = seen.clone();
    game(WorldKind::Server)
        .setup(move |world| {
            world.insert_service(Box::new(service));
            Ok(())
        })
        .systems(crate_a)
        .systems(crate_b)
}

fn schedule_error(error: &lactose_core::Error) -> ScheduleError {
    let source = std::error::Error::source(error).expect("a schedule error is the source");
    source.downcast_ref::<ScheduleError>().unwrap().clone()
}

#[test]
fn app_orders_two_crates_sets_after_sets_input() {
    let error = two_crates(&Passes::default()).build().err().unwrap();
    assert!(matches!(schedule_error(&error), ScheduleError::UnorderedConflict { .. }), "{error}");
    for (before, after, expected) in
        [(&CRATE_A, &CRATE_B, ["a", "b"]), (&CRATE_B, &CRATE_A, ["b", "a"])]
    {
        let seen = Passes::default();
        let builder = two_crates(&seen).systems(move |registration| {
            registration.order(before, after);
        });
        let (executor, simulation) = directed(builder);
        seen.0.lock().unwrap().clear();
        let _simulation = step(&executor, simulation, TickInput::default());
        let names: Vec<&str> = seen.0.lock().unwrap().iter().map(|(name, _)| *name).collect();
        assert_eq!(names, expected);
    }
    // Edges the schedules refuse, at build.
    let refuse = |before: &'static SystemSet, after: &'static SystemSet| {
        let builder = two_crates(&Passes::default()).systems(move |registration| {
            registration.order(&CRATE_A, &CRATE_B).order(before, after);
        });
        schedule_error(&builder.build().err().unwrap())
    };
    static NOWHERE: SystemSet = SystemSet::new("test::nowhere");
    assert!(matches!(refuse(&CRATE_A, &NOWHERE), ScheduleError::UnknownOrderedSet { .. }));
    assert!(matches!(refuse(&NOWHERE, &CRATE_A), ScheduleError::UnknownOrderedSet { .. }));
    // Against the stage order: `Post` before `Input`.
    assert!(matches!(refuse(&LATE, &CRATE_A), ScheduleError::Cycle { .. }));
    // Two sets in different schedules.
    let builder = two_crates(&Passes::default()).systems(passes).systems(|registration| {
        registration
            .order(&CRATE_A, &CRATE_B)
            .order(&GATHER_A_SET, &GATHER_B_SET)
            .order(&CRATE_A, &EXTRACT_SET);
    });
    let service = Passes::default();
    let builder = builder.setup(move |world| {
        world.insert_service(Box::new(service));
        Ok(())
    });
    assert!(matches!(
        schedule_error(&builder.build().err().unwrap()),
        ScheduleError::UnknownOrderedSet { .. }
    ));
}

// ---- Build refusals ----

#[test]
fn build_refuses_what_the_contract_rules_out() {
    let kind = |builder: SimulationBuilder| builder.build().err().map(|error| error.kind());
    let policy =
        |a, s, r| RegionPolicy { activation_radius: a, streaming_radius: s, release_radius: r };
    assert_eq!(kind(game(WorldKind::Server).policy(policy(1, 1, 3))), Some(ErrorKind::InvalidData));
    assert_eq!(kind(game(WorldKind::Server).policy(policy(1, 3, 2))), Some(ErrorKind::InvalidData));
    assert_eq!(kind(game(WorldKind::Server).policy(policy(0, 0, 0))), Some(ErrorKind::InvalidData));
    assert_eq!(kind(game(WorldKind::Server).policy(policy(0, 1, 1))), None);
    assert_eq!(
        kind(game(WorldKind::Client).regions(anchored(false))),
        Some(ErrorKind::InvalidData)
    );
    let empty = RegionControl::Anchored { atlas: RegionAtlas::new(), pinned: BTreeSet::new() };
    assert_eq!(kind(game(WorldKind::Server).regions(empty)), Some(ErrorKind::InvalidData));
    let stray = RegionControl::Anchored { atlas: atlas(), pinned: BTreeSet::from([RegionId(5)]) };
    assert_eq!(kind(game(WorldKind::Server).regions(stray)), Some(ErrorKind::InvalidData));
    // A world resource that is not derived.
    let not_derived = game(WorldKind::Server).setup(|world| {
        world.insert_resource(NotDerived(1));
        Ok(())
    });
    assert_eq!(kind(not_derived), Some(ErrorKind::InvalidData));
    // A setup function's own error.
    let failing = game(WorldKind::Server)
        .setup(|_| Err(lactose_core::Error::new(ErrorKind::NotFound, "no pool")));
    assert_eq!(kind(failing), Some(ErrorKind::NotFound));
    // A resource a system names that no setup function inserts.
    let missing = SimulationBuilder::new(WorldKind::Server, types()).systems(register_systems);
    let error = missing.build().err().unwrap();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert!(matches!(schedule_error(&error), ScheduleError::MissingResource { .. }));
    // An atlas refuses a region or a cell placed twice.
    let mut atlas = RegionAtlas::new();
    atlas.insert(RegionId(1), RegionCell { x: 0, y: 0 }).unwrap();
    assert_eq!(
        atlas.insert(RegionId(1), RegionCell { x: 1, y: 0 }).err().unwrap().kind(),
        ErrorKind::InvalidData
    );
    assert_eq!(
        atlas.insert(RegionId(2), RegionCell { x: 0, y: 0 }).err().unwrap().kind(),
        ErrorKind::InvalidData
    );
    assert_eq!(atlas.len(), 1);
}

#[test]
fn within_yields_the_regions_in_range_ascending() {
    let atlas = atlas();
    let centre = RegionCell { x: 0, y: 0 };
    let near: Vec<RegionId> = atlas.within(centre, 1).collect();
    assert_eq!(near, [region_at(0, 0), region_at(1, 0), region_at(0, 1), region_at(1, 1)]);
    assert_eq!(atlas.within(RegionCell { x: -5, y: -5 }, 1).count(), 0);
    assert_eq!(atlas.within(centre, 100).count(), atlas.len());
    assert_eq!(RegionCell { x: -2, y: 3 }.distance(RegionCell { x: 1, y: -1 }), 4);
    assert_eq!(
        RegionCell { x: i32::MIN, y: 0 }.distance(RegionCell { x: i32::MAX, y: 0 }),
        u32::MAX
    );
    assert_eq!(atlas.region_at(PINNED_CELL), Some(PINNED));
    assert_eq!(atlas.cell(PINNED), Some(PINNED_CELL));
}

// ---- Log retention ----

#[test]
fn the_log_is_trimmed_to_the_retention_and_a_lagging_consumer_sees_none() {
    const RETENTION: u32 = 3;
    let executor = executor();
    let mut simulation = SimulationBuilder::new(WorldKind::Prediction, types())
        .log_retention(RETENTION)
        .build()
        .unwrap();
    let region = RegionId(1);
    simulation
        .world_mut()
        .create_region(RegionConfig { id: region, seed: 1, mode: RegionMode::Simulated })
        .unwrap();
    let retained = |simulation: &lactose_sim::Simulation| {
        let log = simulation.world().structural_log();
        log.since(&mut log.cursor_at_start()).unwrap().count()
    };
    let mut lagging = None;
    for n in 1..=20_u64 {
        // One structural change per tick.
        simulation.world_mut().spawn(net(n), region, (Health(1),)).unwrap();
        simulation = step(&executor, simulation, TickInput::default());
        // The ring is full once it holds the end of `RETENTION + 1` steps.
        if n > u64::from(RETENTION) {
            assert_eq!(retained(&simulation), RETENTION as usize, "after step {n}");
        }
        if n == 10 {
            lagging = Some(simulation.world().structural_log().cursor());
        }
        let mut cursor = lagging;
        if let Some(cursor) = cursor.as_mut() {
            let behind = n - 10;
            let valid = simulation.world().structural_log().since(cursor).is_some();
            assert_eq!(valid, behind <= u64::from(RETENTION), "{behind} steps behind");
        }
    }
    // A restore clears the log and starts an epoch: the ring's stale cursors are ignored
    // until fresh ones replace them, and the trim resumes.
    let snapshot = simulation.world().snapshot();
    simulation.world_mut().restore(&snapshot).unwrap();
    for n in 21..=30_u64 {
        simulation.world_mut().spawn(net(n), region, (Health(1),)).unwrap();
        simulation = step(&executor, simulation, TickInput::default());
    }
    assert_eq!(retained(&simulation), RETENTION as usize);
    assert_eq!(
        SimulationBuilder::DEFAULT_LOG_RETENTION,
        120,
        "two seconds at sixty ticks per second"
    );
}

// ---- The presentation drain ----

#[component(key = "test::footstep", derived)]
struct Footstep(#[field(id = 1)] u32);

impl PresentEvent for Footstep {
    const KIND: PresentKind = PresentKind::from_name("test::footstep");
}

#[system(name = "test::footsteps", worlds = [server, prediction])]
fn footsteps(present: PresentEvents, key: StableKeyOf, _health: &Health) {
    present.emit(*key, Footstep(1));
}

#[test]
fn the_server_drains_presentation_events_and_other_kinds_keep_them() {
    let executor = executor();
    for kind in [WorldKind::Server, WorldKind::Prediction] {
        let mut simulation = SimulationBuilder::new(kind, types())
            .systems(|registration| {
                registration.stage(Stage::Post).add(&FOOTSTEPS);
            })
            .build()
            .unwrap();
        let region = RegionId(1);
        let world = simulation.world_mut();
        world
            .create_region(RegionConfig { id: region, seed: 1, mode: RegionMode::Simulated })
            .unwrap();
        world
            .spawn(
                net(1),
                region,
                (Health(1), lactose_ecs::ClaimedBy { player: lactose_core::PlayerId(1) }),
            )
            .unwrap();
        for _ in 0..5 {
            simulation = step(&executor, simulation, TickInput::default());
        }
        let batch = simulation.world_mut().take_present_events();
        let expected = if kind == WorldKind::Server { 0 } else { 5 };
        assert_eq!(batch.emitted.len(), expected, "{kind:?}");
    }
}

// ---- Layout mode ----

#[test]
fn layout_mode_covers_both_chunk_orders_and_refuses_another_slab() {
    let mode = |capacity| layout_mode(capacity, 0x5eed).unwrap();
    assert_eq!(mode(ChunkCapacity::Production), LayoutConfig::default());
    assert_eq!(layout_mode(ChunkCapacity::Production, 99).unwrap(), LayoutConfig::default());
    let perturbed = |chunk_rows, chunk_order| LayoutConfig {
        chunk_rows,
        seed: 0x5eed,
        permute_rows: true,
        chunk_order,
        split_batches: true,
    };
    assert_eq!(mode(ChunkCapacity::Rows(7)), perturbed(Some(7), ChunkOrder::Reversed));
    assert_eq!(mode(ChunkCapacity::Rows(1)), perturbed(Some(1), ChunkOrder::Shuffled));
    assert_eq!(mode(ChunkCapacity::Rows(64)), perturbed(Some(64), ChunkOrder::Shuffled));
    assert_eq!(mode(ChunkCapacity::Bytes(16384)), perturbed(None, ChunkOrder::Reversed));
    assert_eq!(
        layout_mode(ChunkCapacity::Bytes(4096), 1).err().unwrap().kind(),
        ErrorKind::Unsupported
    );
    let simulation = SimulationBuilder::new(WorldKind::Server, TypeTable::new())
        .layout(mode(ChunkCapacity::Rows(7)))
        .build()
        .unwrap();
    assert_eq!(*simulation.world().layout(), perturbed(Some(7), ChunkOrder::Reversed));
}
