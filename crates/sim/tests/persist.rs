//! Saves, loads and journal replay (README, "Saves and loads" and "Journal replay";
//! architecture section 15.5): a fresh simulation from a save point, an in-place reload and
//! an un-reloaded twin hash equal for K ticks; an unload and reload keeps every persistent
//! column; a journal recorded from one run and replayed by a small driver reproduces the
//! hashes.

#![forbid(unsafe_code)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::*;
use lactose_asset_api::PinnedTable;
use lactose_bundle::Baseline;
use lactose_core::{PlayerId, Tick};
use lactose_ecs::{ClaimedBy, Executor, RegionConfig, RegionId, RegionMode, WorldKind};
use lactose_sim::journal::{Journal, JournalFrame, JournalLoad};
use lactose_sim::{RegionBundle, SavePoint, SessionEvent, Simulation, TickInput};

/// The regions of the directed world: the centre and its eight neighbours.
fn neighbourhood() -> Vec<RegionId> {
    atlas().within(cell_of(region_at(2, 2)), 1).collect()
}

/// A directed server holding the neighbourhood, both players joined, run to tick `ticks`.
fn directed_world(executor: &Executor, ticks: u32) -> Simulation {
    let mut simulation = game(WorldKind::Server).build().unwrap();
    for tick in 1..=ticks {
        let mut frame = script(tick);
        if tick == 1 {
            for region in neighbourhood() {
                frame.loaded.insert(region, fresh(region));
            }
            frame.sessions.insert(PlayerId(1), SessionEvent::Join { region: region_at(2, 2) });
        }
        // The script's joins name regions of the neighbourhood or are held until they load.
        simulation = step(executor, simulation, frame);
    }
    simulation
}

fn save_of(executor: &Executor, simulation: Simulation) -> (Simulation, SavePoint) {
    match executor.block_on(simulation.save_point(executor)) {
        Ok(pair) => pair,
        Err(failure) => panic!("{:?}", failure.error),
    }
}

/// The bundles that load `save`, with the tables `simulation` had in force.
fn bundles(
    save: &SavePoint,
    tables: &BTreeMap<RegionId, PinnedTable>,
) -> BTreeMap<RegionId, RegionBundle> {
    save.regions
        .iter()
        .map(|export| {
            let mut bundle = reload(export, baseline(export.region), types().registry());
            bundle.assets = tables.get(&export.region).cloned();
            (export.region, bundle)
        })
        .collect()
}

/// Every joined player joins again, in the region its row is in; a row the game kept for
/// a player who left stays unclaimed.
fn joins(simulation: &mut Simulation) -> BTreeMap<PlayerId, SessionEvent> {
    let mut rows = Vec::new();
    simulation
        .world_mut()
        .query::<(&Player, &ClaimedBy)>()
        .for_each(|key, (player, _)| rows.push((key, player.id)));
    rows.into_iter()
        .map(|(key, player)| {
            let region = simulation.world().region_of(key).unwrap();
            (player, SessionEvent::Join { region })
        })
        .collect()
}

#[test]
fn a_fresh_load_an_in_place_reload_and_an_untouched_twin_hash_equal() {
    reload_at(60, 2);
}

/// Player 2 left at tick 150 and the game kept its row: the window in which a sample left
/// on the row would drive it from state no save holds.
#[test]
fn a_reload_while_a_left_players_row_is_kept_hashes_equal() {
    reload_at(160, 1);
}

