//! What the integration tests share: the components and resources of a small game, the
//! systems of architecture section 6.1 written as a game crate would, and helpers that
//! build worlds, executors and schedules.

#![allow(dead_code, reason = "each test file uses part of the shared module")]

use std::sync::Arc;

use lactose_core::{ComponentKey, EntityRef, PlainArray, PlayerId, StableKey, Tick};
use lactose_ecs::{
    ChangeMark, Channel, ChannelWriter, ChunkOrder, ClaimedBy, CommandChannel, Commands, Ctx,
    Event, EventWriter, Events, InRegion, LayoutConfig, LogEntry, Mut, OwnedBy, PredictedOrigin,
    PresentEvent, PresentEvents, PresentKind, Referrer, RegionConfig, RegionId, RegionMode, Res,
    ResMut, Schedule, StableKeyOf, SystemSet, World, WorldBuilder, WorldKind, finalize, system,
};
use lactose_math::{ExactSum, Scalar, Vec2};
use lactose_reflect::{Padding, Registry, TypeRole, component, resource, value};
use lactose_task::Executor;

// ---- Components ----

/// Where an entity is; replicated, so tracked per row.
#[component(key = "test::position", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct Position {
    #[field(id = 1)]
    pub x: Scalar,
    #[field(id = 2)]
    pub y: Scalar,
}

/// How an entity moves; untracked.
#[component(key = "test::velocity", persistent)]
#[derive(PartialEq, Debug)]
pub struct Velocity {
    #[field(id = 1)]
    pub x: Scalar,
    #[field(id = 2)]
    pub y: Scalar,
}

