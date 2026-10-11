//! What the integration tests share: a deterministic generator of valid samples, the
//! client's quantiser as `lactose-input-host` states it, and a small game on top of this
//! crate (a kernel that records what it reads from `&PlayerInput`, a session applier, a
//! scripted client per player and the frames a server and a prediction world are fed), so
//! each test states what the frame carried and asserts on what the kernel saw and on the
//! world's hash.

#![allow(dead_code, reason = "each test file uses part of the shared module")]

use lactose_core::{NetId, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{
    ClaimedBy, Executor, LayoutConfig, Mut, RegionConfig, RegionId, RegionMode, World, WorldKind,
    system,
};
use lactose_input::{
    AXIS_SCALE, Action, Actions, Axis2, Pitch, PlayerInput, PlayerInputApplier, Yaw, player_entity,
};
use lactose_math::{Quat, Scalar, Vec3};
use lactose_reflect::component;
use lactose_sim::{
    PlayerSample, Registration, SessionApplier, SessionEvent, Simulation, SimulationBuilder, Stage,
    StreamingAnchor, TickInput, TypeTable,
};

/// The region every entity of the programs lives in, created before the first tick.
pub const REGION: RegionId = RegionId(1);
/// A region never loaded: a `Join` for it is held, so its player has samples and no row.
pub const ELSEWHERE: RegionId = RegionId(2);

/// Joins with `PlayerInput::IDLE` among its components; skips ticks; leaves and rejoins.
pub const IDLE_SPAWNED: PlayerId = PlayerId(1);
/// Joins without the component, so the applier's first `World::insert` adds it; sends a
/// malformed image every `MALFORMED_EVERY` ticks.
pub const BARE_SPAWNED: PlayerId = PlayerId(2);
/// Joins a region that never loads: samples, no entity.
pub const HELD: PlayerId = PlayerId(3);
/// Never joins: samples, no entity.
pub const STRANGER: PlayerId = PlayerId(4);
/// Claims two anchored rows and a lesser row without an anchor, spawned directly: the
/// lookup must skip the unanchored row and take the lesser anchored key.
pub const TWICE: PlayerId = PlayerId(9);

/// `TWICE`'s claimed row without an anchor: the least key it claims.
pub const UNANCHORED: u64 = 10;
/// `TWICE`'s lesser anchored row: its entity.
pub const LESSER: u64 = 20;
/// `TWICE`'s greater anchored row.
pub const GREATER: u64 = 25;
/// The local player's entity in the prediction programs, spawned as `lactose-predict`
/// spawns it: claimed and anchored.
pub const LOCAL_ENTITY: u64 = 100;

/// Ticks of a program.
pub const TICKS: u32 = 60;
/// `IDLE_SPAWNED` sends nothing on these ticks.
pub const GAP_EVERY: u32 = 7;
/// `BARE_SPAWNED`'s image is malformed on these ticks.
pub const MALFORMED_EVERY: u32 = 4;
/// `BARE_SPAWNED`'s first sample.
pub const BARE_FIRST: u32 = 3;
/// `IDLE_SPAWNED` leaves at this tick and joins again at `REJOIN`.
pub const LEAVE: u32 = 40;
pub const REJOIN: u32 = 45;

/// A cooked `Net` key.
pub fn key(n: u64) -> StableKey {
    StableKey::Net(NetId::cooked(n))
}

pub fn executor(workers: u32) -> Executor {
    Executor::builder().frame_workers(workers).background_workers(1).build().unwrap()
}

/// SplitMix64: a reproducible stream for sweeps and scripts.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Uniform in `lo..=hi`.
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        let span = (i64::from(hi) - i64::from(lo) + 1) as u64;
        (i64::from(lo) + (self.next_u64() % span) as i64) as i32
    }
}

/// A movement vector inside the valid disc, by rejection, with zero, the axes and the
/// disc's edge over-represented.
pub fn valid_movement(rng: &mut Rng) -> Axis2 {
    let s = i64::from(AXIS_SCALE);
    match rng.next_u32() % 6 {
        0 => Axis2::ZERO,
        1 => Axis2 { x: [AXIS_SCALE, -AXIS_SCALE, 0][rng.next_u32() as usize % 3], y: 0 },
        2 => Axis2 { x: 0, y: [AXIS_SCALE, -AXIS_SCALE][rng.next_u32() as usize % 2] },
        _ => loop {
            let bound = i32::from(AXIS_SCALE);
            let (x, y) = (rng.range(-bound, bound), rng.range(-bound, bound));
            let (wx, wy) = (i64::from(x), i64::from(y));
            if wx * wx + wy * wy <= s * s + 2 * s {
                break Axis2 { x: x as i16, y: y as i16 };
            }
        },
    }
}