/// The 15.5 tests with the save taken at tick `at`, when `players` are joined.
fn reload_at(at: u32, players: usize) {
    const K: u32 = 90;
    let executor = executor();
    let original = directed_world(&executor, at);
    let (mut original, save) = save_of(&executor, original);
    assert_eq!(save.tick, Tick(at));
    assert_eq!(
        save.regions.iter().map(|export| export.region).collect::<Vec<_>>(),
        neighbourhood()
    );
    let tables: BTreeMap<RegionId, PinnedTable> = neighbourhood()
        .into_iter()
        .map(|region| (region, original.world().region_assets(region).unwrap().clone()))
        .collect();
    let rejoin = joins(&mut original);
    assert_eq!(rejoin.len(), players);

    // The fresh simulation: built from the save point, loaded through the frame.
    let fresh_load =
        game(WorldKind::Server).tick(save.tick).next_net_id(save.next_net_id).build().unwrap();
    let mut first = script(at + 1);
    first.loaded = bundles(&save, &tables);
    first.sessions.extend(rejoin.clone());
    let mut fresh_load = step(&executor, fresh_load, first);

    // The in-place reload: every region unloaded and loaded again in one frame.
    let mut reload_frame = script(at + 1);
    reload_frame.unloaded = neighbourhood().into_iter().collect();
    reload_frame.loaded = bundles(&save, &tables);
    reload_frame.sessions.extend(rejoin);
    original = step(&executor, original, reload_frame);
    let reloaded = original.lifecycle_mut().take_exports();
    assert_eq!(reloaded.len(), neighbourhood().len(), "every region went and came back");
    assert!(save.regions.iter().map(|export| export.rows.rows.len()).sum::<usize>() > 2 * 9);

    // The twin ran the same program and never reloaded.
    let mut twin = directed_world(&executor, at + 1);

    for tick in at + 1..=at + K {
        if tick > at + 1 {
            original = step(&executor, original, script(tick));
            fresh_load = step(&executor, fresh_load, script(tick));
            twin = step(&executor, twin, script(tick));
        }
        let hash = original.hash().unwrap();
        assert_eq!(fresh_load.hash().unwrap(), hash, "fresh load at tick {tick}");
        assert_eq!(twin.hash().unwrap(), hash, "untouched twin at tick {tick}");
    }
    assert_eq!(original.tick(), Tick(at + K));
}

#[test]
fn an_unload_and_reload_keeps_every_persistent_column() {
    let executor = executor();
    let mut simulation = directed_world(&executor, 40);
    let region = region_at(2, 2);
    let mut unload = TickInput::default();
    unload.unloaded.insert(region);
    simulation = step(&executor, simulation, unload);
    let exports = simulation.lifecycle_mut().take_exports();
    assert_eq!(exports.len(), 1);
    let export = exports.into_iter().next().unwrap();
    assert!(!export.rows.rows.is_empty());
    // Back, frozen so nothing runs over it before the copy.
    let mut bundle = reload(&export, baseline(region), types().registry());
    bundle.mode = RegionMode::Frozen;
    let mut frame = TickInput::default();
    frame.loaded.insert(region, bundle);
    simulation = step(&executor, simulation, frame);
    let (_, save) = save_of(&executor, simulation);
    let again = save.regions.iter().find(|copy| copy.region == region).unwrap();
    assert_eq!(again, &export);
}

// ---- Journal replay ----

/// What the replay driver needs that a journal names by hash: every cooked baseline.
fn baselines() -> BTreeMap<u64, Arc<Baseline>> {
    atlas()
        .within(cell_of(region_at(2, 2)), 100)
        .map(|region| {
            let baseline = baseline(region);
            (baseline.content_hash(), baseline)
        })
        .collect()
}

/// The frame a journal entry records, rebuilt: baselines fetched by hash, tables pinned
/// again from a store.
fn replayed(frame: &JournalFrame, baselines: &BTreeMap<u64, Arc<Baseline>>) -> TickInput {
    let pin = |table: &lactose_asset_api::VersionTable| {
        let entries: Vec<(u64, u64)> =
            table.iter().map(|(id, version)| (id.to_bits(), version.to_bits())).collect();
        common::table(&entries)
    };
    let loaded = frame
        .loaded
        .iter()
        .map(|(&region, load): (&RegionId, &JournalLoad)| {
            let bundle = RegionBundle {
                seed: load.seed,
                mode: load.mode,
                baseline: baselines[&load.baseline_hash].clone(),
                overlay: load.overlay.clone(),
                resources: load.resources.clone(),
                transient_resources: load.transient_resources.clone(),
                assets: load.assets.as_ref().map(pin),
            };
            (region, bundle)
        })
        .collect();
    TickInput {
        time_scale: frame.time_scale,
        failures: frame.failures.clone(),
        unloaded: frame.unloaded.clone(),
        loaded,
        assets: frame.assets.iter().map(|(&region, table)| (region, pin(table))).collect(),
        modes: frame.modes.clone(),
        sessions: frame.sessions.clone(),
        inputs: frame.inputs.clone(),
    }
}

