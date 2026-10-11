//! The applier in a running simulation (README, "The applier"): what a kernel taking
//! `&PlayerInput` sees on a tick with a sample and on one without, what a malformed image
//! and a player without an entity change (nothing), which of two claimed rows is the
//! player's, the prediction world applying the same bytes to the same bits, the writer
//! declaration, and every result identical under every layout of SOUR's layout mode and
//! on one and four workers. The same programs record under SOUR in `tests/sour.rs`.

#![forbid(unsafe_code)]

mod common;

use std::collections::BTreeMap;

use common::*;
use lactose_core::{ErrorKind, PlayerId, Tick};
use lactose_ecs::{
    ClaimedBy, LayoutConfig, RegionConfig, RegionMode, ScheduleError, World, WorldKind,
};
use lactose_input::{PlayerInput, PlayerInputApplier, player_entity};
use lactose_math::Vec3;
use lactose_reflect::Reflect;
use lactose_sim::{InputApplier, PlayerSample, SimulationBuilder};

fn seen(world: &World, key: lactose_core::StableKey) -> &Seen {
    world.get::<Seen>(key).expect("the row records what it saw")
}

/// The server program, checked tick by tick against what each player's frame entry was.
#[test]
fn a_kernel_sees_the_sample_and_idle_without_one() {
    let mut idle_runs = 0;
    let mut bare_runs = 0;
    let mut previous_entity = None;
    run_server(Program::FULL, LayoutConfig::default(), 2, &mut |tick, simulation| {
        let world = simulation.world();
        // `IDLE_SPAWNED`: spawned idle at tick 1, so the kernel runs from the first tick; on
        // a tick without a sample it reads `IDLE`; gone between its leave and its rejoin.
        let entity = player_entity(world, IDLE_SPAWNED);
        if (LEAVE..REJOIN).contains(&tick) {
            assert_eq!(entity, None, "tick {tick}");
            idle_runs = 0;
        } else {
            let entity = entity.unwrap_or_else(|| panic!("tick {tick}: no entity"));
            if tick == REJOIN {
                assert_ne!(Some(entity), previous_entity, "a rejoin spawns a new entity");
            }
            previous_entity = Some(entity);
            idle_runs += 1;
            let record = seen(world, entity);
            let expected = if idle_spawned_sends(tick) {
                client_sample(IDLE_SPAWNED, tick)
            } else {
                PlayerInput::IDLE
            };
            assert!(record.reads(&expected), "tick {tick}: {record:?} for {expected:?}");
            assert_eq!(record.runs, idle_runs, "tick {tick}");
            if !idle_spawned_sends(tick) {
                assert_eq!(record.aim, -Vec3::Z);
                assert_eq!(record.walk, Vec3::ZERO);
            }
            assert_eq!(world.get::<PlayerInput>(entity), Some(&expected), "tick {tick}");
        }
        // `BARE_SPAWNED`: no component, so no kernel, until its first sample, which the
        // applier's `World::insert` adds; a malformed image afterwards reads as `IDLE`.
        let entity = player_entity(world, BARE_SPAWNED).expect("joined at tick 1");
        let record = seen(world, entity);
        if tick < BARE_FIRST {
            assert_eq!(world.get::<PlayerInput>(entity), None, "tick {tick}");
            assert_eq!(record.runs, 0);
        } else {
            bare_runs += 1;
            let expected = if bare_spawned_valid(tick) {
                client_sample(BARE_SPAWNED, tick)
            } else {
                PlayerInput::IDLE
            };
            assert!(record.reads(&expected), "tick {tick}: {record:?} for {expected:?}");
            assert_eq!(record.runs, bare_runs, "tick {tick}");
        }
        // `HELD` and `STRANGER` have no entity, and the held join is still held.
        assert_eq!(player_entity(world, HELD), None);
        assert_eq!(player_entity(world, STRANGER), None);
        assert_eq!(simulation.lifecycle().held_join(HELD), Some(ELSEWHERE));
        // `TWICE`: the lesser anchored key receives every sample; the others stay idle.
        assert_eq!(player_entity(world, TWICE), Some(key(LESSER)));
        assert!(seen(world, key(LESSER)).reads(&client_sample(TWICE, tick)), "tick {tick}");
        for other in [UNANCHORED, GREATER] {
            assert!(seen(world, key(other)).reads(&PlayerInput::IDLE), "tick {tick}");
            assert_eq!(seen(world, key(other)).runs, tick);
        }
    });
    // The script presses and releases `Throw`, so the edge counters are exercised.
    let executor = executor(1);
    let mut simulation = build_server(LayoutConfig::default());
    for tick in 1..=TICKS {
        simulation = executor
            .block_on(simulation.step(&executor, server_frame(Program::FULL, tick)))
            .unwrap();
    }
    let record = seen(simulation.world(), key(LESSER));
    assert!(record.throws > 0 && record.releases > 0, "{record:?}");
}