/// A sample `validate` accepts: any yaw, a pitch within the limit, any held set, edges
/// consistent with it.
pub fn valid_sample(rng: &mut Rng) -> PlayerInput {
    let held = Actions::from_bits(rng.next_u32());
    PlayerInput {
        movement: valid_movement(rng),
        yaw: Yaw(rng.next_u32() as u16),
        pitch: Pitch(rng.range(-i32::from(Pitch::LIMIT), i32::from(Pitch::LIMIT)) as i16),
        held,
        pressed: Actions::from_bits(rng.next_u32()).intersection(held),
        released: Actions::from_bits(rng.next_u32()).without(held),
    }
}

/// The movement half of `Quantiser::next` as `lactose-input-host` states it, in `f32`: a
/// NaN component counts as zero, the vector is scaled down to the unit disc if longer,
/// its length computed without overflow (here by dividing by the larger component first,
/// as `hypot` does, which the boundary lints forbid), then each component times
/// `AXIS_SCALE`, rounded to nearest, ties away from zero, saturating.
pub fn quantise_movement(x: f32, y: f32) -> Axis2 {
    let (x, y) = (if x.is_nan() { 0.0 } else { x }, if y.is_nan() { 0.0 } else { y });
    let larger = if x.abs() > y.abs() { x.abs() } else { y.abs() };
    // A component beyond one makes the vector longer than one: scale it down by the larger
    // component first, so the squares cannot overflow.
    let (x, y) = if larger > 1.0 {
        let (x, y) = (x / larger, y / larger);
        let length = (x * x + y * y).sqrt();
        (x / length, y / length)
    } else {
        let length = (x * x + y * y).sqrt();
        if length > 1.0 { (x / length, y / length) } else { (x, y) }
    };
    let quantise = |c: f32| (c * f32::from(AXIS_SCALE)).round() as i16;
    Axis2 { x: quantise(x), y: quantise(y) }
}

/// `Scalar`s compared by their canonical bits.
pub fn same_bits(a: Scalar, b: Scalar) -> bool {
    a.to_canonical_bits() == b.to_canonical_bits()
}

pub fn same_vec3(a: Vec3, b: Vec3) -> bool {
    same_bits(a.x, b.x) && same_bits(a.y, b.y) && same_bits(a.z, b.z)
}

pub fn same_quat(a: Quat, b: Quat) -> bool {
    same_bits(a.x, b.x) && same_bits(a.y, b.y) && same_bits(a.z, b.z) && same_bits(a.w, b.w)
}

/// What the observing kernel read from its row's `&PlayerInput` on the last tick it ran,
/// and counts over the run: persistent, so the world hash covers every value a kernel
/// derives from a sample.
#[component(key = "test::seen", persistent)]
#[derive(PartialEq, Debug, Default)]
pub struct Seen {
    #[field(id = 1)]
    pub aim: Vec3,
    #[field(id = 2)]
    pub walk: Vec3,
    /// The sum of every tick's `walk`: diverges for good once one tick did.
    #[field(id = 3)]
    pub walked: Vec3,
    #[field(id = 4)]
    pub facing: Quat,
    #[field(id = 5)]
    pub movement: Axis2,
    #[field(id = 6)]
    pub yaw: Yaw,
    #[field(id = 7)]
    pub pitch: Pitch,
    #[field(id = 8)]
    pub held: Actions,
    #[field(id = 9)]
    pub pressed: Actions,
    #[field(id = 10)]
    pub released: Actions,
    /// Ticks the kernel ran on this row.
    #[field(id = 11)]
    pub runs: u32,
    /// Ticks with `Throw` pressed.
    #[field(id = 12)]
    pub throws: u32,
    /// Ticks with `Throw` released.
    #[field(id = 13)]
    pub releases: u32,
}

