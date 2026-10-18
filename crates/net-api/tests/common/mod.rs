//! What the integration tests share: a game with replicated and unreplicated components
//! and a server system that writes them, the server and client builds `app` writes, the
//! README's client hook written through the erased writes, the README's fold, a packer
//! that turns gathered frames into staged frames for one client that sees everything, and
//! the helpers.

#![allow(dead_code, reason = "each test file uses part of the shared module")]

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};

use lactose_asset_api::AssetSet;
use lactose_bundle::{Baseline, BaselineRow, ComponentData, ComponentSchema, FieldLayout, Overlay};
use lactose_core::{ComponentKey, Error, ErrorKind, NetId, Result, StableKey, Tick, xxh64};
use lactose_ecs::{
    Executor, LayoutConfig, Mut, RegionConfig, RegionId, RegionMode, StructuralChange, World,
    WorldKind, system,
};
use lactose_net_api::{
    GatherState, InterpolationSlot, OverlayPayload, RegionChange, RegionRelevance, RemoteSlot,
    StagedFrame, StagedInbox, StagedSpawn, StagingFrame, Tombstone, TombstoneCause,
};
use lactose_reflect::{Reflect, Registry, component};
use lactose_sim::harness::{ChunkCapacity, layout_mode};
use lactose_sim::{
    RegionControl, Registration, Simulation, SimulationBuilder, Stage, StagedState, TickInput,
    TypeTable,
};

// ---- The game ----

/// Replicated, written every tick by `Mover`.
#[component(key = "test::position", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct Position {
    #[field(id = 1)]
    pub x: i32,
    #[field(id = 2)]
    pub y: i32,
}

/// Replicated, written by tests between runs.
#[component(key = "test::health", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct Health(#[field(id = 1)] pub i32);

/// Replicated, added and removed by tests between runs.
#[component(key = "test::status", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct Status(#[field(id = 1)] pub u32);

/// Replicated and zero-sized.
#[component(key = "test::marked", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct Marked;