/// A malformed image leaves the world as if the entry were absent: the hashes of the two
/// runs agree at every tick, and every tick runs.
#[test]
fn a_malformed_image_changes_nothing() {
    let mut steps = 0;
    let sent = run_server(Program::FULL, LayoutConfig::default(), 2, &mut |_, _| steps += 1);
    let absent = Program { malformed: false, ..Program::FULL };
    let without = run_server(absent, LayoutConfig::default(), 2, &mut |_, _| {});
    assert_eq!(steps, TICKS);
    assert_eq!(sent, without);
    // The malformed entries were really there: one every `MALFORMED_EVERY` ticks, of every
    // kind.
    let kinds: Vec<u32> = (BARE_FIRST..=TICKS)
        .filter(|&tick| !bare_spawned_valid(tick))
        .map(|tick| tick / MALFORMED_EVERY % 7)
        .collect();
    assert_eq!(kinds.len() as u32, TICKS / MALFORMED_EVERY);
    assert!((0..7).all(|kind| kinds.contains(&kind)));
}

/// Samples for players without an entity change nothing: the hashes agree with the run
/// that never sent them.
#[test]
fn a_sample_for_a_player_without_an_entity_changes_nothing() {
    let with = run_server(Program::FULL, LayoutConfig::default(), 2, &mut |_, _| {});
    let without = run_server(
        Program { strays: false, ..Program::FULL },
        LayoutConfig::default(),
        2,
        &mut |_, _| {},
    );
    assert_eq!(with, without);
}

/// The applier called directly: `InvalidData` for every malformed image and nothing
/// written; `Ok(())` and nothing written for a player without an entity; the value
/// inserted otherwise.
#[test]
fn the_applier_called_directly() {
    let mut simulation = build_server(LayoutConfig::default());
    let world = simulation.world_mut();
    let mut applier = PlayerInputApplier::new();
    let before = world.get::<PlayerInput>(key(LESSER)).copied();
    for tick in (MALFORMED_EVERY..=TICKS).step_by(MALFORMED_EVERY as usize) {
        let sample =
            PlayerSample { input_tick: Tick(tick), substituted: false, sample: malformed(tick) };
        let error = applier.apply(world, TWICE, &sample).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData, "tick {tick}");
        assert_eq!(world.get::<PlayerInput>(key(LESSER)).copied(), before);
    }
    let sample = client_sample(TWICE, 5);
    let entry = sample.to_player_sample(Tick(5));
    let rows = world.entities_in(REGION).count();
    applier.apply(world, STRANGER, &entry).unwrap();
    assert_eq!(world.entities_in(REGION).count(), rows);
    assert_eq!(world.get::<PlayerInput>(key(LESSER)).copied(), before);
    // `substituted` and `input_tick` are not read: the value is all that is written.
    let substituted = PlayerSample { substituted: true, input_tick: Tick(999), ..entry };
    applier.apply(world, TWICE, &substituted).unwrap();
    assert_eq!(world.get::<PlayerInput>(key(LESSER)), Some(&sample));
    assert_eq!(world.get::<PlayerInput>(key(GREATER)), Some(&PlayerInput::IDLE));
    assert_eq!(world.get::<PlayerInput>(key(UNANCHORED)), Some(&PlayerInput::IDLE));
}

/// The lookup over the claim index, in a world where the claims were made in an order
/// unrelated to the keys and the answer moves as rows lose their anchor or claim.
#[test]
fn the_lesser_anchored_key_is_the_players_entity() {
    let mut simulation = build_server(LayoutConfig::default());
    let world = simulation.world_mut();
    assert_eq!(player_entity(world, TWICE), Some(key(LESSER)));
    // A lesser anchored key appearing later wins.
    let claim = ClaimedBy { player: TWICE };
    let anchor = lactose_sim::StreamingAnchor { priority: 0 };
    world.spawn(key(15), REGION, (claim, anchor)).unwrap();
    assert_eq!(player_entity(world, TWICE), Some(key(15)));
    world.despawn(key(15)).unwrap();
    world.remove::<lactose_sim::StreamingAnchor>(key(LESSER)).unwrap();
    assert_eq!(player_entity(world, TWICE), Some(key(GREATER)));
    world.remove::<ClaimedBy>(key(GREATER)).unwrap();
    assert_eq!(player_entity(world, TWICE), None);
    // An anchor without a claim is nobody's entity.
    world.spawn(key(5), REGION, (anchor,)).unwrap();
    assert_eq!(player_entity(world, TWICE), None);
    assert_eq!(player_entity(world, PlayerId(77)), None);
}