impl Seen {
    /// Whether the last tick's reading is `input`'s: its integers exactly and every derived
    /// value bit for bit.
    pub fn reads(&self, input: &PlayerInput) -> bool {
        same_vec3(self.aim, input.aim())
            && same_vec3(self.walk, input.world_movement())
            && same_quat(self.facing, input.yaw.rotation())
            && self.movement == input.movement
            && self.yaw == input.yaw
            && self.pitch == input.pitch
            && self.held == input.held
            && self.pressed == input.pressed
            && self.released == input.released
    }

    /// The bytes of the record, for comparing two worlds bit for bit.
    pub fn image(&self) -> Vec<u8> {
        lactose_reflect::bytemuck::bytes_of(self).to_vec()
    }
}

/// The game kernel: takes `&PlayerInput` in `Stage::Simulation` and records everything a
/// kernel can derive from it.
#[system(name = "test::observe", worlds = [server, prediction])]
fn observe(input: &PlayerInput, seen: Mut<Seen>) {
    let walk = input.world_movement();
    seen.aim = input.aim();
    seen.walk = walk;
    seen.walked += walk;
    seen.facing = input.yaw.rotation();
    seen.movement = input.movement;
    seen.yaw = input.yaw;
    seen.pitch = input.pitch;
    seen.held = input.held;
    seen.pressed = input.pressed;
    seen.released = input.released;
    seen.runs += 1;
    seen.throws += u32::from(input.pressed.contains(Action::Throw));
    seen.releases += u32::from(input.released.contains(Action::Throw));
}

pub fn register_game(registration: &mut Registration) {
    registration.stage(Stage::Simulation).add(&OBSERVE);
}

/// The session applier: a `Join` spawns the player's entity under a fresh `NetId` with the
/// claim and the anchor, and with `PlayerInput::IDLE` for every player but
/// `BARE_SPAWNED`; a `Leave` despawns what `player_entity` names.
pub struct Sessions;

impl SessionApplier for Sessions {
    fn apply(&mut self, world: &mut World, player: PlayerId, event: SessionEvent) -> Result<()> {
        match event {
            SessionEvent::Join { region } => {
                let key = StableKey::Net(world.allocate_net_id()?);
                let (claim, anchor) = (ClaimedBy { player }, StreamingAnchor { priority: 0 });
                if player == BARE_SPAWNED {
                    world.spawn(key, region, (Seen::default(), claim, anchor))
                } else {
                    world.spawn(key, region, (PlayerInput::IDLE, Seen::default(), claim, anchor))
                }
            }
            SessionEvent::Leave => match player_entity(world, player) {
                Some(key) => world.despawn(key),
                None => Ok(()),
            },
        }
    }
}

/// The type table of every world here.
pub fn types() -> TypeTable {
    let mut types = TypeTable::new();
    lactose_input::register_types(&mut types).unwrap();
    types.register::<Seen>().unwrap();
    types
}

/// A builder with this crate and the game wired in as `app` wires them.
pub fn builder(kind: WorldKind, layout: LayoutConfig) -> SimulationBuilder {
    SimulationBuilder::new(kind, types())
        .layout(layout)
        .systems(lactose_input::register_systems)
        .systems(register_game)
        .inputs(Box::new(PlayerInputApplier::new()))
        .sessions(Box::new(Sessions))
}

/// Spawns `TWICE`'s three claimed rows, each with an idle sample, so the kernel runs on
/// all three and a write to the wrong one shows in the hash.
pub fn spawn_twice(world: &mut World) {
    let (claim, anchor) = (ClaimedBy { player: TWICE }, StreamingAnchor { priority: 0 });
    let row = || (PlayerInput::IDLE, Seen::default(), claim);
    world.spawn(key(GREATER), REGION, (PlayerInput::IDLE, Seen::default(), claim, anchor)).unwrap();
    world.spawn(key(UNANCHORED), REGION, row()).unwrap();
    world.spawn(key(LESSER), REGION, (PlayerInput::IDLE, Seen::default(), claim, anchor)).unwrap();
}

/// The server program's world: `REGION` created and `TWICE`'s rows spawned.
pub fn build_server(layout: LayoutConfig) -> Simulation {
    let mut simulation = builder(WorldKind::Server, layout).build().unwrap();
    let world = simulation.world_mut();
    world.create_region(RegionConfig { id: REGION, seed: 7, mode: RegionMode::Simulated }).unwrap();
    spawn_twice(world);
    simulation
}

