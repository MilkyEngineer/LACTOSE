//! The input sync point (README, "The input sync point" and "Time"): the order of `sim`'s
//! six systems, the order a frame's fields and entries are applied in, held joins, time.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_asset_api::{AssetFailure, AssetId};
use lactose_core::{ErrorKind, PlayerId, Tick};
use lactose_ecs::{RegionMode, WorldKind};
use lactose_math::Scalar;
use lactose_sim::{
    AssetFailureEvent, CurrentTick, RegionRequest, SessionEvent, SimulationBuilder, TICK_RATE_HZ,
    TickDelta, TickInput, TimeScale, TypeTable,
};

const CENTRE: lactose_ecs::RegionId = region_at(2, 2);

/// The frame of the order test: two joins into the centre, its bundle, and both players'
/// inputs, each map filled in `order`.
fn first_frame(reversed: bool) -> TickInput {
    let mut frame = TickInput::default();
    let players: Vec<u32> = if reversed { vec![2, 1] } else { vec![1, 2] };
    for &player in &players {
        frame.sessions.insert(PlayerId(player), SessionEvent::Join { region: CENTRE });
    }
    frame.loaded.insert(CENTRE, fresh(CENTRE));
    for &player in &players {
        frame.inputs.insert(PlayerId(player), sample(1, player as i32, 0));
    }
    frame
}

fn traced(log: &Log) -> SimulationBuilder {
    let log_service = log.clone();
    game(WorldKind::Server)
        .regions(anchored(false))
        .inputs(Box::new(Inputs { log: Some(log.clone()) }))
        .sessions(Box::new(Sessions { log: Some(log.clone()) }))
        .staged_state(Box::new(Staged { log: log.clone() }))
        .setup(move |world| {
            world.insert_service(Box::new(log_service));
            Ok(())
        })
        .systems(register_observer)
}

#[test]
fn regions_then_sessions_then_inputs_then_staged_state_then_the_policy() {
    let executor = executor();
    let mut hashes = Vec::new();
    let mut logs = Vec::new();
    for reversed in [false, true] {
        let log = Log::default();
        let simulation = traced(&log).build().unwrap();
        let simulation = step(&executor, simulation, first_frame(reversed));
        let t1 = Tick(1);
        let expected = vec![
            Call::Session {
                tick: t1,
                player: PlayerId(1),
                event: SessionEvent::Join { region: CENTRE },
                loaded: vec![CENTRE],
            },
            Call::Session {
                tick: t1,
                player: PlayerId(2),
                event: SessionEvent::Join { region: CENTRE },
                loaded: vec![CENTRE],
            },
            Call::Input { tick: t1, player: PlayerId(1), input_tick: Some(t1) },
            Call::Input { tick: t1, player: PlayerId(2), input_tick: Some(t1) },
            // The bundle loaded `Frozen`; the policy has not run yet.
            Call::Staged { tick: t1, modes: vec![(CENTRE, RegionMode::Frozen)] },
            // A set after `sets::INPUT` sees the policy's decision.
            Call::After { tick: t1, modes: vec![(CENTRE, RegionMode::Simulated)] },
        ];
        let calls = log.take();
        assert_eq!(calls, expected, "reversed: {reversed}");
        logs.push(calls);
        hashes.push(simulation.hash().unwrap());
    }
    assert_eq!(logs[0], logs[1]);
    assert_eq!(hashes[0], hashes[1], "a frame assembled in any order is one input");
}

#[test]
fn a_frame_is_applied_field_by_field_in_declaration_order() {
    let executor = executor();
    let log = Log::default();
    let simulation = game(WorldKind::Server)
        .inputs(Box::new(Inputs { log: Some(log.clone()) }))
        .sessions(Box::new(Sessions { log: Some(log.clone()) }))
        .build()
        .unwrap();
    let region = region_at(0, 0);
    let other = region_at(1, 0);
    let mut first = TickInput::default();
    first.loaded.insert(region, fresh(region));
    let mut simulation = step(&executor, simulation, first);
    let before = simulation.world().region(region).unwrap().seed;

    let failure =
        AssetFailure { id: AssetId::from_bits(7), version: None, kind: ErrorKind::NotFound };
    let mut frame = TickInput { time_scale: Some(TimeScale::PAUSED), ..TickInput::default() };
    frame.failures = vec![failure, failure];
    // Unloads before loads: the region is replaced in one frame.
    frame.unloaded.insert(region);
    let mut replacement = fresh(region);
    replacement.seed = before + 1;
    replacement.assets = None;
    frame.loaded.insert(region, replacement);
    frame.loaded.insert(other, fresh(other));
    // Tables and modes after loads: they reach this frame's regions.
    frame.assets.insert(region, table(&[(5, 9)]));
    frame.modes.insert(other, RegionMode::Frozen);
    // Sessions after regions, inputs after sessions.
    frame.sessions.insert(PlayerId(4), SessionEvent::Join { region: other });
    frame.inputs.insert(PlayerId(4), sample(2, 1, 1));
    simulation = step(&executor, simulation, frame);

    let world = simulation.world();
    assert_eq!(*world.resource::<TimeScale>().unwrap(), TimeScale::PAUSED);
    assert_eq!(world.region(region).unwrap().seed, before + 1);
    assert_eq!(world.region_assets(region).unwrap().table().len(), 1);
    assert_eq!(world.region(other).unwrap().mode, RegionMode::Frozen);
    assert!(simulation.lifecycle().held_join(PlayerId(4)).is_none());
    let calls = log.take();
    assert!(matches!(calls[0], Call::Session { player: PlayerId(4), .. }));
    assert_eq!(
        calls[1],
        Call::Input { tick: Tick(2), player: PlayerId(4), input_tick: Some(Tick(2)) }
    );
    let mut simulation = simulation;
    let stamped = simulation.lifecycle_mut().take_asset_failures();
    let event = AssetFailureEvent { tick: Tick(2), failure };
    assert_eq!(stamped, vec![event, event]);
    assert!(simulation.lifecycle_mut().take_load_failures().is_empty());
}

