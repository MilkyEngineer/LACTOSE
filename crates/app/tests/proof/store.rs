//! "The store" and "Baselines": what a region's unload keeps, what a load takes back, and
//! the baseline each diff is against, under `Directed` and `Anchored`.

use std::collections::BTreeSet;
use std::sync::Arc;

use lactose_app::{SavedWorld, Server, Start, WorldConfig};
use lactose_bundle::ComponentData;
use lactose_core::{ComponentKey, ErrorKind, StableKey};
use lactose_ecs::{Executor, RegionConfig, RegionId, RegionMode};
use lactose_persist::RegionRecords;
use lactose_sim::{RegionExport, RegionRequest, SessionEvent, TickInput};

use crate::common::{
    Content, Counter, anchored, capture, executor, kept, load, loaded, logged, server, session,
    step, table_of, try_step,
};

/// Spawns a counter at zero into loaded `region`, an unjournaled edit.
fn spawn_counter(server: &mut Server, region: u64) -> StableKey {
    let world = server.simulation_mut().world_mut();
    let key = StableKey::Net(world.allocate_net_id().expect("a net id"));
    world.spawn(key, RegionId(region), (Counter(0),)).expect("the spawn");
    key
}

/// The counter of `key`, if it is loaded.
fn counter(server: &Server, key: StableKey) -> Option<u32> {
    server.simulation().world().get::<Counter>(key).map(|counter| counter.0)
}

/// A frame that unloads `region`.
fn unload(region: u64) -> TickInput {
    let mut frame = TickInput::default();
    frame.unloaded.insert(RegionId(region));
    frame
}

fn save(executor: &Executor, server: Server) -> (Server, SavedWorld) {
    executor.block_on(server.save_point(executor)).expect("the save point")
}

fn ids(saved: &SavedWorld) -> BTreeSet<u64> {
    saved.regions.keys().map(|region| region.0).collect()
}

/// The 25 regions of the grid.
fn grid_ids() -> BTreeSet<u64> {
    (100..125).collect()
}

/// A saved world of the 25 grid regions, from a `Directed` server, with a counter in 112.
fn saved_grid(executor: &Executor, content: &Arc<Content>) -> SavedWorld {
    let server = server(executor, WorldConfig::headless(), Start::Fresh, content);
    let (mut server, out) = step(executor, server, load(content, grid_ids()));
    assert!(out.failures.is_empty(), "{out:?}");
    spawn_counter(&mut server, 112);
    let (server, _) = step(executor, server, TickInput::default());
    let (server, saved) = save(executor, server);
    assert_eq!(ids(&saved), grid_ids());
    server.shutdown();
    saved
}

/// The regions a step asked to load.
fn loads(requests: &[RegionRequest]) -> BTreeSet<u64> {
    requests
        .iter()
        .filter_map(|request| match *request {
            RegionRequest::Load { region } => Some(region.0),
            RegionRequest::Cancel { .. } | RegionRequest::Release { .. } => None,
        })
        .collect()
}

