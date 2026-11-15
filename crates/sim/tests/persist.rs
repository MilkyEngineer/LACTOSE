//! Saves, loads and journal replay (README, "Saves and loads" and "Journal replay";
//! architecture section 15.5): a fresh simulation from a save point, an in-place reload and
//! an un-reloaded twin hash equal for K ticks; an unload and reload keeps every persistent
//! column; a journal recorded from one run and replayed by a small driver reproduces the
//! hashes.

#![forbid(unsafe_code)]

mod common;

use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};

use common::*;
use lactose_asset_api::PinnedTable;
use lactose_bundle::Baseline;
use lactose_core::{EntityRef, NetId, PlayerId, PredictionKey, StableKey, Tick};
use lactose_ecs::{
    ClaimedBy, Executor, OwnedBy, PredictedOrigin, RegionConfig, RegionId, RegionMode, WorldKind,
};
use lactose_math::Scalar;
use lactose_reflect::{Reflect, bytemuck};
use lactose_sim::journal::{Journal, JournalFrame, JournalLoad};
use lactose_sim::{
    RegionBundle, RegionExport, SavePoint, SessionEvent, Simulation, SimulationBuilder, TickInput,
};

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
        binds: BTreeMap::new(),
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
    // No entity of the program has a predicted origin, so its world holds no bind.
    let journal = journal_at(&mut simulation, checkpoint, BTreeMap::new());
    record_after(executor, simulation, &mut driver, journal, after)
}

/// The journal of a checkpoint taken from `simulation`, with the session state a save does
/// not carry and `binds`, and no frame yet.
fn journal_at(
    simulation: &mut Simulation,
    checkpoint: SavePoint,
    binds: BTreeMap<PredictionKey, NetId>,
) -> Journal {
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
    Journal { checkpoint, binds, tables, modes, players, held_joins, requested, frames: Vec::new() }
}