/// Not replicated: drives `Position` on the server and never reaches a client.
#[component(key = "test::mover", persistent)]
#[derive(PartialEq, Debug)]
pub struct Mover(#[field(id = 1)] pub i32);

/// Not replicated, written every tick: never gathered.
#[component(key = "test::secret", persistent)]
#[derive(PartialEq, Debug)]
pub struct Secret(#[field(id = 1)] pub i32);

#[system(name = "test::walk", worlds = [server])]
pub fn walk(mover: &Mover, position: Mut<Position>, secret: Option<Mut<Secret>>) {
    position.x += mover.0;
    if let Some(mut secret) = secret {
        secret.0 += 1;
    }
}

pub fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<Position>()?;
    types.register::<Health>()?;
    types.register::<Status>()?;
    types.register::<Marked>()?;
    types.register::<Mover>()?;
    types.register::<Secret>()?;
    lactose_net_api::register_types(types)
}

pub fn types() -> TypeTable {
    let mut types = TypeTable::new();
    register_types(&mut types).expect("the game registers");
    types
}

pub fn register_game(registration: &mut Registration) {
    registration.stage(Stage::Simulation).add(&WALK);
}

/// The replicated component keys, ascending.
pub fn replicated() -> Vec<ComponentKey> {
    let mut keys = vec![Position::KEY, Health::KEY, Status::KEY, Marked::KEY];
    keys.sort();
    keys
}

// ---- Keys, regions, bytes ----

pub const REGION: RegionId = RegionId(1);
pub const OTHER_REGION: RegionId = RegionId(2);
pub const THIRD_REGION: RegionId = RegionId(3);

pub fn net(n: u64) -> NetId {
    NetId::runtime(NonZeroU64::new(n).expect("non-zero")).expect("a runtime counter")
}

pub fn key(n: u64) -> StableKey {
    StableKey::Net(net(n))
}

pub fn data<T: Reflect>(value: T) -> ComponentData {
    ComponentData { component: T::KEY, bytes: lactose_reflect::bytemuck::bytes_of(&value).to_vec() }
}

pub fn bytes<T: Reflect>(value: &T) -> &[u8] {
    lactose_reflect::bytemuck::bytes_of(value)
}

pub fn slot(index: u32) -> InterpolationSlot {
    InterpolationSlot { index, generation: 1 }
}

pub fn schemas(registry: &Registry) -> Vec<ComponentSchema> {
    registry
        .iter()
        .map(|info| {
            let fields = info.layout().fields().iter().filter_map(|field| {
                Some(FieldLayout {
                    id: field.id()?,
                    offset: u32::try_from(field.offset()).expect("small"),
                    size: u32::try_from(field.size()).expect("small"),
                })
            });
            let size = u32::try_from(info.layout().size()).expect("small");
            ComponentSchema::new(info.key(), info.version(), size, fields).expect("a schema")
        })
        .collect()
}

/// A cooked baseline of the game's schemas holding `rows`.
pub fn baseline(rows: Vec<BaselineRow>) -> Arc<Baseline> {
    Arc::new(
        Baseline::from_rows(AssetSet::new(), schemas(types().registry()), rows)
            .expect("a baseline"),
    )
}

pub fn seed_of(region: RegionId) -> u64 {
    region.0 + 100
}

/// The message of a region whose state is its cooked baseline.
pub fn relevance(region: RegionId, baseline: &Baseline) -> RegionRelevance {
    RegionRelevance {
        region,
        seed: seed_of(region),
        baseline_hash: baseline.content_hash(),
        overlay: OverlayPayload::Cooked,
        resources: Vec::new(),
        transient_resources: Vec::new(),
        assets: None,
    }
}

// ---- Execution ----

pub fn executor(workers: u32) -> Executor {
    Executor::builder().frame_workers(workers).background_workers(1).build().expect("an executor")
}

pub fn step(executor: &Executor, simulation: Simulation, input: TickInput) -> Simulation {
    executor.block_on(simulation.step(executor, input)).expect("the step runs")
}

pub fn passes(executor: &Executor, simulation: Simulation) -> Simulation {
    executor.block_on(simulation.run_passes(executor)).expect("the passes run")
}

/// The gathered frame, taken as `app` takes it between runs.
pub fn take(simulation: &mut Simulation) -> StagingFrame {
    simulation.world_mut().service_mut::<GatherState>().expect("setup inserted it").take()
}

/// Every layout SOUR's layout mode runs, for three seeds, and the production layout.
pub fn layouts() -> Vec<LayoutConfig> {
    let mut layouts = vec![LayoutConfig::default()];
    for seed in [1, 2, 3] {
        for capacity in [
            ChunkCapacity::Rows(1),
            ChunkCapacity::Rows(7),
            ChunkCapacity::Rows(64),
            ChunkCapacity::Bytes(16 * 1024),
        ] {
            layouts.push(layout_mode(capacity, seed).expect("a layout the ECS honours"));
        }
    }
    layouts
}

// ---- The server ----

pub fn server_builder(layout: LayoutConfig) -> SimulationBuilder {
    SimulationBuilder::new(WorldKind::Server, types())
        .layout(layout)
        .regions(RegionControl::Directed)
        .setup(lactose_net_api::setup)
        .systems(register_game)
        .systems(lactose_net_api::register_systems)
}

pub fn server(layout: LayoutConfig) -> Simulation {
    server_builder(layout).build().expect("the server builds")
}

pub fn create_regions(world: &mut World, regions: &[RegionId]) {
    for &region in regions {
        let config =
            RegionConfig { id: region, seed: seed_of(region), mode: RegionMode::Simulated };
        world.create_region(config).expect("a new region");
    }
}

// ---- The client ----

/// One frame entry the hook skipped: what, for which key, and the error's kind.
pub type Skipped = Arc<Mutex<Vec<(&'static str, StableKey, ErrorKind)>>>;

/// `lactose-net`'s hook as the README states it, written through the erased writes:
/// spawns (upserts for live keys), moves, removals, rows, tombstones, in that order, each
/// failing entry logged into `skipped` and skipped.
/// How many times each path of the hook ran, by name: `spawn`, `upsert`, `bind`, `slot set`,
/// `slot removed`, `move`, `remove`, `row` and `tombstone`.
pub type Paths = Arc<Mutex<BTreeMap<&'static str, usize>>>;

#[derive(Default)]
pub struct ClientHook {
    pub skipped: Skipped,
    pub paths: Paths,
}

impl StagedState for ClientHook {
    fn apply(&mut self, world: &mut World, tick: Tick) -> Result<()> {
        while let Some(frame) =
            world.service_mut::<StagedInbox>().expect("setup inserted it").pop_through(tick)
        {
            apply_frame(world, &frame, &self.skipped, &self.paths);
        }
        Ok(())
    }
}

pub fn apply_frame(world: &mut World, frame: &StagedFrame, skipped: &Skipped, paths: &Paths) {
    let skip = |what: &'static str, key: StableKey, error: Error| {
        skipped.lock().expect("never poisoned").push((what, key, error.kind()));
    };
    let ran = |path: &'static str| {
        *paths.lock().expect("never poisoned").entry(path).or_default() += 1;
    };
    for (&net, spawn) in &frame.spawns {
        let key = spawn.key(net);
        if let Err(error) = apply_spawn(world, key, net, spawn, &ran) {
            skip("spawn", key, error);
        }
    }
    for (&key, &region) in &frame.moves {
        if world.contains(key) && world.region_of(key) != Some(region) {
            match world.move_to_region(key, region) {
                Ok(()) => ran("move"),
                Err(error) => skip("move", key, error),
            }
        }
    }
    for &(key, component) in &frame.removed {
        if world.contains(key) {
            match world.remove_erased(key, component) {
                Ok(_) => ran("remove"),
                Err(error) => skip("remove", key, error),
            }
        }
    }
    for row in frame.rows() {
        if world.contains(row.key) {
            let data = ComponentData { component: row.component, bytes: row.bytes.to_vec() };
            match world.insert_erased(row.key, &data) {
                Ok(_) => ran("row"),
                Err(error) => skip("row", row.key, error),
            }
        }
    }
    for (&key, tombstone) in &frame.despawns {
        // A `Left` tombstone of an entity a load of its region placed there is the
        // region's to settle.
        let loaded_there = tombstone.cause == TombstoneCause::Left
            && world.region_of(key) == Some(tombstone.region);
        if loaded_there {
            ran("tombstone skipped");
        } else if world.contains(key) {
            match world.despawn(key) {
                Ok(()) => ran("tombstone"),
                Err(error) => skip("tombstone", key, error),
            }
        }
    }
}

