//! A save's sections against each other and its header, on the server's own paths:
//! `Server::encode_save` refuses a save its next start's `load` would refuse, before any
//! byte is written, and `load` refuses a cooked entity two regions would both hold, which
//! only the baselines show, and a prediction key two sections bind to two `NetId`s
//! (`README.md`, "Saves and loads").

use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::sync::Arc;

use lactose_app::{
    Cooked, SavedRegion, SavedWorld, Server, Start, WorldConfig, build_server, load as app_load,
};
use lactose_asset_api::{AssetSet, PinnedTable, VersionTable};
use lactose_bundle::{Baseline, BaselineRow, ComponentData};
use lactose_core::{Error, ErrorKind, NetId, PlayerId, PredictionKey, Result, StableKey, Tick};
use lactose_ecs::{Executor, RegionId, RegionMode};
use lactose_persist::{Container, ContainerHeader, SchemaTable};
use lactose_reflect::{Reflect, bytemuck};
use lactose_sim::{RegionBundle, TickInput};

use crate::common::{Content, Counter, GAME, executor, load, server, step, table_of};

/// The save of `server` as `Headless::save` encodes it.
fn encoded(executor: &Executor, server: Server) -> (Server, Result<Vec<u8>>) {
    let (server, saved) = executor.block_on(server.save_point(executor)).expect("the save point");
    let bytes = executor.block_on(server.encode_save(executor, saved));
    (server, bytes)
}

/// D142: a direct spawn under a runtime ID at or above a server world's counter moves the
/// counter past it, so the server's next save holds the counter above it, encodes, and a
/// fresh process started from it allocates above it.
#[test]
fn a_direct_runtime_spawn_above_the_counter_saves_and_loads() {
    let executor = executor(2);
    let content = Content::new();
    let fresh = server(&executor, WorldConfig::headless(), Start::Fresh, &content);
    let (mut first, out) = step(&executor, fresh, load(&content, [1]));
    assert!(out.failures.is_empty(), "{out:?}");

    let world = first.simulation_mut().world_mut();
    let direct = NetId::runtime(NonZeroU64::new(1000).unwrap()).unwrap();
    world.spawn(StableKey::Net(direct), RegionId(1), (Counter(0),)).expect("a direct spawn");
    assert_eq!(world.next_net_id().get(), 1001);
    let (first, _) = step(&executor, first, TickInput::default());
    let (first, saved) = encoded(&executor, first);
    let bytes = saved.expect("the save encodes");
    first.shutdown();

    let cooked = Arc::clone(&content) as Arc<dyn Cooked>;
    let loaded = app_load(&GAME, &bytes, &*cooked).expect("the save loads");
    assert_eq!(loaded.next_net_id.get(), 1001);
    let resumed = server(&executor, WorldConfig::headless(), Start::Loaded(loaded), &content);
    let (mut resumed, out) = step(&executor, resumed, load(&content, [1]));
    assert!(out.failures.is_empty(), "{out:?}");
    let world = resumed.simulation_mut().world_mut();
    assert!(world.contains(StableKey::Net(direct)), "the direct spawn was saved");
    assert_eq!(world.allocate_net_id().unwrap().to_bits(), 1001);
    resumed.shutdown();
    executor.shutdown();
}

/// Cooked content with a baseline of its own per region: region 1's holds the cooked
/// entity [`home_row`], region 2's is empty. Tables and pins are [`Content`]'s.
struct Homes {
    content: Arc<Content>,
    by_region: BTreeMap<RegionId, Arc<Baseline>>,
}

/// The cooked entity of region 1.
fn home_row() -> NetId {
    NetId::cooked(1_001)
}

impl Homes {
    fn new() -> Arc<Self> {
        let types = GAME.types().expect("the types");
        let schemas = SchemaTable::from_registry(types.registry()).expect("the table");
        let counter = schemas.schema(Counter::KEY).expect("a saved type").clone();
        let bytes = bytemuck::bytes_of(&Counter(5)).to_vec();
        let row = BaselineRow {
            id: home_row(),
            components: vec![ComponentData { component: Counter::KEY, bytes }],
        };
        let home = Baseline::from_rows(AssetSet::new(), [counter], [row]).expect("a baseline");
        let empty = Baseline::from_rows(AssetSet::new(), [], []).expect("empty");
        let by_region =
            BTreeMap::from([(RegionId(1), Arc::new(home)), (RegionId(2), Arc::new(empty))]);
        Arc::new(Self { content: Content::new(), by_region })
    }

    /// A frame loading regions 1 and 2 fresh, region 1 in `home`.
    fn load(&self, home: RegionMode) -> TickInput {
        let mut frame = TickInput::default();
        for (&region, baseline) in &self.by_region {
            let mode = if region == RegionId(1) { home } else { RegionMode::Simulated };
            let bundle = RegionBundle {
                seed: region.0,
                mode,
                baseline: Arc::clone(baseline),
                overlay: None,
                resources: Vec::new(),
                transient_resources: Vec::new(),
                assets: Some(self.pin(&table_of(region)).expect("published")),
            };
            frame.loaded.insert(region, bundle);
        }
        frame
    }
}

impl Cooked for Homes {
    fn baseline(&self, content_hash: u64) -> Result<Arc<Baseline>> {
        let found = self.by_region.values().find(|b| b.content_hash() == content_hash);
        found.cloned().ok_or_else(|| Error::new(ErrorKind::NotFound, "no baseline has that hash"))
    }

    fn table(&self, region: RegionId) -> Option<VersionTable> {
        self.content.table(region)
    }

    fn pin(&self, table: &VersionTable) -> Result<PinnedTable> {
        self.content.pin(table)
    }
}