#[test]
fn a_journal_replayed_by_a_small_driver_reproduces_the_hashes() {
    const TICKS: u32 = 180;
    let executor = executor();
    // The checkpoint is the empty world at tick zero; the frames are fed back from the
    // lifecycle's requests while recording.
    let (mut simulation, checkpoint) = save_of(&executor, program(Default::default()));
    let mut driver = Driver::default();
    let mut journal = Journal {
        checkpoint,
        tables: BTreeMap::new(),
        modes: BTreeMap::new(),
        players: BTreeMap::new(),
        held_joins: BTreeMap::new(),
        requested: Default::default(),
        frames: Vec::new(),
    };
    let mut recorded = Vec::new();
    for tick in 1..=TICKS {
        let frame = program_frame(tick, &mut driver);
        let entry = JournalFrame::record(Tick(tick), &frame);
        assert_eq!(entry.sessions, frame.sessions);
        assert_eq!(entry.inputs, frame.inputs);
        assert_eq!(entry.loaded.len(), frame.loaded.len());
        for (region, load) in &entry.loaded {
            let bundle = &frame.loaded[region];
            assert_eq!(load.baseline_hash, bundle.baseline.content_hash());
            assert_eq!(load.assets.as_ref(), bundle.assets.as_ref().map(PinnedTable::table));
        }
        journal.frames.push(entry);
        simulation = step(&executor, simulation, frame);
        driver.observe(&mut simulation);
        recorded.push(simulation.hash().unwrap());
    }
    assert!(journal.frames.iter().any(|frame| !frame.loaded.is_empty()));

    // The replay: a simulation from the checkpoint, stepped with the journal's frames.
    let baselines = baselines();
    let mut replay = program(Default::default());
    assert_eq!(replay.tick(), journal.checkpoint.tick);
    let mut hashes = Vec::new();
    for frame in &journal.frames {
        replay = step(&executor, replay, replayed(frame, &baselines));
        assert_eq!(replay.tick(), frame.tick);
        hashes.push(replay.hash().unwrap());
    }
    assert_eq!(hashes, recorded);
}

/// What one step left in the lifecycle besides its requests, as a replay must reproduce it.
fn outputs(simulation: &mut Simulation) -> String {
    let lifecycle = simulation.lifecycle_mut();
    let exports: Vec<RegionId> = lifecycle.take_exports().iter().map(|e| e.region).collect();
    let failures = lifecycle.take_load_failures().len();
    format!("exports {exports:?} load failures {failures}")
}

/// Records the program with a checkpoint after tick `at`, the session state a save does
/// not carry, then `after` frames, with bundles arriving `delay` frames late; returns the
/// journal, the hashes and the outputs.
fn record_from(
    executor: &Executor,
    at: u32,
    after: u32,
    delay: u32,
) -> (Journal, Vec<u64>, Vec<String>) {
    let mut simulation = program(Default::default());
    let mut driver = Driver::default();
    driver.delay = delay;
    for tick in 1..=at {
        let frame = program_frame(tick, &mut driver);
        simulation = step(executor, simulation, frame);
        driver.observe(&mut simulation);
    }
    let (mut simulation, checkpoint) = save_of(executor, simulation);
    let world = simulation.world();
    let mut tables = BTreeMap::new();
    let mut modes = BTreeMap::new();
    for region in world.regions() {
        if let Some(table) = world.region_assets(region) {
            tables.insert(region, table.table().clone());
        }
        modes.insert(region, world.region(region).unwrap().mode);
    }
    let mut players = BTreeMap::new();
    let mut held_joins = BTreeMap::new();
    for player in [PlayerId(1), PlayerId(2)] {
        if let Some(key) = player_entity(simulation.world_mut(), player) {
            players.insert(player, key);
        }
        if let Some(region) = simulation.lifecycle().held_join(player) {
            held_joins.insert(player, region);
        }
    }
    let requested = atlas()
        .within(cell_of(region_at(2, 2)), 100)
        .filter(|&region| simulation.lifecycle().is_requested(region))
        .collect();
    let mut journal =
        Journal { checkpoint, tables, modes, players, held_joins, requested, frames: Vec::new() };
    let mut hashes = Vec::new();
    let mut seen = Vec::new();
    for tick in at + 1..=at + after {
        let frame = program_frame(tick, &mut driver);
        journal.frames.push(JournalFrame::record(Tick(tick), &frame));
        simulation = step(executor, simulation, frame);
        // The driver answers what the run asked, as `observe` would.
        let requests = simulation.lifecycle_mut().take_requests();
        for &request in &requests {
            driver.answer(request);
        }
        let rest = outputs(&mut simulation);
        seen.push(format!("{tick}: {requests:?} {rest}"));
        hashes.push(simulation.hash().unwrap());
    }
    (journal, hashes, seen)
}