fn apply_spawn(
    world: &mut World,
    key: StableKey,
    net: NetId,
    spawn: &StagedSpawn,
    ran: &dyn Fn(&'static str),
) -> Result<()> {
    if world.contains(key) {
        for data in &spawn.components {
            world.insert_erased(key, data)?;
        }
        match spawn.slot {
            Some(slot) => {
                world.insert_erased(key, &data(RemoteSlot { slot }))?;
                ran("slot set");
            }
            None => {
                if world.remove::<RemoteSlot>(key)?.is_some() {
                    ran("slot removed");
                }
            }
        }
        ran("upsert");
        return Ok(());
    }
    world.spawn_erased(key, spawn.region, &spawn.components)?;
    ran("spawn");
    if let Some(origin) = spawn.origin {
        world.bind(origin, net)?;
        ran("bind");
    }
    if let Some(slot) = spawn.slot {
        world.insert(key, RemoteSlot { slot })?;
        ran("slot set");
    }
    Ok(())
}

pub fn client_builder(start: Tick, hook: ClientHook) -> SimulationBuilder {
    SimulationBuilder::new(WorldKind::Client, types())
        .tick(start)
        .regions(RegionControl::Directed)
        .setup(lactose_net_api::setup)
        .systems(register_game)
        .systems(lactose_net_api::register_systems)
        .staged_state(Box::new(hook))
}

/// A client world at `start` with the hook, and the hook's skip log.
pub fn client(start: Tick) -> (Simulation, Skipped) {
    let hook = ClientHook::default();
    let skipped = Arc::clone(&hook.skipped);
    (client_builder(start, hook).build().expect("the client builds"), skipped)
}

pub fn inbox(simulation: &mut Simulation) -> &mut StagedInbox {
    simulation.world_mut().service_mut::<StagedInbox>().expect("setup inserted it")
}

/// What a client has stored: baselines by content hash, overlays by digest.
#[derive(Default)]
pub struct Store {
    pub baselines: BTreeMap<u64, Arc<Baseline>>,
    pub overlays: BTreeMap<RegionId, Arc<Overlay>>,
}

impl Store {
    pub fn with(baseline: &Arc<Baseline>) -> Self {
        let mut store = Self::default();
        store.baselines.insert(baseline.content_hash(), Arc::clone(baseline));
        store
    }
}

/// The README's fold of one due frame into the step's `TickInput`.
pub fn fold_frame(
    input: &mut TickInput,
    frame: &StagedFrame,
    loaded: &[RegionId],
    store: &Store,
) -> Result<()> {
    if let Some(scale) = frame.time_scale {
        input.time_scale = Some(scale);
    }
    if frame.full {
        input.loaded.clear();
        input.assets.clear();
        input.unloaded.extend(loaded.iter().copied());
    }
    for (&region, change) in &frame.regions {
        match change {
            RegionChange::Relevant(relevance) => {
                let baseline =
                    store.baselines.get(&relevance.baseline_hash).cloned().ok_or_else(|| {
                        Error::new(ErrorKind::NotFound, "the baseline is not in the store")
                    })?;
                let cached = store.overlays.get(&region);
                input.loaded.insert(region, relevance.to_bundle(baseline, cached, None)?);
            }
            RegionChange::Irrelevant => {
                input.loaded.remove(&region);
                input.assets.remove(&region);
                input.unloaded.insert(region);
            }
            RegionChange::Assets(_) => {}
        }
    }
    Ok(())
}

/// `app` on a client between runs: `frames` pushed into the inbox, then the frames due at
/// the tick the next step runs folded into its `TickInput`. Returns the input and which
/// frames the inbox accepted.
pub fn between_runs(
    simulation: &mut Simulation,
    frames: impl IntoIterator<Item = StagedFrame>,
    store: &Store,
) -> (TickInput, Vec<bool>) {
    let accepted =
        frames.into_iter().map(|frame| inbox(simulation).push(Arc::new(frame))).collect();
    let loaded: Vec<RegionId> = simulation.world().regions().collect();
    let next = Tick(simulation.tick().0 + 1);
    let mut input = TickInput::default();
    let inbox = simulation.world().service::<StagedInbox>().expect("setup inserted it");
    for frame in inbox.due(next) {
        fold_frame(&mut input, frame, &loaded, store).expect("the fold");
    }
    (input, accepted)
}

// ---- Rows, as a world holds them ----

/// Every replicated row of `world`: (key, component) to (region, bytes).
pub fn replicated_rows(world: &World) -> BTreeMap<(StableKey, ComponentKey), (RegionId, Vec<u8>)> {
    let mut rows = BTreeMap::new();
    for component in replicated() {
        for column in world.columns_erased(component) {
            let size = column.element_size();
            for (row, (&key, &region)) in column.keys().iter().zip(column.regions()).enumerate() {
                rows.insert(
                    (key, component),
                    (region, column.bytes()[row * size..][..size].to_vec()),
                );
            }
        }
    }
    rows
}

/// A gathered frame as plain values, for comparisons and digests.
pub fn frame_rows(frame: &StagingFrame) -> Vec<(RegionId, StableKey, ComponentKey, Tick, Vec<u8>)> {
    frame
        .rows()
        .map(|row| (row.region, row.key, row.component, row.tick, row.bytes.to_vec()))
        .collect()
}

/// A digest of everything a gathered frame holds.
pub fn frame_digest(frame: &StagingFrame) -> u64 {
    let text =
        format!("{} {} {:?} {:?}", frame.tick.0, frame.full, frame_rows(frame), frame.structural());
    xxh64(text.as_bytes(), 0)
}

// ---- The packer: gathered frames into staged frames for one client ----

/// A stand-in for `lactose-net`'s packer for one client, at full precision: its mirror of
/// the server's replicated state, built from gathered frames alone; the regions relevant
/// to the client and those it was told of; the entities the client has claimed; and the
/// slots it allocated. The client holds the mirror's entities in the regions it was told
/// are relevant.
pub struct Packer {
    baselines: BTreeMap<RegionId, Arc<Baseline>>,
    mirror: BTreeMap<StableKey, (RegionId, BTreeMap<ComponentKey, Vec<u8>>)>,
    relevant: BTreeSet<RegionId>,
    told: BTreeSet<RegionId>,
    claimed: BTreeSet<StableKey>,
    claim_changes: BTreeSet<StableKey>,
    next_slot: u32,
}

impl Packer {
    /// A packer for a client to which `relevant` are relevant, each region's state its
    /// cooked baseline in `baselines`.
    pub fn new(baselines: BTreeMap<RegionId, Arc<Baseline>>, relevant: &[RegionId]) -> Self {
        Self {
            baselines,
            mirror: BTreeMap::new(),
            relevant: relevant.iter().copied().collect(),
            told: BTreeSet::new(),
            claimed: BTreeSet::new(),
            claim_changes: BTreeSet::new(),
            next_slot: 0,
        }
    }

    /// Makes `region` relevant or not, from the next frame packed.
    pub fn set_relevant(&mut self, region: RegionId, relevant: bool) {
        if relevant {
            self.relevant.insert(region);
        } else {
            self.relevant.remove(&region);
        }
    }

    /// The regions the client was told are relevant, by the last frame packed.
    pub fn told(&self) -> &BTreeSet<RegionId> {
        &self.told
    }

    /// The client claims `key`, or releases it, from the next frame packed.
    pub fn set_claimed(&mut self, key: StableKey, claimed: bool) {
        let changed = if claimed { self.claimed.insert(key) } else { self.claimed.remove(&key) };
        if changed {
            self.claim_changes.insert(key);
        }
    }

    /// Whether the client has claimed `key`.
    pub fn claimed(&self, key: StableKey) -> bool {
        self.claimed.contains(&key)
    }

    /// The entities the client holds after a frame: the mirror's, in the regions it knows.
    fn held(&self, regions: &BTreeSet<RegionId>) -> BTreeMap<StableKey, RegionId> {
        self.mirror
            .iter()
            .filter(|(_, entity)| regions.contains(&entity.0))
            .map(|(&key, entity)| (key, entity.0))
            .collect()
    }

    /// The spawn of `key`: under its `NetId`, with the origin the server bound for a
    /// predicted key, a fresh slot unless claimed, and every component the mirror holds.
    fn spawn_of(&mut self, key: StableKey, world: &World) -> (NetId, StagedSpawn) {
        let (net, origin) = match key {
            StableKey::Net(net) => (net, None),
            StableKey::Predicted(predicted) => {
                (world.alias(predicted).expect("the server bound it"), Some(predicted))
            }
            StableKey::Placeholder(_) => panic!("a gathered key is never a placeholder"),
        };
        let slot = if self.claimed.contains(&key) {
            None
        } else {
            self.next_slot += 1;
            Some(slot(self.next_slot))
        };
        let (region, components) = &self.mirror[&key];
        let spawn = StagedSpawn {
            origin,
            region: *region,
            slot,
            components: components
                .iter()
                .map(|(&component, bytes)| ComponentData { component, bytes: bytes.clone() })
                .collect(),
        };
        (net, spawn)
    }

    /// The staged frame for `gathered`, from the server world `world` it was gathered in.
    pub fn pack(&mut self, gathered: &StagingFrame, world: &World) -> StagedFrame {
        let before = self.held(&self.told);
        if gathered.full {
            self.mirror.clear();
        }
        let mut removed = BTreeSet::new();
        for entry in gathered.structural() {
            match entry.change {
                StructuralChange::Spawned { key, region } => {
                    self.mirror.insert(key, (region, BTreeMap::new()));
                }
                StructuralChange::Despawned { key, .. } => {
                    self.mirror.remove(&key);
                }
                StructuralChange::Moved { key, to, .. } => {
                    if let Some(entity) = self.mirror.get_mut(&key) {
                        entity.0 = to;
                    }
                }
                StructuralChange::Removed { key, component } => {
                    if let Some(entity) = self.mirror.get_mut(&key)
                        && entity.1.remove(&component).is_some()
                    {
                        removed.insert((key, component));
                    }
                }
                StructuralChange::Added { .. } => {}
            }
        }
        for row in gathered.rows() {
            let entity =
                self.mirror.entry(row.key).or_insert_with(|| (row.region, BTreeMap::new()));
            entity.0 = row.region;
            entity.1.insert(row.component, row.bytes.to_vec());
        }
        let unloaded: BTreeSet<RegionId> = self.told.difference(&self.relevant).copied().collect();
        let loaded: BTreeSet<RegionId> = self.relevant.difference(&self.told).copied().collect();
        self.told.clone_from(&self.relevant);
        let after = self.held(&self.told);

        if gathered.full {
            let mut frame = StagedFrame::resync(gathered.tick);
            for &region in &self.told {
                let message = relevance(region, &self.baselines[&region]);
                frame.regions.insert(region, RegionChange::Relevant(message));
            }
            for &key in after.keys() {
                let (net, spawn) = self.spawn_of(key, world);
                frame.spawns.insert(net, spawn);
            }
            frame.time_scale = Some(lactose_sim::TimeScale::REAL_TIME);
            frame.acked_input = Some(gathered.tick);
            self.claim_changes.clear();
            return frame;
        }

        let mut frame = StagedFrame::new(gathered.tick);
        for &region in &unloaded {
            frame.regions.insert(region, RegionChange::Irrelevant);
        }
        for &region in &loaded {
            let message = relevance(region, &self.baselines[&region]);
            frame.regions.insert(region, RegionChange::Relevant(message));
        }
        // What the client still holds when its hook runs: the step unloads first.
        let kept: BTreeMap<StableKey, RegionId> =
            before.into_iter().filter(|(_, region)| !unloaded.contains(region)).collect();
        for (&key, &region) in &kept {
            if !after.contains_key(&key) {
                let tombstone = match self.mirror.get(&key) {
                    Some(&(now, _)) => {
                        Tombstone { region: now, tick: gathered.tick, cause: TombstoneCause::Left }
                    }
                    None => {
                        Tombstone { region, tick: gathered.tick, cause: TombstoneCause::Destroyed }
                    }
                };
                frame.despawns.insert(key, tombstone);
            }
        }
        let claim_changes = std::mem::take(&mut self.claim_changes);
        for (&key, &region) in &after {
            let fresh = !kept.contains_key(&key);
            if fresh || claim_changes.contains(&key) {
                let (net, spawn) = self.spawn_of(key, world);
                frame.spawns.insert(net, spawn);
            }
            if !fresh && kept[&key] != region {
                frame.moves.insert(key, region);
            }
        }
        for (key, component) in removed {
            if kept.contains_key(&key) && after.contains_key(&key) {
                frame.removed.insert((key, component));
            }
        }
        for row in gathered.rows() {
            if kept.contains_key(&row.key) && after.contains_key(&row.key) {
                assert!(frame.push_row(row.key, row.component, row.tick, true, row.bytes));
            }
        }
        frame
    }
}

/// Ensures `result` failed with `kind`.
pub fn assert_kind<T: std::fmt::Debug>(result: Result<T>, kind: ErrorKind) {
    match result {
        Err(error) => assert_eq!(error.kind(), kind, "{error}"),
        Ok(value) => panic!("expected {kind:?}, got Ok({value:?})"),
    }
}