/// The prediction program's world: `REGION` created, the local player `IDLE_SPAWNED`'s
/// entity spawned as `lactose-predict` spawns it, and `TWICE`'s rows.
pub fn build_prediction(layout: LayoutConfig) -> Simulation {
    let mut simulation = builder(WorldKind::Prediction, layout).build().unwrap();
    let world = simulation.world_mut();
    world.create_region(RegionConfig { id: REGION, seed: 7, mode: RegionMode::Simulated }).unwrap();
    let (claim, anchor) = (ClaimedBy { player: IDLE_SPAWNED }, StreamingAnchor { priority: 0 });
    world
        .spawn(key(LOCAL_ENTITY), REGION, (PlayerInput::IDLE, Seen::default(), claim, anchor))
        .unwrap();
    spawn_twice(world);
    simulation
}

/// The client's held set at `tick`: some of the named actions and one of the game's bits,
/// changing every few ticks, so some ticks carry edges and some do not.
fn held_at(player: PlayerId, tick: u32) -> Actions {
    if tick == 0 {
        return Actions::EMPTY;
    }
    let mut rng = Rng::new(u64::from(player.0) << 32 | u64::from(tick / 3));
    let bits = rng.next_u32();
    let named = bits & ((1 << Action::NAMED) - 1);
    let game = if bits & (1 << 20) != 0 { 1 << (Actions::GAME_FIRST + 2) } else { 0 };
    Actions::from_bits(named | game)
}

/// The scripted client's sample for `tick` (from 1), as its quantiser produces it:
/// movement in the disc, an absolute yaw that keeps turning, a pitch within the limit, and
/// edges from the previous tick's held set.
pub fn client_sample(player: PlayerId, tick: u32) -> PlayerInput {
    let mut rng = Rng::new(0x00c1_1e47 ^ (u64::from(player.0) << 40) ^ u64::from(tick));
    let held = held_at(player, tick);
    let previous = held_at(player, tick - 1);
    PlayerInput {
        movement: valid_movement(&mut rng),
        yaw: Yaw(tick.wrapping_mul(1237).wrapping_add(player.0 * 977) as u16),
        pitch: Pitch(rng.range(-i32::from(Pitch::LIMIT), i32::from(Pitch::LIMIT)) as i16),
        held,
        pressed: held.without(previous),
        released: previous.without(held),
    }
}

/// The malformed images the applier must drop, one per way to be malformed, cycled by tick.
pub fn malformed(tick: u32) -> Vec<u8> {
    let valid = client_sample(BARE_SPAWNED, tick);
    let image = valid.to_sample_bytes();
    let forged = |input: PlayerInput| input.to_sample_bytes().to_vec();
    match tick / MALFORMED_EVERY % 7 {
        0 => image[..19].to_vec(),
        1 => [&image[..], &[0]].concat(),
        2 => forged(PlayerInput { pitch: Pitch(Pitch::LIMIT + 1), ..valid }),
        3 => forged(PlayerInput { movement: Axis2 { x: i16::MIN, y: 0 }, ..valid }),
        4 => forged(PlayerInput { movement: Axis2 { x: AXIS_SCALE, y: AXIS_SCALE }, ..valid }),
        5 => forged(PlayerInput {
            held: Actions::EMPTY,
            pressed: Actions::from_bits(0b10),
            released: Actions::EMPTY,
            ..valid
        }),
        _ => forged(PlayerInput {
            held: Actions::from_bits(1),
            pressed: Actions::EMPTY,
            released: Actions::from_bits(1),
            ..valid
        }),
    }
}

/// What a variant of the server program feeds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Program {
    /// `BARE_SPAWNED`'s malformed images are in the frame (`true`), or the entry is absent.
    pub malformed: bool,
    /// Samples for `HELD` and `STRANGER`, who have no entity, are in the frame.
    pub strays: bool,
}

impl Program {
    pub const FULL: Self = Self { malformed: true, strays: true };
}

/// Whether `IDLE_SPAWNED`'s client sends a sample for `tick`.
pub fn idle_spawned_sends(tick: u32) -> bool {
    !tick.is_multiple_of(GAP_EVERY)
}

/// Whether `BARE_SPAWNED`'s entry at `tick` is a valid sample.
pub fn bare_spawned_valid(tick: u32) -> bool {
    tick >= BARE_FIRST && !tick.is_multiple_of(MALFORMED_EVERY)
}