#[test]
fn a_directed_unload_is_kept_and_a_fresh_load_takes_it_back() {
    let log = capture();
    let executor = executor(2);
    let content = Content::new();
    let server = server(&executor, WorldConfig::headless(), Start::Fresh, &content);
    let bundle = content.bundle(RegionId(1), RegionMode::Simulated);
    let loaded_with = Arc::clone(&bundle.baseline);
    let mut first = TickInput::default();
    first.loaded.insert(RegionId(1), bundle);
    let (mut server, _) = step(&executor, server, first);
    let key = spawn_counter(&mut server, 1);
    for _ in 0..3 {
        server = step(&executor, server, TickInput::default()).0;
    }
    assert_eq!(counter(&server, key), Some(3));
    // A refused bundle records nothing.
    let (server, out) = step(&executor, server, load(&content, [1]));
    assert_eq!(out.failures.loads.len(), 1);
    assert!(Arc::ptr_eq(server.baseline(RegionId(1)).expect("loaded"), &loaded_with));

    let (server, out) = step(&executor, server, unload(1));
    assert!(out.failures.is_empty(), "{out:?}");
    assert_eq!((loaded(&server), kept(&server)), (BTreeSet::new(), BTreeSet::from([1])));
    assert!(server.baseline(RegionId(1)).is_none());
    assert!(
        Arc::ptr_eq(&server.kept()[&RegionId(1)].baseline, &loaded_with),
        "diffed against the bundle it was loaded with"
    );
    let (server, saved) = save(&executor, server);
    assert_eq!(ids(&saved), BTreeSet::from([1]), "a save after the unload holds it");

    // A frame loading it fresh, `Frozen`, loads the stored state with the frame's mode and
    // table.
    assert_eq!(logged(&log, tracing::Level::INFO, "loads the saved state"), 0);
    let (server, out) = step(&executor, server, {
        let mut fresh = TickInput::default();
        fresh.loaded.insert(RegionId(1), content.bundle(RegionId(1), RegionMode::Frozen));
        fresh
    });
    assert!(out.failures.is_empty(), "{out:?}");
    assert_eq!((loaded(&server), kept(&server)), (BTreeSet::from([1]), BTreeSet::new()));
    let world = server.simulation().world();
    assert_eq!(world.region_mode(RegionId(1)), Some(RegionMode::Frozen));
    assert_eq!(
        world.region_assets(RegionId(1)).map(|pinned| pinned.table().clone()),
        Some(table_of(RegionId(1)))
    );
    assert_eq!(counter(&server, key), Some(4), "the saved row, not the cooked baseline");
    assert_eq!(logged(&log, tracing::Level::INFO, "loads the saved state"), 1);
    server.shutdown();
    executor.shutdown();
}

#[test]
fn an_anchored_saved_world_loads_its_pinned_region_and_answers_a_join_from_the_store() {
    let executor = executor(2);
    let content = Content::new();
    let saved = saved_grid(&executor, &content);
    let server = server(&executor, anchored([100]), Start::Loaded(saved), &content);
    assert_eq!(kept(&server), grid_ids());
    let all = |server: &Server| {
        let mut both = loaded(server);
        both.extend(kept(server));
        assert_eq!(both, grid_ids(), "every region is loaded or kept");
    };

    // The pinned region is requested at the first sync point and loads from the store at
    // the next step.
    let (server, out) = step(&executor, server, TickInput::default());
    assert!(out.failures.is_empty(), "{out:?}");
    assert_eq!(loads(&out.requests), BTreeSet::from([100]));
    assert_eq!(loaded(&server), BTreeSet::new());
    let (server, out) = step(&executor, server, TickInput::default());
    assert!(out.failures.is_empty(), "{out:?}");
    assert_eq!(loaded(&server), BTreeSet::from([100]));
    all(&server);

    // A join into a saved region: its loads are answered from the store at the next step.
    let (server, out) =
        step(&executor, server, session(1, SessionEvent::Join { region: RegionId(124) }));
    assert!(out.failures.is_empty(), "{out:?}");
    let near: BTreeSet<u64> = [112, 113, 114, 117, 118, 119, 122, 123, 124].into();
    assert_eq!(loads(&out.requests), near);
    all(&server);
    let (mut server, out) = step(&executor, server, TickInput::default());
    assert!(out.failures.is_empty(), "{out:?}");
    let mut expected = near.clone();
    expected.insert(100);
    assert_eq!(loaded(&server), expected);
    assert!(!server.kept().contains_key(&RegionId(124)));
    all(&server);
    for _ in 0..5 {
        let out;
        (server, out) = step(&executor, server, TickInput::default());
        assert!(out.failures.is_empty(), "{out:?}");
    }
    let (server, saved) = save(&executor, server);
    assert_eq!(ids(&saved), grid_ids(), "a save after K ticks holds every region");
    server.shutdown();
    executor.shutdown();
}

