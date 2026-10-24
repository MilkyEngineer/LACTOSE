//! What the integration tests share: a small game written as a game crate writes it (the
//! game of `crates/sim/tests/common`, with an owned cooked row and a derived cooked column
//! added to its baselines), the hooks `app` and `input` write, the save and load path as
//! `app` drives it through this crate, and a driver that answers the lifecycle with this
//! crate's records and bundles.

#![allow(dead_code, reason = "each test file uses part of the shared module")]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, LazyLock};

use lactose_asset_api::{
    Asset, AssetId, AssetKind, AssetSet, AssetVersion, LoadPriority, PinnedTable, PublishedAssets,
    Readiness, UntypedHandle, VersionTable,
};
use lactose_bundle::{Baseline, BaselineRow, ComponentData};
use lactose_core::{EntityRef, NetId, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{
    ClaimedBy, Commands, Ctx, Executor, InRegion, Mut, OwnedBy, RegionId, RegionMode, Res, ResMut,
    StableKeyOf, SystemSet, Without, World, WorldKind, finalize, system,
};
use lactose_math::{ExactSum, Scalar};
use lactose_persist::{Container, ContainerHeader, Loader, Migrations, RegionRecords, SchemaTable};
use lactose_reflect::{Reflect, component, resource};
use lactose_sim::{
    InputApplier, PlayerSample, RegionAtlas, RegionBundle, RegionCell, RegionControl, RegionExport,
    RegionRequest, Registration, SavePoint, SessionApplier, SessionEvent, Simulation,
    SimulationBuilder, Stage, StreamingAnchor, TickDelta, TickInput, TypeTable, sets,
};

// ---- Components ----

/// Where an NPC is; replicated, so tracked per row.
#[component(key = "game::position", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct Position {
    #[field(id = 1)]
    pub x: Scalar,
    #[field(id = 2)]
    pub y: Scalar,
}

/// How an NPC moves.
#[component(key = "game::velocity", persistent)]
#[derive(PartialEq, Debug)]
pub struct Velocity {
    #[field(id = 1)]
    pub x: Scalar,
    #[field(id = 2)]
    pub y: Scalar,
}

