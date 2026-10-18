//! What the replication tests share: a game with replicated and unreplicated components,
//! the server and client builds, the client hook and the fold of `lactose-net-api`'s
//! README (written as that crate's tests write them), and a model of `lactose-net`'s
//! packer and receiver for one client, built on this crate's types and held to the rules
//! of the README's "Handles and packets" and "Interest and the packer": datagrams over a
//! link that loses, delays, reorders and duplicates them, control messages over a reliable
//! ordered channel, both in logical time measured in server ticks.

#![allow(dead_code, reason = "each test file uses part of the shared module")]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};

use lactose_asset_api::AssetSet;
use lactose_bundle::{Baseline, ComponentData, ComponentSchema, FieldLayout};
use lactose_core::{ComponentKey, Error, ErrorKind, NetId, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{
    Executor, LayoutConfig, Mut, RegionConfig, RegionId, RegionMode, StructuralChange, World,
    WorldKind, system,
};
use lactose_math::{Quat, Scalar, Vec3};
use lactose_net_api::{
    GatherState, InterpolationSlot, OverlayPayload, RegionChange, RegionRelevance, RemoteSlot,
    StagedFrame, StagedInbox, StagedSpawn, StagingFrame, Tombstone, TombstoneCause,
};
use lactose_net_host::InterpolationBuffer;
use lactose_net_host::interest::{ClientInterest, SendBudget};
use lactose_net_host::interpolation::{InterpolationReader, InterpolationWriter, ReceivedState};
use lactose_net_host::packet::{
    AckField, ConnectionHandles, EntityHandle, PacketRecord, PacketSequence,
};
use lactose_net_host::transport::ConnectionId;
use lactose_reflect::{Reflect, Registry, component};
use lactose_sim::{
    RegionControl, Registration, Simulation, SimulationBuilder, Stage, StagedState, TickInput,
    TimeScale, TypeTable,
};

// ---- The game ----

/// Replicated, written every tick by `Mover`.
#[component(key = "replication::position", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct Position {
    #[field(id = 1)]
    pub x: i32,
    #[field(id = 2)]
    pub y: i32,
}

/// Replicated, written and removed by the scripts.
#[component(key = "replication::health", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct Health(#[field(id = 1)] pub i32);

/// Replicated, added and removed by the scripts.
#[component(key = "replication::status", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct Status(#[field(id = 1)] pub u32);

/// Replicated and zero-sized.
#[component(key = "replication::marked", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct Marked;

/// Not replicated: drives `Position` on the server.
#[component(key = "replication::mover", persistent)]
#[derive(PartialEq, Debug)]
pub struct Mover(#[field(id = 1)] pub i32);

#[system(name = "replication::walk", worlds = [server])]
pub fn walk(mover: &Mover, position: Mut<Position>) {
    position.x += mover.0;
}

pub fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<Position>()?;
    types.register::<Health>()?;
    types.register::<Status>()?;
    types.register::<Marked>()?;
    types.register::<Mover>()?;
    lactose_net_api::register_types(types)
}

pub fn types() -> TypeTable {
    let mut types = TypeTable::new();
    register_types(&mut types).expect("the game registers");
    types
}

fn register_game(registration: &mut Registration) {
    registration.stage(Stage::Simulation).add(&WALK);
}

/// The replicated component keys, ascending.
pub fn replicated() -> Vec<ComponentKey> {
    let mut keys = vec![Position::KEY, Health::KEY, Status::KEY, Marked::KEY];
    keys.sort();
    keys
}

/// Each registered component's size, which the receiver checks every row against.
pub fn sizes(registry: &Registry) -> BTreeMap<ComponentKey, usize> {
    registry.iter().map(|info| (info.key(), info.layout().size())).collect()
}

// ---- Keys, regions, bytes ----

pub const REGIONS: [RegionId; 4] = [RegionId(1), RegionId(2), RegionId(3), RegionId(4)];

pub fn net(n: u64) -> NetId {
    NetId::runtime(NonZeroU64::new(n).expect("non-zero")).expect("a runtime counter")
}

pub fn key(n: u64) -> StableKey {
    StableKey::Net(net(n))
}

pub fn net_of(key: StableKey) -> NetId {
    match key {
        StableKey::Net(net) => net,
        other => panic!("the model's keys are runtime net IDs, not {other:?}"),
    }
}

