//! Section 15.5's tests through this crate (README, "Proof for milestone 1"; architecture
//! section 15.5 and D124), recorded under SOUR: a server reloads every region in place at
//! tick T from this crate's bundles, a fresh process loads the container's bytes, and an
//! untouched twin never reloads; all three hash equal for K ticks. The two-frame reload
//! likewise, and an unload and reload through `from_export` and `bundle` exports the same
//! region. Each test records the reloaded server's hash after every step.

#![forbid(unsafe_code)]

mod common;
mod sour;

use std::collections::BTreeMap;

use common::*;
use lactose_asset_api::PinnedTable;
use lactose_core::{PlayerId, Tick};
use lactose_ecs::{ClaimedBy, Executor, LayoutConfig, RegionId, RegionMode, WorldKind};
use lactose_persist::{Container, Loader, Migrations, RegionRecords};
use lactose_sim::harness::run_recorded;
use lactose_sim::{RegionBundle, RegionExport, SavePoint, SessionEvent, Simulation, TickInput};
use sour::{Sour, record};

/// How many ticks the three runs are compared for after the reload.
const K: u32 = 90;

/// The regions of the directed world: the centre and its eight neighbours.
fn neighbourhood() -> Vec<RegionId> {
    atlas().within(cell_of(region_at(2, 2)), 1).collect()
}

fn directed(layout: LayoutConfig) -> lactose_sim::SimulationBuilder {
    game(WorldKind::Server).layout(layout)
}

/// The frame of `tick` in the directed world: the script, with the neighbourhood loaded
/// fresh at tick 1.
fn directed_frame(tick: u32) -> TickInput {
    let mut frame = script(tick);
    if tick == 1 {
        for region in neighbourhood() {
            frame.loaded.insert(region, fresh(region));
        }
    }
    frame
}

/// The tables and modes `simulation` has in force: what `app` keeps beside a save, since
/// neither is in one.
fn in_force(simulation: &Simulation) -> BTreeMap<RegionId, (PinnedTable, RegionMode)> {
    let world = simulation.world();
    world
        .regions()
        .map(|region| {
            let table = world.region_assets(region).unwrap().clone();
            (region, (table, world.region_mode(region).unwrap()))
        })
        .collect()
}

/// The bundle of `records` through this crate's loader, with the table and mode in force.
fn bundle(
    records: &RegionRecords,
    kept: &BTreeMap<RegionId, (PinnedTable, RegionMode)>,
) -> RegionBundle {
    let migrations = Migrations::new();
    let loader = Loader::new(schemas(), &migrations);
    let (table, mode) = kept[&records.region].clone();
    loader.bundle(records, baseline(records.region), mode, Some(table)).unwrap()
}