#[test]
fn a_held_join_is_applied_at_the_sync_point_its_region_loads() {
    let executor = executor();
    let log = Log::default();
    let simulation = game(WorldKind::Server)
        .sessions(Box::new(Sessions { log: Some(log.clone()) }))
        .build()
        .unwrap();
    let region = region_at(3, 1);
    let simulation = step(&executor, simulation, join(1, region));
    assert_eq!(simulation.lifecycle().held_join(PlayerId(1)), Some(region));
    assert!(log.take().is_empty(), "the region is not loaded");

    let simulation = step(&executor, simulation, TickInput::default());
    assert!(log.take().is_empty());

    let mut frame = TickInput::default();
    frame.loaded.insert(region, fresh(region));
    let mut simulation = step(&executor, simulation, frame);
    let calls = log.take();
    assert_eq!(
        calls,
        vec![Call::Session {
            tick: Tick(3),
            player: PlayerId(1),
            event: SessionEvent::Join { region },
            loaded: vec![region],
        }]
    );
    assert_eq!(simulation.lifecycle().held_join(PlayerId(1)), None);
    assert!(player_entity(simulation.world_mut(), PlayerId(1)).is_some());
}

#[test]
fn a_join_requests_its_region_and_its_anchor_then_drives_the_policy() {
    let executor = executor();
    let log = Log::default();
    let simulation = game(WorldKind::Server)
        .regions(anchored(false))
        .sessions(Box::new(Sessions { log: Some(log.clone()) }))
        .build()
        .unwrap();
    let mut driver = Driver::default();
    let mut simulation = step(&executor, simulation, join(1, CENTRE));
    driver.observe(&mut simulation);
    // The held join is the anchor: the policy requests its streaming ring, ascending.
    let requested: Vec<_> = std::mem::take(&mut driver.requests);
    let mut expected: Vec<_> =
        atlas().within(cell_of(CENTRE), 2).map(|region| RegionRequest::Load { region }).collect();
    expected.sort();
    assert_eq!(requested, expected);
    assert!(log.take().is_empty());

    let mut frame = TickInput::default();
    driver.fill(&mut frame);
    let mut simulation = step(&executor, simulation, frame);
    driver.observe(&mut simulation);
    let calls = log.take();
    assert_eq!(calls.len(), 1, "the join applies at the sync point its region loads");
    assert!(matches!(&calls[0], Call::Session { tick: Tick(2), .. }));
    // The spawned anchor now drives the policy: the activation ring is simulated, the
    // rest of the streaming ring frozen.
    let modes = loaded(&simulation);
    for (region, mode) in modes {
        let near = cell_of(region).distance(cell_of(CENTRE)) <= 1;
        let expected = if near { RegionMode::Simulated } else { RegionMode::Frozen };
        assert_eq!(mode, expected, "{region:?}");
    }
    assert_eq!(simulation.lifecycle().held_join(PlayerId(1)), None);
}