/// The server's frame for `tick`.
pub fn server_frame(program: Program, tick: u32) -> TickInput {
    let mut frame = TickInput::default();
    if tick == 1 {
        frame.sessions.insert(IDLE_SPAWNED, SessionEvent::Join { region: REGION });
        frame.sessions.insert(BARE_SPAWNED, SessionEvent::Join { region: REGION });
        frame.sessions.insert(HELD, SessionEvent::Join { region: ELSEWHERE });
    }
    if tick == LEAVE {
        frame.sessions.insert(IDLE_SPAWNED, SessionEvent::Leave);
    }
    if tick == REJOIN {
        frame.sessions.insert(IDLE_SPAWNED, SessionEvent::Join { region: REGION });
    }
    let entry = |player: PlayerId| client_sample(player, tick).to_player_sample(Tick(tick));
    if idle_spawned_sends(tick) {
        frame.inputs.insert(IDLE_SPAWNED, entry(IDLE_SPAWNED));
    }
    if bare_spawned_valid(tick) {
        frame.inputs.insert(BARE_SPAWNED, entry(BARE_SPAWNED));
    } else if tick >= BARE_FIRST && program.malformed {
        let sample =
            PlayerSample { input_tick: Tick(tick), substituted: false, sample: malformed(tick) };
        frame.inputs.insert(BARE_SPAWNED, sample);
    }
    if program.strays {
        frame.inputs.insert(HELD, entry(HELD));
        frame.inputs.insert(STRANGER, entry(STRANGER));
    }
    frame.inputs.insert(TWICE, entry(TWICE));
    frame
}

/// The prediction world's frame for `tick`: the local player's own sample, the bytes the
/// server receives, up to the tick it leaves; `TWICE`'s, as if forwarded.
pub fn prediction_frame(tick: u32) -> TickInput {
    let mut frame = TickInput::default();
    let entry = |player: PlayerId| client_sample(player, tick).to_player_sample(Tick(tick));
    if idle_spawned_sends(tick) && tick < LEAVE {
        frame.inputs.insert(IDLE_SPAWNED, entry(IDLE_SPAWNED));
    }
    frame.inputs.insert(TWICE, entry(TWICE));
    frame
}

/// Runs the server program under `layout` on `workers`, calling `check` after every tick,
/// and returns the hash after every tick.
pub fn run_server(
    program: Program,
    layout: LayoutConfig,
    workers: u32,
    check: &mut dyn FnMut(u32, &Simulation),
) -> Vec<u64> {
    let executor = executor(workers);
    let mut simulation = build_server(layout);
    let mut hashes = Vec::new();
    for tick in 1..=TICKS {
        let frame = server_frame(program, tick);
        simulation = executor.block_on(simulation.step(&executor, frame)).unwrap();
        check(tick, &simulation);
        hashes.push(simulation.hash().unwrap());
    }
    hashes
}

/// Runs the prediction program as `run_server` runs the server's.
pub fn run_prediction(
    layout: LayoutConfig,
    workers: u32,
    check: &mut dyn FnMut(u32, &Simulation),
) -> Vec<u64> {
    let executor = executor(workers);
    let mut simulation = build_prediction(layout);
    let mut hashes = Vec::new();
    for tick in 1..=TICKS {
        simulation = executor.block_on(simulation.step(&executor, prediction_frame(tick))).unwrap();
        check(tick, &simulation);
        hashes.push(simulation.hash().unwrap());
    }
    hashes
}

/// The layouts of SOUR's layout mode (`lactose_sim::harness::layout_mode`) for every
/// capacity the matrix names, each under several seeds, the runner's among them, and the
/// production layout.
pub fn layouts() -> Vec<(String, LayoutConfig)> {
    use lactose_sim::harness::{ChunkCapacity, layout_mode};
    let capacities = [
        ChunkCapacity::Rows(1),
        ChunkCapacity::Rows(7),
        ChunkCapacity::Rows(64),
        ChunkCapacity::Bytes(16 * 1024),
    ];
    let mut layouts = vec![("production".to_owned(), LayoutConfig::default())];
    for capacity in capacities {
        for seed in [1, 0x5eed_1a70, 0xdead_beef] {
            let layout = layout_mode(capacity, seed).unwrap();
            layouts.push((format!("{capacity:?} seed {seed:#x}"), layout));
        }
    }
    layouts
}
