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
    StagedFrame, StagedInbox, StagedSpawn, StagingFrame, Tombstone,
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
#[derive(Default)]
pub struct ClientHook {
    pub skipped: Skipped,
}

impl StagedState for ClientHook {
    fn apply(&mut self, world: &mut World, tick: Tick) -> Result<()> {
        while let Some(frame) =
            world.service_mut::<StagedInbox>().expect("setup inserted it").pop_through(tick)
        {
            apply_frame(world, &frame, &self.skipped);
        }
        Ok(())
    }
}

pub fn apply_frame(world: &mut World, frame: &StagedFrame, skipped: &Skipped) {
    let skip = |what: &'static str, key: StableKey, error: Error| {
        skipped.lock().expect("never poisoned").push((what, key, error.kind()));
    };
    for (&net, spawn) in &frame.spawns {
        let key = spawn.key(net);
        if let Err(error) = apply_spawn(world, key, net, spawn) {
            skip("spawn", key, error);
        }
    }
    for (&key, &region) in &frame.moves {
        if world.contains(key)
            && world.region_of(key) != Some(region)
            && let Err(error) = world.move_to_region(key, region)
        {
            skip("move", key, error);
        }
    }
    for &(key, component) in &frame.removed {
        if world.contains(key)
            && let Err(error) = world.remove_erased(key, component)
        {
            skip("remove", key, error);
        }
    }
    for row in frame.rows() {
        if world.contains(row.key) {
            let data = ComponentData { component: row.component, bytes: row.bytes.to_vec() };
            if let Err(error) = world.insert_erased(row.key, &data) {
                skip("row", row.key, error);
            }
        }
    }
    for (&key, Tombstone { .. }) in &frame.despawns {
        if world.contains(key)
            && let Err(error) = world.despawn(key)
        {
            skip("tombstone", key, error);
        }
    }
}

fn apply_spawn(world: &mut World, key: StableKey, net: NetId, spawn: &StagedSpawn) -> Result<()> {
    if world.contains(key) {
        for data in &spawn.components {
            world.insert_erased(key, data)?;
        }
        match spawn.slot {
            Some(slot) => {
                world.insert_erased(key, &data(RemoteSlot { slot }))?;
            }
            None => {
                world.remove::<RemoteSlot>(key)?;
            }
        }
        return Ok(());
    }
    world.spawn_erased(key, spawn.region, &spawn.components)?;
    if let Some(origin) = spawn.origin {
        world.bind(origin, net)?;
    }
    if let Some(slot) = spawn.slot {
        world.insert(key, RemoteSlot { slot })?;
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

// ---- The packer: gathered frames into staged frames for one client that sees all ----

/// A stand-in for `lactose-net`'s packer for one client to whom every region is relevant,
/// at full precision: its mirror of what the client holds, built from gathered frames
/// alone, and the slots it allocated.
pub struct Packer {
    pub baseline: Arc<Baseline>,
    mirror: BTreeMap<StableKey, (RegionId, BTreeMap<ComponentKey, Vec<u8>>)>,
    next_slot: u32,
}

impl Packer {
    pub fn new(baseline: Arc<Baseline>) -> Self {
        Self { baseline, mirror: BTreeMap::new(), next_slot: 0 }
    }

    fn spawn_of(&mut self, key: StableKey) -> (NetId, StagedSpawn) {
        let StableKey::Net(net) = key else { panic!("the scene spawns `Net` keys") };
        let (region, components) = &self.mirror[&key];
        self.next_slot += 1;
        let spawn = StagedSpawn {
            origin: None,
            region: *region,
            slot: Some(slot(self.next_slot)),
            components: components
                .iter()
                .map(|(&component, bytes)| ComponentData { component, bytes: bytes.clone() })
                .collect(),
        };
        (net, spawn)
    }

    /// The staged frame for `gathered`; `regions` are the server's regions, all relevant.
    pub fn pack(&mut self, gathered: &StagingFrame, regions: &[RegionId]) -> StagedFrame {
        let known: BTreeMap<StableKey, RegionId> =
            self.mirror.iter().map(|(&key, entity)| (key, entity.0)).collect();
        if gathered.full {
            self.mirror.clear();
        }
        let mut spawned = BTreeSet::new();
        let mut moved = BTreeSet::new();
        let mut removed = BTreeSet::new();
        for entry in gathered.structural() {
            match entry.change {
                StructuralChange::Spawned { key, region } => {
                    self.mirror.insert(key, (region, BTreeMap::new()));
                    spawned.insert(key);
                }
                StructuralChange::Despawned { key, .. } => {
                    self.mirror.remove(&key);
                }
                StructuralChange::Moved { key, to, .. } => {
                    if let Some(entity) = self.mirror.get_mut(&key) {
                        entity.0 = to;
                        moved.insert(key);
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

        let mut frame = if gathered.full {
            let mut frame = StagedFrame::resync(gathered.tick);
            for &region in regions {
                frame
                    .regions
                    .insert(region, RegionChange::Relevant(relevance(region, &self.baseline)));
            }
            let keys: Vec<StableKey> = self.mirror.keys().copied().collect();
            for key in keys {
                let (net, spawn) = self.spawn_of(key);
                frame.spawns.insert(net, spawn);
            }
            frame.time_scale = Some(lactose_sim::TimeScale::REAL_TIME);
            frame.acked_input = Some(gathered.tick);
            return frame;
        } else {
            StagedFrame::new(gathered.tick)
        };
        for (&key, &region) in &known {
            if !self.mirror.contains_key(&key) {
                frame.despawns.insert(key, Tombstone { region, tick: gathered.tick });
            }
        }
        let fresh: Vec<StableKey> = spawned
            .iter()
            .copied()
            .filter(|key| self.mirror.contains_key(key) && !known.contains_key(key))
            .collect();
        for &key in &fresh {
            let (net, spawn) = self.spawn_of(key);
            frame.spawns.insert(net, spawn);
        }
        for key in moved {
            if known.contains_key(&key)
                && let Some(entity) = self.mirror.get(&key)
            {
                frame.moves.insert(key, entity.0);
            }
        }
        for (key, component) in removed {
            if known.contains_key(&key) && self.mirror.contains_key(&key) {
                frame.removed.insert((key, component));
            }
        }
        for row in gathered.rows() {
            if known.contains_key(&row.key) && self.mirror.contains_key(&row.key) {
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