pub fn data<T: Reflect>(value: T) -> ComponentData {
    ComponentData { component: T::KEY, bytes: lactose_reflect::bytemuck::bytes_of(&value).to_vec() }
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

/// The cooked baseline every region shares unless a test cooks one: empty, so every
/// entity a client holds in it came from a spawn.
pub fn empty_baseline() -> Arc<Baseline> {
    Arc::new(
        Baseline::from_rows(AssetSet::new(), schemas(types().registry()), Vec::new())
            .expect("a baseline"),
    )
}

/// The cooked baseline of each region, which the server and the client both hold.
#[derive(Clone)]
pub struct Baselines {
    empty: Arc<Baseline>,
    cooked: BTreeMap<RegionId, Arc<Baseline>>,
}

impl Default for Baselines {
    fn default() -> Self {
        Self { empty: empty_baseline(), cooked: BTreeMap::new() }
    }
}

impl Baselines {
    pub fn of(&self, region: RegionId) -> &Arc<Baseline> {
        self.cooked.get(&region).unwrap_or(&self.empty)
    }
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

// ---- The builds ----

pub fn executor(workers: u32) -> Executor {
    Executor::builder().frame_workers(workers).background_workers(1).build().expect("an executor")
}

pub fn server_builder(layout: LayoutConfig) -> SimulationBuilder {
    SimulationBuilder::new(WorldKind::Server, types())
        .layout(layout)
        .regions(RegionControl::Directed)
        .setup(lactose_net_api::setup)
        .systems(register_game)
        .systems(lactose_net_api::register_systems)
}

/// A server with the four regions and `entities` entities spread over them: every one
/// with `Position` and `Health`, every third moving, every fifth `Marked`.
pub fn server(layout: LayoutConfig, entities: u64) -> Simulation {
    let mut simulation = server_builder(layout).build().expect("the server builds");
    let world = simulation.world_mut();
    for &region in &REGIONS {
        let config =
            RegionConfig { id: region, seed: seed_of(region), mode: RegionMode::Simulated };
        world.create_region(config).expect("a new region");
    }
    for n in 1..=entities {
        spawn_entity(world, n);
    }
    simulation
}

/// Spawns entity `n` as the population does.
pub fn spawn_entity(world: &mut World, n: u64) {
    let region = REGIONS[usize::try_from(n % 4).expect("small")];
    let at = i32::try_from(n).expect("small");
    world.spawn(key(n), region, (Position { x: at, y: -at }, Health(at))).expect("a spawn");
    if n.is_multiple_of(3) {
        world.insert(key(n), Mover(at % 5 - 2)).expect("insert");
    }
    if n.is_multiple_of(5) {
        world.insert(key(n), Marked).expect("insert");
    }
}

pub fn client_builder(start: Tick, layout: LayoutConfig, hook: ClientHook) -> SimulationBuilder {
    SimulationBuilder::new(WorldKind::Client, types())
        .tick(start)
        .layout(layout)
        .regions(RegionControl::Directed)
        .setup(lactose_net_api::setup)
        .systems(register_game)
        .systems(lactose_net_api::register_systems)
        .staged_state(Box::new(hook))
}

// ---- The client hook and the fold, as `lactose-net-api`'s README states them ----

/// One frame entry the hook skipped: what, for which key, and the error's kind.
pub type Skipped = Arc<Mutex<Vec<(&'static str, StableKey, ErrorKind)>>>;

/// `lactose-net`'s hook through the erased writes: spawns (upserts for live keys), moves,
/// removals, rows, tombstones, in that order, each failing entry logged and skipped.
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

fn apply_frame(world: &mut World, frame: &StagedFrame, skipped: &Skipped) {
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
    for (&key, tombstone) in &frame.despawns {
        // A Left tombstone of an entity its region holds again: the region was loaded
        // after the entity left, or its unload is still to come; either settles it.
        let settled = tombstone.cause == TombstoneCause::Left
            && world.region_of(key) == Some(tombstone.region);
        if world.contains(key)
            && !settled
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
            Some(slot) => world.insert_erased(key, &data(RemoteSlot { slot })).map(|_| ())?,
            None => world.remove::<RemoteSlot>(key).map(|_| ())?,
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

/// The fold of one due frame into the step's `TickInput`.
fn fold_frame(
    input: &mut TickInput,
    frame: &StagedFrame,
    loaded: &[RegionId],
    baselines: &Baselines,
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
                let baseline = baselines.of(region);
                if relevance.baseline_hash != baseline.content_hash() {
                    return Err(Error::new(ErrorKind::NotFound, "the baseline is not stored"));
                }
                input.loaded.insert(region, relevance.to_bundle(Arc::clone(baseline), None, None)?);
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

/// The replicated rows of `world` residing in `regions`.
pub fn rows_in(
    world: &World,
    regions: &BTreeSet<RegionId>,
) -> BTreeMap<(StableKey, ComponentKey), (RegionId, Vec<u8>)> {
    let mut rows = replicated_rows(world);
    rows.retain(|_, (region, _)| regions.contains(region));
    rows
}

// ---- Randomness and the links ----

/// A deterministic generator for the scripts and the links' choices.
pub struct Rng(pub u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(0x9E37_79B9_7F4A_7C15 ^ seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// How the links behave: the percentage of datagrams lost and duplicated, and the delay
/// of every message, datagram or control, in ticks.
#[derive(Clone, Copy, Debug)]
pub struct LinkConfig {
    pub loss: u64,
    pub duplicates: u64,
    pub min_delay: u32,
    pub max_delay: u32,
}

impl LinkConfig {
    pub const PERFECT: Self = Self { loss: 0, duplicates: 0, min_delay: 1, max_delay: 1 };
    pub const POOR: Self = Self { loss: 20, duplicates: 5, min_delay: 1, max_delay: 8 };

    fn delay(&self, rng: &mut Rng) -> u32 {
        let spread = u64::from(self.max_delay - self.min_delay) + 1;
        self.min_delay + u32::try_from(rng.below(spread)).expect("small")
    }
}

/// An unreliable leg: each datagram arrives after the delay, or never, or twice.
struct Leg<T> {
    in_flight: BTreeMap<(u32, u64), T>,
}

impl<T: Clone> Leg<T> {
    fn new() -> Self {
        Self { in_flight: BTreeMap::new() }
    }

    fn send(&mut self, now: u32, message: T, link: &LinkConfig, rng: &mut Rng) {
        if rng.chance(link.loss) {
            return;
        }
        let copies = if rng.chance(link.duplicates) { 2 } else { 1 };
        for _ in 0..copies {
            let at = now + link.delay(rng);
            self.in_flight.insert((at, rng.next()), message.clone());
        }
    }

    fn due(&mut self, now: u32) -> Vec<T> {
        let later = self.in_flight.split_off(&(now + 1, 0));
        std::mem::replace(&mut self.in_flight, later).into_values().collect()
    }
}

/// The reliable ordered channel: every message arrives, in the order sent.
struct Reliable<T> {
    queue: VecDeque<(u32, T)>,
}

impl<T> Reliable<T> {
    fn new() -> Self {
        Self { queue: VecDeque::new() }
    }

    fn send(&mut self, now: u32, message: T, link: &LinkConfig, rng: &mut Rng) {
        let earliest = self.queue.back().map_or(0, |&(at, _)| at);
        self.queue.push_back(((now + link.delay(rng)).max(earliest), message));
    }

    fn due(&mut self, now: u32) -> Vec<T> {
        let mut due = Vec::new();
        while self.queue.front().is_some_and(|&(at, _)| at <= now) {
            due.push(self.queue.pop_front().expect("checked").1);
        }
        due
    }
}

// ---- The wire ----

/// An entity sent whole: its handle and key, its region, whether the receiving player has
/// it claimed, and every replicated component.
#[derive(Clone, Debug)]
pub struct WireSpawn {
    pub handle: EntityHandle,
    pub key: StableKey,
    pub region: RegionId,
    pub claimed: bool,
    pub components: Vec<ComponentData>,
}

/// One row by handle.
#[derive(Clone, Debug)]
pub struct WireRow {
    pub handle: EntityHandle,
    pub component: ComponentKey,
    pub tick: Tick,
    pub exact: bool,
    pub bytes: Vec<u8>,
}

/// One server datagram for the client.
#[derive(Clone, Debug)]
pub struct Packet {
    pub sequence: PacketSequence,
    pub tick: Tick,
    pub spawns: Vec<WireSpawn>,
    pub moves: Vec<(EntityHandle, RegionId)>,
    pub removed: Vec<(EntityHandle, ComponentKey)>,
    pub rows: Vec<WireRow>,
    pub tombstones: Vec<(EntityHandle, Tombstone)>,
}

impl Packet {
    pub fn new(sequence: PacketSequence, tick: Tick) -> Self {
        Self {
            sequence,
            tick,
            spawns: Vec::new(),
            moves: Vec::new(),
            removed: Vec::new(),
            rows: Vec::new(),
            tombstones: Vec::new(),
        }
    }
}

/// A message on the reliable channel to the client.
#[derive(Clone, Debug)]
pub enum Control {
    /// A region entering or leaving the client's set, stamped with the server tick it
    /// applies from; `id` is what the delivery acknowledgement names.
    Region { id: u64, tick: Tick, region: RegionId, change: RegionChange },
    /// The full relevance-set snapshot of a join or a resync.
    Full { tick: Tick, regions: Vec<(RegionId, RegionRelevance)>, spawns: Vec<WireSpawn> },
}

/// A message on the reliable channel to the server.
#[derive(Clone, Copy, Debug)]
pub enum Upward {
    /// The reliable channel delivered the region change `id`.
    Delivered(u64),
    /// The client asks for a full frame.
    Resync,
}

// ---- The packer ----

/// One entity of the packer's mirror.
#[derive(Clone, Debug)]
pub struct MirrorEntity {
    pub region: RegionId,
    /// Each replicated row: the tick it last changed by, and its bytes.
    pub rows: BTreeMap<ComponentKey, (Tick, Vec<u8>)>,
    /// Each replicated component removed since it last had a row, with the tick.
    pub removed: BTreeMap<ComponentKey, Tick>,
}

impl MirrorEntity {
    fn new(region: RegionId) -> Self {
        Self { region, rows: BTreeMap::new(), removed: BTreeMap::new() }
    }
}

/// `lactose-net`'s packer for one client: the mirror, the client's interest, the regions
/// relevant to it and those it may name, and the claims.
pub struct ServerModel {
    pub mirror: BTreeMap<StableKey, MirrorEntity>,
    pub interest: ClientInterest,
    /// The regions relevant to the client.
    pub relevant: BTreeSet<RegionId>,
    /// The relevant regions whose relevance the reliable channel delivered: the packer
    /// names an entity of a region only once the region is here.
    pub named: BTreeSet<RegionId>,
    /// The relevance message each relevant, unnamed region waits on.
    pending: BTreeMap<RegionId, u64>,
    next_message: u64,
    pub claimed: BTreeSet<StableKey>,
    sequence: PacketSequence,
    pub joined: bool,
    baselines: Baselines,
}

impl ServerModel {
    pub fn new(relevant: &[RegionId], baselines: Baselines) -> Self {
        Self {
            mirror: BTreeMap::new(),
            interest: ClientInterest::new(PlayerId(1), ConnectionId(1), SendBudget::default()),
            relevant: relevant.iter().copied().collect(),
            named: BTreeSet::new(),
            pending: BTreeMap::new(),
            next_message: 0,
            claimed: BTreeSet::new(),
            sequence: PacketSequence(u16::MAX - 500),
            joined: false,
            baselines,
        }
    }

    /// Whether the client's set holds `key`: relevant and not leaving, or re-entering.
    fn holds(&self, key: StableKey) -> bool {
        self.interest
            .entities
            .get(&key)
            .is_some_and(|entity| entity.tombstone.is_none() || entity.reentering)
    }

    fn enter(&mut self, key: StableKey) {
        self.interest.enter(key).expect("far fewer entities than handles");
        self.interest.set_claimed(key, self.claimed.contains(&key));
    }

    fn leave(&mut self, key: StableKey, region: RegionId, tick: Tick, cause: TombstoneCause) {
        self.interest.leave(key, Tombstone { region, tick, cause });
    }

    /// Absorbs one gathered frame into the mirror and, once the client has joined, into
    /// its interest.
    pub fn absorb(&mut self, gathered: &StagingFrame) {
        let tick = gathered.tick;
        if gathered.full {
            assert!(!self.joined, "the model joins after the first gather");
            self.mirror.clear();
        }
        for entry in gathered.structural() {
            match entry.change {
                StructuralChange::Spawned { key, region } => {
                    self.mirror.insert(key, MirrorEntity::new(region));
                    if self.named.contains(&region) {
                        self.enter(key);
                    }
                }
                StructuralChange::Despawned { key, region } => {
                    self.mirror.remove(&key);
                    self.leave(key, region, tick, TombstoneCause::Destroyed);
                }
                StructuralChange::Moved { key, to, .. } => {
                    if let Some(entity) = self.mirror.get_mut(&key) {
                        entity.region = to;
                    }
                    match (self.holds(key), self.named.contains(&to)) {
                        // Named by the region it went to, which the client does not
                        // hold it in.
                        (true, false) => self.leave(key, to, tick, TombstoneCause::Left),
                        (false, true) => self.enter(key),
                        // An entity the client may fail to move (its old region unloading
                        // in the same step) arrives whole beside its move.
                        (true, true) => self.interest.resend_whole(key),
                        (false, false) => {}
                    }
                }
                StructuralChange::Removed { key, component } => {
                    if let Some(entity) = self.mirror.get_mut(&key)
                        && entity.rows.remove(&component).is_some()
                    {
                        entity.removed.insert(component, tick);
                    }
                }
                StructuralChange::Added { .. } => {}
            }
        }
        for row in gathered.rows() {
            let entity =
                self.mirror.entry(row.key).or_insert_with(|| MirrorEntity::new(row.region));
            entity.region = row.region;
            entity.rows.insert(row.component, (tick, row.bytes.to_vec()));
            entity.removed.remove(&row.component);
        }
    }

    /// Makes `region` relevant or not from `tick`, telling the client on `control` once
    /// it has joined.
    fn set_relevant(&mut self, region: RegionId, relevant: bool, tick: Tick) -> Option<Control> {
        if relevant == self.relevant.contains(&region) {
            return None;
        }
        let id = self.next_message;
        self.next_message += 1;
        let change = if relevant {
            self.relevant.insert(region);
            self.pending.insert(region, id);
            RegionChange::Relevant(relevance(region, self.baselines.of(region)))
        } else {
            self.relevant.remove(&region);
            self.pending.remove(&region);
            if self.named.remove(&region) {
                let leaving: Vec<StableKey> = self
                    .mirror
                    .iter()
                    .filter(|(_, entity)| entity.region == region)
                    .map(|(&key, _)| key)
                    .collect();
                for key in leaving {
                    if self.holds(key) {
                        self.leave(key, region, tick, TombstoneCause::Left);
                    }
                }
            }
            RegionChange::Irrelevant
        };
        self.joined.then_some(Control::Region { id, tick, region, change })
    }

    fn set_claimed(&mut self, key: StableKey, claimed: bool) {
        if claimed {
            self.claimed.insert(key);
        } else {
            self.claimed.remove(&key);
        }
        self.interest.set_claimed(key, claimed);
    }

    /// The reliable channel delivered message `id`: a region waiting on it is named, and
    /// its entities enter.
    fn delivered(&mut self, id: u64) {
        let Some(region) =
            self.pending.iter().find(|&(_, &pending)| pending == id).map(|(&region, _)| region)
        else {
            return;
        };
        self.pending.remove(&region);
        self.named.insert(region);
        let entering: Vec<StableKey> = self
            .mirror
            .iter()
            .filter(|(_, entity)| entity.region == region)
            .map(|(&key, _)| key)
            .collect();
        for key in entering {
            self.enter(key);
        }
    }

    /// A join or a resync at `tick`: every relevant region is named at once, every entity
    /// residing in one enters, the interest starts over, and the full frame names them.
    fn resync(&mut self, tick: Tick) -> Control {
        self.named.clone_from(&self.relevant);
        self.pending.clear();
        let entering: Vec<StableKey> = self
            .mirror
            .iter()
            .filter(|(_, entity)| self.named.contains(&entity.region))
            .map(|(&key, _)| key)
            .collect();
        for key in entering {
            self.enter(key);
        }
        self.interest.resync(tick);
        self.joined = true;
        let regions = self
            .relevant
            .iter()
            .map(|&region| (region, relevance(region, self.baselines.of(region))));
        let spawns = self
            .interest
            .entities
            .iter()
            .map(|(&key, entity)| self.wire_spawn(key, entity.handle, entity.claimed))
            .collect();
        Control::Full { tick, regions: regions.collect(), spawns }
    }

    fn wire_spawn(&self, key: StableKey, handle: EntityHandle, claimed: bool) -> WireSpawn {
        let entity = &self.mirror[&key];
        let components = entity
            .rows
            .iter()
            .map(|(&component, (_, bytes))| ComponentData { component, bytes: bytes.clone() })
            .collect();
        WireSpawn { handle, key, region: entity.region, claimed, components }
    }

    /// The packet for `tick`: a tombstone for every entity owed one; every entity nothing
    /// of which is acknowledged whole, as a spawn beside a move to its region and a removal
    /// of every component the mirror records as removed; every
    /// claimed entity as an exact row of each of its components; every other entity as
    /// the rows changed and the components removed since its acknowledged tick.
    pub fn pack(&mut self, tick: Tick) -> Packet {
        let mut packet = Packet::new(self.sequence, tick);
        let mut record = PacketRecord { sequence: self.sequence, tick, ..PacketRecord::default() };
        for (&key, entity) in &self.interest.entities {
            let handle = entity.handle;
            if let Some(tombstone) = entity.tombstone {
                packet.tombstones.push((handle, tombstone));
                record.tombstones.push((handle, tombstone));
                continue;
            }
            let mirror = &self.mirror[&key];
            let carried = match entity.acked {
                None => {
                    packet.spawns.push(self.wire_spawn(key, handle, entity.claimed));
                    packet.moves.push((handle, mirror.region));
                    // The client may hold a component the server removed while nothing
                    // of the entity was acknowledged; the spawn's upsert keeps it.
                    for &component in mirror.removed.keys() {
                        packet.removed.push((handle, component));
                    }
                    true
                }
                Some(acked) => {
                    let mut carried = entity.claimed;
                    for (&component, (changed, bytes)) in &mirror.rows {
                        if entity.claimed || changed.0 > acked.0 {
                            packet.rows.push(WireRow {
                                handle,
                                component,
                                tick: *changed,
                                exact: entity.claimed,
                                bytes: bytes.clone(),
                            });
                            carried = true;
                        }
                    }
                    for (&component, &at) in &mirror.removed {
                        if at.0 > acked.0 {
                            packet.removed.push((handle, component));
                            carried = true;
                        }
                    }
                    carried
                }
            };
            if carried {
                record.entities.push((handle, tick));
            }
        }
        self.interest.sent(record);
        self.sequence = self.sequence.next();
        packet
    }
}

// ---- The receiver ----

/// What the receiver did with what arrived.
#[derive(Clone, Copy, Default, Debug)]
pub struct ReceiverStats {
    pub staged: u32,
    /// Dropped for a tick not after the newest staged, or while a full frame is awaited.
    pub late: u32,
    /// Dropped whole as malformed.
    pub malformed: u32,
    /// Frames staged with region changes alone because one frame would have held two
    /// changes of one region; the packet that met them was dropped.
    pub split: u32,
    pub unknown_tombstones: u32,
}

/// `lactose-net`'s receiver for one connection on the network thread.
pub struct ReceiverModel {
    pub handles: ConnectionHandles,
    pub slots: BTreeMap<StableKey, InterpolationSlot>,
    writer: InterpolationWriter,
    pub reader: InterpolationReader,
    newest: Option<Tick>,
    pub awaiting_full: bool,
    /// Region changes that arrived and are not yet in a staged frame, in arrival order.
    pending: VecDeque<(Tick, RegionId, RegionChange)>,
    sizes: BTreeMap<ComponentKey, usize>,
    ack: Option<AckField>,
    /// The frames staged and not yet handed to `app`.
    pub staged: Vec<StagedFrame>,
    pub stats: ReceiverStats,
}

impl ReceiverModel {
    pub fn new(sizes: BTreeMap<ComponentKey, usize>) -> Self {
        let (writer, reader) = InterpolationBuffer::split(1024);
        Self {
            handles: ConnectionHandles::new(),
            slots: BTreeMap::new(),
            writer,
            reader,
            newest: None,
            awaiting_full: true,
            pending: VecDeque::new(),
            sizes,
            ack: None,
            staged: Vec::new(),
            stats: ReceiverStats::default(),
        }
    }

    /// The acknowledgement field for the packets staged so far; `None` before the first,
    /// since a field always covers its `latest`.
    pub fn ack(&self) -> Option<AckField> {
        self.ack
    }

    /// A control message, in channel order; returns what to acknowledge on the reliable
    /// channel.
    pub fn control(&mut self, control: Control) -> Option<Upward> {
        match control {
            Control::Region { id, tick, region, change } => {
                self.pending.push_back((tick, region, change));
                Some(Upward::Delivered(id))
            }
            Control::Full { tick, regions, spawns } => {
                assert!(self.awaiting_full, "a full frame answers a request or a join");
                self.awaiting_full = false;
                self.handles.clear();
                for (_, slot) in std::mem::take(&mut self.slots) {
                    self.writer.release(slot).expect("held");
                }
                let mut frame = StagedFrame::resync(tick);
                for (region, message) in regions {
                    frame.regions.insert(region, RegionChange::Relevant(message));
                }
                for spawn in &spawns {
                    self.handles.insert(spawn.handle, spawn.key).expect("fresh handles");
                    self.stage_spawn(&mut frame, spawn);
                }
                frame.time_scale = Some(TimeScale::REAL_TIME);
                frame.acked_input = Some(tick);
                // Changes stamped at or before the full frame are in it.
                self.pending.retain(|&(stamp, _, _)| stamp.0 > tick.0);
                self.newest = Some(tick);
                self.staged.push(frame);
                self.stats.staged += 1;
                None
            }
        }
    }

    /// The client asks for a full frame: nothing is staged until it arrives.
    pub fn request_full(&mut self) {
        self.awaiting_full = true;
    }

    fn resolve(handles: &ConnectionHandles, handle: EntityHandle) -> Result<StableKey> {
        handles.key_of(handle).ok_or_else(|| Error::new(ErrorKind::InvalidData, "no such handle"))
    }

    fn check_length(&self, component: ComponentKey, bytes: &[u8]) -> Result<()> {
        if self.sizes.get(&component) == Some(&bytes.len()) {
            Ok(())
        } else {
            Err(Error::new(ErrorKind::InvalidData, "a row of another length"))
        }
    }

    /// Checks everything `packet` carries against the handle mirror and the registry and
    /// returns the mirror as staging it would leave it, changing nothing.
    fn validate(&self, packet: &Packet) -> Result<ConnectionHandles> {
        let mut handles = self.handles.clone();
        for spawn in &packet.spawns {
            handles.insert(spawn.handle, spawn.key)?;
            for data in &spawn.components {
                self.check_length(data.component, &data.bytes)?;
            }
        }
        for row in &packet.rows {
            Self::resolve(&handles, row.handle)?;
            self.check_length(row.component, &row.bytes)?;
        }
        for &(handle, _) in packet.moves.iter() {
            Self::resolve(&handles, handle)?;
        }
        for &(handle, _) in packet.removed.iter() {
            Self::resolve(&handles, handle)?;
        }
        Ok(handles)
    }

    /// A datagram: staged whole as one frame, or dropped whole and not acknowledged.
    pub fn packet(&mut self, packet: &Packet) {
        let after_newest = self.newest.is_none_or(|newest| packet.tick.0 > newest.0);
        if self.awaiting_full || !after_newest {
            self.stats.late += 1;
            return;
        }
        // The region changes this frame takes: those stamped at or before it, in order,
        // up to a second change of one region.
        let mut regions = BTreeMap::new();
        let mut split = false;
        for (stamp, region, change) in &self.pending {
            if stamp.0 > packet.tick.0 {
                break;
            }
            if regions.contains_key(region) {
                split = true;
                break;
            }
            regions.insert(*region, change.clone());
        }
        if split {
            let mut frame = StagedFrame::new(packet.tick);
            self.pending.drain(..regions.len());
            frame.regions = regions;
            self.newest = Some(packet.tick);
            self.staged.push(frame);
            self.stats.split += 1;
            return;
        }
        let Ok(mut handles) = self.validate(packet) else {
            self.stats.malformed += 1;
            return;
        };
        let mut frame = StagedFrame::new(packet.tick);
        self.pending.drain(..regions.len());
        frame.regions = regions;
        for spawn in &packet.spawns {
            self.stage_spawn(&mut frame, spawn);
        }
        for &(handle, region) in &packet.moves {
            frame.moves.insert(Self::resolve(&handles, handle).expect("validated"), region);
        }
        for &(handle, component) in &packet.removed {
            frame.removed.insert((Self::resolve(&handles, handle).expect("validated"), component));
        }
        for row in &packet.rows {
            let key = Self::resolve(&handles, row.handle).expect("validated");
            assert!(frame.push_row(key, row.component, row.tick, row.exact, &row.bytes));
            if row.component == Position::KEY
                && let Some(&slot) = self.slots.get(&key)
            {
                self.writer.push(slot, state(row.tick, &row.bytes)).expect("held");
            }
        }
        for &(handle, tombstone) in &packet.tombstones {
            match handles.release(handle) {
                Some(key) => {
                    frame.despawns.insert(key, tombstone);
                    if let Some(slot) = self.slots.remove(&key) {
                        self.writer.release(slot).expect("held");
                    }
                }
                None => self.stats.unknown_tombstones += 1,
            }
        }
        self.handles = handles;
        self.newest = Some(packet.tick);
        self.ack = Some(AckField::received(self.ack, packet.sequence));
        self.staged.push(frame);
        self.stats.staged += 1;
    }

    /// Stages a spawn with its interpolation slot: none for a claimed entity, whose slot is
    /// released, else the slot it holds or a new one.
    fn stage_spawn(&mut self, frame: &mut StagedFrame, spawn: &WireSpawn) {
        let slot = if spawn.claimed {
            if let Some(slot) = self.slots.remove(&spawn.key) {
                self.writer.release(slot).expect("held");
            }
            None
        } else {
            let slot = match self.slots.get(&spawn.key) {
                Some(&slot) => slot,
                None => self.writer.allocate().expect("far fewer entities than slots"),
            };
            self.slots.insert(spawn.key, slot);
            if let Some(position) = spawn.components.iter().find(|c| c.component == Position::KEY) {
                self.writer.push(slot, state(frame.tick, &position.bytes)).expect("held");
            }
            Some(slot)
        };
        let staged = StagedSpawn {
            origin: None,
            region: spawn.region,
            slot,
            components: spawn.components.clone(),
        };
        frame.spawns.insert(net_of(spawn.key), staged);
    }
}

/// The interpolation state of a `Position` row.
fn state(tick: Tick, bytes: &[u8]) -> ReceivedState {
    let word = |at: usize| {
        Scalar::from_i32(i32::from_ne_bytes(bytes[at..at + 4].try_into().expect("four bytes")))
    };
    ReceivedState {
        tick,
        teleported: Tick(0),
        position: Vec3::new(word(0), word(4), Scalar::ZERO),
        rotation: Quat::IDENTITY,
    }
}

// ---- The whole: server, links, receiver, client ----

/// A change the script asks of the packer, applied after the next step's gather.
#[derive(Clone, Copy, Debug)]
pub enum Change {
    Relevant(RegionId, bool),
    Claimed(StableKey, bool),
    /// The client asks for a full frame.
    Resync,
}

/// One server, one client, and the network between, stepped one server tick at a time.
pub struct Replication {
    pub server: Option<Simulation>,
    pub model: ServerModel,
    pub receiver: ReceiverModel,
    pub client: Option<Simulation>,
    pub skipped: Skipped,
    layout: LayoutConfig,
    baselines: Baselines,
    pub link: LinkConfig,
    rng: Rng,
    down: Leg<Packet>,
    up: Leg<AckField>,
    control_down: Reliable<Control>,
    control_up: Reliable<Upward>,
    changes: Vec<Change>,
    /// While set, staged frames wait on the network thread instead of reaching `app`.
    pub hold: bool,
    held: Vec<StagedFrame>,
    /// How many ticks the client's replicated world runs behind the server.
    pub lag: u32,
    /// Every frame handed to `app`, with the tick of the step that folded it.
    pub frames: Vec<Arc<StagedFrame>>,
    /// The replicated components of every claimed entity, by server tick.
    history: BTreeMap<Tick, BTreeMap<StableKey, BTreeSet<ComponentKey>>>,
    /// How many (frame, entity) pairs of exact rows were checked against `history`.
    pub whole_checks: u32,
    /// How many region loads the client's world refused.
    pub load_failures: usize,
}

impl Replication {
    pub fn new(layout: LayoutConfig, entities: u64, relevant: &[RegionId], seed: u64) -> Self {
        let baselines = Baselines::default();
        Self {
            server: Some(server(layout, entities)),
            model: ServerModel::new(relevant, baselines.clone()),
            receiver: ReceiverModel::new(sizes(types().registry())),
            client: None,
            skipped: Skipped::default(),
            layout,
            baselines,
            link: LinkConfig::PERFECT,
            rng: Rng::new(seed),
            down: Leg::new(),
            up: Leg::new(),
            control_down: Reliable::new(),
            control_up: Reliable::new(),
            changes: Vec::new(),
            hold: false,
            held: Vec::new(),
            lag: 2,
            frames: Vec::new(),
            history: BTreeMap::new(),
            whole_checks: 0,
            load_failures: 0,
        }
    }

    /// Cooks `baseline` for `region` on both ends, before the join.
    pub fn cook(&mut self, region: RegionId, baseline: Baseline) {
        assert!(!self.model.joined, "cooked before the join");
        self.baselines.cooked.insert(region, Arc::new(baseline));
        self.model.baselines = self.baselines.clone();
    }

    pub fn world_mut(&mut self) -> &mut World {
        self.server.as_mut().expect("present between ticks").world_mut()
    }

    pub fn server(&self) -> &Simulation {
        self.server.as_ref().expect("present between ticks")
    }

    pub fn client(&self) -> &Simulation {
        self.client.as_ref().expect("joined")
    }

    pub fn now(&self) -> u32 {
        self.server().tick().0
    }

    pub fn change(&mut self, change: Change) {
        self.changes.push(change);
    }

    pub fn rng(&mut self) -> &mut Rng {
        &mut self.rng
    }

    /// One server tick: the step and its gather into the mirror; the script's changes;
    /// what the server received; the packet; what the client received; its frames handed
    /// to `app`; the client's replicated world stepped to `lag` ticks behind.
    pub fn tick(&mut self, executor: &Executor) {
        let mut server = self.server.take().expect("present between ticks");
        server = executor.block_on(server.step(executor, TickInput::default())).expect("a step");
        server = executor.block_on(server.run_passes(executor)).expect("the passes");
        let gathered =
            server.world_mut().service_mut::<GatherState>().expect("setup inserted it").take();
        let tick = server.tick();
        let now = tick.0;
        self.model.absorb(&gathered);
        let rows = replicated_rows(server.world());
        self.server = Some(server);

        let mut resync = !self.model.joined;
        for change in std::mem::take(&mut self.changes) {
            let control = match change {
                Change::Relevant(region, relevant) => {
                    self.model.set_relevant(region, relevant, tick)
                }
                Change::Claimed(key, claimed) => {
                    self.model.set_claimed(key, claimed);
                    None
                }
                Change::Resync => {
                    self.receiver.request_full();
                    self.control_up.send(now, Upward::Resync, &self.link, &mut self.rng);
                    None
                }
            };
            if let Some(control) = control {
                self.control_down.send(now, control, &self.link, &mut self.rng);
            }
        }
        for ack in self.up.due(now) {
            self.model.interest.acknowledge(ack);
        }
        for message in self.control_up.due(now) {
            match message {
                Upward::Delivered(id) => self.model.delivered(id),
                Upward::Resync => resync = true,
            }
        }
        if resync {
            let full = self.model.resync(tick);
            self.control_down.send(now, full, &self.link, &mut self.rng);
        }
        let mut claimed = BTreeMap::new();
        for ((key, component), _) in rows {
            if self.model.claimed.contains(&key) {
                claimed.entry(key).or_insert_with(BTreeSet::new).insert(component);
            }
        }
        self.history.insert(tick, claimed);
        let packet = self.model.pack(tick);
        self.down.send(now, packet, &self.link, &mut self.rng);

        for control in self.control_down.due(now) {
            if let Some(upward) = self.receiver.control(control) {
                self.control_up.send(now, upward, &self.link, &mut self.rng);
            }
        }
        for packet in self.down.due(now) {
            self.receiver.packet(&packet);
        }
        if let Some(ack) = self.receiver.ack() {
            self.up.send(now, ack, &self.link, &mut self.rng);
        }
        let staged = std::mem::take(&mut self.receiver.staged);
        for frame in &staged {
            self.check_whole(frame);
        }
        self.held.extend(staged);
        if !self.hold {
            self.hand_over();
        }
        self.step_client(executor, now.saturating_sub(self.lag));
    }

    /// Every frame holding an exact row of an entity holds one of each replicated
    /// component the server's entity had at the frame's tick.
    fn check_whole(&mut self, frame: &StagedFrame) {
        let mut exact: BTreeMap<StableKey, BTreeSet<ComponentKey>> = BTreeMap::new();
        for row in frame.rows().filter(|row| row.exact) {
            exact.entry(row.key).or_default().insert(row.component);
        }
        for (key, components) in exact {
            let server = &self.history[&frame.tick][&key];
            assert_eq!(&components, server, "{key:?} in the frame for {:?}", frame.tick);
            self.whole_checks += 1;
        }
    }

    /// Hands every held frame to `app`, which pushes it into the client's inbox: the
    /// first, a full frame, builds the client at its tick.
    fn hand_over(&mut self) {
        for frame in std::mem::take(&mut self.held) {
            let frame = Arc::new(frame);
            if self.client.is_none() {
                assert!(frame.full, "the first frame is the join's");
                let hook = ClientHook { skipped: Arc::clone(&self.skipped) };
                let builder = client_builder(frame.tick, self.layout, hook);
                self.client = Some(builder.build().expect("the client builds"));
            }
            let client = self.client.as_mut().expect("built");
            let inbox = client.world_mut().service_mut::<StagedInbox>().expect("setup");
            assert!(inbox.push(Arc::clone(&frame)), "the receiver rules keep the inbox rule");
            self.frames.push(frame);
        }
    }

    /// Steps the client's replicated world to `through`, each step folding the frames
    /// due at its tick.
    pub fn step_client(&mut self, executor: &Executor, through: u32) {
        let Some(mut client) = self.client.take() else { return };
        while client.tick().0 < through {
            let loaded: Vec<RegionId> = client.world().regions().collect();
            let next = Tick(client.tick().0 + 1);
            let mut input = TickInput::default();
            let inbox = client.world().service::<StagedInbox>().expect("setup");
            for frame in inbox.due(next) {
                fold_frame(&mut input, frame, &loaded, &self.baselines).expect("the fold");
            }
            client = executor.block_on(client.step(executor, input)).expect("a client step");
            self.load_failures += client.lifecycle_mut().take_load_failures().len();
        }
        self.client = Some(client);
    }

    /// The client's world equals the server's in the relevant regions: the same regions
    /// loaded, the same replicated rows; the handle mirrors equal; every unclaimed entity
    /// presented through a slot holding a state, every claimed one through none.
    pub fn assert_converged(&self, what: &str) {
        let server = self.server().world();
        let client = self.client().world();
        let relevant = &self.model.relevant;
        assert_eq!(self.load_failures, 0, "{what}: every region load the frames asked for");
        assert_eq!(&client.regions().collect::<BTreeSet<_>>(), relevant, "{what}: regions");
        let expected = rows_in(server, relevant);
        let held = replicated_rows(client);
        if held != expected {
            let missing: Vec<_> = expected.keys().filter(|k| !held.contains_key(k)).collect();
            let extra: Vec<_> = held.keys().filter(|k| !expected.contains_key(k)).collect();
            let differ: Vec<_> =
                held.iter().filter(|(k, v)| expected.get(k).is_some_and(|e| e != *v)).collect();
            panic!("{what}: missing {missing:?}, extra {extra:?}, differing {differ:?}");
        }
        let interest = &self.model.interest;
        assert!(interest.entities.values().all(|e| e.tombstone.is_none() && e.acked.is_some()));
        let table: Vec<_> = interest.handles.iter().collect();
        let mirror: Vec<_> = self.receiver.handles.iter().collect();
        assert_eq!(mirror, table, "{what}: handles");
        let keys: BTreeSet<StableKey> = held.keys().map(|&(key, _)| key).collect();
        let mut unclaimed = 0;
        for &key in &keys {
            let slot = client.get::<RemoteSlot>(key).map(|remote| remote.slot);
            assert_eq!(slot.is_none(), self.model.claimed.contains(&key), "{what}: {key:?}");
            if let Some(slot) = slot {
                assert_eq!(self.receiver.slots.get(&key), Some(&slot), "{what}: {key:?}");
                let sample = self.receiver.reader.sample(slot, at_tick(self.now()));
                assert_ne!(sample, lactose_net_host::interpolation::RemoteSample::MISSING);
                unclaimed += 1;
            }
        }
        assert_eq!(self.receiver.slots.len(), unclaimed, "{what}: slots");
    }
}

pub fn at_tick(tick: u32) -> lactose_net_host::interpolation::PresentationTime {
    lactose_net_host::interpolation::PresentationTime { tick: Tick(tick), fraction: 0 }
}

// ---- The scripts ----

/// One script step on the server between runs: a few writes, additions, removals, moves
/// and despawns of random live entities, and now and then a spawn.
pub fn churn(world: &mut World, rng: &mut Rng, next: &mut u64) {
    for _ in 0..3 {
        let target = key(1 + rng.below(*next - 1));
        if !world.contains(target) {
            continue;
        }
        let value = rng.below(1000);
        match rng.below(14) {
            0 | 1 => world.insert(target, Health(i32::try_from(value).expect("small"))).map(drop),
            2 => world.insert(target, Status(u32::try_from(value).expect("small"))).map(drop),
            3 => world.remove::<Status>(target).map(drop),
            4 => world.insert(target, Marked).map(drop),
            5 => world.remove::<Marked>(target).map(drop),
            6 => world.remove::<Health>(target).map(drop),
            7..=9 => {
                let to = REGIONS[usize::try_from(value % 4).expect("small")];
                if world.region_of(target) == Some(to) {
                    Ok(())
                } else {
                    world.move_to_region(target, to)
                }
            }
            10 => world.despawn(target),
            _ => Ok(()),
        }
        .expect("a live entity takes every change");
    }
    if rng.chance(25) {
        spawn_entity(world, *next);
        *next += 1;
    }
}

/// Toggles the relevance of one of the regions after the first, and the claim of a random
/// entity, now and then.
pub fn change_interest(replication: &mut Replication, rng: &mut Rng, next: u64) {
    if rng.chance(4) {
        let region = REGIONS[1 + usize::try_from(rng.below(3)).expect("small")];
        let relevant = !replication.model.relevant.contains(&region);
        replication.change(Change::Relevant(region, relevant));
    }
    if rng.chance(4) {
        let target = key(1 + rng.below(next - 1));
        let claimed = !replication.model.claimed.contains(&target);
        replication.change(Change::Claimed(target, claimed));
    }
}

/// Stops every mover and lets the link settle, so the client catches up.
pub fn calm(replication: &mut Replication, executor: &Executor, next: u64) {
    let world = replication.world_mut();
    for n in 1..next {
        if world.contains(key(n)) {
            let _ = world.remove::<Mover>(key(n)).unwrap();
        }
    }
    replication.link = LinkConfig::PERFECT;
    for _ in 0..40 {
        replication.tick(executor);
    }
}