/// The decoded save of a server over `homes` that loaded region 1 in `home` and ran `play`.
fn played(
    executor: &Executor,
    homes: &Arc<Homes>,
    home: RegionMode,
    play: impl FnOnce(&mut Server),
) -> Container {
    let cooked = Arc::clone(homes) as Arc<dyn Cooked>;
    let fresh = build_server(executor, &GAME, WorldConfig::headless(), Start::Fresh, cooked)
        .expect("the server builds");
    let (mut server, out) = step(executor, fresh, homes.load(home));
    assert!(out.failures.is_empty(), "{out:?}");
    play(&mut server);
    let (server, _) = step(executor, server, TickInput::default());
    let (server, bytes) = encoded(executor, server);
    server.shutdown();
    Container::decode(&bytes.expect("the server's own save")).expect("it decodes")
}

/// A container of region 1's section from `home` and region 2's from `away`, under
/// `away`'s header, assembled as a splice is: no check across the sections.
fn spliced(home: &Container, away: &Container) -> Vec<u8> {
    let header: &ContainerHeader = away.header();
    let sections = [home.region(RegionId(1)), away.region(RegionId(2))]
        .into_iter()
        .map(|records| records.expect("a section").encode_section().expect("encodes"))
        .collect();
    Container::assemble(header, sections).expect("assembles")
}

#[test]
fn a_cooked_entity_spawned_elsewhere_while_still_a_row_at_home_is_refused_at_load() {
    let executor = executor(2);
    let homes = Homes::new();
    let cooked = Arc::clone(&homes) as Arc<dyn Cooked>;
    let moved = played(&executor, &homes, RegionMode::Simulated, |server| {
        let world = server.simulation_mut().world_mut();
        world.move_to_region(StableKey::Net(home_row()), RegionId(2)).expect("the move");
    });
    // The play the move happened in destroys the row at home: its save loads.
    app_load(&GAME, &moved.encode().expect("encodes"), &*cooked).expect("the mover's save");

    // Region 1 from a play that never moved the entity: frozen, so its section has no
    // record of the row, which only region 1's baseline shows; simulated, so the counter
    // system changed the row and the sections alone show it.
    let id = format!("NetId {:#x}", home_row().to_bits());
    for (home, shown_by) in [
        (RegionMode::Frozen, "region 1's baseline holds as a row its section does not destroy"),
        (RegionMode::Simulated, "which the section of region 1 holds as a row of its baseline"),
    ] {
        let stay = played(&executor, &homes, home, |_| {});
        let in_home = stay.region(RegionId(1)).expect("region 1");
        assert_eq!(in_home.overlay.is_none(), home == RegionMode::Frozen);
        app_load(&GAME, &stay.encode().expect("encodes"), &*cooked).expect("the stayer's save");
        let error = app_load(&GAME, &spliced(&stay, &moved), &*cooked).expect_err("refused");
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
        let spawns = format!("region 2 spawns {id}");
        for named in [spawns.as_str(), shown_by] {
            assert!(error.message().contains(named), "`{error}` names `{named}`");
        }
    }
    executor.shutdown();
}

/// D142: no world's export binds one prediction key to two `NetId`s, so a splice whose
/// sections do is refused before any world exists, by `load` and by `build_server` alike.
#[test]
fn a_save_whose_sections_bind_one_origin_twice_fails_at_load() {
    let executor = executor(2);
    let homes = Homes::new();
    let cooked = Arc::clone(&homes) as Arc<dyn Cooked>;
    let key = PredictionKey { player: PlayerId(3), input_tick: Tick(2), ordinal: 0 };
    let bound_in = |region: u64, skip: u32| {
        let mut id = None;
        let played = played(&executor, &homes, RegionMode::Simulated, |server| {
            let world = server.simulation_mut().world_mut();
            for _ in 0..skip {
                world.allocate_net_id().expect("a net id");
            }
            let net = world.allocate_net_id().expect("a net id");
            let predicted = StableKey::Predicted(key);
            world.spawn(predicted, RegionId(region), (Counter(0),)).expect("the spawn");
            world.bind(key, net).expect("the bind");
            id = Some(net);
        });
        (played, id.expect("bound"))
    };
    let (home, first) = bound_in(1, 0);
    let (away, second) = bound_in(2, 1);
    assert_ne!(first, second);
    app_load(&GAME, &home.encode().expect("encodes"), &*cooked).expect("one bind loads");
    app_load(&GAME, &away.encode().expect("encodes"), &*cooked).expect("one bind loads");

    let named =
        [format!("{key:?}"), format!("{first:?} in region 1"), format!("{second:?} in region 2")];
    let refused = |error: Error| {
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
        for name in &named {
            assert!(error.message().contains(name.as_str()), "`{error}` names `{name}`");
        }
    };
    refused(app_load(&GAME, &spliced(&home, &away), &*cooked).expect_err("refused"));

    // The same sections as a saved world handed to the build.
    let regions = [home.region(RegionId(1)), away.region(RegionId(2))].into_iter().map(|records| {
        let records = records.expect("a section").clone();
        let baseline = homes.baseline(records.baseline_hash).expect("a cooked baseline");
        (records.region, Arc::new(SavedRegion { records, baseline }))
    });
    let header = away.header();
    let saved = SavedWorld {
        tick: header.tick,
        next_net_id: header.next_net_id,
        regions: regions.collect(),
    };
    let build =
        build_server(&executor, &GAME, WorldConfig::headless(), Start::Loaded(saved), cooked);
    refused(build.err().expect("the build fails"));
    executor.shutdown();
}
