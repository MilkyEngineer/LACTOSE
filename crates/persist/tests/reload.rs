//! Section 15.5's tests through this crate (README, "Proof for milestone 1"; architecture
//! section 15.5 and D124), recorded under SOUR: a server reloads every region in place at
//! tick T from this crate's bundles, a fresh process loads the container's bytes, and an
//! untouched twin never reloads; all three hash equal for K ticks. The two-frame reload
//! likewise, and an unload and reload through `from_export` and `bundle` exports the same
//! region. Each test records the reloaded server's hash after every step.

#![forbid(unsafe_code)]

use crate::common;
use crate::glue;

use std::collections::BTreeMap;

use common::*;
use glue::{SessionSetup, record};
use lactose_asset_api::PinnedTable;
use lactose_core::{EntityRef, PlayerId, PredictionKey, StableKey, Tick};
use lactose_ecs::{ClaimedBy, Executor, LayoutConfig, OwnedBy, RegionId, RegionMode, WorldKind};
use lactose_persist::{Container, Loader, Migrations, RegionRecords};
use lactose_sim::harness::run_recorded;
use lactose_sim::{RegionBundle, RegionExport, SavePoint, SessionEvent, Simulation, TickInput};

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
    setup: SessionSetup,
    /// Opened by [`Run::original`] with the layout the original's world reports.
    recorder: Option<glue::Recorder>,
    test: &'static str,
}

impl Run {
    fn new(test: &'static str) -> Self {
        Self { setup: SessionSetup::from_env().unwrap(), recorder: None, test }
    }

    fn executor(&self) -> &Executor {
        &self.setup.executor
    }

    /// Steps `simulation` through `frame`, recording the hash.
    fn step(&mut self, simulation: Simulation, frame: TickInput) -> Simulation {
        let recorder = self.recorder.as_mut().expect("the original opened the recorder");
        let mut hashed = |tick: Tick, hash: u64| record(recorder, tick, hash);
        run_recorded(&self.setup.executor, simulation, [frame], &mut hashed).unwrap()
    }

    /// The directed world built with the session's layout and run to tick `ticks`,
    /// recorded.
    fn original(&mut self, ticks: u32) -> Simulation {
        let mut simulation = directed(self.setup.layout).build().unwrap();
        self.recorder = Some(self.setup.recorder(self.test, simulation.world().layout()).unwrap());
        for tick in 1..=ticks {
            simulation = self.step(simulation, directed_frame(tick));
        }
        simulation
    }

    /// An unrecorded copy of the program, run to tick `ticks`.
    fn twin(&self, ticks: u32) -> Simulation {
        let mut simulation = directed(self.setup.layout).build().unwrap();
        for tick in 1..=ticks {
            simulation = step(self.executor(), simulation, directed_frame(tick));
        }
        simulation
    }

    /// `builder` built with the session's layout, with the recorder opened for it.
    fn open(&mut self, builder: lactose_sim::SimulationBuilder) -> Simulation {
        let simulation = builder.layout(self.setup.layout).build().unwrap();
        self.recorder = Some(self.setup.recorder(self.test, simulation.world().layout()).unwrap());
        simulation
    }