/// The replay driver: restores what the journal holds before its first step (the
/// regions, an unjournaled edit the tool makes; the players' claims; the held joins and
/// requested regions through `restore_session_state`), then steps the journal's frames.
fn replay_from(executor: &Executor, journal: &Journal) -> (Vec<u64>, Vec<String>) {
    let save = &journal.checkpoint;
    let mut simulation = game(WorldKind::Server)
        .regions(anchored(true))
        .tick(save.tick)
        .next_net_id(save.next_net_id)
        .build()
        .unwrap();
    let pin = |table: &lactose_asset_api::VersionTable| {
        let entries: Vec<(u64, u64)> =
            table.iter().map(|(id, version)| (id.to_bits(), version.to_bits())).collect();
        common::table(&entries)
    };
    for export in &save.regions {
        let bundle = reload(export, baseline(export.region), types().registry());
        let world = simulation.world_mut();
        let mode = journal.modes[&export.region];
        world
            .create_region(RegionConfig { id: export.region, seed: export.rows.seed, mode })
            .unwrap();
        world.load_region(export.region, &bundle.baseline, bundle.overlay.as_deref()).unwrap();
        for resource in &bundle.resources {
            world.insert_region_resource_erased(export.region, resource.clone()).unwrap();
        }
        for &resource in &bundle.transient_resources {
            world.insert_region_resource_zeroed(export.region, resource).unwrap();
        }
        if let Some(table) = journal.tables.get(&export.region) {
            world.set_region_assets(export.region, pin(table)).unwrap();
        }
    }
    for (&player, &key) in &journal.players {
        let world = simulation.world_mut();
        world.insert(key, ClaimedBy { player }).unwrap();
        world.insert(key, lactose_sim::StreamingAnchor { priority: 0 }).unwrap();
    }
    simulation
        .lifecycle_mut()
        .restore_session_state(journal.held_joins.clone(), journal.requested.clone());
    let baselines = baselines();
    let mut hashes = Vec::new();
    let mut seen = Vec::new();
    for frame in &journal.frames {
        simulation = step(executor, simulation, replayed(frame, &baselines));
        let requests = simulation.lifecycle_mut().take_requests();
        let rest = outputs(&mut simulation);
        seen.push(format!("{}: {requests:?} {rest}", frame.tick.0));
        hashes.push(simulation.hash().unwrap());
    }
    (hashes, seen)
}

#[test]
fn a_journal_from_a_mid_run_checkpoint_replays_with_its_session_state() {
    let executor = executor();
    // Inside the held-join window, around the leave at 150 and the join again at 170. With
    // a slow store, regions are still requested at the checkpoint, so a replay that did
    // not restore them would request them again.
    for delay in [0, 4] {
        for at in [1_u32, 2, 3, 60, 149, 150, 151, 169, 170, 171, 200] {
            let (journal, recorded, original) = record_from(&executor, at, 60, delay);
            let (hashes, replayed) = replay_from(&executor, &journal);
            let first = hashes.iter().zip(&recorded).position(|(a, b)| a != b);
            assert_eq!(first, None, "delay {delay}, checkpoint {at}: first differing step");
            assert_eq!(replayed, original, "delay {delay}, checkpoint {at}: lifecycle outputs");
        }
    }
}