#[test]
fn a_leave_cancels_a_held_join() {
    let executor = executor();
    let log = Log::default();
    let simulation = game(WorldKind::Server)
        .regions(anchored(false))
        .sessions(Box::new(Sessions { log: Some(log.clone()) }))
        .build()
        .unwrap();
    let mut driver = Driver::default();
    let mut simulation = step(&executor, simulation, join(1, CENTRE));
    driver.observe(&mut simulation);
    assert!(simulation.lifecycle().is_requested(CENTRE));

    let mut frame = TickInput::default();
    frame.sessions.insert(PlayerId(1), SessionEvent::Leave);
    let mut simulation = step(&executor, simulation, frame);
    assert_eq!(simulation.lifecycle().held_join(PlayerId(1)), None);
    assert!(log.take().is_empty(), "nothing reached the applier");
    let requests = simulation.lifecycle_mut().take_requests();
    assert!(requests.contains(&RegionRequest::Cancel { region: CENTRE }));
    assert!(!simulation.lifecycle().is_requested(CENTRE));

    // A bundle that still arrives is refused, and nothing loads.
    let mut late = TickInput::default();
    late.loaded.insert(CENTRE, fresh(CENTRE));
    let mut simulation = step(&executor, simulation, late);
    let failures = simulation.lifecycle_mut().take_load_failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].error.kind(), ErrorKind::InvalidData);
    assert!(loaded(&simulation).is_empty());
}

#[test]
fn a_join_outside_the_atlas_is_a_session_failure() {
    let executor = executor();
    let log = Log::default();
    let simulation = game(WorldKind::Server)
        .regions(anchored(false))
        .sessions(Box::new(Sessions { log: Some(log.clone()) }))
        .build()
        .unwrap();
    let mut simulation = step(&executor, simulation, join(1, lactose_ecs::RegionId(5)));
    let failures = simulation.lifecycle_mut().take_session_failures();
    assert_eq!(failures.len(), 1);
    assert_eq!((failures[0].tick, failures[0].player), (Tick(1), PlayerId(1)));
    assert_eq!(failures[0].error.kind(), ErrorKind::NotFound);
    assert_eq!(simulation.lifecycle().held_join(PlayerId(1)), None);
    assert!(simulation.lifecycle_mut().take_requests().is_empty());
    assert!(log.take().is_empty());
}

#[test]
fn without_an_applier_inputs_are_dropped_after_their_input_tick_is_set() {
    let executor = executor();
    let simulation = SimulationBuilder::new(WorldKind::Server, TypeTable::new()).build().unwrap();
    let mut frame = TickInput::default();
    frame.inputs.insert(PlayerId(3), sample(7, 0, 0));
    frame.inputs.insert(PlayerId(9), sample(8, 0, 0));
    let simulation = step(&executor, simulation, frame);
    assert_eq!(simulation.world().input_tick(PlayerId(3)), Some(Tick(7)));
    assert_eq!(simulation.world().input_tick(PlayerId(9)), Some(Tick(8)));
}

#[test]
fn a_client_world_drops_sessions_and_runs_nothing_of_the_game() {
    let executor = executor();
    let log = Log::default();
    let simulation = game(WorldKind::Client)
        .sessions(Box::new(Sessions { log: Some(log.clone()) }))
        .build()
        .unwrap();
    let region = region_at(1, 1);
    let mut frame = join(1, region);
    frame.loaded.insert(region, fresh(region));
    let simulation = step(&executor, simulation, frame);
    assert!(log.take().is_empty());
    assert_eq!(simulation.lifecycle().held_join(PlayerId(1)), None);
    let hash = simulation.world_hash(region).unwrap();
    let mut simulation = simulation;
    for _ in 0..20 {
        simulation = step(&executor, simulation, TickInput::default());
    }
    // A simulated region with a spawner, and no system of the game ran.
    assert_eq!(simulation.world().region(region).unwrap().entity_count, 2);
    assert_eq!(simulation.world_hash(region).unwrap(), hash);
}

#[test]
fn time_enters_through_the_frame_and_the_delta_never_changes() {
    let executor = executor();
    let simulation = SimulationBuilder::new(WorldKind::Prediction, TypeTable::new())
        .tick(Tick(40))
        .build()
        .unwrap();
    let delta = Scalar::ONE / Scalar::from_u32(60);
    assert_eq!(TICK_RATE_HZ, 60);
    assert_eq!(simulation.world().resource::<TickDelta>().unwrap().0, delta);
    assert_eq!(TickDelta::of_rate(60).0, delta);
    assert_eq!(*simulation.world().resource::<TimeScale>().unwrap(), TimeScale::REAL_TIME);
    assert_eq!(simulation.world().resource::<CurrentTick>().unwrap().get(), Tick(40));

    let frame =
        TickInput { time_scale: Some(TimeScale { thousandths: 2000 }), ..TickInput::default() };
    let simulation = step(&executor, simulation, frame);
    let simulation = step(&executor, simulation, TickInput::default());
    let world = simulation.world();
    assert_eq!(simulation.tick(), Tick(42));
    assert_eq!(world.resource::<CurrentTick>().unwrap().get(), Tick(42));
    assert_eq!(world.resource::<TimeScale>().unwrap().thousandths, 2000, "kept until changed");
    assert_eq!(world.resource::<TickDelta>().unwrap().0, delta);
}

#[test]
#[should_panic(expected = "at least one tick")]
fn a_zero_rate_has_no_delta() {
    let _ = TickDelta::of_rate(0);
}
