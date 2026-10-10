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
use lactose_ecs::{Executor, RegionId, RegionMode, WorldKind};
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

/// Every player joins again, in the region its row is in.
fn joins(simulation: &mut Simulation) -> BTreeMap<PlayerId, SessionEvent> {
    let mut rows = Vec::new();
    simulation.world_mut().query::<&Player>().for_each(|key, player| rows.push((key, player.id)));
    rows.into_iter()
        .map(|(key, player)| {
            let region = simulation.world().region_of(key).unwrap();
            (player, SessionEvent::Join { region })
        })
        .collect()
}

#[test]
fn a_fresh_load_an_in_place_reload_and_an_untouched_twin_hash_equal() {
    const T: u32 = 60;
    const K: u32 = 90;
    let executor = executor();
    let original = directed_world(&executor, T);
    let (mut original, save) = save_of(&executor, original);
    assert_eq!(save.tick, Tick(T));
    assert_eq!(
        save.regions.iter().map(|export| export.region).collect::<Vec<_>>(),
        neighbourhood()
    );
    let tables: BTreeMap<RegionId, PinnedTable> = neighbourhood()
        .into_iter()
        .map(|region| (region, original.world().region_assets(region).unwrap().clone()))
        .collect();
    let rejoin = joins(&mut original);
    assert_eq!(rejoin.len(), 2);

    // The fresh simulation: built from the save point, loaded through the frame.
    let fresh_load =
        game(WorldKind::Server).tick(save.tick).next_net_id(save.next_net_id).build().unwrap();
    let mut first = script(T + 1);
    first.loaded = bundles(&save, &tables);
    first.sessions.extend(rejoin.clone());
    let mut fresh_load = step(&executor, fresh_load, first);

    // The in-place reload: every region unloaded and loaded again in one frame.
    let mut reload_frame = script(T + 1);
    reload_frame.unloaded = neighbourhood().into_iter().collect();
    reload_frame.loaded = bundles(&save, &tables);
    reload_frame.sessions.extend(rejoin);
    original = step(&executor, original, reload_frame);
    let reloaded = original.lifecycle_mut().take_exports();
    assert_eq!(reloaded.len(), neighbourhood().len(), "every region went and came back");
    assert!(save.regions.iter().map(|export| export.rows.rows.len()).sum::<usize>() > 2 * 9);

    // The twin ran the same program and never reloaded.
    let mut twin = directed_world(&executor, T + 1);

    for tick in T + 1..=T + K {
        if tick > T + 1 {
            original = step(&executor, original, script(tick));
            fresh_load = step(&executor, fresh_load, script(tick));
            twin = step(&executor, twin, script(tick));
        }
        let hash = original.hash().unwrap();
        assert_eq!(fresh_load.hash().unwrap(), hash, "fresh load at tick {tick}");
        assert_eq!(twin.hash().unwrap(), hash, "untouched twin at tick {tick}");
    }
    assert_eq!(original.tick(), Tick(T + K));
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