/// The bundles of every section of `bytes`, a container.
fn bundles(
    bytes: &[u8],
    kept: &BTreeMap<RegionId, (PinnedTable, RegionMode)>,
) -> BTreeMap<RegionId, RegionBundle> {
    let container = Container::decode(bytes).unwrap();
    container.regions().map(|records| (records.region, bundle(records, kept))).collect()
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

/// The recorded run of `test`: the directed world to tick `at`, each step recorded.
struct Run {
    sour: Sour,
    recorder: sour::Recorder,
    test: &'static str,
}

impl Run {
    fn new(test: &'static str) -> Self {
        let sour = Sour::from_env().unwrap();
        let recorder = sour.recorder(test, &sour.layout).unwrap();
        Self { sour, recorder, test }
    }

    fn executor(&self) -> &Executor {
        &self.sour.executor
    }

    /// Steps `simulation` through `frame`, recording the hash.
    fn step(&mut self, simulation: Simulation, frame: TickInput) -> Simulation {
        let recorder = &mut self.recorder;
        let mut hashed = |tick: Tick, hash: u64| record(recorder, tick, hash);
        run_recorded(&self.sour.executor, simulation, [frame], &mut hashed).unwrap()
    }

    /// The directed world built with the session's layout and run to tick `ticks`,
    /// recorded.
    fn original(&mut self, ticks: u32) -> Simulation {
        let mut simulation = directed(self.sour.layout).build().unwrap();
        assert_eq!(*simulation.world().layout(), self.sour.layout);
        for tick in 1..=ticks {
            simulation = self.step(simulation, directed_frame(tick));
        }
        simulation
    }

    /// An unrecorded copy of the program, run to tick `ticks`.
    fn twin(&self, ticks: u32) -> Simulation {
        let mut simulation = directed(self.sour.layout).build().unwrap();
        for tick in 1..=ticks {
            simulation = step(self.executor(), simulation, directed_frame(tick));
        }
        simulation
    }

    fn finish(self) {
        self.sour.finish(self.test).unwrap();
    }
}

/// The save of the original at tick `at`, as a container, with what `app` keeps beside it.
fn saved(run: &Run, original: Simulation, at: u32) -> (Simulation, SavePoint, Vec<u8>) {
    let (original, save) = save_of(run.executor(), original);
    assert_eq!(save.tick, Tick(at));
    let regions: Vec<RegionId> = save.regions.iter().map(|export| export.region).collect();
    assert_eq!(regions, neighbourhood());
    let bytes = save_bytes(&save);
    (original, save, bytes)
}

/// Section 15.5's first and fourth tests with the save taken at tick `at`, when `players`
/// are joined: the original reloads in place in one frame from the container, a fresh
/// process loads the container, and the twin never reloads.
fn reload_at(test: &'static str, at: u32, players: usize) {
    let mut run = Run::new(test);
    let original = run.original(at);
    let (mut original, save, bytes) = saved(&run, original, at);
    let kept = in_force(&original);
    let rejoin = joins(&mut original);
    assert_eq!(rejoin.len(), players);
    let header = Container::index(&bytes).unwrap();
    assert_eq!((header.header.tick, header.header.next_net_id), (save.tick, save.next_net_id));

    // The fresh process: built from the header, every section loaded in its first frame,
    // every player joining again in the same frame.
    let fresh = directed(run.sour.layout)
        .tick(header.header.tick)
        .next_net_id(header.header.next_net_id)
        .build()
        .unwrap();
    let mut first = script(at + 1);
    first.loaded = bundles(&bytes, &kept);
    first.sessions.extend(rejoin.clone());
    let mut fresh = step(run.executor(), fresh, first);

    // The in-place reload: every region unloaded and loaded again in one frame.
    let mut reload = script(at + 1);
    reload.unloaded = neighbourhood().into_iter().collect();
    reload.loaded = bundles(&bytes, &kept);
    reload.sessions.extend(rejoin);
    original = run.step(original, reload);
    let reloaded = original.lifecycle_mut().take_exports();
    assert_eq!(reloaded.len(), neighbourhood().len(), "every region went and came back");
    assert!(save.regions.iter().map(|export| export.rows.rows.len()).sum::<usize>() > 2 * 9);

    let mut twin = run.twin(at + 1);
    for tick in at + 1..=at + K {
        if tick > at + 1 {
            original = run.step(original, script(tick));
            fresh = step(run.executor(), fresh, script(tick));
            twin = step(run.executor(), twin, script(tick));
        }
        let hash = original.hash().unwrap();
        assert_eq!(fresh.hash().unwrap(), hash, "fresh process at tick {tick}");
        assert_eq!(twin.hash().unwrap(), hash, "untouched twin at tick {tick}");
    }
    assert_eq!(original.tick(), Tick(at + K));
    run.finish();
}

#[test]
fn a_reload_in_place_a_fresh_process_and_an_untouched_twin_hash_equal() {
    reload_at("lactose_persist::reload::in_place_at_60", 60, 2);
}

/// Player 2 left at tick 150 and the game kept its row: the save is taken in the window in
/// which a sample left on the row would drive it from state no save holds.
#[test]
fn a_save_while_a_left_players_row_is_kept_reloads_equal() {
    reload_at("lactose_persist::reload::left_player_at_160", 160, 1);
}

/// The two-frame reload: every region unloaded at tick `at + 1`, the exports the unload
/// left taken through `from_export` and `bundle` into the frame of `at + 2`. The exports
/// are the records of the save taken at `at`, and a fresh process that loads the container
/// at `at + 2` hashes equal to the reloaded server for K ticks.
#[test]
fn the_two_frame_reload_and_a_fresh_process_hash_equal() {
    let test = "lactose_persist::reload::two_frames_at_60";
    let at = 60;
    let mut run = Run::new(test);
    let original = run.original(at);
    let (mut original, save, bytes) = saved(&run, original, at);
    let kept = in_force(&original);
    let rejoin = joins(&mut original);
    assert_eq!(rejoin.len(), 2);

    let mut unload = script(at + 1);
    unload.unloaded = neighbourhood().into_iter().collect();
    original = run.step(original, unload);
    let exports = original.lifecycle_mut().take_exports();
    assert_eq!(exports.len(), neighbourhood().len());
    let container = Container::decode(&bytes).unwrap();
    let mut load = script(at + 2);
    for export in &exports {
        let records = records_of(export);
        assert_eq!(Some(&records), container.region(export.region), "the unload is the save");
        load.loaded.insert(export.region, bundle(&records, &kept));
    }
    load.sessions.extend(rejoin.clone());
    original = run.step(original, load);

    // The fresh process sees the same two frames: nothing loaded, then the container.
    let fresh = directed(run.sour.layout).tick(save.tick).next_net_id(save.next_net_id);
    let mut fresh = step(run.executor(), fresh.build().unwrap(), script(at + 1));
    let mut load = script(at + 2);
    load.loaded = bundles(&bytes, &kept);
    load.sessions.extend(rejoin);
    fresh = step(run.executor(), fresh, load);
    for tick in at + 2..=at + K {
        if tick > at + 2 {
            original = run.step(original, script(tick));
            fresh = step(run.executor(), fresh, script(tick));
        }
        assert_eq!(fresh.hash().unwrap(), original.hash().unwrap(), "at tick {tick}");
    }
    run.finish();
}

/// Section 15.5's second test: a region unloaded and loaded again through `from_export`
/// and `bundle` exports the same `RegionExport`.
#[test]
fn an_unload_and_reload_exports_the_same_region() {
    let test = "lactose_persist::reload::unload_and_reload";
    let mut run = Run::new(test);
    let mut simulation = run.original(40);
    let region = region_at(2, 2);
    let kept = in_force(&simulation);
    let mut unload = TickInput::default();
    unload.unloaded.insert(region);
    simulation = run.step(simulation, unload);
    let exports = simulation.lifecycle_mut().take_exports();
    assert_eq!(exports.len(), 1);
    let export: RegionExport = exports.into_iter().next().unwrap();
    assert!(!export.rows.rows.is_empty());
    // Back, frozen so nothing runs over it before the copy.
    let mut bundle = bundle(&records_of(&export), &kept);
    bundle.mode = RegionMode::Frozen;
    let mut frame = TickInput::default();
    frame.loaded.insert(region, bundle);
    simulation = run.step(simulation, frame);
    let (_, save) = save_of(run.executor(), simulation);
    let again = save.regions.iter().find(|copy| copy.region == region).unwrap();
    assert_eq!(again, &export);
    run.finish();
}