/// Records `after` frames of the program into `journal` from where `simulation` stands;
/// returns the journal, the hashes and the outputs.
fn record_after(
    executor: &Executor,
    mut simulation: Simulation,
    driver: &mut Driver,
    mut journal: Journal,
    after: u32,
) -> (Journal, Vec<u64>, Vec<String>) {
    let at = simulation.tick().0;
    let mut hashes = Vec::new();
    let mut seen = Vec::new();
    for tick in at + 1..=at + after {
        let frame = program_frame(tick, driver);
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

/// The replay driver's start: builds with the journal's binds and restores what the journal
/// holds before its first step (the regions, an unjournaled edit the tool makes; the
/// players' claims; the held joins and requested regions through `restore_session_state`).
fn restored(journal: &Journal) -> Simulation {
    let save = &journal.checkpoint;
    let mut simulation = game(WorldKind::Server)
        .regions(anchored(true))
        .tick(save.tick)
        .next_net_id(save.next_net_id)
        .binds(journal.binds.clone())
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
    simulation
}

/// The replay driver: [`restored`], then the journal's frames.
fn replay_from(executor: &Executor, journal: &Journal) -> (Vec<u64>, Vec<String>) {
    let mut simulation = restored(journal);
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

// ---- Binds across a save (D142) ----

/// The region of the owned row, the cooked rock of its baseline.
const OWNED: RegionId = region_at(1, 1);
/// The region of the owner.
const OWNERS: RegionId = region_at(2, 1);

/// The rock of [`OWNED`], which the owner owns.
fn rock() -> StableKey {
    StableKey::Net(cooked(OWNED, 2))
}

/// The prediction key of the owner, when it has one.
fn owner_origin() -> PredictionKey {
    PredictionKey { player: PlayerId(7), input_tick: Tick(1), ordinal: 0x0D1 }
}

/// The binds a fresh process reads from `exports`: each row's `PredictedOrigin` with the
/// row's `NetId`, as `lactose-persist`'s `Loader::binds` reads them from records.
fn binds_in<'e>(
    exports: impl IntoIterator<Item = &'e RegionExport>,
) -> BTreeMap<PredictionKey, NetId> {
    let mut binds = BTreeMap::new();
    for row in exports.into_iter().flat_map(|export| &export.rows.rows) {
        for data in row.components.iter().filter(|data| data.component == PredictedOrigin::KEY) {
            let origin: PredictedOrigin = bytemuck::checked::pod_read_unaligned(&data.bytes);
            let key = PredictionKey {
                player: origin.player,
                input_tick: origin.input_tick,
                ordinal: origin.ordinal,
            };
            assert_eq!(binds.insert(key, row.id), None, "one key under one NetId");
        }
    }
    binds
}

fn owner_of(simulation: &Simulation, key: StableKey) -> Option<StableKey> {
    simulation.world().get::<OwnedBy>(key).and_then(|owned| owned.owner.get())
}

/// What [`owner_in_an_unloaded_region`] saw.
struct Compared {
    /// The owner's key in the original.
    owner: StableKey,
    /// The ticks at which the two hash apart, before and after the owner's region loads.
    before: Vec<u32>,
    after: Vec<u32>,
    /// The rock's owner in the fresh simulation after its first step.
    fresh_owner: Option<StableKey>,
}

/// The history of the two tests: [`OWNED`] and [`OWNERS`] loaded under `Directed`; an owner
/// spawned in `OWNERS`, with a predicted origin and bound when `predicted`, under a runtime
/// `Net` key otherwise; the rock of `OWNED` given to it; a save point at tick 10. The
/// owner's region then unloads in the original, which reloads `OWNED` in place from the
/// save point in the same frame; a fresh simulation built with the save's tick and counter
/// and, when `with_binds`, the binds read from both exports loads `OWNED` alone. `K` ticks
/// later both load the owner's region, the original from the export its unload left, the
/// fresh one from the save point.
fn owner_in_an_unloaded_region(predicted: bool, with_binds: bool) -> Compared {
    const K: u32 = 30;
    let executor = executor();
    let mut original = game(WorldKind::Server).build().unwrap();
    let mut first = TickInput::default();
    first.loaded.insert(OWNED, fresh(OWNED));
    first.loaded.insert(OWNERS, fresh(OWNERS));
    original = step(&executor, original, first);
    let world = original.world_mut();
    let net = world.allocate_net_id().unwrap();
    let at = Position { x: Scalar::ONE, y: Scalar::ONE };
    let owner = if predicted {
        let key = StableKey::Predicted(owner_origin());
        world.spawn(key, OWNERS, (at,)).unwrap();
        world.bind(owner_origin(), net).unwrap();
        key
    } else {
        let key = StableKey::Net(net);
        world.spawn(key, OWNERS, (at,)).unwrap();
        key
    };
    world.insert(rock(), OwnedBy { owner: EntityRef::new(owner) }).unwrap();
    for _ in 2..=10 {
        original = step(&executor, original, TickInput::default());
    }
    let (mut original, save) = save_of(&executor, original);
    assert_eq!(save.tick, Tick(10));
    let export = |region: RegionId| save.regions.iter().find(|e| e.region == region).unwrap();
    let binds = binds_in(&save.regions);
    if predicted {
        assert_eq!(binds, BTreeMap::from([(owner_origin(), net)]));
    } else {
        assert!(binds.is_empty());
    }

    let mut fresh = game(WorldKind::Server).tick(save.tick).next_net_id(save.next_net_id);
    if with_binds {
        fresh = fresh.binds(binds);
    }
    let mut fresh = fresh.build().unwrap();
    // Before any step: the bind, no entity under either key, the saved counter.
    let bound = (with_binds && predicted).then_some(net);
    assert_eq!(fresh.world().alias(owner_origin()), bound);
    assert!(!fresh.world().contains(owner) && !fresh.world().contains(StableKey::Net(net)));
    assert_eq!(fresh.world().next_net_id(), save.next_net_id);

    let reload_owned = || reload(export(OWNED), baseline(OWNED), types().registry());
    let mut in_place = TickInput::default();
    in_place.unloaded.insert(OWNED);
    in_place.unloaded.insert(OWNERS);
    in_place.loaded.insert(OWNED, reload_owned());
    original = step(&executor, original, in_place);
    let left = original.lifecycle_mut().take_exports();
    let kept = left.into_iter().find(|e| e.region == OWNERS).expect("the owner's region left");
    assert_eq!(&kept, export(OWNERS), "the unload left the saved state");
    let mut alone = TickInput::default();
    alone.loaded.insert(OWNED, reload_owned());
    fresh = step(&executor, fresh, alone);
    assert_eq!(loaded(&original), loaded(&fresh));
    assert_eq!(owner_of(&original, rock()), Some(owner));
    let fresh_owner = owner_of(&fresh, rock());

    let mut before = Vec::new();
    for tick in 11..=11 + K {
        if tick > 11 {
            original = step(&executor, original, TickInput::default());
            fresh = step(&executor, fresh, TickInput::default());
        }
        if original.hash().unwrap() != fresh.hash().unwrap() {
            before.push(tick);
        }
    }
    let mut back = TickInput::default();
    back.loaded.insert(OWNERS, reload(&kept, baseline(OWNERS), types().registry()));
    original = step(&executor, original, back);
    let mut back = TickInput::default();
    back.loaded.insert(OWNERS, reload(export(OWNERS), baseline(OWNERS), types().registry()));
    fresh = step(&executor, fresh, back);
    let mut after = Vec::new();
    for tick in 12 + K..=12 + 2 * K {
        if tick > 12 + K {
            original = step(&executor, original, TickInput::default());
            fresh = step(&executor, fresh, TickInput::default());
        }
        if original.hash().unwrap() != fresh.hash().unwrap() {
            after.push(tick);
        }
    }
    assert_eq!(original.tick(), Tick(12 + 2 * K));
    Compared { owner, before, after, fresh_owner }
}

/// The whole-project review's probe D1 at this crate's level: a row owned from a region
/// that is not loaded reloads to the same owner in a fresh process given the save's binds.
#[test]
fn a_row_owned_from_an_unloaded_region_reloads_to_the_same_owner() {
    let with = owner_in_an_unloaded_region(true, true);
    assert_eq!(with.fresh_owner, Some(with.owner), "the predicted origin, at the first load");
    assert_eq!((with.before, with.after), (Vec::new(), Vec::new()));

    // Without the binds the fresh process stores the owner's `Net` key and hashes apart.
    let without = owner_in_an_unloaded_region(true, false);
    assert!(matches!(without.fresh_owner, Some(StableKey::Net(_))), "{:?}", without.fresh_owner);
    assert_eq!(without.before.first(), Some(&11), "the failure the binds prevent");
}

/// The control: an owner whose origin key is its `Net` key reloads alike with the binds and
/// without them, so the difference above is the bind.
#[test]
fn an_owner_with_a_net_key_in_an_unloaded_region_reloads_alike() {
    for with_binds in [true, false] {
        let compared = owner_in_an_unloaded_region(false, with_binds);
        assert_eq!(compared.fresh_owner, Some(compared.owner));
        assert_eq!((compared.before, compared.after), (Vec::new(), Vec::new()));
    }
}

#[test]
fn a_simulation_built_with_binds_holds_them_before_its_first_step() {
    let executor = executor();
    let key = |ordinal| PredictionKey { player: PlayerId(3), input_tick: Tick(2), ordinal };
    let id = |bits| NetId::from_bits(bits).unwrap();
    let binds = BTreeMap::from([(key(1), id(4)), (key(2), id(50))]);

    // Bound before any setup function runs; the second call replaced the first; a bind
    // above the builder's counter moved the counter past it.
    let seen = Arc::new(Mutex::new(None));
    let in_setup = Arc::clone(&seen);
    let simulation = game(WorldKind::Server)
        .next_net_id(NonZeroU64::new(10).unwrap())
        .binds(BTreeMap::from([(key(9), id(9))]))
        .binds(binds.clone())
        .setup(move |world| {
            *in_setup.lock().unwrap() = Some((world.alias(key(1)), world.alias(key(2))));
            Ok(())
        })
        .build()
        .unwrap();
    assert_eq!(*seen.lock().unwrap(), Some((Some(id(4)), Some(id(50)))));
    let world = simulation.world();
    assert_eq!(world.alias(key(9)), None, "the second call replaced the first");
    for (&predicted, &net) in &binds {
        assert_eq!(world.alias(predicted), Some(net));
        assert!(!world.contains(StableKey::Predicted(predicted)));
        assert!(!world.contains(StableKey::Net(net)));
    }
    assert_eq!(world.next_net_id().get(), 51);

    // A region whose row carries a bound origin, from another world's export: its load
    // binds nothing new and loads the row under the origin key.
    let mut source = game(WorldKind::Server).build().unwrap();
    let mut frame = TickInput::default();
    frame.loaded.insert(OWNERS, fresh(OWNERS));
    source = step(&executor, source, frame);
    let world = source.world_mut();
    world.spawn(StableKey::Predicted(key(1)), OWNERS, (Health(3),)).unwrap();
    world.bind(key(1), id(4)).unwrap();
    let (_, save) = save_of(&executor, source);
    let mut simulation = simulation;
    let mut frame = TickInput::default();
    frame.loaded.insert(OWNERS, reload(&save.regions[0], baseline(OWNERS), types().registry()));
    simulation = step(&executor, simulation, frame);
    let world = simulation.world();
    assert!(world.contains(StableKey::Predicted(key(1))));
    assert_eq!(world.get::<Health>(StableKey::Predicted(key(1))), Some(&Health(3)));
    assert_eq!((world.alias(key(1)), world.alias(key(2))), (Some(id(4)), Some(id(50))));
    assert_eq!(world.alias(key(9)), None);

    // A prediction and a client world take binds the same way.
    for kind in [WorldKind::Prediction, WorldKind::Client] {
        let simulation =
            SimulationBuilder::new(kind, types()).binds(binds.clone()).build().unwrap();
        for (&predicted, &net) in &binds {
            assert_eq!(simulation.world().alias(predicted), Some(net), "{kind:?}");
        }
    }
}

/// D142 for a replay: a run of the program under `Anchored` in which every region loaded
/// at tick 20 but the pinned one gets a bound owner with a predicted origin, and each owns
/// a row of the pinned region; a checkpoint at the first step after which one of the
/// owners' regions has unloaded. The journal's binds, read from the checkpoint's exports
/// and the export the unload left, hold that owner's bind; the replay driver builds with
/// them and reproduces every hash and output. With the binds emptied the replay differs at
/// its first tick.
#[test]
fn a_journal_from_a_checkpoint_with_a_predicted_owner_unloaded_replays_the_hashes() {
    let executor = executor();
    let mut simulation = program(Default::default());
    let mut driver = Driver::default();
    let advance = |simulation: Simulation, driver: &mut Driver| {
        let frame = program_frame(simulation.tick().0 + 1, driver);
        let mut simulation = step(&executor, simulation, frame);
        driver.observe(&mut simulation);
        simulation
    };
    for _ in 1..=20 {
        simulation = advance(simulation, &mut driver);
    }
    let regions: Vec<RegionId> =
        simulation.world().regions().filter(|&region| region != PINNED).collect();
    assert!(regions.len() > 1, "{regions:?}");
    let mut owners = BTreeMap::new();
    let world = simulation.world_mut();
    for (ordinal, &region) in (0..).zip(&regions) {
        let origin = PredictionKey { player: PlayerId(9), input_tick: Tick(20), ordinal };
        let at = Position { x: Scalar::ONE, y: Scalar::ONE };
        world.spawn(StableKey::Predicted(origin), region, (at,)).unwrap();
        let net = world.allocate_net_id().unwrap();
        world.bind(origin, net).unwrap();
        let owned = StableKey::Net(world.allocate_net_id().unwrap());
        let owner = OwnedBy { owner: EntityRef::new(StableKey::Predicted(origin)) };
        world.spawn(owned, PINNED, (at, owner)).unwrap();
        owners.insert(region, (origin, net, owned));
    }
    while regions.iter().all(|&region| simulation.world().region(region).is_some()) {
        simulation = advance(simulation, &mut driver);
        assert!(simulation.tick().0 < 400, "no owner's region unloaded");
    }
    let (left, &(origin, net, owned)) =
        owners.iter().find(|&(&region, _)| simulation.world().region(region).is_none()).unwrap();
    assert!(driver.stored.contains_key(left), "the unload left an export");
    assert_eq!(simulation.world().alias(origin), Some(net), "an unloaded owner keeps its bind");

    let (mut simulation, checkpoint) = save_of(&executor, simulation);
    let binds = binds_in(checkpoint.regions.iter().chain(driver.stored.values()));
    assert_eq!(binds.get(&origin), Some(&net), "the bind of the export the unload left");
    assert_eq!(binds.len(), owners.len(), "{binds:?}");
    assert!(!checkpoint.regions.iter().any(|export| export.region == *left));
    let journal = journal_at(&mut simulation, checkpoint, binds);
    let (mut journal, recorded, original) =
        record_after(&executor, simulation, &mut driver, journal, 60);

    // The binds are held before the first region loads: the restored world, before its
    // first step, stores the unloaded owner under its origin key.
    let start = restored(&journal);
    let stored = start.world().get::<OwnedBy>(owned).and_then(|row| row.owner.get());
    assert_eq!(stored, Some(StableKey::Predicted(origin)), "the owner before the first step");
    drop(start);
    let (hashes, replayed) = replay_from(&executor, &journal);
    assert_eq!(hashes, recorded);
    assert_eq!(replayed, original);

    journal.binds.clear();
    let (hashes, _) = replay_from(&executor, &journal);
    let first = hashes.iter().zip(&recorded).position(|(a, b)| a != b);
    assert_eq!(first, Some(0), "without the binds the replay differs at its first tick");
}