#[test]
fn join_then_leave_keeps_every_region_loaded_and_unloaded_in_one_step() {
    let executor = executor(2);
    let content = Content::new();
    let saved = saved_grid(&executor, &content);
    let server = server(&executor, anchored([]), Start::Loaded(saved), &content);
    // The join's anchor wants all 25 (streaming radius 2 around the centre); the store
    // answers every load.
    let (server, out) =
        step(&executor, server, session(1, SessionEvent::Join { region: RegionId(112) }));
    assert!(out.failures.is_empty(), "{out:?}");
    assert_eq!(loads(&out.requests), grid_ids());
    assert_eq!(loaded(&server), BTreeSet::new());
    // The leave in the step that loads the 25 bundles: each is accepted and unloads again.
    let (server, out) = step(&executor, server, session(1, SessionEvent::Leave));
    assert!(out.failures.is_empty(), "every bundle is accepted: {out:?}");
    assert_eq!(loaded(&server), BTreeSet::new());
    assert_eq!(kept(&server), grid_ids(), "reconciled against the world, not the frame");
    for region in grid_ids() {
        assert!(server.baseline(RegionId(region)).is_none());
    }
    let (server, saved) = save(&executor, server);
    assert_eq!(ids(&saved), grid_ids());
    server.shutdown();
    executor.shutdown();
}

/// `region`'s bundle with a resource no registry knows, which the sync point refuses.
fn refused_bundle(content: &Content, region: u64) -> lactose_sim::RegionBundle {
    let mut bundle = content.bundle(RegionId(region), RegionMode::Simulated);
    let component = ComponentKey::from_name("proof::unknown");
    bundle.resources.push(ComponentData { component, bytes: Vec::new() });
    bundle
}

#[test]
fn a_directed_reload_refused_keeps_the_export_and_one_accepted_loads() {
    let executor = executor(2);
    let content = Content::new();
    let server = server(&executor, WorldConfig::headless(), Start::Fresh, &content);
    let (mut server, _) = step(&executor, server, load(&content, [1, 2]));
    let one = spawn_counter(&mut server, 1);
    let two = spawn_counter(&mut server, 2);
    let (server, _) = step(&executor, server, TickInput::default());
    let world = server.simulation().world();
    let rows = world.export_region(RegionId(1)).expect("the export");
    let export = RegionExport { region: RegionId(1), rows };
    let baseline = server.baseline(RegionId(1)).expect("loaded");
    let expected =
        RegionRecords::from_export(&export, baseline, server.schemas()).expect("the diff");

    // Unloaded and reloaded in one frame: region 1's reload is refused, region 2's
    // accepted.
    let mut frame = unload(1);
    frame.unloaded.insert(RegionId(2));
    frame.loaded.insert(RegionId(1), refused_bundle(&content, 1));
    let reload = content.bundle(RegionId(2), RegionMode::Simulated);
    let reloaded_with = Arc::clone(&reload.baseline);
    frame.loaded.insert(RegionId(2), reload);
    let (server, out) = step(&executor, server, frame);
    assert_eq!(out.failures.loads.len(), 1, "{out:?}");
    assert_eq!(out.failures.loads[0].region, RegionId(1));

    assert_eq!(kept(&server), BTreeSet::from([1]));
    assert_eq!(server.kept()[&RegionId(1)].records, expected, "the export's records");
    assert!(server.baseline(RegionId(1)).is_none());
    assert_eq!(loaded(&server), BTreeSet::from([2]));
    assert!(Arc::ptr_eq(server.baseline(RegionId(2)).expect("loaded"), &reloaded_with));
    assert_eq!(counter(&server, two), None, "the accepted reload is the cooked bundle");
    assert_eq!(counter(&server, one), None);
    server.shutdown();
    executor.shutdown();
}

#[test]
fn an_unload_whose_diff_fails_stops_the_run_without_the_server() {
    let log = capture();
    let executor = executor(2);
    let content = Content::new();
    let mut server = server(&executor, WorldConfig::headless(), Start::Fresh, &content);
    let config = RegionConfig { id: RegionId(9), seed: 9, mode: RegionMode::Simulated };
    server.simulation_mut().world_mut().create_region(config).expect("the region");
    // A save point refuses it and hands the server back.
    let error = executor.block_on(server.save_point(&executor)).err().expect("no baseline");
    assert_eq!(error.error.kind(), ErrorKind::NotFound);
    let server = error.server.expect("a save point keeps the server");
    let error = try_step(&executor, server, unload(9)).err().expect("the diff fails");
    assert_eq!(error.error.kind(), ErrorKind::NotFound);
    assert!(error.server.is_none(), "a server missing a region is never handed back");
    assert_eq!(logged(&log, tracing::Level::ERROR, "an error stops the run"), 2);
    executor.shutdown();
}