/// Mass, summed by a reduction.
#[component(key = "game::mass", persistent)]
#[derive(PartialEq, Debug)]
pub struct Mass(#[field(id = 1)] pub Scalar);

/// Hit points.
#[component(key = "game::health", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct Health(#[field(id = 1)] pub i32);

/// Age in ticks; an NPC despawns when it is old.
#[component(key = "game::age", persistent)]
#[derive(PartialEq, Debug)]
pub struct Age(#[field(id = 1)] pub u32);

/// What an NPC follows: a relationship.
#[component(key = "game::follows", persistent)]
#[derive(PartialEq, Debug)]
pub struct Follows {
    #[field(id = 1)]
    pub target: EntityRef,
}

/// Spawns NPCs every so often.
#[component(key = "game::spawner", persistent)]
#[derive(PartialEq, Debug)]
pub struct Spawner {
    #[field(id = 1)]
    pub every: u32,
}

/// Contacts this tick: transient, zeroed at every input sync point.
#[component(key = "game::contacts", transient)]
#[derive(PartialEq, Debug)]
pub struct Contacts(#[field(id = 1)] pub u32);

/// A presence marker: no field.
#[component(key = "game::marker", persistent)]
#[derive(PartialEq, Debug)]
pub struct Marker;

/// The saved identity of a player's entity, by which a join after a load re-claims it.
#[component(key = "game::player", persistent)]
#[derive(PartialEq, Debug)]
pub struct Player {
    #[field(id = 1)]
    pub id: PlayerId,
}

/// A player's position, in sixty-fourths of a region.
#[component(key = "game::walker", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct Walker {
    #[field(id = 1)]
    pub x: i32,
    #[field(id = 2)]
    pub y: i32,
}

/// The quantised input `lactose-input` would define: transient, zeroed at the input sync
/// point and written by the applier, with [`input_writer`] its declared writer.
#[component(key = "game::input", transient)]
#[derive(PartialEq, Debug)]
pub struct InputSample {
    #[field(id = 1)]
    pub dx: i32,
    #[field(id = 2)]
    pub dy: i32,
}

/// A derived cache: never hashed or saved; a baseline's cooked value stays after a load.
#[component(key = "game::cache", derived)]
#[derive(PartialEq, Debug)]
pub struct Cache(#[field(id = 1)] pub u64);

// ---- Resources ----

/// Configuration: a world resource, so `derived`.
#[resource(key = "game::gravity", derived)]
pub struct Gravity(#[field(id = 1)] pub Scalar);

/// What the mass reduction writes: a world resource, so `derived`.
#[resource(key = "game::total_mass", derived)]
pub struct TotalMass(#[field(id = 1)] pub Scalar);

/// A region resource, saved with the region.
#[resource(key = "game::weather", persistent)]
#[derive(PartialEq, Debug)]
pub struct Weather(#[field(id = 1)] pub u32);

/// A transient region resource: hashed, zeroed every tick, not saved.
#[resource(key = "game::rain", transient)]
#[derive(PartialEq, Debug)]
pub struct Rain(#[field(id = 1)] pub u32);

/// World-global persistent state: a resource of the pinned region.
#[resource(key = "game::score", persistent)]
#[derive(PartialEq, Debug)]
pub struct Score(#[field(id = 1)] pub u64);

// ---- The grid ----

/// Regions per side of the test grid.
pub const GRID: i32 = 5;
/// Units of `Walker` per region.
pub const CELL: i32 = 64;
/// The pinned region, far from the grid.
pub const PINNED: RegionId = RegionId(999);
/// Its cell.
pub const PINNED_CELL: RegionCell = RegionCell { x: 40, y: 40 };

/// The region of grid cell (`x`, `y`).
pub const fn region_at(x: i32, y: i32) -> RegionId {
    RegionId(100 + (y * GRID + x) as u64)
}

/// The cell of a grid region.
pub fn cell_of(region: RegionId) -> RegionCell {
    if region == PINNED {
        return PINNED_CELL;
    }
    let index = region.0 as i32 - 100;
    RegionCell { x: index % GRID, y: index / GRID }
}

/// The grid and the pinned region.
pub fn atlas() -> RegionAtlas {
    let mut atlas = RegionAtlas::new();
    for y in 0..GRID {
        for x in 0..GRID {
            atlas.insert(region_at(x, y), RegionCell { x, y }).unwrap();
        }
    }
    atlas.insert(PINNED, PINNED_CELL).unwrap();
    atlas
}

/// `Anchored` over the grid, with or without the pinned region.
pub fn anchored(pinned: bool) -> RegionControl {
    let pinned = if pinned { BTreeSet::from([PINNED]) } else { BTreeSet::new() };
    RegionControl::Anchored { atlas: atlas(), pinned }
}

// ---- The game's systems (architecture section 6.1) ----

pub static MOVEMENT: SystemSet = SystemSet::new("game::movement");

/// A player walks by its input.
#[system(name = "game::walk", worlds = [server, prediction], in_set = MOVEMENT)]
pub fn walk(input: &InputSample, walker: Mut<Walker>) {
    walker.x += 4 * input.dx;
    walker.y += 4 * input.dy;
}

/// A player whose position left its region moves to the region it is now over; the ECS
/// drops a move to an unloaded region.
#[system(name = "game::cross", worlds = [server, prediction], after = walk)]
pub fn cross(key: StableKeyOf, walker: &Walker, at: &InRegion, cmd: Commands) {
    let (x, y) = (walker.x.div_euclid(CELL), walker.y.div_euclid(CELL));
    if (0..GRID).contains(&x) && (0..GRID).contains(&y) && region_at(x, y) != at.region {
        cmd.move_to_region(*key, region_at(x, y));
    }
}

#[system(name = "game::apply_gravity", worlds = [server, prediction], in_set = MOVEMENT)]
pub fn apply_gravity(
    dt: Res<TickDelta>,
    g: Res<Gravity>,
    vel: Mut<Velocity>,
    _mass: &Mass,
    _npc: Without<Player>,
) {
    vel.y -= g.0 * dt.0;
}

#[system(name = "game::integrate", worlds = [server, prediction], after = apply_gravity)]
pub fn integrate(dt: Res<TickDelta>, vel: &Velocity, pos: Mut<Position>) {
    let next = Position { x: pos.x + vel.x * dt.0, y: pos.y + vel.y * dt.0 };
    pos.set_if_neq(next);
}

#[system(name = "game::total_mass", worlds = [server], reduce = ExactSum<Scalar>)]
pub fn total_mass(mass: &Mass) -> Scalar {
    mass.0
}

#[finalize(total_mass)]
pub fn store_total(sum: Scalar, out: ResMut<TotalMass>) {
    out.0 = sum;
}

/// The transient column's writer.
#[system(name = "game::count_contacts", worlds = [server, prediction], after = integrate)]
pub fn count_contacts(pos: &Position, contacts: Mut<Contacts>) {
    contacts.0 += 1 + u32::from(pos.y < Scalar::ZERO);
}

/// The transient column's reader, ordered after its writer.
#[system(name = "game::wear", worlds = [server, prediction], after = count_contacts)]
pub fn wear(contacts: &Contacts, hp: Mut<Health>) {
    hp.0 -= contacts.0 as i32;
}

/// NPCs age and go.
#[system(name = "game::age", worlds = [server, prediction])]
pub fn age(key: StableKeyOf, age: Mut<Age>, cmd: Commands) {
    age.0 += 1;
    if age.0 > 90 {
        cmd.despawn(*key);
    }
}

/// A spawner spawns an NPC that follows it, every so often, with a velocity drawn from
/// the RNG keyed by the spawner.
#[system(name = "game::spawn_npcs", worlds = [server, prediction], after = integrate)]
pub fn spawn_npcs(ctx: Ctx, cmd: Commands, key: StableKeyOf, spawner: &Spawner, pos: &Position) {
    if ctx.tick().0 % spawner.every == 0 {
        let mut rng = ctx.rng(*key);
        let vx = Scalar::from_i32(rng.next_below(9) as i32 - 4);
        let vy = rng.next_scalar();
        cmd.spawn((
            Position { x: pos.x, y: pos.y },
            Velocity { x: vx, y: vy },
            Mass(Scalar::from_u32(1 + rng.next_below(3))),
            Health(500),
            Age(0),
            Contacts(0),
            Follows { target: EntityRef::new(*key) },
        ));
    }
}

/// Region resources move at the sync point that begins `Post`: the weather of every
/// simulated region, the rain, and the pinned region's score.
#[system(name = "game::weather", worlds = [server], exclusive)]
pub fn weather(world: &mut World) {
    let regions: Vec<RegionId> = world.regions().collect();
    for region in regions {
        if world.region(region).is_some_and(|info| info.mode == RegionMode::Simulated) {
            let _ = world.update_region_resource::<Weather>(region, |w| w.0 = w.0.wrapping_add(3));
            let _ = world.update_region_resource::<Rain>(region, |r| r.0 += 1);
            let _ = world.update_region_resource::<Score>(region, |s| s.0 += region.0);
        }
    }
}

/// Every type of the game.
pub fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<Position>()?;
    types.register::<Velocity>()?;
    types.register::<Mass>()?;
    types.register::<Health>()?;
    types.register::<Age>()?;
    types.register::<Follows>()?;
    types.register::<Spawner>()?;
    types.register::<Contacts>()?;
    types.register::<Marker>()?;
    types.register::<Player>()?;
    types.register::<Walker>()?;
    types.register::<InputSample>()?;
    types.register::<Cache>()?;
    types.register::<Gravity>()?;
    types.register::<TotalMass>()?;
    types.register::<Weather>()?;
    types.register::<Rain>()?;
    types.register::<Score>()?;
    Ok(())
}

/// Every resource a system of the game names.
pub fn setup(world: &mut World) -> Result<()> {
    world.insert_resource(Gravity(Scalar::ONE));
    world.insert_resource(TotalMass(Scalar::ZERO));
    Ok(())
}

/// Every system of the game, placed in its stage.
pub fn register_systems(registration: &mut Registration) {
    registration
        .stage(Stage::Simulation)
        .sets(&[&MOVEMENT])
        .add(&WALK)
        .add(&CROSS)
        .add(&APPLY_GRAVITY)
        .add(&INTEGRATE)
        .add(&TOTAL_MASS)
        .add(&COUNT_CONTACTS)
        .add(&WEAR)
        .add(&AGE)
        .add(&SPAWN_NPCS);
    registration.stage(Stage::Post).add(&WEATHER);
    registration.stage(Stage::Input).sets(&[&SAMPLES]).add(&INPUT_WRITER);
}

/// The input crate's set at the sync point, after `sim`'s.
pub static SAMPLES: SystemSet = SystemSet::new("game::samples").after(&[&sets::INPUT]);

/// The declared writer of [`InputSample`]: the applier writes it inside `sim`'s systems,
/// and this tells the schedule so, for the readers.
#[system(
    name = "game::input_writer",
    worlds = [server, prediction],
    exclusive,
    in_set = SAMPLES,
    writes = [InputSample]
)]
pub fn input_writer(_world: &mut World) {}

/// The game's type table.
pub fn types() -> TypeTable {
    let mut types = TypeTable::new();
    register_types(&mut types).unwrap();
    types
}

/// The game's saved schemas, as `app` builds them once from the registry.
pub fn schemas() -> &'static SchemaTable {
    static TABLE: LazyLock<SchemaTable> =
        LazyLock::new(|| SchemaTable::from_registry(types().registry()).unwrap());
    &TABLE
}

/// A simulation of the game, as `app` builds it, with the game's hooks.
pub fn game(kind: WorldKind) -> SimulationBuilder {
    SimulationBuilder::new(kind, types())
        .setup(setup)
        .systems(register_systems)
        .inputs(Box::new(Inputs))
        .sessions(Box::new(Sessions))
}

// ---- The hooks ----

/// The player's own entity: the claimed row that is also an anchor, from world state.
pub fn player_entity(world: &mut World, player: PlayerId) -> Option<StableKey> {
    let mut claimed = Vec::new();
    world.query::<&ClaimedBy>().for_each(|key, claim| {
        if claim.player == player {
            claimed.push(key);
        }
    });
    claimed.into_iter().find(|&key| world.get::<StreamingAnchor>(key).is_some())
}

/// The input applier `lactose-input` would write.
pub struct Inputs;

impl InputApplier for Inputs {
    fn apply(&mut self, world: &mut World, player: PlayerId, sample: &PlayerSample) -> Result<()> {
        let value: InputSample = lactose_reflect::decode(&sample.sample)?;
        let key = player_entity(world, player).ok_or_else(|| {
            lactose_core::Error::new(lactose_core::ErrorKind::NotFound, "the player has no entity")
        })?;
        world.insert(key, value)
    }
}

/// The session applier `app` would write: spawns the player's entity at the centre of its
/// region, or re-claims the row the game keeps for the player, and on a leave releases the
/// claim and the anchor and keeps the row.
pub struct Sessions;

impl SessionApplier for Sessions {
    fn apply(&mut self, world: &mut World, player: PlayerId, event: SessionEvent) -> Result<()> {
        match event {
            SessionEvent::Join { region } => {
                let mut kept = None;
                world.query::<&Player>().for_each(|key, row| {
                    if row.id == player && kept.is_none() {
                        kept = Some(key);
                    }
                });
                let key = match kept {
                    Some(key) => key,
                    None => {
                        let key = StableKey::Net(world.allocate_net_id()?);
                        let cell = cell_of(region);
                        let walker =
                            Walker { x: cell.x * CELL + CELL / 2, y: cell.y * CELL + CELL / 2 };
                        world.spawn(key, region, (walker, Player { id: player }, Health(1000)))?;
                        key
                    }
                };
                world.insert(key, InputSample { dx: 0, dy: 0 })?;
                world.insert(key, ClaimedBy { player })?;
                world.insert(key, StreamingAnchor { priority: 0 })
            }
            SessionEvent::Leave => {
                let Some(key) = player_entity(world, player) else { return Ok(()) };
                world.remove::<StreamingAnchor>(key)?;
                world.remove::<ClaimedBy>(key)?;
                Ok(())
            }
        }
    }
}

// ---- Frames ----

/// A player's sample.
pub fn sample(input_tick: u32, dx: i32, dy: i32) -> PlayerSample {
    let value = InputSample { dx, dy };
    PlayerSample {
        input_tick: Tick(input_tick),
        substituted: false,
        sample: lactose_reflect::bytemuck::bytes_of(&value).to_vec(),
    }
}

// ---- Assets ----

/// A store that publishes every version it is asked for.
struct Store;

struct Blob;

impl Asset for Blob {
    const KIND: AssetKind = AssetKind::from_name("test::blob");
    const FORMAT_VERSION: u32 = 1;
}

impl PublishedAssets for Store {
    fn request(&self, _: AssetId, _: AssetVersion, _: LoadPriority) {}

    fn get(&self, id: AssetId, version: AssetVersion) -> Readiness<UntypedHandle> {
        Readiness::Ready(UntypedHandle::new(id, version, Blob))
    }
}

/// A pinned table of `(asset, version)` pairs.
pub fn table(entries: &[(u64, u64)]) -> PinnedTable {
    let table: VersionTable = entries
        .iter()
        .map(|&(id, version)| (AssetId::from_bits(id), AssetVersion::from_bits(version).unwrap()))
        .collect();
    PinnedTable::pin(&Store, &table).ready().unwrap()
}

/// The table a region is first given.
pub fn first_table(region: RegionId) -> PinnedTable {
    table(&[(region.0, 1)])
}

// ---- Baselines ----

pub fn data<T: Reflect>(value: T) -> ComponentData {
    ComponentData { component: T::KEY, bytes: lactose_reflect::bytemuck::bytes_of(&value).to_vec() }
}

/// The cooked ID of a region's `n`th cooked entity.
pub fn cooked(region: RegionId, n: u64) -> NetId {
    NetId::cooked(region.0 * 1000 + n)
}

/// The document schema of every saved type of the game, as the cooker writes a baseline's,
/// with the derived ones beside them, which a baseline may carry columns of.
pub fn cooked_schemas() -> Vec<lactose_bundle::ComponentSchema> {
    types()
        .registry()
        .iter()
        .map(|info| {
            let fields = info.layout().fields().iter().filter_map(|field| {
                Some(lactose_bundle::FieldLayout {
                    id: field.id()?,
                    offset: field.offset() as u32,
                    size: field.size() as u32,
                })
            });
            let size = info.layout().size() as u32;
            lactose_bundle::ComponentSchema::new(info.key(), info.version(), size, fields).unwrap()
        })
        .collect()
}

/// A region's cooked baseline: a spawner, and a marked rock the spawner owns, with a
/// cooked cache no save carries.
pub fn baseline(region: RegionId) -> Arc<Baseline> {
    static BASELINES: LazyLock<std::sync::Mutex<BTreeMap<RegionId, Arc<Baseline>>>> =
        LazyLock::new(Default::default);
    let mut cache = BASELINES.lock().unwrap();
    Arc::clone(cache.entry(region).or_insert_with(|| cook(region)))
}

fn cook(region: RegionId) -> Arc<Baseline> {
    let cell = cell_of(region);
    let at = Position { x: Scalar::from_i32(cell.x * 10), y: Scalar::from_i32(cell.y * 10) };
    let owner = OwnedBy { owner: EntityRef::new(StableKey::Net(cooked(region, 1))) };
    let rows = [
        BaselineRow {
            id: cooked(region, 1),
            components: vec![data(at), data(Spawner { every: 7 + (region.0 % 5) as u32 })],
        },
        BaselineRow {
            id: cooked(region, 2),
            components: vec![
                data(at),
                data(Marker),
                data(Mass(Scalar::TWO)),
                data(owner),
                data(Cache(region.0)),
            ],
        },
    ];
    Arc::new(Baseline::from_rows(AssetSet::new(), cooked_schemas(), rows).unwrap())
}

/// The seed of a fresh region.
pub fn seed_of(region: RegionId) -> u64 {
    region.0.wrapping_mul(0x9E37_79B9)
}

/// The bundle of a fresh region in its cooked state, with its first table.
pub fn fresh(region: RegionId) -> RegionBundle {
    let mut resources = vec![data(Weather(region.0 as u32))];
    if region == PINNED {
        resources.push(data(Score(0)));
    }
    RegionBundle {
        seed: seed_of(region),
        mode: RegionMode::Simulated,
        baseline: baseline(region),
        overlay: None,
        resources,
        transient_resources: vec![Rain::KEY],
        assets: Some(first_table(region)),
    }
}

// ---- The save and the load, as `app` drives them through this crate ----

/// A region's records: the one path an unload, a save and a relevance message share.
pub fn records_of(export: &RegionExport) -> RegionRecords {
    RegionRecords::from_export(export, &baseline(export.region), schemas()).unwrap()
}

/// The bundle that loads `records` with `assets`, through the loader.
pub fn bundle_of(records: &RegionRecords, assets: Option<PinnedTable>) -> RegionBundle {
    let migrations = Migrations::new();
    let loader = Loader::new(schemas(), &migrations);
    loader.bundle(records, baseline(records.region), RegionMode::Simulated, assets).unwrap()
}

/// The container bytes of `save`: one section per region, made in reverse region order
/// as workers finishing out of order would hand them over, then assembled.
pub fn save_bytes(save: &SavePoint) -> Vec<u8> {
    let header = ContainerHeader::of(save, schemas());
    let sections = save
        .regions
        .iter()
        .rev()
        .map(|export| records_of(export).encode_section().unwrap())
        .collect();
    Container::assemble(&header, sections).unwrap()
}

/// The bundles of a container's every region, with `tables`.
pub fn bundles_of(
    container: &Container,
    tables: &BTreeMap<RegionId, PinnedTable>,
) -> BTreeMap<RegionId, RegionBundle> {
    container
        .regions()
        .map(|records| (records.region, bundle_of(records, tables.get(&records.region).cloned())))
        .collect()
}

/// The save copy of `simulation`, taken at a sync point.
pub fn save_of(executor: &Executor, simulation: Simulation) -> (Simulation, SavePoint) {
    match executor.block_on(simulation.save_point(executor)) {
        Ok(pair) => pair,
        Err(failure) => panic!("{:?}", failure.error),
    }
}

/// Answers the lifecycle's requests the way `app` would: a requested region arrives in the
/// next frame, from the records of its stored export if it was unloaded before, otherwise
/// fresh; a cancelled request is dropped; every export becomes records.
#[derive(Default)]
pub struct Driver {
    /// Regions owed a bundle.
    pending: BTreeSet<RegionId>,
    pub stored: BTreeMap<RegionId, RegionRecords>,
    pub requests: Vec<RegionRequest>,
}

impl Driver {
    /// Takes the lifecycle's outputs after a step.
    pub fn observe(&mut self, simulation: &mut Simulation) {
        let lifecycle = simulation.lifecycle_mut();
        let requests = lifecycle.take_requests();
        let exports = lifecycle.take_exports();
        let failures = lifecycle.take_load_failures();
        assert!(failures.is_empty(), "the driver's bundles load: {failures:?}");
        for request in requests {
            match request {
                RegionRequest::Load { region } => {
                    self.pending.insert(region);
                }
                RegionRequest::Cancel { region } => {
                    self.pending.remove(&region);
                }
                RegionRequest::Release { .. } => {}
            }
            self.requests.push(request);
        }
        for export in exports {
            self.stored.insert(export.region, records_of(&export));
        }
    }

    /// The bundles for the next frame, added to `frame`.
    pub fn fill(&mut self, frame: &mut TickInput) {
        for region in core::mem::take(&mut self.pending) {
            let bundle = match self.stored.remove(&region) {
                Some(records) => bundle_of(&records, Some(first_table(region))),
                None => fresh(region),
            };
            frame.loaded.insert(region, bundle);
        }
    }
}

// ---- The program ----

/// What the players do at `tick`: player 1 joins at tick 1 and walks east and west,
/// player 2 joins at tick 3 and walks north and south, leaves at 150 and joins again at
/// 170 where its row was kept; the time scale changes at 100.
pub fn script(tick: u32) -> TickInput {
    let mut frame = TickInput::default();
    match tick {
        1 => {
            frame.sessions.insert(PlayerId(1), SessionEvent::Join { region: region_at(1, 2) });
        }
        3 => {
            frame.sessions.insert(PlayerId(2), SessionEvent::Join { region: region_at(2, 2) });
        }
        150 => {
            frame.sessions.insert(PlayerId(2), SessionEvent::Leave);
        }
        170 => {
            frame.sessions.insert(PlayerId(2), SessionEvent::Join { region: region_at(2, 2) });
        }
        100 => frame.time_scale = Some(lactose_sim::TimeScale { thousandths: 500 }),
        _ => {}
    }
    let east = if tick % 120 < 60 { 1 } else { -1 };
    let north = if (tick + 30) % 100 < 50 { 1 } else { -1 };
    if tick >= 2 {
        frame.inputs.insert(PlayerId(1), sample(tick, east, 0));
    }
    if tick >= 4 && !(150..170).contains(&tick) {
        frame.inputs.insert(PlayerId(2), sample(tick, 0, north));
    }
    frame
}

/// The program's simulation: the game on a server under `Anchored`, the pinned region
/// included, with `layout`.
pub fn program(layout: lactose_ecs::LayoutConfig) -> Simulation {
    game(WorldKind::Server).regions(anchored(true)).layout(layout).build().unwrap()
}

/// The frame of `tick`: the script, plus what the driver owes.
pub fn program_frame(tick: u32, driver: &mut Driver) -> TickInput {
    let mut frame = script(tick);
    driver.fill(&mut frame);
    frame
}

/// An executor for a test.
pub fn executor() -> Executor {
    Executor::builder().frame_workers(2).background_workers(1).build().unwrap()
}

/// Steps once.
pub fn step(executor: &Executor, simulation: Simulation, frame: TickInput) -> Simulation {
    executor.block_on(simulation.step(executor, frame)).unwrap()
}