    fn finish(self) {
        self.setup.finish(self.test).unwrap();
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
    let fresh = directed(run.setup.layout)
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
    let fresh = directed(run.setup.layout).tick(save.tick).next_net_id(save.next_net_id);
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

// ---- Binds across a save (D142) ----

/// The rock of `A`, which the owner owns.
fn rock() -> StableKey {
    StableKey::Net(cooked(A, 2))
}

/// The prediction key of the owner, when it has one.
fn owner_origin() -> PredictionKey {
    PredictionKey { player: PlayerId(7), input_tick: Tick(1), ordinal: 0x0D1 }
}

fn owner_of(simulation: &Simulation) -> Option<StableKey> {
    simulation.world().get::<OwnedBy>(rock()).and_then(|owned| owned.owner.get())
}

/// The bundle that loads `records` frozen, with the region's first table.
fn frozen(records: &RegionRecords) -> RegionBundle {
    let mut bundle = bundle_of(records, Some(first_table(records.region)));
    bundle.mode = RegionMode::Frozen;
    bundle
}

fn loaded_regions(simulation: &Simulation) -> Vec<RegionId> {
    simulation.world().regions().collect()
}

/// What a fresh process saw beside the original.
struct Resumed {
    /// The owner's key in the original.
    owner: StableKey,
    /// The rock's owner in the fresh process once `A` loaded.
    fresh_owner: Option<StableKey>,
    /// The ticks at which the two hash apart.
    differ: Vec<u32>,
}

/// The whole-project review's probe D1 through this crate's bytes: `A` and `B` loaded
/// fresh; an owner spawned in `B`, with a predicted origin and bound when `predicted`,
/// under a runtime `Net` key otherwise; the rock of `A` given to it; the save at tick 10
/// through the container's bytes. `B` then unloads in the original, which reloads `A` in
/// place from the bytes; a fresh process decodes them, builds with the header's tick and
/// counter and, when `with_binds`, `Loader::binds` over both sections, and loads `A` alone.
/// Both load `A` frozen, so the first comparison is of the state as loaded, and simulate
/// it from the next step; `K` ticks later both load `B`, the original from the export its
/// unload left, the fresh process from the bytes. The original's steps are recorded when
/// `recorded`.
fn owner_in_an_unloaded_region(
    run: &mut Run,
    predicted: bool,
    with_binds: bool,
    recorded: bool,
) -> Resumed {
    const K: u32 = 40;
    let executor = run.executor().clone();
    let advance = |run: &mut Run, simulation: Simulation, frame: TickInput| {
        if recorded { run.step(simulation, frame) } else { step(&executor, simulation, frame) }
    };
    let mut original = if recorded {
        run.open(game(WorldKind::Server))
    } else {
        directed(run.setup.layout).build().unwrap()
    };
    let mut first = TickInput::default();
    first.loaded.insert(A, fresh(A));
    first.loaded.insert(B, fresh(B));
    original = advance(run, original, first);
    let world = original.world_mut();
    let net = world.allocate_net_id().unwrap();
    let owner = if predicted {
        let key = StableKey::Predicted(owner_origin());
        world.spawn(key, B, (pos(1, 1),)).unwrap();
        world.bind(owner_origin(), net).unwrap();
        key
    } else {
        let key = StableKey::Net(net);
        world.spawn(key, B, (pos(1, 1),)).unwrap();
        key
    };
    world.insert(rock(), OwnedBy { owner: EntityRef::new(owner) }).unwrap();
    for _ in 2..=10 {
        original = advance(run, original, TickInput::default());
    }
    let (mut original, save) = save_of(&executor, original);
    let bytes = save_bytes(&save);
    let container = Container::decode(&bytes).unwrap();
    let header = container.header();
    assert_eq!((header.tick, header.next_net_id), (Tick(10), save.next_net_id));
    let migrations = Migrations::new();
    let binds = Loader::new(schemas(), &migrations).binds(container.regions()).unwrap();
    if predicted {
        assert_eq!(binds, BTreeMap::from([(owner_origin(), net)]));
    } else {
        assert!(binds.is_empty(), "{binds:?}");
    }
    let saved = |region| container.region(region).unwrap();

    let mut in_place = TickInput::default();
    in_place.unloaded.insert(A);
    in_place.unloaded.insert(B);
    in_place.loaded.insert(A, frozen(saved(A)));
    original = advance(run, original, in_place);
    let left = original.lifecycle_mut().take_exports();
    let kept = records_of(left.iter().find(|export| export.region == B).unwrap());
    assert_eq!(&kept, saved(B), "the unload left the saved state");

    let mut fresh = directed(run.setup.layout).tick(header.tick).next_net_id(header.next_net_id);
    if with_binds {
        fresh = fresh.binds(binds);
    }
    let mut fresh = fresh.build().unwrap();
    let bound = (with_binds && predicted).then_some(net);
    assert_eq!(fresh.world().alias(owner_origin()), bound, "bound before any step");
    let mut alone = TickInput::default();
    alone.loaded.insert(A, frozen(saved(A)));
    fresh = step(&executor, fresh, alone);
    assert_eq!(owner_of(&original), Some(owner));
    let fresh_owner = owner_of(&fresh);
    // The fresh process exports the save's records for `A`, with the binds or without.
    let export = RegionExport { region: A, rows: fresh.world().export_region(A).unwrap() };
    assert_eq!(&records_of(&export), saved(A));

    let mut differ = Vec::new();
    for tick in 11..=11 + 2 * K {
        let mut frame = TickInput::default();
        if tick == 12 {
            frame.modes.insert(A, RegionMode::Simulated);
        }
        let (mut in_original, mut in_fresh) = (frame.clone(), frame);
        if tick == 12 + K {
            in_original.loaded.insert(B, bundle_of(&kept, Some(first_table(B))));
            in_fresh.loaded.insert(B, bundle_of(saved(B), Some(first_table(B))));
        }
        if tick > 11 {
            original = advance(run, original, in_original);
            fresh = step(&executor, fresh, in_fresh);
        }
        if original.hash().unwrap() != fresh.hash().unwrap() {
            differ.push(tick);
        }
    }
    assert_eq!(loaded_regions(&original), vec![A, B]);
    assert_eq!(loaded_regions(&fresh), vec![A, B]);
    Resumed { owner, fresh_owner, differ }
}

/// The whole-project review's probe D1 at the container's bytes: equal with the binds,
/// recorded under SOUR; built without them the fresh process stores the owner's `Net` key
/// and hashes apart from the first comparison, which is the failure the binds prevent.
#[test]
fn a_fresh_process_that_leaves_a_predicted_owners_region_unloaded_hashes_equal() {
    let mut run = Run::new("lactose_persist::reload::predicted_owner_unloaded");
    let with = owner_in_an_unloaded_region(&mut run, true, true, true);
    assert_eq!(with.fresh_owner, Some(with.owner), "the predicted origin in both");
    assert_eq!(with.differ, Vec::<u32>::new());
    let without = owner_in_an_unloaded_region(&mut run, true, false, false);
    assert!(matches!(without.fresh_owner, Some(StableKey::Net(_))), "{:?}", without.fresh_owner);
    assert_eq!(without.differ.first(), Some(&11), "{:?}", without.differ);
    run.finish();
}

/// The control: the same history with an owner whose origin key is its `Net` key, equal
/// with the binds and without them, so the difference above is the bind.
#[test]
fn an_owner_with_a_net_key_in_an_unloaded_region_hashes_equal() {
    let mut run = Run::new("lactose_persist::reload::net_owner_unloaded");
    for (with_binds, recorded) in [(true, true), (false, false)] {
        let compared = owner_in_an_unloaded_region(&mut run, false, with_binds, recorded);
        assert_eq!(compared.fresh_owner, Some(compared.owner));
        assert_eq!(compared.differ, Vec::<u32>::new(), "with binds: {with_binds}");
    }
    run.finish();
}