/// Mass, summed by a reduction.
#[component(key = "test::mass", persistent)]
#[derive(PartialEq, Debug)]
pub struct Mass(#[field(id = 1)] pub Scalar);

/// Hit points; replicated.
#[component(key = "test::health", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct Health(#[field(id = 1)] pub i32);

/// A counter that extract follows.
#[component(key = "test::counter", persistent, extracted)]
#[derive(PartialEq, Debug)]
pub struct Counter(#[field(id = 1)] pub u64);

/// A bomb's fuse.
#[component(key = "test::bomb", persistent)]
#[derive(PartialEq, Debug)]
pub struct Bomb {
    #[field(id = 1)]
    pub fuse: u32,
}

/// Who threw a bomb: a relationship.
#[component(key = "test::thrown_by", persistent)]
#[derive(PartialEq, Debug)]
pub struct ThrownBy {
    #[field(id = 1)]
    pub thrower: EntityRef,
}

/// What an entity holds.
#[value]
#[repr(u8)]
#[derive(PartialEq, Eq, Debug)]
pub enum ItemKind {
    None = 0,
    Bomb = 1,
}

/// The held item, a `#[value]` enum in a field (architecture section 6).
#[component(key = "test::held_item", persistent)]
#[derive(PartialEq, Debug)]
pub struct HeldItem {
    #[field(id = 1)]
    pub kind: ItemKind,
}

/// A player's input this tick, written at the input sync point.
#[component(key = "test::player_input", derived)]
#[derive(PartialEq, Debug)]
pub struct PlayerInput {
    #[field(id = 1)]
    pub aim: Vec2,
    #[field(id = 2)]
    pub throw: bool,
    #[field(padding)]
    pub _pad: Padding<7>,
}

/// One contact of a sword.
#[value]
#[derive(PartialEq, Debug)]
pub struct Hit {
    #[field(id = 1)]
    pub key: EntityRef,
    #[field(id = 2)]
    pub amount: i32,
}

/// The contacts of this tick.
#[component(key = "test::hit_list", derived)]
#[derive(PartialEq, Debug)]
pub struct HitList {
    #[field(id = 1)]
    pub hits: PlainArray<Hit, 2>,
    #[field(id = 2)]
    pub len: u32,
}

impl HitList {
    pub fn iter(&self) -> impl Iterator<Item = &Hit> {
        self.hits.iter().take(self.len as usize)
    }
}

/// An animation phase in sixteenths of a cycle.
#[component(key = "test::anim_state", persistent)]
#[derive(PartialEq, Debug)]
pub struct AnimState {
    #[field(id = 1)]
    pub phase: u32,
}

impl AnimState {
    /// Whether the phase crossed `sixteenths` this tick.
    pub fn phase_crossed(&self, sixteenths: u32) -> bool {
        self.phase % 16 == sixteenths
    }
}

/// A per-tick accumulator, zeroed at the input sync point.
#[component(key = "test::scratch", transient)]
#[derive(PartialEq, Debug)]
pub struct Scratch(#[field(id = 1)] pub u32);

// ---- Resources ----

/// The fixed tick delta.
#[resource(key = "test::tick_delta", persistent)]
pub struct TickDelta(#[field(id = 1)] pub Scalar);

/// Gravity.
#[resource(key = "test::gravity", persistent)]
pub struct Gravity(#[field(id = 1)] pub Scalar);

/// What the mass reduction wrote.
#[resource(key = "test::total_mass", persistent)]
pub struct TotalMass(#[field(id = 1)] pub Scalar);

/// How many physics commands the physics stage has seen.
#[resource(key = "test::impulse_count", persistent)]
pub struct ImpulseCount(#[field(id = 1)] pub u32);

/// A region resource: the weather of the region.
#[resource(key = "test::weather", persistent)]
#[derive(PartialEq, Debug)]
pub struct Weather(#[field(id = 1)] pub u32);

/// A region resource that is not saved.
#[resource(key = "test::region_scratch", derived)]
pub struct RegionScratch(#[field(id = 1)] pub u32);

// ---- Events and channels ----

/// Damage to an entity.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Damage {
    pub target: StableKey,
    pub source: StableKey,
    pub amount: i32,
}

impl Event for Damage {
    fn target(&self) -> StableKey {
        self.target
    }
}

/// A sound request to presentation.
#[derive(Clone, Copy, PartialEq, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct AudioRequest {
    pub sound: u32,
}

impl AudioRequest {
    pub const FOOTSTEP: Self = Self { sound: 1 };
}

impl PresentEvent for AudioRequest {
    const KIND: PresentKind = PresentKind::from_name("test::audio");
}

/// The physics command channel of `lactose-physics-api`, in miniature.
pub struct Physics;

/// One physics command.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Impulse {
    pub body: StableKey,
    pub impulse: Vec2,
}

impl CommandChannel for Physics {
    type Entry = Impulse;

    fn remap(entry: &mut Impulse, f: &mut dyn FnMut(&mut StableKey)) {
        f(&mut entry.body);
    }
}

/// What `lactose-physics-api` adds to the writer of its channel.
pub trait PhysicsCommands {
    fn apply_impulse(&mut self, body: StableKey, impulse: Vec2);
}

impl PhysicsCommands for ChannelWriter<'_, Physics> {
    fn apply_impulse(&mut self, body: StableKey, impulse: Vec2) {
        self.push(Impulse { body, impulse });
    }
}

// ---- The section 6.1 systems ----

pub mod sets {
    use super::SystemSet;

    pub static INPUT: SystemSet = SystemSet::new("test::input");
    pub static MOVEMENT: SystemSet = SystemSet::new("test::movement").after(&[&INPUT]);
}

/// Per-entity kernel: `Mut` deref-mut sets the row tick.
#[system(name = "game_sim::apply_gravity", worlds = [server, prediction], in_set = sets::MOVEMENT)]
pub fn apply_gravity(dt: Res<TickDelta>, g: Res<Gravity>, vel: Mut<Velocity>, _mass: &Mass) {
    vel.y -= g.0 * dt.0;
}

/// Integrates velocity into the tracked position, after gravity.
#[system(name = "game_sim::integrate", worlds = [server, prediction], after = apply_gravity)]
pub fn integrate(dt: Res<TickDelta>, vel: &Velocity, pos: Mut<Position>) {
    let next = Position { x: pos.x + vel.x * dt.0, y: pos.y + vel.y * dt.0 };
    pos.set_if_neq(next);
}

/// Reduction with an exact accumulator, finalized into a resource.
#[system(name = "game_sim::total_mass", worlds = [server], reduce = ExactSum<Scalar>)]
pub fn total_mass(mass: &Mass) -> Scalar {
    mass.0
}

#[finalize(total_mass)]
pub fn store_total(sum: Scalar, out: ResMut<TotalMass>) {
    out.0 = sum;
}

/// Spawn that references the spawned entity in the same command buffer.
#[system(name = "game_sim::throw_bomb", worlds = [server, prediction])]
pub fn throw_bomb(
    ctx: Ctx,
    cmd: Commands,
    physics: ChannelWriter<Physics>,
    key: StableKeyOf,
    held: &HeldItem,
    input: &PlayerInput,
) {
    if input.throw && held.kind == ItemKind::Bomb {
        let bomb = cmd.spawn((Position { x: Scalar::ZERO, y: Scalar::ZERO }, Bomb { fuse: 180 }));
        cmd.set(*key, HeldItem { kind: ItemKind::None });
        cmd.set(bomb, ThrownBy { thrower: EntityRef::new(*key) });
        let spread = ctx.rng(bomb).next_scalar();
        physics.apply_impulse(bomb, Vec2::new(input.aim.x * spread, input.aim.y * spread));
    }
}

/// Cross-entity effect through a typed writer.
#[system(name = "game_sim::sword_hit", worlds = [server, prediction])]
pub fn sword_hit(damage: EventWriter<Damage>, hits: &HitList, key: StableKeyOf) {
    for h in hits.iter() {
        if let Some(target) = h.key.get() {
            damage.emit(Damage { target, source: *key, amount: h.amount });
        }
    }
}

#[system(name = "game_sim::apply_damage", worlds = [server, prediction], after = sword_hit)]
pub fn apply_damage(events: Events<Damage>, key: StableKeyOf, hp: Mut<Health>) {
    for d in events.for_target(*key) {
        hp.0 -= d.amount;
    }
}

/// Presentation event keyed for rollback.
#[system(name = "game_sim::footsteps", worlds = [server, prediction], after = animate)]
pub fn footsteps(present: PresentEvents, key: StableKeyOf, anim: &AnimState) {
    if anim.phase_crossed(8) {
        present.emit(*key, AudioRequest::FOOTSTEP);
    }
}

/// Advances every animation phase.
#[system(name = "game_sim::animate", worlds = [server, prediction])]
pub fn animate(anim: Mut<AnimState>) {
    anim.phase += 1;
}

/// The input sync point: the player's input is for this tick's slot.
#[system(name = "game_sim::read_input", worlds = [server, prediction], exclusive)]
pub fn read_input(world: &mut World) {
    let tick = world.tick();
    world.set_input_tick(PLAYER, tick);
}

/// The physics stage reads the physics channel the kernels wrote.
#[system(name = "game_sim::physics", worlds = [server, prediction])]
pub fn physics(impulses: Channel<Physics>, count: ResMut<ImpulseCount>) {
    count.0 += impulses.len() as u32;
}

/// The schedule of the section 6.1 systems: input, simulation, physics.
pub fn game_schedule(kind: WorldKind) -> Schedule {
    let mut builder = Schedule::builder(kind);
    builder.stage("input", &[&sets::INPUT]).add(&READ_INPUT);
    builder
        .stage("simulation", &[&sets::MOVEMENT])
        .add(&APPLY_GRAVITY)
        .add(&INTEGRATE)
        .add(&TOTAL_MASS)
        .add(&THROW_BOMB)
        .add(&SWORD_HIT)
        .add(&APPLY_DAMAGE)
        .add(&FOOTSTEPS)
        .add(&ANIMATE);
    builder.stage("physics", &[]).add(&PHYSICS);
    builder.build().unwrap()
}

/// The player `net(1)`, claimed by `PLAYER`, about to throw a bomb and hitting the
/// unclaimed `net(2)`.
pub fn populate(world: &mut World) {
    let player = net(1);
    let npc = net(2);
    let hit = |key: StableKey, amount| Hit { key: EntityRef::new(key), amount };
    let none = Hit { key: EntityRef::NONE, amount: 0 };
    world
        .spawn(
            player,
            REGION,
            (
                pos(0, 100),
                vel(1, 0),
                Mass(Scalar::from_i32(3)),
                Health(100),
                HeldItem { kind: ItemKind::Bomb },
                PlayerInput {
                    aim: Vec2::new(Scalar::from_i32(4), Scalar::TWO),
                    throw: true,
                    _pad: pad(),
                },
                HitList { hits: PlainArray([hit(npc, 10), none]), len: 1 },
                AnimState { phase: 7 },
            ),
        )
        .unwrap();
    world.insert(player, ClaimedBy { player: PLAYER }).unwrap();
    world
        .spawn(
            npc,
            REGION,
            (
                pos(5, 100),
                vel(0, 0),
                Mass(Scalar::from_i32(5)),
                Health(100),
                AnimState { phase: 3 },
                HitList { hits: PlainArray([none, none]), len: 0 },
            ),
        )
        .unwrap();
}

// ---- Helpers ----

pub const REGION: RegionId = RegionId(10);
pub const OTHER_REGION: RegionId = RegionId(20);
pub const PLAYER: PlayerId = PlayerId(3);

/// Every type above, and the world's own.
pub fn base_registry() -> Registry {
    let mut registry = Registry::new();
    registry.register::<InRegion>().unwrap();
    registry.register::<OwnedBy>().unwrap();
    registry.register::<ClaimedBy>().unwrap();
    registry.register::<PredictedOrigin>().unwrap();
    registry.register::<Position>().unwrap();
    registry.register::<Velocity>().unwrap();
    registry.register::<Mass>().unwrap();
    registry.register::<Health>().unwrap();
    registry.register::<Counter>().unwrap();
    registry.register::<Bomb>().unwrap();
    registry.register::<ThrownBy>().unwrap();
    registry.register::<HeldItem>().unwrap();
    registry.register::<PlayerInput>().unwrap();
    registry.register::<HitList>().unwrap();
    registry.register::<AnimState>().unwrap();
    registry.register::<Scratch>().unwrap();
    registry.register::<TickDelta>().unwrap();
    registry.register::<Gravity>().unwrap();
    registry.register::<TotalMass>().unwrap();
    registry.register::<ImpulseCount>().unwrap();
    registry.register::<Weather>().unwrap();
    registry.register::<RegionScratch>().unwrap();
    registry
}

/// The document schema of every component of `registry`, as `lactose-persist` writes it.
pub fn schemas(registry: &Registry) -> Vec<lactose_bundle::ComponentSchema> {
    registry
        .iter()
        .map(|info| {
            let fields = info.layout().fields().iter().filter_map(|field| {
                Some(lactose_bundle::FieldLayout {
                    id: field.id()?,
                    offset: field.offset() as u32,
                    size: field.size() as u32,
                })
            });
            lactose_bundle::ComponentSchema::new(
                info.key(),
                info.version(),
                info.layout().size() as u32,
                fields,
            )
            .unwrap()
        })
        .collect()
}

/// The registry of every type above.
pub fn registry() -> Arc<Registry> {
    Arc::new(base_registry())
}

/// A world of `kind` with `layout`, one simulated region and the resources the systems
/// above read.
pub fn world(kind: WorldKind, layout: LayoutConfig) -> World {
    world_with(kind, layout, registry())
}

/// [`world`] with another registry.
pub fn world_with(kind: WorldKind, layout: LayoutConfig, registry: Arc<Registry>) -> World {
    let mut world = WorldBuilder::new(kind, registry).layout(layout).build().unwrap();
    world
        .create_region(RegionConfig { id: REGION, seed: 0x5EED, mode: RegionMode::Simulated })
        .unwrap();
    world.insert_resource(TickDelta(Scalar::ONE));
    world.insert_resource(Gravity(Scalar::TWO));
    world.insert_resource(TotalMass(Scalar::ZERO));
    world.insert_resource(ImpulseCount(0));
    world
}

/// A small executor: one worker of each class.
pub fn executor() -> Executor {
    Executor::builder().frame_workers(1).background_workers(1).build().unwrap()
}

/// Runs one tick and hands the world back.
pub fn tick(schedule: &mut Schedule, world: World, executor: &Executor) -> World {
    executor.block_on(schedule.run_tick(world, executor)).unwrap()
}

/// The layouts SOUR's layout mode runs: every capacity, chunk order and permutation, with
/// batch splitting. Under Miri a representative few.
pub fn layouts() -> Vec<LayoutConfig> {
    let mut layouts = Vec::new();
    for chunk_rows in [Some(1), Some(7), Some(64), None] {
        for chunk_order in [ChunkOrder::Natural, ChunkOrder::Reversed, ChunkOrder::Shuffled] {
            for permute_rows in [false, true] {
                layouts.push(LayoutConfig {
                    chunk_rows,
                    seed: 0xC0FFEE,
                    permute_rows,
                    chunk_order,
                    split_batches: permute_rows,
                });
            }
        }
    }
    if cfg!(miri) { layouts.into_iter().step_by(7).collect() } else { layouts }
}

/// A `NetId` key.
pub fn net(id: u64) -> StableKey {
    StableKey::Net(lactose_core::NetId::from_bits(id).unwrap())
}

/// A tick value.
pub const fn t(tick: u32) -> Tick {
    Tick(tick)
}

/// A position.
pub fn pos(x: i32, y: i32) -> Position {
    Position { x: Scalar::from_i32(x), y: Scalar::from_i32(y) }
}

/// A velocity.
pub fn vel(x: i32, y: i32) -> Velocity {
    Velocity { x: Scalar::from_i32(x), y: Scalar::from_i32(y) }
}

/// No padding.
pub fn pad<const N: usize>() -> Padding<N> {
    Padding::default()
}

// ---- What a refused write must leave unchanged ----

/// One chunk's column, as the world stores it.
#[derive(PartialEq, Debug)]
pub struct Column {
    component: ComponentKey,
    keys: Vec<StableKey>,
    regions: Vec<RegionId>,
    bytes: Vec<u8>,
    row_ticks: Option<Vec<Tick>>,
    column_tick: Tick,
}

/// Everything a direct write can change that a consumer can observe, for the keys and
/// players a test names.
#[derive(PartialEq, Debug)]
pub struct WorldState {
    mark: ChangeMark,
    columns: Vec<Column>,
    /// For each mark, the rows of every tracked column changed since it: the stamps'
    /// points, which column ticks alone do not show.
    changed: Vec<Option<Vec<(ComponentKey, StableKey)>>>,
    log: Vec<LogEntry>,
    referrers: Vec<Vec<Referrer>>,
    owned: Vec<Vec<StableKey>>,
    claimed: Vec<Vec<StableKey>>,
    live: Vec<bool>,
}

/// The observable state of `world`: every column with its stamps and ticks, the rows changed
/// since each of `marks`, the structural log, and the reverse index, the ownership and the
/// claims of `keys` and `players`.
pub fn world_state(
    world: &World,
    marks: &[ChangeMark],
    keys: &[StableKey],
    players: &[PlayerId],
) -> WorldState {
    let components: Vec<ComponentKey> = world
        .registry()
        .iter()
        .filter(|info| info.role() == TypeRole::Component)
        .map(|info| info.key())
        .collect();
    let mut columns = Vec::new();
    for &component in &components {
        for column in world.columns_erased(component) {
            columns.push(Column {
                component,
                keys: column.keys().to_vec(),
                regions: column.regions().to_vec(),
                bytes: column.bytes().to_vec(),
                row_ticks: column.row_ticks().map(<[_]>::to_vec),
                column_tick: column.column_tick(),
            });
        }
    }
    let changed = marks
        .iter()
        .map(|&mark| {
            let mut rows = Vec::new();
            for &component in &components {
                for column in world.columns_erased(component) {
                    if column.row_ticks().is_none() {
                        continue;
                    }
                    for row in column.changed_since(mark)? {
                        rows.push((component, column.keys()[row]));
                    }
                }
            }
            rows.sort();
            Some(rows)
        })
        .collect();
    let mut cursor = world.structural_log().cursor_at_start();
    let log =
        world.structural_log().since(&mut cursor).expect("a current cursor").copied().collect();
    WorldState {
        mark: world.mark(),
        columns,
        changed,
        log,
        referrers: keys.iter().map(|&key| world.referrers(key).collect()).collect(),
        owned: keys.iter().map(|&key| world.owned_by(key).collect()).collect(),
        claimed: players.iter().map(|&player| world.claimed(player).collect()).collect(),
        live: keys.iter().map(|&key| world.contains(key)).collect(),
    }
}