/// The prediction world, fed the local player's own sample bytes, records the same bits as
/// the server's entity for that player on every tick both simulate it.
#[test]
fn the_prediction_world_applies_the_same_bytes_to_the_same_bits() {
    let mut server: BTreeMap<u32, (Vec<u8>, Vec<u8>)> = BTreeMap::new();
    run_server(Program::FULL, LayoutConfig::default(), 4, &mut |tick, simulation| {
        let world = simulation.world();
        if tick < LEAVE {
            let entity = player_entity(world, IDLE_SPAWNED).unwrap();
            server.insert(tick, (seen(world, entity).image(), seen(world, key(LESSER)).image()));
        }
    });
    let mut compared = 0;
    run_prediction(LayoutConfig::default(), 4, &mut |tick, simulation| {
        let world = simulation.world();
        assert_eq!(player_entity(world, IDLE_SPAWNED), Some(key(LOCAL_ENTITY)));
        assert_eq!(player_entity(world, TWICE), Some(key(LESSER)));
        if let Some((local, twice)) = server.get(&tick) {
            assert_eq!(&seen(world, key(LOCAL_ENTITY)).image(), local, "tick {tick}");
            assert_eq!(&seen(world, key(LESSER)).image(), twice, "tick {tick}");
            compared += 1;
        }
    });
    assert_eq!(compared, LEAVE - 1);
}

/// Every hash of both programs is the same under every layout of SOUR's layout mode and on
/// one and four workers.
#[test]
fn the_hashes_are_the_same_under_every_layout_and_worker_count() {
    let reference = run_server(Program::FULL, LayoutConfig::default(), 1, &mut |_, _| {});
    let prediction = run_prediction(LayoutConfig::default(), 1, &mut |_, _| {});
    let mut runs = 0;
    for (name, layout) in layouts() {
        for workers in [1, 4] {
            let mut check = |tick: u32, simulation: &lactose_sim::Simulation| {
                assert_eq!(player_entity(simulation.world(), TWICE), Some(key(LESSER)), "{name}");
                let _ = tick;
            };
            let hashes = run_server(Program::FULL, layout, workers, &mut check);
            assert_eq!(hashes, reference, "server, {name}, {workers} workers");
            let hashes = run_prediction(layout, workers, &mut check);
            assert_eq!(hashes, prediction, "prediction, {name}, {workers} workers");
            runs += 1;
        }
    }
    assert_eq!(runs, 13 * 2);
}

/// The writer declaration is what lets a kernel in `Stage::Simulation` read the transient
/// column: without `register_systems` the build fails with `TransientBeforeWriter`, for
/// both world kinds that run kernels.
#[test]
fn a_reader_needs_the_writer_declaration() {
    for kind in [WorldKind::Server, WorldKind::Prediction] {
        builder(kind, LayoutConfig::default()).build().unwrap();
        let error = SimulationBuilder::new(kind, types())
            .systems(register_game)
            .inputs(Box::new(PlayerInputApplier::new()))
            .build()
            .err()
            .expect("a reader with no declared writer");
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        let source = std::error::Error::source(&error).expect("the schedule error");
        match source.downcast_ref::<ScheduleError>() {
            Some(ScheduleError::TransientBeforeWriter { reader, component }) => {
                assert_eq!((*reader, *component), ("test::observe", PlayerInput::KEY));
            }
            other => panic!("{kind:?}: {other:?}"),
        }
    }
    // A client world runs no kernel; the declaration is skipped there and the build holds.
    let mut client = SimulationBuilder::new(WorldKind::Client, types())
        .systems(lactose_input::register_systems)
        .inputs(Box::new(PlayerInputApplier::new()))
        .build()
        .unwrap();
    let region = RegionConfig { id: REGION, seed: 1, mode: RegionMode::Simulated };
    client.world_mut().create_region(region).unwrap();
}
