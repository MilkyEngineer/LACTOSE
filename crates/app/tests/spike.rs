//! The milestone 1 netcode spike (plan decision 5.3): a server and a client built by this
//! crate in one process, over a transport the test implements, not a socket. The link
//! between them delays every datagram by three frames each way (100 ms round trip), loses
//! about one message in a hundred, and orders the datagrams delivered in one frame by a
//! draw: all from its seed. The client's clock runs half a percent slow, wakes late on a
//! quarter of its frames and stalls once, from a seed of its own, and reaches the timer
//! root only through `ClockEstimate` (`Client::clock`), which the spike's estimator
//! computes from its own round trip. The spike's packer sends every entity the player has
//! claimed whole on every update (D121, architecture section 9.4) and the others as the
//! rows changed since their acknowledged tick; its input packet carries the last eight
//! samples.
//!
//! Four scenes: a dynamic capsule character walking on static geometry, a kinematic one,
//! fifty claimed dynamic bodies thrown and interacting, and a claim of a crate the
//! character pushes and of a stack it bumps. Each runs under SOUR as a recording test,
//! which records after every tick a hash of both worlds and of every datagram either end
//! sent that frame and, as a stream of its own named as the test with `::physics` appended,
//! a hash of both worlds' physics bytes (D145). The ignored measurement tests replay each
//! scene to 36,000 ticks in release, with the clock script and on the server's clock,
//! computing no physics hash, and print the figures of each run as one JSON object; one
//! more, not recorded, measures a claim made on approach, before the character touches the
//! crate.
//!
//! The seeds of the link and of the clock script are the test's, outside both worlds.
//! Inside them every reference, ordering rule and RNG key is a `StableKey`.

#![forbid(unsafe_code)]
#![allow(missing_docs)]

use crate::protocol;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::num::NonZeroU32;
use std::ops::Range;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use lactose_app::{
    Client, ClientError, ClientOutput, Cooked, CrateRegistration, Game, Start, TICK_PERIOD,
    Workers, WorldConfig, build_client, build_server, start_executor,
};
use lactose_asset_api::{
    Asset, AssetId, AssetKind, AssetSet, AssetVersion, LoadPriority, PinnedTable, PublishedAssets,
    Readiness, UntypedHandle, VersionTable,
};
use lactose_bundle::{Baseline, ComponentData, Overlay};
use lactose_core::{
    ComponentKey, Error, ErrorKind, NetId, PlayerId, PredictionKey, Result, StableKey, Tick, xxh64,
};
use lactose_ecs::{
    ClaimedBy, Commands, Ctx, LayoutConfig, Mut, RegionId, RegionMode, Res, StableKeyOf,
    StructuralChange, World, system,
};
use lactose_input::{
    AXIS_SCALE, Action, Actions, Axis2, PlayerInput, SampleHistory, player_entity,
};
use lactose_math::{Scalar, Vec3};
use lactose_net_api::{
    InterpolationSlot, OverlayPayload, RegionChange, RegionRelevance, StagedFrame, StagedSpawn,
    StagingFrame, Tombstone, TombstoneCause,
};
use lactose_net_host::interest::{ClientInterest, SendBudget};
use lactose_net_host::interpolation::{InterpolationReader, InterpolationWriter};
use lactose_net_host::message::{JoinAccepted, ResyncReason, ResyncRequest};
use lactose_net_host::packet::{
    AckField, ConnectionHandles, EntityHandle, PacketRecord, PacketSequence,
};
use lactose_net_host::transport::{
    ConnectionId, Datagram, LinkStats, MAX_DATAGRAM_BYTES, Transport, TransportEvent,
};
use lactose_net_host::{ClockEstimate, InterpolationBuffer};
use lactose_physics_api::{
    BodyType, Collider, RigidBody, Shape, Teleported, Transform, Velocity, locked_axes, snapshot,
};
use lactose_predict::{CorrectionCause, PredictionStats, PredictionTolerance, SlotBytes};
use lactose_reflect::{Persistence, Reflect, TypeInfo, TypeRole, component};
use lactose_sim::harness::{ChunkCapacity, layout_mode, layout_mode_of};
use lactose_sim::{
    PlayerSample, RegionBundle, Registration, SessionApplier, SessionEvent, Stage, StreamingAnchor,
    TickDelta, TickInput, TimeScale, TypeTable,
};
use lactose_task::{ChaosConfig, Executor};
use protocol::{Capacity, Layout, RunConfig, Session};

// ---- The scenes' game ----

/// The player, the one region and the one connection.
const PLAYER: PlayerId = PlayerId(1);
const REGION: RegionId = RegionId(1);
const CONNECTION: ConnectionId = ConnectionId(1);

/// Every scene repeats its script every five seconds of ticks: the character is put back
/// at home at each cycle's first tick, and the crates with it.
const CYCLE: u32 = 300;
/// The bodies one throw spawns, and the ticks each lives.
const THROWN: u32 = 50;
const LIFETIME: u32 = 240;
/// The phases of a cycle at which the server claims the lone crate, then the stack, and
/// releases them all.
const CLAIM_LONE: u32 = 100;
const CLAIM_STACK: u32 = 160;
const RELEASE: u32 = 260;
/// The phase at which the server claims the lone crate on approach: some twenty ticks
/// before the character reaches it, more than the lead.
const CLAIM_APPROACH: u32 = 60;
/// `Hop` states: idle, set in `Stage::Post` once the character is past the line, spent by
/// the next tick's steering as a jump.
const HOP_IDLE: u32 = 0;
const HOP_PENDING: u32 = 1;
const HOP_SPENT: u32 = 2;

/// A body's remaining ticks; the body despawns at the last.
#[component(key = "spike::lifetime", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct Lifetime(#[field(id = 1)] pub u32);

/// A crate of the claim scene: zero for the lone crate, one to three up the stack.
#[component(key = "spike::pushable", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct Pushable(#[field(id = 1)] pub u32);

/// The carrier column (D126): written in `Stage::Post` once the character crosses the
/// line, read by the next tick's steering, which turns it into a jump.
#[component(key = "spike::hop", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct Hop(#[field(id = 1)] pub u32);

fn scalar(value: f64) -> Scalar {
    Scalar::from_f64(value)
}

/// Where the character stands at each cycle's start.
fn home() -> Vec3 {
    Vec3::new(Scalar::ZERO, scalar(0.95), Scalar::ZERO)
}

/// Where crate `index` rests at each cycle's start: the lone crate ahead of the character,
/// the stack further on, a little to its right.
fn crate_home(index: u32) -> Vec3 {
    if index == 0 {
        Vec3::new(Scalar::ZERO, scalar(0.4), scalar(-2.5))
    } else {
        let height = scalar(0.4) + scalar(0.801) * Scalar::from_u32(index - 1);
        Vec3::new(scalar(0.1), height, scalar(-5.5))
    }
}

/// Steers the character by its input: a turn of up to two radians a second and a walk of
/// up to four metres a second along its facing, through `RootMotion` as the anim crate
/// defines it, applied to the `Transform` of a kinematic body or to the `Velocity` of a
/// dynamic one; a pending `Hop` becomes a jump. The first tick of every cycle puts the
/// character back at home.
#[system(name = "spike::steer", worlds = [server, prediction])]
fn steer(
    ctx: Ctx,
    dt: Res<TickDelta>,
    input: &PlayerInput,
    body: &RigidBody,
    hop: Mut<Hop>,
    transform: Mut<Transform>,
    velocity: Mut<Velocity>,
) {
    if ctx.tick().0 % CYCLE == 0 {
        transform.set_if_neq(Transform::at(home()));
        velocity.set_if_neq(Velocity::ZERO);
        hop.set_if_neq(Hop(HOP_IDLE));
        return;
    }
    let axis = Scalar::from_i32(i32::from(AXIS_SCALE));
    let turn = Scalar::from_i32(i32::from(input.movement.x)) / axis * Scalar::TWO;
    let forward = Scalar::from_i32(i32::from(input.movement.y)) / axis * Scalar::from_i32(4);
    let motion = lactose_anim::RootMotion {
        translation: Vec3::new(Scalar::ZERO, Scalar::ZERO, -(forward * dt.0)),
        yaw: -(turn * dt.0),
    };
    let jump = hop.0 == HOP_PENDING;
    if jump {
        hop.set_if_neq(Hop(HOP_SPENT));
    }
    if body.body_type == BodyType::Kinematic {
        let moved = motion.apply(&transform);
        transform.set_if_neq(moved);
    } else {
        let mut next = motion.velocity(&transform, &velocity, dt.0);
        if jump {
            next.linear.y = Scalar::from_i32(3);
        }
        velocity.set_if_neq(next);
    }
}

/// Marks the hop once the character has walked two metres from home.
#[system(name = "spike::mark_hop", worlds = [server, prediction])]
fn mark_hop(transform: &Transform, hop: Mut<Hop>) {
    if hop.0 == HOP_IDLE && transform.translation.z < Scalar::from_i32(-2) {
        hop.set_if_neq(Hop(HOP_PENDING));
    }
}

/// On a pressed `Throw`, fifty spheres in a five by five by two block in front of the
/// character, thrown forward and up. Spawned from the claimed character with the player's
/// input tick set, so each is `Predicted` on both ends and claimed by the player.
#[system(name = "spike::throw", worlds = [server, prediction], after = steer)]
fn throw(input: &PlayerInput, transform: &Transform, commands: Commands) {
    if !input.pressed.contains(Action::Throw) {
        return;
    }
    let spacing = scalar(0.45);
    let linear = transform.rotation * Vec3::new(Scalar::ZERO, Scalar::TWO, Scalar::from_i32(-4));
    for index in 0..THROWN {
        let (column, row, layer) = (index % 5, (index / 5) % 5, index / 25);
        let local = Vec3::new(
            spacing * Scalar::from_i32(column as i32 - 2),
            Scalar::HALF + spacing * Scalar::from_u32(layer),
            -(scalar(1.2) + spacing * Scalar::from_u32(row)),
        );
        let at = transform.translation + transform.rotation * local;
        commands.spawn((
            Transform::at(at),
            Velocity { linear, angular: Vec3::ZERO },
            RigidBody::dynamic(),
            Collider::new(Shape::sphere(scalar(0.2))),
            Lifetime(LIFETIME),
        ));
    }
}

/// Counts a thrown body's lifetime down and despawns it at the end.
#[system(name = "spike::expire", worlds = [server, prediction])]
fn expire(key: StableKeyOf, lifetime: Mut<Lifetime>, commands: Commands) {
    if lifetime.0 <= 1 {
        commands.despawn(key.key());
    } else {
        lifetime.0 -= 1;
    }
}

/// The server's claims of the claim scene: the lone crate while the character pushes it,
/// the stack as the pushed crate meets it, all released later; every cycle's first tick
/// puts the crates back.
#[system(name = "spike::claims", worlds = [server], after = throw, writes = [ClaimedBy])]
fn claims(
    ctx: Ctx,
    key: StableKeyOf,
    pushable: &Pushable,
    transform: Mut<Transform>,
    velocity: Mut<Velocity>,
    commands: Commands,
) {
    let phase = ctx.tick().0 % CYCLE;
    let claim = ClaimedBy { player: PLAYER };
    match phase {
        0 => {
            transform.set_if_neq(Transform::at(crate_home(pushable.0)));
            velocity.set_if_neq(Velocity::ZERO);
        }
        CLAIM_LONE if pushable.0 == 0 => commands.set(key.key(), claim),
        CLAIM_STACK if pushable.0 > 0 => commands.set(key.key(), claim),
        RELEASE => commands.remove::<ClaimedBy>(key.key()),
        _ => {}
    }
}

/// The server's claim of the lone crate on approach, before the character touches it,
/// released as the claim scene's are; every cycle's first tick puts the crate back.
#[system(name = "spike::claim_on_approach", worlds = [server], after = throw, writes = [ClaimedBy])]
fn claim_on_approach(
    ctx: Ctx,
    key: StableKeyOf,
    pushable: &Pushable,
    transform: Mut<Transform>,
    velocity: Mut<Velocity>,
    commands: Commands,
) {
    match ctx.tick().0 % CYCLE {
        0 => {
            transform.set_if_neq(Transform::at(crate_home(pushable.0)));
            velocity.set_if_neq(Velocity::ZERO);
        }
        CLAIM_APPROACH => commands.set(key.key(), ClaimedBy { player: PLAYER }),
        RELEASE => commands.remove::<ClaimedBy>(key.key()),
        _ => {}
    }
}

fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<Lifetime>()?;
    types.register::<Pushable>()?;
    types.register::<Hop>()
}

fn register_systems(registration: &mut Registration) {
    let simulation = registration.stage(Stage::Simulation);
    simulation.add(&STEER).add(&THROW).add(&EXPIRE).add(&CLAIMS);
    registration.stage(Stage::Post).add(&MARK_HOP);
}

fn register_approach_systems(registration: &mut Registration) {
    let simulation = registration.stage(Stage::Simulation);
    simulation.add(&STEER).add(&THROW).add(&EXPIRE).add(&CLAIM_ON_APPROACH);
    registration.stage(Stage::Post).add(&MARK_HOP);
}

static SPIKE: CrateRegistration = CrateRegistration {
    name: "spike",
    types: register_types,
    setup: None,
    systems: register_systems,
    migrations: None,
};

/// The spike's crate with the claim on approach in place of the claim scene's claims.
static SPIKE_APPROACH: CrateRegistration = CrateRegistration {
    name: "spike",
    types: register_types,
    setup: None,
    systems: register_approach_systems,
    migrations: None,
};

/// The dynamic character's game: the capsule and thrown-bodies scenes.
static DYNAMIC: Game =
    Game { name: "spike-dynamic", crates: &[SPIKE], order: None, sessions: Some(dynamic) };
/// The kinematic character's game.
static KINEMATIC: Game =
    Game { name: "spike-kinematic", crates: &[SPIKE], order: None, sessions: Some(kinematic) };
/// The dynamic character among four crates.
static CRATES: Game =
    Game { name: "spike-crates", crates: &[SPIKE], order: None, sessions: Some(crates) };
/// The dynamic character and the lone crate, claimed on approach: the measurement's one
/// contact between claimed bodies.
static APPROACH: Game = Game {
    name: "spike-approach",
    crates: &[SPIKE_APPROACH],
    order: None,
    sessions: Some(lone_crate),
};
/// The dynamic character among unclaimed bodies, which the prediction world holds as
/// kinematic proxies: the slot's measurement alone, outside the recorded scenes.
static PROXIES: Game =
    Game { name: "spike-proxies", crates: &[SPIKE], order: None, sessions: Some(proxies) };

fn dynamic() -> Box<dyn SessionApplier> {
    Box::new(Arena { kinematic: false, crates: 0, proxies: 0 })
}

fn kinematic() -> Box<dyn SessionApplier> {
    Box::new(Arena { kinematic: true, crates: 0, proxies: 0 })
}

fn crates() -> Box<dyn SessionApplier> {
    Box::new(Arena { kinematic: false, crates: 4, proxies: 0 })
}

fn lone_crate() -> Box<dyn SessionApplier> {
    Box::new(Arena { kinematic: false, crates: 1, proxies: 0 })
}

fn proxies() -> Box<dyn SessionApplier> {
    Box::new(Arena { kinematic: false, crates: 0, proxies: PREDICTION_ROWS - FIXED })
}

/// The arena's fixed colliders: a floor and four walls.
const FIXED: u32 = 5;
/// The fixed colliders and proxies a prediction world holds at most, together (D139).
const PREDICTION_ROWS: u32 = 64;

/// The session applier: a join builds the arena (a floor and four walls, fixed, the first
/// `crates` crates of the claim scene, and `proxies` unclaimed boxes at rest behind the
/// character, ten to a row) and the player's capsule, claimed and anchored.
struct Arena {
    kinematic: bool,
    crates: u32,
    proxies: u32,
}

impl SessionApplier for Arena {
    fn apply(&mut self, world: &mut World, player: PlayerId, event: SessionEvent) -> Result<()> {
        let SessionEvent::Join { region } = event else { return Ok(()) };
        let (ten, half) = (Scalar::from_i32(10), Scalar::HALF);
        let thin = scalar(0.25);
        let statics: [_; FIXED as usize] = [
            (
                Vec3::new(Scalar::ZERO, -half, Scalar::ZERO),
                Vec3::new(scalar(12.0), half, scalar(12.0)),
            ),
            (Vec3::new(ten, Scalar::ONE, Scalar::ZERO), Vec3::new(thin, Scalar::ONE, ten)),
            (Vec3::new(-ten, Scalar::ONE, Scalar::ZERO), Vec3::new(thin, Scalar::ONE, ten)),
            (Vec3::new(Scalar::ZERO, Scalar::ONE, ten), Vec3::new(ten, Scalar::ONE, thin)),
            (Vec3::new(Scalar::ZERO, Scalar::ONE, -ten), Vec3::new(ten, Scalar::ONE, thin)),
        ];
        for (at, extents) in statics {
            let key = StableKey::Net(world.allocate_net_id()?);
            world.spawn(key, region, (Transform::at(at), Collider::new(Shape::cuboid(extents))))?;
        }
        if self.crates > 0 {
            let extents = Vec3::new(scalar(0.4), scalar(0.4), scalar(0.4));
            for index in 0..self.crates {
                let key = StableKey::Net(world.allocate_net_id()?);
                let row = (
                    Pushable(index),
                    Transform::at(crate_home(index)),
                    Velocity::ZERO,
                    RigidBody::dynamic(),
                    Collider::new(Shape::cuboid(extents)),
                );
                world.spawn(key, region, row)?;
            }
        }
        let quarter = scalar(0.25);
        for index in 0..self.proxies {
            let key = StableKey::Net(world.allocate_net_id()?);
            let (column, row) = (index % 10, index / 10);
            let at = Vec3::new(
                scalar(-4.5) + Scalar::from_u32(column),
                quarter,
                Scalar::from_u32(3 + row),
            );
            let row = (
                Transform::at(at),
                Velocity::ZERO,
                RigidBody::dynamic(),
                Collider::new(Shape::cuboid(Vec3::new(quarter, quarter, quarter))),
            );
            world.spawn(key, region, row)?;
        }
        let body = if self.kinematic {
            RigidBody::kinematic()
        } else {
            let mut body = RigidBody::dynamic();
            body.locked_axes = locked_axes::ROTATION_X | locked_axes::ROTATION_Z;
            body.can_sleep = false;
            body
        };
        let key = StableKey::Net(world.allocate_net_id()?);
        let row = (
            ClaimedBy { player },
            StreamingAnchor { priority: 0 },
            Transform::at(home()),
            Velocity::ZERO,
            body,
            Collider::new(Shape::capsule(Scalar::HALF, scalar(0.4))),
            Hop(HOP_IDLE),
        );
        world.spawn(key, region, row)
    }
}

/// The four scenes, the measurement's claim on approach, and the slot's thrown scene among
/// proxies.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scene {
    Capsule,
    Kinematic,
    Thrown,
    Claim,
    Approach,
    Proxies,
}

impl Scene {
    fn name(self) -> &'static str {
        match self {
            Self::Capsule => "dynamic capsule",
            Self::Kinematic => "kinematic character",
            Self::Thrown => "fifty thrown bodies",
            Self::Claim => "claim in contact",
            Self::Approach => "claim on approach",
            Self::Proxies => "fifty thrown bodies among proxies",
        }
    }

    fn game(self) -> &'static Game {
        match self {
            Self::Capsule | Self::Thrown => &DYNAMIC,
            Self::Kinematic => &KINEMATIC,
            Self::Claim => &CRATES,
            Self::Approach => &APPROACH,
            Self::Proxies => &PROXIES,
        }
    }

    /// The scripted sample of `tick`. The walk: idle, ahead, a turn right, a turn left,
    /// back, idle; the claim scenes walk ahead into the crates and stop; the thrown scenes
    /// hold `Throw` over phases 60 to 89 beside the walk.
    fn input(self, tick: Tick) -> PlayerInput {
        let phase = tick.0 % CYCLE;
        let stick = |turn: f64, forward: f64| Axis2 {
            x: (turn * f64::from(AXIS_SCALE)) as i16,
            y: (forward * f64::from(AXIS_SCALE)) as i16,
        };
        let movement = if matches!(self, Self::Claim | Self::Approach) {
            if (30..200).contains(&phase) { stick(0.0, 0.5) } else { Axis2::ZERO }
        } else {
            match phase {
                30..120 => stick(0.0, 0.5),
                120..180 => stick(0.4, 0.6),
                180..240 => stick(-0.6, 0.6),
                240..280 => stick(0.0, -0.4),
                _ => Axis2::ZERO,
            }
        };
        let mut input = PlayerInput { movement, ..PlayerInput::IDLE };
        if matches!(self, Self::Thrown | Self::Proxies) {
            let throw = Actions::EMPTY.with(Action::Throw);
            if (60..90).contains(&phase) {
                input.held = throw;
            }
            if phase == 60 {
                input.pressed = throw;
            }
            if phase == 90 {
                input.released = throw;
            }
        }
        input
    }

    /// The link's faults beyond its random loss: in the thrown scene, every fourth cycle
    /// from the second loses the client's datagrams over the frames that carry the press,
    /// and every fourth from the third cuts both directions for forty frames around it.
    fn faults(self) -> Faults {
        Faults { thrown: self == Self::Thrown }
    }
}

// ---- The cooked content both ends hold in memory ----

struct Store;

struct Blob;

impl Asset for Blob {
    const KIND: AssetKind = AssetKind::from_name("spike::blob");
    const FORMAT_VERSION: u32 = 1;
}

impl PublishedAssets for Store {
    fn request(&self, _: AssetId, _: AssetVersion, _: LoadPriority) {}

    fn get(&self, id: AssetId, version: AssetVersion) -> Readiness<UntypedHandle> {
        Readiness::Ready(UntypedHandle::new(id, version, Blob))
    }
}

fn table_of(region: RegionId) -> VersionTable {
    [(AssetId::from_bits(region.0), AssetVersion::from_bits(1).expect("non-zero"))]
        .into_iter()
        .collect()
}

/// The region's cooked content: an empty baseline and one asset.
struct InMemory {
    baseline: Arc<Baseline>,
}

impl InMemory {
    fn new() -> Result<Self> {
        Ok(Self { baseline: Arc::new(Baseline::from_rows(AssetSet::new(), [], [])?) })
    }

    fn fresh(&self) -> Result<RegionBundle> {
        Ok(RegionBundle {
            seed: REGION.0,
            mode: RegionMode::Simulated,
            baseline: Arc::clone(&self.baseline),
            overlay: None,
            resources: Vec::new(),
            transient_resources: Vec::new(),
            assets: Some(self.pin(&table_of(REGION))?),
        })
    }
}

impl Cooked for InMemory {
    fn baseline(&self, content_hash: u64) -> Result<Arc<Baseline>> {
        if self.baseline.content_hash() == content_hash {
            Ok(Arc::clone(&self.baseline))
        } else {
            Err(Error::new(ErrorKind::NotFound, "no cooked baseline has that hash"))
        }
    }

    fn table(&self, region: RegionId) -> Option<VersionTable> {
        (region == REGION).then(|| table_of(region))
    }

    fn pin(&self, table: &VersionTable) -> Result<PinnedTable> {
        PinnedTable::pin(&Store, table)
            .ready()
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "the store has not published it"))
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidData, message.into())
}

// ---- The link ----

/// The delay each way, in frames: a 100 ms round trip at sixty frames a second.
const DELAY: u32 = 3;
/// The share of messages the link loses, per thousand; every datagram of a message shares
/// its fate.
const LOSS_PERMILLE: u64 = 10;
/// What the link draws for, mixed into each draw.
const DRAW_LOSS: u8 = 1;
const DRAW_ORDER: u8 = 2;

/// The end a datagram leaves from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Server,
    Client,
}

/// The scene's link faults: windows of frames in which one direction or both lose every
/// datagram but the control messages, which wait for the window's end.
#[derive(Clone, Copy, Debug)]
struct Faults {
    thrown: bool,
}

impl Faults {
    const NONE: Self = Self { thrown: false };

    /// The window `frame` lies in for datagrams leaving `from`, if any.
    fn window(self, from: Side, frame: u32) -> Option<Range<u32>> {
        if !self.thrown {
            return None;
        }
        let start = frame / CYCLE * CYCLE;
        let window = match (frame / CYCLE) % 4 {
            1 if from == Side::Client => start + 48..start + 66,
            2 => start + 50..start + 90,
            _ => return None,
        };
        window.contains(&frame).then_some(window)
    }
}

/// Both directions of the link, shared by the two ends' transports. `now` is the global
/// frame, which the loop advances; a datagram sent in frame `n` is delivered in frame
/// `n + DELAY`, never earlier or later, in an order drawn per datagram, so the link
/// reorders only within a frame.
struct Link {
    seed: u64,
    now: u32,
    faults: Faults,
    /// What each end will receive, by (frame, drawn order, send count).
    to_client: BTreeMap<(u32, u64, u64), Vec<u8>>,
    to_server: BTreeMap<(u32, u64, u64), Vec<u8>>,
    count: u64,
    /// The xxh64 of every datagram sent since the record last took them, in send order.
    sent: Vec<u64>,
    dropped: u64,
}

impl Link {
    fn new(seed: u64, faults: Faults) -> Self {
        Self {
            seed,
            now: 0,
            faults,
            to_client: BTreeMap::new(),
            to_server: BTreeMap::new(),
            count: 0,
            sent: Vec::new(),
            dropped: 0,
        }
    }

    /// A draw for `what` about a datagram, a pure function of the seed and its arguments.
    fn draw(&self, what: u8, from: Side, message: u16, index: u8) -> u64 {
        let mut bytes = [0_u8; 9];
        bytes[0] = what;
        bytes[1] = u8::from(from == Side::Client);
        bytes[2..4].copy_from_slice(&message.to_le_bytes());
        bytes[4] = index;
        bytes[5..9].copy_from_slice(&self.now.to_le_bytes());
        xxh64(&bytes, self.seed)
    }

    fn send(&mut self, from: Side, datagram: &[u8]) {
        self.sent.push(xxh64(datagram, 0));
        let kind = datagram[0];
        let message = u16::from_le_bytes([datagram[1], datagram[2]]);
        let index = datagram[3];
        let control = kind == FULL || kind == RESYNC;
        let mut at = self.now + DELAY;
        if let Some(window) = self.faults.window(from, self.now) {
            if !control {
                self.dropped += 1;
                return;
            }
            at = window.end + DELAY;
        } else if !control && self.draw(DRAW_LOSS, from, message, 0) % 1000 < LOSS_PERMILLE {
            self.dropped += 1;
            return;
        }
        let order = self.draw(DRAW_ORDER, from, message, index);
        let queue = match from {
            Side::Server => &mut self.to_client,
            Side::Client => &mut self.to_server,
        };
        queue.insert((at, order, self.count), datagram.to_vec());
        self.count += 1;
    }

    fn deliver(&mut self, to: Side) -> Vec<Vec<u8>> {
        let queue = match to {
            Side::Server => &mut self.to_server,
            Side::Client => &mut self.to_client,
        };
        let later = queue.split_off(&(self.now + 1, 0, 0));
        std::mem::replace(queue, later).into_values().collect()
    }
}

fn lock(link: &Mutex<Link>) -> std::sync::MutexGuard<'_, Link> {
    link.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One end of the link as a `Transport`: one connection, connected at the first poll.
struct Throwaway {
    link: Arc<Mutex<Link>>,
    side: Side,
    connected: bool,
    stats: LinkStats,
}

impl Throwaway {
    fn new(link: Arc<Mutex<Link>>, side: Side) -> Self {
        Self { link, side, connected: false, stats: LinkStats::default() }
    }
}

impl Transport for Throwaway {
    fn send(&mut self, to: ConnectionId, payload: &[u8]) -> Result<()> {
        if to != CONNECTION {
            return Err(Error::new(ErrorKind::NotFound, "no such connection"));
        }
        if payload.len() > MAX_DATAGRAM_BYTES {
            return Err(invalid("a datagram over the limit"));
        }
        self.stats.datagrams_sent += 1;
        self.stats.bytes_sent += payload.len() as u64;
        lock(&self.link).send(self.side, payload);
        Ok(())
    }

    fn poll(&mut self, events: &mut Vec<TransportEvent>) {
        if !self.connected {
            self.connected = true;
            events.push(TransportEvent::Connected(CONNECTION));
        }
        let delivered = lock(&self.link).deliver(self.side);
        for payload in delivered {
            self.stats.datagrams_received += 1;
            self.stats.bytes_received += payload.len() as u64;
            let datagram = Datagram::new(CONNECTION, payload).expect("under the limit when sent");
            events.push(TransportEvent::Received(datagram));
        }
    }

    fn stats(&self, connection: ConnectionId) -> Option<LinkStats> {
        (connection == CONNECTION).then_some(self.stats)
    }

    fn disconnect(&mut self, _: ConnectionId) {}
}

// ---- Messages over datagrams ----

/// The message kinds.
const DELTA: u8 = 1;
const FULL: u8 = 2;
const INPUT: u8 = 3;
const RESYNC: u8 = 4;
/// A datagram's header: kind, message number (u16), fragment index, fragment count.
const HEADER: usize = 5;
const FRAGMENT: usize = MAX_DATAGRAM_BYTES - HEADER;

/// Splits messages into datagrams of at most `MAX_DATAGRAM_BYTES`.
#[derive(Default)]
struct Messenger {
    next: u16,
}

impl Messenger {
    fn send(&mut self, transport: &mut dyn Transport, kind: u8, body: &[u8]) -> Result<()> {
        let fragments: Vec<&[u8]> =
            if body.is_empty() { vec![body] } else { body.chunks(FRAGMENT).collect() };
        let count = u8::try_from(fragments.len()).map_err(|_| invalid("a message too long"))?;
        for (index, fragment) in fragments.into_iter().enumerate() {
            let mut datagram = Vec::with_capacity(HEADER + fragment.len());
            datagram.push(kind);
            datagram.extend_from_slice(&self.next.to_le_bytes());
            datagram.push(index as u8);
            datagram.push(count);
            datagram.extend_from_slice(fragment);
            transport.send(CONNECTION, &datagram)?;
        }
        self.next = self.next.wrapping_add(1);
        Ok(())
    }
}

/// A message being reassembled: its fragment count and the fragments arrived, by index.
type Fragments = (u8, BTreeMap<u8, Vec<u8>>);

/// What one poll delivered, reassembled: each message whose fragments all arrived, in the
/// order its first fragment did. Fragments of one message share a delivery frame.
fn receive(transport: &mut dyn Transport, events: &mut Vec<TransportEvent>) -> Vec<(u8, Vec<u8>)> {
    events.clear();
    transport.poll(events);
    let mut partial: BTreeMap<(u8, u16), Fragments> = BTreeMap::new();
    let mut order = Vec::new();
    for event in events.drain(..) {
        let TransportEvent::Received(datagram) = event else { continue };
        let bytes = datagram.payload();
        let id = (bytes[0], u16::from_le_bytes([bytes[1], bytes[2]]));
        let entry = partial.entry(id).or_insert_with(|| {
            order.push(id);
            (bytes[4], BTreeMap::new())
        });
        entry.1.insert(bytes[3], bytes[HEADER..].to_vec());
    }
    order
        .into_iter()
        .filter_map(|id| {
            let (count, fragments) = partial.remove(&id)?;
            (fragments.len() == usize::from(count))
                .then(|| (id.0, fragments.into_values().flatten().collect()))
        })
        .collect()
}

// ---- Encoding ----

#[derive(Default)]
struct Out(Vec<u8>);

impl Out {
    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn len(&mut self, len: usize) {
        self.u32(u32::try_from(len).expect("small"));
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.len(bytes.len());
        self.0.extend_from_slice(bytes);
    }

    fn key(&mut self, key: StableKey) {
        match key {
            StableKey::Net(net) => {
                self.u8(0);
                self.u64(net.to_bits());
            }
            StableKey::Predicted(predicted) => {
                self.u8(1);
                self.u32(predicted.player.0);
                self.u32(predicted.input_tick.0);
                self.u32(predicted.ordinal);
            }
            StableKey::Placeholder(_) => unreachable!("a placeholder never leaves its buffer"),
        }
    }

    fn handle(&mut self, handle: EntityHandle) {
        self.u16(handle.index);
        self.u16(handle.generation);
    }

    fn data(&mut self, data: &ComponentData) {
        self.u64(data.component.0);
        self.bytes(&data.bytes);
    }

    fn option_tick(&mut self, tick: Option<Tick>) {
        match tick {
            Some(tick) => {
                self.u8(1);
                self.u32(tick.0);
            }
            None => self.u8(0),
        }
    }
}

struct In<'a> {
    bytes: &'a [u8],
}

impl<'a> In<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.bytes.len() < n {
            return Err(invalid("a message shorter than it says"));
        }
        let (head, rest) = self.bytes.split_at(n);
        self.bytes = rest;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().expect("two")))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("four")))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("eight")))
    }

    fn len(&mut self) -> Result<usize> {
        Ok(self.u32()? as usize)
    }

    fn bytes(&mut self) -> Result<Vec<u8>> {
        let len = self.len()?;
        Ok(self.take(len)?.to_vec())
    }

    fn tick(&mut self) -> Result<Tick> {
        Ok(Tick(self.u32()?))
    }

    fn key(&mut self) -> Result<StableKey> {
        match self.u8()? {
            0 => NetId::from_bits(self.u64()?)
                .map(StableKey::Net)
                .ok_or_else(|| invalid("the zero NetId")),
            1 => Ok(StableKey::Predicted(PredictionKey {
                player: PlayerId(self.u32()?),
                input_tick: self.tick()?,
                ordinal: self.u32()?,
            })),
            _ => Err(invalid("an unknown key tag")),
        }
    }

    fn handle(&mut self) -> Result<EntityHandle> {
        Ok(EntityHandle { index: self.u16()?, generation: self.u16()? })
    }

    fn data(&mut self) -> Result<ComponentData> {
        Ok(ComponentData { component: ComponentKey(self.u64()?), bytes: self.bytes()? })
    }

    fn option_tick(&mut self) -> Result<Option<Tick>> {
        Ok(match self.u8()? {
            0 => None,
            _ => Some(self.tick()?),
        })
    }

    fn done(&self) -> Result<()> {
        if self.bytes.is_empty() { Ok(()) } else { Err(invalid("trailing bytes")) }
    }
}

/// An entity sent whole: its handle on the connection, its origin key and the `NetId` the
/// server bound to it, whether the player has it claimed, and every replicated component.
#[derive(Clone, Debug)]
struct WireSpawn {
    handle: EntityHandle,
    key: StableKey,
    net: NetId,
    claimed: bool,
    components: Vec<ComponentData>,
}

impl WireSpawn {
    fn encode(&self, out: &mut Out) {
        out.handle(self.handle);
        out.key(self.key);
        out.u64(self.net.to_bits());
        out.u8(u8::from(self.claimed));
        out.len(self.components.len());
        for data in &self.components {
            out.data(data);
        }
    }

    fn decode(input: &mut In<'_>) -> Result<Self> {
        let handle = input.handle()?;
        let key = input.key()?;
        let net = NetId::from_bits(input.u64()?).ok_or_else(|| invalid("the zero NetId"))?;
        let claimed = input.u8()? != 0;
        let count = input.len()?;
        let components = (0..count).map(|_| input.data()).collect::<Result<_>>()?;
        Ok(Self { handle, key, net, claimed, components })
    }
}

/// One row by handle.
#[derive(Clone, Debug)]
struct WireRow {
    handle: EntityHandle,
    component: ComponentKey,
    tick: Tick,
    exact: bool,
    bytes: Vec<u8>,
}

/// The server's update: everything the packer owes the client at `tick`.
#[derive(Clone, Debug)]
struct Delta {
    sequence: PacketSequence,
    tick: Tick,
    /// The newest input packet the server received, for the client's round trip.
    echo: Option<u16>,
    acked_input: Option<Tick>,
    substituted: Vec<Tick>,
    spawns: Vec<WireSpawn>,
    removed: Vec<(EntityHandle, ComponentKey)>,
    rows: Vec<WireRow>,
    tombstones: Vec<(EntityHandle, Tombstone)>,
}

impl Delta {
    fn encode(&self) -> Vec<u8> {
        let mut out = Out::default();
        out.u16(self.sequence.0);
        out.u32(self.tick.0);
        match self.echo {
            Some(echo) => {
                out.u8(1);
                out.u16(echo);
            }
            None => out.u8(0),
        }
        out.option_tick(self.acked_input);
        out.len(self.substituted.len());
        for tick in &self.substituted {
            out.u32(tick.0);
        }
        out.len(self.spawns.len());
        for spawn in &self.spawns {
            spawn.encode(&mut out);
        }
        out.len(self.removed.len());
        for &(handle, component) in &self.removed {
            out.handle(handle);
            out.u64(component.0);
        }
        out.len(self.rows.len());
        for row in &self.rows {
            out.handle(row.handle);
            out.u64(row.component.0);
            out.u32(row.tick.0);
            out.u8(u8::from(row.exact));
            out.bytes(&row.bytes);
        }
        out.len(self.tombstones.len());
        for &(handle, tombstone) in &self.tombstones {
            out.handle(handle);
            out.u64(tombstone.region.0);
            out.u32(tombstone.tick.0);
            out.u8(u8::from(tombstone.cause == TombstoneCause::Left));
        }
        out.0
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        let mut input = In::new(bytes);
        let sequence = PacketSequence(input.u16()?);
        let tick = input.tick()?;
        let echo = match input.u8()? {
            0 => None,
            _ => Some(input.u16()?),
        };
        let acked_input = input.option_tick()?;
        let count = input.len()?;
        let substituted = (0..count).map(|_| input.tick()).collect::<Result<_>>()?;
        let count = input.len()?;
        let spawns = (0..count).map(|_| WireSpawn::decode(&mut input)).collect::<Result<_>>()?;
        let count = input.len()?;
        let mut removed = Vec::with_capacity(count);
        for _ in 0..count {
            removed.push((input.handle()?, ComponentKey(input.u64()?)));
        }
        let count = input.len()?;
        let mut rows = Vec::with_capacity(count);
        for _ in 0..count {
            rows.push(WireRow {
                handle: input.handle()?,
                component: ComponentKey(input.u64()?),
                tick: input.tick()?,
                exact: input.u8()? != 0,
                bytes: input.bytes()?,
            });
        }
        let count = input.len()?;
        let mut tombstones = Vec::with_capacity(count);
        for _ in 0..count {
            let handle = input.handle()?;
            let region = RegionId(input.u64()?);
            let tick = input.tick()?;
            let cause =
                if input.u8()? == 0 { TombstoneCause::Destroyed } else { TombstoneCause::Left };
            tombstones.push((handle, Tombstone { region, tick, cause }));
        }
        input.done()?;
        Ok(Self {
            sequence,
            tick,
            echo,
            acked_input,
            substituted,
            spawns,
            removed,
            rows,
            tombstones,
        })
    }
}

/// The full relevance-set snapshot of a join or a resync, on the reliable path: the
/// player's entity on a join, the time scale and the acknowledgement, the region's
/// relevance message and every entity whole.
#[derive(Clone, Debug)]
struct Full {
    tick: Tick,
    join: Option<StableKey>,
    time_scale: TimeScale,
    acked_input: Tick,
    relevance: RegionRelevance,
    spawns: Vec<WireSpawn>,
}

impl Full {
    fn encode(&self) -> Vec<u8> {
        let mut out = Out::default();
        out.u32(self.tick.0);
        match self.join {
            Some(key) => {
                out.u8(1);
                out.key(key);
            }
            None => out.u8(0),
        }
        out.u32(self.time_scale.thousandths);
        out.u32(self.acked_input.0);
        let relevance = &self.relevance;
        out.u64(relevance.region.0);
        out.u64(relevance.seed);
        out.u64(relevance.baseline_hash);
        match &relevance.overlay {
            OverlayPayload::Cooked => out.u8(0),
            OverlayPayload::Full(overlay) => {
                out.u8(1);
                out.bytes(&overlay.encode());
            }
            OverlayPayload::Digest(digest) => {
                out.u8(2);
                out.u64(*digest);
            }
        }
        out.len(relevance.resources.len());
        for data in &relevance.resources {
            out.data(data);
        }
        out.len(relevance.transient_resources.len());
        for key in &relevance.transient_resources {
            out.u64(key.0);
        }
        match &relevance.assets {
            Some(table) => {
                out.u8(1);
                out.len(table.iter().len());
                for (id, version) in table.iter() {
                    out.u64(id.to_bits());
                    out.u64(version.to_bits());
                }
            }
            None => out.u8(0),
        }
        out.len(self.spawns.len());
        for spawn in &self.spawns {
            spawn.encode(&mut out);
        }
        out.0
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        let mut input = In::new(bytes);
        let tick = input.tick()?;
        let join = match input.u8()? {
            0 => None,
            _ => Some(input.key()?),
        };
        let time_scale = TimeScale { thousandths: input.u32()? };
        let acked_input = input.tick()?;
        let region = RegionId(input.u64()?);
        let seed = input.u64()?;
        let baseline_hash = input.u64()?;
        let overlay = match input.u8()? {
            0 => OverlayPayload::Cooked,
            1 => OverlayPayload::Full(Arc::new(Overlay::decode(&input.bytes()?)?)),
            _ => OverlayPayload::Digest(input.u64()?),
        };
        let count = input.len()?;
        let resources = (0..count).map(|_| input.data()).collect::<Result<_>>()?;
        let count = input.len()?;
        let transient_resources =
            (0..count).map(|_| Ok(ComponentKey(input.u64()?))).collect::<Result<_>>()?;
        let assets = match input.u8()? {
            0 => None,
            _ => {
                let count = input.len()?;
                let mut entries = Vec::with_capacity(count);
                for _ in 0..count {
                    let id = AssetId::from_bits(input.u64()?);
                    let version = AssetVersion::from_bits(input.u64()?)
                        .ok_or_else(|| invalid("the zero asset version"))?;
                    entries.push((id, version));
                }
                Some(entries.into_iter().collect())
            }
        };
        let count = input.len()?;
        let spawns = (0..count).map(|_| WireSpawn::decode(&mut input)).collect::<Result<_>>()?;
        input.done()?;
        let relevance = RegionRelevance {
            region,
            seed,
            baseline_hash,
            overlay,
            resources,
            transient_resources,
            assets,
        };
        Ok(Self { tick, join, time_scale, acked_input, relevance, spawns })
    }
}

/// The client's input packet: its number, its acknowledgement of the server's updates,
/// and the last eight samples by tick.
#[derive(Clone, Debug)]
struct InputPacket {
    sequence: u16,
    ack: Option<AckField>,
    samples: Vec<(Tick, PlayerInput)>,
}

impl InputPacket {
    fn encode(&self) -> Vec<u8> {
        let mut out = Out::default();
        out.u16(self.sequence);
        match self.ack {
            Some(ack) => {
                out.u8(1);
                out.u16(ack.latest.0);
                out.u32(ack.bits);
            }
            None => out.u8(0),
        }
        out.u8(self.samples.len() as u8);
        for (tick, input) in &self.samples {
            out.u32(tick.0);
            out.0.extend_from_slice(&input.to_sample_bytes());
        }
        out.0
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        let mut input = In::new(bytes);
        let sequence = input.u16()?;
        let ack = match input.u8()? {
            0 => None,
            _ => Some(AckField { latest: PacketSequence(input.u16()?), bits: input.u32()? }),
        };
        let count = input.u8()?;
        let mut samples = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            let tick = input.tick()?;
            let sample = PlayerInput::from_sample_bytes(input.take(size_of::<PlayerInput>())?)?;
            samples.push((tick, sample));
        }
        input.done()?;
        Ok(Self { sequence, ack, samples })
    }
}

fn encode_resync(request: &ResyncRequest) -> Vec<u8> {
    let mut out = Out::default();
    out.u32(request.tick.0);
    out.u8(match request.reason {
        ResyncReason::BehindCatchUp => 1,
        _ => 2,
    });
    out.0
}

fn decode_resync(bytes: &[u8]) -> Result<ResyncRequest> {
    let mut input = In::new(bytes);
    let tick = input.tick()?;
    let reason = match input.u8()? {
        1 => ResyncReason::BehindCatchUp,
        _ => return Err(invalid("the spike requests a resync only when behind")),
    };
    input.done()?;
    Ok(ResyncRequest { tick, reason })
}

// ---- The server's packer ----

/// Every replicated component of `key`, sorted by component.
fn replicated(world: &World, key: StableKey) -> Vec<ComponentData> {
    let mut components: Vec<ComponentData> = world
        .registry()
        .iter()
        .filter(|info| info.role() == TypeRole::Component && info.tracking().replicated)
        .filter_map(|info| {
            let bytes = world.get_erased(key, info.key())?;
            Some(ComponentData { component: info.key(), bytes: bytes.to_vec() })
        })
        .collect();
    components.sort_by_key(|data| data.component);
    components
}

/// The `NetId` a spawn names: the key's own, or the one the server bound to a predicted
/// spawn.
fn net_of(world: &World, key: StableKey) -> Result<NetId> {
    match key {
        StableKey::Net(net) => Ok(net),
        StableKey::Predicted(predicted) => world
            .alias(predicted)
            .ok_or_else(|| invalid("a predicted spawn the server never bound")),
        StableKey::Placeholder(_) => Err(invalid("a placeholder in the world")),
    }
}

/// One entity of the packer's mirror: each replicated row with the tick it last changed
/// by, and each replicated component removed since it last had one.
#[derive(Default)]
struct Mirrored {
    rows: BTreeMap<ComponentKey, (Tick, Vec<u8>)>,
    removed: BTreeMap<ComponentKey, Tick>,
}

/// The spike's packer for its one client, on the shape of `lactose-net-host`'s model
/// (`tests/common`): the mirror the gather feeds, the client's interest, and the input
/// ticks the server substituted, owed to the client until it acknowledges a packet that
/// carried them.
struct Packer {
    mirror: BTreeMap<StableKey, Mirrored>,
    interest: ClientInterest,
    joined: bool,
    sequence: PacketSequence,
    owed: BTreeSet<Tick>,
    carried: VecDeque<(PacketSequence, Vec<Tick>)>,
}

impl Packer {
    fn new() -> Self {
        Self {
            mirror: BTreeMap::new(),
            interest: ClientInterest::new(PLAYER, CONNECTION, SendBudget::default()),
            joined: false,
            sequence: PacketSequence(0),
            owed: BTreeSet::new(),
            carried: VecDeque::new(),
        }
    }

    fn absorb(&mut self, gathered: &StagingFrame) -> Result<()> {
        let tick = gathered.tick;
        if gathered.full {
            if self.joined {
                return Err(invalid("a full gather after the join"));
            }
            self.mirror.clear();
        }
        for entry in gathered.structural() {
            match entry.change {
                StructuralChange::Spawned { key, .. } => {
                    self.mirror.insert(key, Mirrored::default());
                    if self.joined {
                        self.interest.enter(key)?;
                    }
                }
                StructuralChange::Despawned { key, region } => {
                    self.mirror.remove(&key);
                    let cause = TombstoneCause::Destroyed;
                    self.interest.leave(key, Tombstone { region, tick, cause });
                }
                StructuralChange::Removed { key, component } => {
                    if let Some(entity) = self.mirror.get_mut(&key)
                        && entity.rows.remove(&component).is_some()
                    {
                        entity.removed.insert(component, tick);
                    }
                }
                StructuralChange::Moved { .. } => return Err(invalid("one region moves nothing")),
                StructuralChange::Added { .. } => {}
            }
        }
        for row in gathered.rows() {
            let entity = self.mirror.entry(row.key).or_default();
            entity.rows.insert(row.component, (tick, row.bytes.to_vec()));
            entity.removed.remove(&row.component);
        }
        Ok(())
    }

    /// The claims of the player, as the server world holds them.
    fn sync_claims(&mut self, world: &World) {
        let keys: Vec<StableKey> = self.interest.entities.keys().copied().collect();
        for key in keys {
            let claimed = world.get::<ClaimedBy>(key).is_some_and(|claim| claim.player == PLAYER);
            self.interest.set_claimed(key, claimed);
        }
    }

    fn spawn(
        &self,
        world: &World,
        key: StableKey,
        handle: EntityHandle,
        claimed: bool,
    ) -> Result<WireSpawn> {
        let entity = self.mirror.get(&key).ok_or_else(|| invalid("an entity not mirrored"))?;
        let components = entity
            .rows
            .iter()
            .map(|(&component, (_, bytes))| ComponentData { component, bytes: bytes.clone() })
            .collect();
        Ok(WireSpawn { handle, key, net: net_of(world, key)?, claimed, components })
    }

    /// A join or a resync at `tick`: every mirrored entity enters, the interest starts
    /// over, and the full frame names each one whole. What was owed is superseded.
    fn resync(&mut self, tick: Tick, world: &World) -> Result<Vec<WireSpawn>> {
        let keys: Vec<StableKey> = self.mirror.keys().copied().collect();
        for key in keys {
            self.interest.enter(key)?;
        }
        self.sync_claims(world);
        self.interest.resync(tick);
        self.joined = true;
        self.owed.clear();
        self.carried.clear();
        self.interest
            .entities
            .iter()
            .map(|(&key, entity)| self.spawn(world, key, entity.handle, entity.claimed))
            .collect()
    }

    /// The update for `tick`: a tombstone for every entity owed one; every entity nothing
    /// of which is acknowledged, whole as a spawn; every claimed entity as an exact row of
    /// each of its components; every other as the rows changed and the components removed
    /// since its acknowledged tick; and the substituted input ticks still owed.
    fn pack(
        &mut self,
        world: &World,
        tick: Tick,
        echo: Option<u16>,
        acked_input: Option<Tick>,
    ) -> Result<Delta> {
        let mut delta = Delta {
            sequence: self.sequence,
            tick,
            echo,
            acked_input,
            substituted: self.owed.iter().copied().collect(),
            spawns: Vec::new(),
            removed: Vec::new(),
            rows: Vec::new(),
            tombstones: Vec::new(),
        };
        let mut record = PacketRecord { sequence: self.sequence, tick, ..PacketRecord::default() };
        for (&key, entity) in &self.interest.entities {
            let handle = entity.handle;
            if let Some(tombstone) = entity.tombstone {
                delta.tombstones.push((handle, tombstone));
                record.tombstones.push((handle, tombstone));
                continue;
            }
            let mirror = self.mirror.get(&key).ok_or_else(|| invalid("an entity not mirrored"))?;
            let carried = match entity.acked {
                None => {
                    delta.spawns.push(self.spawn(world, key, handle, entity.claimed)?);
                    for &component in mirror.removed.keys() {
                        delta.removed.push((handle, component));
                    }
                    true
                }
                Some(acked) => {
                    let mut carried = entity.claimed;
                    for (&component, (changed, bytes)) in &mirror.rows {
                        if entity.claimed || changed.0 > acked.0 {
                            delta.rows.push(WireRow {
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
                            delta.removed.push((handle, component));
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
        if !delta.substituted.is_empty() {
            self.carried.push_back((self.sequence, delta.substituted.clone()));
            if self.carried.len() > usize::from(AckField::WINDOW) {
                self.carried.pop_front();
            }
        }
        self.sequence = self.sequence.next();
        Ok(delta)
    }

    fn acknowledge(&mut self, ack: AckField) {
        self.interest.acknowledge(ack);
        let owed = &mut self.owed;
        self.carried.retain(|(sequence, ticks)| {
            let covered = ack.covers(*sequence);
            if covered {
                for tick in ticks {
                    owed.remove(tick);
                }
            }
            !covered
        });
    }

    /// Every claimed entity the update carries is whole: an exact row of every replicated
    /// component the server's entity has at the update's tick, or a spawn.
    fn check_whole(&self, delta: &Delta, world: &World) -> Result<u32> {
        let mut checked = 0;
        for (&key, entity) in &self.interest.entities {
            if !entity.claimed || entity.tombstone.is_some() {
                continue;
            }
            let expected: Vec<ComponentKey> =
                replicated(world, key).into_iter().map(|data| data.component).collect();
            let sent: Vec<ComponentKey> =
                match delta.spawns.iter().find(|spawn| spawn.handle == entity.handle) {
                    Some(spawn) => spawn.components.iter().map(|data| data.component).collect(),
                    None => delta
                        .rows
                        .iter()
                        .filter(|row| row.handle == entity.handle && row.exact)
                        .map(|row| row.component)
                        .collect(),
                };
            if sent != expected {
                return Err(invalid(format!(
                    "the update of {:?} carries {key:?} claimed as {sent:?}, not whole as {expected:?}",
                    delta.tick
                )));
            }
            checked += 1;
        }
        Ok(checked)
    }

    /// The mirror holds every replicated row of the world, and nothing else.
    fn check_mirror(&self, world: &World) -> Result<()> {
        let keys: BTreeSet<StableKey> = world.entities_in(REGION).collect();
        let mirrored: BTreeSet<StableKey> = self.mirror.keys().copied().collect();
        if keys != mirrored {
            return Err(invalid("the mirror holds other entities than the world"));
        }
        for key in keys {
            let rows: Vec<(ComponentKey, &[u8])> = self.mirror[&key]
                .rows
                .iter()
                .map(|(&component, (_, bytes))| (component, bytes.as_slice()))
                .collect();
            let world_rows = replicated(world, key);
            let expected: Vec<(ComponentKey, &[u8])> =
                world_rows.iter().map(|data| (data.component, data.bytes.as_slice())).collect();
            if rows != expected {
                return Err(invalid(format!("the mirror's {key:?} differs from the world's")));
            }
        }
        Ok(())
    }
}

// ---- The server's inputs ----

/// The player's samples as the server receives and applies them: the redundancy window
/// bounded by the last tick applied, the substitution of a missing sample and the repair
/// of the first delivered after one (architecture section 9.2).
struct Inputs {
    history: SampleHistory,
    /// The first tick a sample arrived for: no tick before it has one.
    start: Option<Tick>,
    consumed: Option<Tick>,
    applied: PlayerInput,
    after_gap: bool,
    /// The newest input packet received, and whether the next update still owes its echo:
    /// each is echoed once, so a packet held across a stall never stands for a round trip.
    newest: Option<u16>,
    echo: Option<u16>,
}

impl Inputs {
    fn new() -> Self {
        Self {
            history: SampleHistory::new(),
            start: None,
            consumed: None,
            applied: PlayerInput::IDLE,
            after_gap: false,
            newest: None,
            echo: None,
        }
    }

    fn receive(&mut self, packet: &InputPacket, server: Tick) {
        let bound = self.consumed.unwrap_or(server);
        for &(tick, input) in &packet.samples {
            if self.history.insert_bounded(bound, tick, input)
                && self.consumed.is_none()
                && self.start.is_none_or(|start| tick.0 < start.0)
            {
                self.start = Some(tick);
            }
        }
        let newer = self.newest.is_none_or(|newest| {
            let ahead = packet.sequence.wrapping_sub(newest);
            ahead != 0 && ahead < 0x8000
        });
        if newer {
            self.newest = Some(packet.sequence);
            self.echo = Some(packet.sequence);
        }
    }

    /// The sample the server applies at `tick`, and whether it is a substitute.
    fn take(&mut self, tick: Tick) -> Option<(PlayerSample, bool)> {
        let start = self.start?;
        if tick.0 < start.0 {
            return None;
        }
        let (input, substituted) = match self.history.get(tick) {
            Some(sample) if self.after_gap => (sample.after_substitute(&self.applied), false),
            Some(sample) => (*sample, false),
            None => (self.applied.substitute(), true),
        };
        self.after_gap = substituted;
        self.applied = input;
        self.consumed = Some(tick);
        let sample = PlayerSample {
            input_tick: tick,
            substituted,
            sample: input.to_sample_bytes().to_vec(),
        };
        Some((sample, substituted))
    }
}

// ---- The client's receiver ----

/// The client's network thread for its one connection: the handle mirror, the
/// interpolation slots, the newest staged tick and the acknowledgement field. A full
/// frame is staged whole and clears the mirror; a delta is staged whole or dropped.
struct Receiver {
    handles: ConnectionHandles,
    slots: BTreeMap<StableKey, InterpolationSlot>,
    writer: InterpolationWriter,
    _reader: InterpolationReader,
    newest: Option<Tick>,
    awaiting_full: bool,
    ack: Option<AckField>,
    dropped: u32,
}

impl Receiver {
    fn new() -> Self {
        let (writer, reader) = InterpolationBuffer::split(1024);
        Self {
            handles: ConnectionHandles::new(),
            slots: BTreeMap::new(),
            writer,
            _reader: reader,
            newest: None,
            awaiting_full: true,
            ack: None,
            dropped: 0,
        }
    }

    fn full(&mut self, full: Full) -> Result<Option<StagedFrame>> {
        if self.newest.is_some_and(|newest| full.tick.0 <= newest.0) {
            self.dropped += 1;
            return Ok(None);
        }
        self.handles.clear();
        for (_, slot) in std::mem::take(&mut self.slots) {
            self.writer.release(slot)?;
        }
        let mut frame = StagedFrame::resync(full.tick);
        frame.time_scale = Some(full.time_scale);
        frame.acked_input = Some(full.acked_input);
        frame.regions.insert(REGION, RegionChange::Relevant(full.relevance));
        for spawn in &full.spawns {
            self.handles.insert(spawn.handle, spawn.key)?;
            self.stage_spawn(&mut frame, spawn)?;
        }
        self.newest = Some(full.tick);
        self.awaiting_full = false;
        Ok(Some(frame))
    }

    fn delta(&mut self, delta: &Delta) -> Result<Option<StagedFrame>> {
        let after_newest = self.newest.is_none_or(|newest| delta.tick.0 > newest.0);
        if self.awaiting_full || !after_newest {
            self.dropped += 1;
            return Ok(None);
        }
        let mut handles = self.handles.clone();
        for spawn in &delta.spawns {
            if handles.insert(spawn.handle, spawn.key).is_err() {
                self.dropped += 1;
                return Ok(None);
            }
        }
        let resolves = |handle| handles.key_of(handle).is_some();
        if !delta.rows.iter().all(|row| resolves(row.handle))
            || !delta.removed.iter().all(|&(handle, _)| resolves(handle))
        {
            self.dropped += 1;
            return Ok(None);
        }
        let mut frame = StagedFrame::new(delta.tick);
        frame.acked_input = delta.acked_input;
        frame.substituted = delta.substituted.iter().copied().collect();
        for spawn in &delta.spawns {
            self.stage_spawn(&mut frame, spawn)?;
        }
        for &(handle, component) in &delta.removed {
            frame.removed.insert((handles.key_of(handle).expect("resolved"), component));
        }
        for row in &delta.rows {
            let key = handles.key_of(row.handle).expect("resolved");
            if !frame.push_row(key, row.component, row.tick, row.exact, &row.bytes) {
                return Err(invalid("the frame refused a row"));
            }
        }
        for &(handle, tombstone) in &delta.tombstones {
            // A tombstone of an entity whose spawn never arrived names nothing here.
            let Some(key) = handles.release(handle) else { continue };
            frame.despawns.insert(key, tombstone);
            if let Some(slot) = self.slots.remove(&key) {
                self.writer.release(slot)?;
            }
        }
        self.handles = handles;
        self.newest = Some(delta.tick);
        self.ack = Some(AckField::received(self.ack, delta.sequence));
        Ok(Some(frame))
    }

    /// Stages a spawn with its interpolation slot: none for a claimed entity, whose slot
    /// is released, else the slot it holds or a new one.
    fn stage_spawn(&mut self, frame: &mut StagedFrame, spawn: &WireSpawn) -> Result<()> {
        let slot = if spawn.claimed {
            if let Some(slot) = self.slots.remove(&spawn.key) {
                self.writer.release(slot)?;
            }
            None
        } else {
            let slot = match self.slots.get(&spawn.key) {
                Some(&slot) => slot,
                None => self.writer.allocate()?,
            };
            self.slots.insert(spawn.key, slot);
            Some(slot)
        };
        let origin = match spawn.key {
            StableKey::Predicted(predicted) => Some(predicted),
            _ => None,
        };
        let staged =
            StagedSpawn { origin, region: REGION, slot, components: spawn.components.clone() };
        frame.spawns.insert(spawn.net, staged);
        Ok(())
    }
}

// ---- The client's clock and its estimate ----

/// The client's clock script.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ClockScript {
    /// The server's period exactly.
    Exact,
    /// Half a percent slow, a late wake of up to three quarters of a period on a quarter
    /// of the frames, and one stall of thirty frames, in which the client does not run.
    Drifting,
}

/// The client's own rate under the drifting script, in thousandths of the server's.
const DRIFT_THOUSANDTHS: u64 = 995;
/// The frames the drifting client stalls over: its one step.
const STALL: Range<u32> = 450..480;
const CLOCK_SEED: u64 = 0x00C1_0C4D;

/// The client's clock: the time of frame `n`, from its base at the join.
struct ClientClock {
    base: Instant,
    joined: u32,
    script: ClockScript,
}

impl ClientClock {
    fn since(&self, frame: u32) -> Duration {
        let period = TICK_PERIOD.as_nanos() as u64;
        let elapsed = u64::from(frame - self.joined) * period;
        match self.script {
            ClockScript::Exact => Duration::from_nanos(elapsed),
            ClockScript::Drifting => {
                let wake = xxh64(&frame.to_le_bytes(), CLOCK_SEED);
                let late = if frame > self.joined && wake.is_multiple_of(4) {
                    (wake >> 8) % (period * 3 / 4)
                } else {
                    0
                };
                Duration::from_nanos(elapsed * DRIFT_THOUSANDTHS / 1000 + late)
            }
        }
    }

    fn now(&self, frame: u32) -> Instant {
        self.base + self.since(frame)
    }

    /// The time of frame `n` in periods, for the estimator.
    fn periods(&self, frame: u32) -> f64 {
        self.since(frame).as_nanos() as f64 / TICK_PERIOD.as_nanos() as f64
    }
}

/// The ticks of margin the estimator keeps on top of half the round trip.
const MARGIN: f64 = 2.0;
/// An error, in ticks, beyond which the estimate snaps instead of adjusting the rate: past
/// the jitter of the late wakes, short of what the stall leaves.
const SNAP_LIMIT: f64 = 4.0;

/// The spike's clock estimate, from its own round trip: half of it plus the margin is the
/// lead, the newest update's tick plus half of it and the time since is the server's tick
/// now, a proportional rate on the smoothed error with an integrated trim tracks it, and
/// an error beyond the limit snaps. `lactose-net` computes the real one at milestone 4.
struct Estimator {
    sent: VecDeque<(u16, f64)>,
    rtt: Option<f64>,
    error: f64,
    trim: i32,
    rate: u32,
    lead: f32,
    updates: u32,
    last_snap: Option<u32>,
}

impl Estimator {
    fn new() -> Self {
        Self {
            sent: VecDeque::new(),
            rtt: None,
            error: 0.0,
            trim: 0,
            rate: 1000,
            lead: INITIAL_LEAD as f32,
            updates: 0,
            last_snap: None,
        }
    }

    fn sent(&mut self, sequence: u16, at: f64) {
        self.sent.push_back((sequence, at));
        if self.sent.len() > 64 {
            self.sent.pop_front();
        }
    }

    fn echoed(&mut self, sequence: u16, at: f64) {
        if let Some(index) = self.sent.iter().position(|&(sent, _)| sent == sequence) {
            let (_, sent) = self.sent.remove(index).expect("found");
            let sample = at - sent;
            self.rtt = Some(self.rtt.map_or(sample, |rtt| rtt + (sample - rtt) / 8.0));
        }
    }

    /// The estimate after frame `frame`, whose client time is `now` periods, given the
    /// newest update's tick and the time it arrived.
    fn update(
        &mut self,
        frame: u32,
        newest: (Tick, f64),
        now: f64,
        client: Tick,
    ) -> Option<ClockEstimate> {
        let rtt = self.rtt?;
        let server = f64::from(newest.0.0) + rtt / 2.0 + (now - newest.1);
        let lead = rtt / 2.0 + MARGIN;
        let error = f64::from(client.0) - (server + lead);
        self.updates += 1;
        let snap = error.abs() > SNAP_LIMIT && self.last_snap.is_none_or(|at| frame >= at + 60);
        if snap {
            self.last_snap = Some(frame);
            self.error = 0.0;
        } else {
            self.error += (error - self.error) / 16.0;
            if self.updates.is_multiple_of(30) {
                self.trim = (self.trim - self.error.round().clamp(-2.0, 2.0) as i32).clamp(-25, 25);
            }
        }
        let proportional = (self.error * 4.0).round() as i32;
        self.rate = (1000 + self.trim - proportional).clamp(970, 1030) as u32;
        self.lead = lead as f32;
        Some(ClockEstimate {
            server_tick: Tick(server.round() as u32),
            lead_ticks: self.lead,
            rate_thousandths: self.rate,
            snap,
        })
    }
}

// ---- The run ----

/// The lead the client starts at, before its first estimate.
const INITIAL_LEAD: u32 = 8;
/// The server sends an update every second tick: 30 Hz.
const SEND_INTERVAL: u32 = 2;
const LINK_SEED: u64 = 0x0011_4C1D;
/// The ticks each recording test records: six cycles, the stall and two fault windows of
/// each kind in the thrown scene inside them.
const RECORDED: u32 = 1_800;
/// The ticks the measurement replays: ten minutes.
const MEASURED: u32 = 36_000;

/// What one run does.
#[derive(Clone, Copy, Debug)]
struct Plan {
    scene: Scene,
    ticks: u32,
    clock: ClockScript,
    link_seed: u64,
    /// The end and the frame at which one bit of what that end sends is flipped: the
    /// lowest bit of the update's first `Transform` row, or of the newest sample's stick.
    flip: Option<(Side, u32)>,
    /// The tick after which the stream hash so far is kept.
    prefix: Option<u32>,
}

impl Plan {
    fn recorded(scene: Scene) -> Self {
        Self {
            scene,
            ticks: RECORDED,
            clock: ClockScript::Drifting,
            link_seed: LINK_SEED,
            flip: None,
            prefix: None,
        }
    }
}

/// What a run reports to its observer.
enum Observed<'a> {
    /// The server world, once built.
    Server(&'a World),
    /// The client's prediction world, once built.
    Client(&'a World),
    /// Both worlds at the end of a frame, before its record: the server's, and the client's
    /// prediction world once it is built.
    Worlds { tick: Tick, server: &'a World, client: Option<&'a World> },
    /// The record of a frame.
    Record(Tick, u64),
    /// The run's end, the client still running: an observer may take the client to run
    /// more frames of its own, at `clock`'s times from the frame after `frame`, and must
    /// put it back.
    Ended { client: &'a mut Option<Client>, clock: &'a ClientClock, frame: u32 },
}

type Observe<'a> = dyn FnMut(Observed<'_>) -> Result<()> + 'a;

/// What a run did, for its assertions and the measurement.
#[derive(Default)]
struct Outcome {
    records: Vec<(Tick, u64)>,
    stream: u64,
    prefix: Option<u64>,
    substituted: Vec<Tick>,
    /// Client frames that resynced, beside the join's snap.
    resyncs: Vec<u32>,
    kept_failures: u32,
    refused: u32,
    ticks_per_frame: [u32; 6],
    /// The client's tick minus the server's, per client frame.
    offsets: Vec<(u32, i64)>,
    rate: u32,
    /// The rate of every estimate, in order.
    rates: Vec<u32>,
    lead: u32,
    estimated_lead: f32,
    rollback_us: Vec<f64>,
    mismatches: Vec<Tick>,
    substitutions: u64,
    repairs: u64,
    divergence_entities: u64,
    divergence_fields: u64,
    compared: u64,
    agreed: u64,
    /// Of the ticks compared, those at which the player's `Hop` agreed.
    hop_agreed: u64,
    /// Of the ticks compared, those that ended with the server's carrier set.
    hop_set_compared: u32,
    /// The server ticks that ended with the player's `Hop` pending: the carrier set.
    hop_pending: u32,
    yaw_errors: Vec<f64>,
    delta_bytes: Vec<usize>,
    full_bytes: Vec<usize>,
    stats: Option<PredictionStats>,
    ring_depth: u32,
    ring_bytes: SlotBytes,
    whole_checks: u32,
    mirror_checks: u32,
    key_checks: u32,
    /// The predicted spawns both ends held at each check of the thrown scene.
    key_counts: Vec<usize>,
    datagrams_dropped: u64,
    deltas_dropped: u32,
    /// Server frames that ended with a loaded region in any mode but `Simulated`, or with
    /// the spike's region unloaded: a correction there inflates the rate (architecture
    /// section 9.4).
    server_frozen_ticks: u32,
    /// The most fixed colliders and proxies the prediction world held at the end of a client
    /// frame, which plan decision 5.5 bounds at [`PREDICTION_ROWS`].
    fixed_and_proxies: usize,
}

/// The client and what the spike keeps beside it.
struct ClientEnd {
    client: Option<Client>,
    receiver: Receiver,
    estimator: Estimator,
    clock: ClientClock,
    sequence: u16,
    newest: Option<(Tick, f64)>,
    arrived: bool,
    joined: u32,
    /// The last frame the client ran.
    ran: u32,
    /// The prediction world's hash and the player's `Hop` at the tick each frame ended on,
    /// compared with the server's as it passes that tick.
    predicted: BTreeMap<Tick, (u64, u32)>,
}

impl ClientEnd {
    fn client(&self) -> &Client {
        self.client.as_ref().expect("held between frames")
    }

    fn push(&mut self, frame: StagedFrame, outcome: &mut Outcome) {
        let client = self.client.as_mut().expect("held between frames");
        if !client.push(Arc::new(frame)) {
            outcome.refused += 1;
        }
    }
}

fn world_config(layout: LayoutConfig) -> WorldConfig {
    WorldConfig { layout, ..WorldConfig::headless() }
}

/// The sign-aware yaw of a rotation about `+Y`.
fn yaw_of(transform: &Transform) -> f64 {
    let rotation = transform.rotation;
    2.0 * rotation.y.to_f64().atan2(rotation.w.to_f64())
}

fn wrap(angle: f64) -> f64 {
    let turn = std::f64::consts::TAU;
    (angle + turn / 2.0).rem_euclid(turn) - turn / 2.0
}

/// The player's `Hop` in `world`.
fn hop_of(world: &World) -> Option<u32> {
    world.get::<Hop>(player_entity(world, PLAYER)?).map(|hop| hop.0)
}

/// The player's predicted spawns in `world`.
fn predicted_keys(world: &World) -> BTreeSet<StableKey> {
    world
        .entities_in(REGION)
        .filter(|key| matches!(key, StableKey::Predicted(predicted) if predicted.player == PLAYER))
        .collect()
}

/// The fixed colliders and proxies of every region of `world`: colliders with no body, and
/// bodies no player claims, as the slot test's queries count them (which need the world
/// `mut`).
fn fixed_and_proxies(world: &World) -> usize {
    world
        .regions()
        .flat_map(|region| world.entities_in(region))
        .filter(|&key| match world.get::<RigidBody>(key) {
            Some(_) => world.get::<ClaimedBy>(key).is_none(),
            None => world.get::<Collider>(key).is_some(),
        })
        .count()
}

/// Runs `plan` on `executor`, both worlds under `layout`, reporting to `observe`.
fn run(
    executor: &Executor,
    layout: LayoutConfig,
    plan: Plan,
    observe: &mut Observe<'_>,
) -> Result<Outcome> {
    let cooked = Arc::new(InMemory::new()?);
    let game = plan.scene.game();
    let shared = Arc::clone(&cooked) as Arc<dyn Cooked>;
    let mut server = build_server(executor, game, world_config(layout), Start::Fresh, shared)?;
    observe(Observed::Server(server.simulation().world()))?;
    let base = Instant::now();
    server.resync(base);
    let mut server = Some(server);

    let link = Arc::new(Mutex::new(Link::new(plan.link_seed, plan.scene.faults())));
    let mut server_end = Throwaway::new(Arc::clone(&link), Side::Server);
    let mut client_side = Throwaway::new(Arc::clone(&link), Side::Client);
    let (mut to_client, mut to_server) = (Messenger::default(), Messenger::default());
    let mut events = Vec::new();
    let mut packer = Packer::new();
    let mut inputs = Inputs::new();
    let mut resync_requested = false;
    let mut first = Some(cooked.fresh()?);
    let mut end: Option<ClientEnd> = None;
    let mut outcome = Outcome::default();
    let mut previous_yaw: Option<f64> = None;

    for n in 1..=plan.ticks {
        lock(&link).now = n;

        // The server's half: what arrived, the frame, the update.
        let current = server.take().expect("held between frames");
        for (kind, body) in receive(&mut server_end, &mut events) {
            match kind {
                INPUT => {
                    let packet = InputPacket::decode(&body)?;
                    inputs.receive(&packet, current.tick());
                    if let Some(ack) = packet.ack {
                        packer.acknowledge(ack);
                    }
                }
                RESYNC => {
                    decode_resync(&body)?;
                    resync_requested = true;
                }
                _ => return Err(invalid("the server received a server's message")),
            }
        }
        let mut owed = Vec::new();
        let (current, out) = {
            let mut frames = |tick: Tick| {
                let mut input = TickInput::default();
                if let Some(bundle) = first.take() {
                    input.loaded.insert(REGION, bundle);
                    input.sessions.insert(PLAYER, SessionEvent::Join { region: REGION });
                }
                if let Some((sample, substituted)) = inputs.take(tick) {
                    if substituted {
                        owed.push(tick);
                    }
                    input.inputs.insert(PLAYER, sample);
                }
                Some(input)
            };
            executor.block_on(current.frame(executor, base + TICK_PERIOD * n, &mut frames))?
        };
        if out.ticks != 1 || current.tick() != Tick(n) || !out.failures.is_empty() {
            return Err(invalid(format!(
                "server frame {n}: {} ticks, {:?}",
                out.ticks, out.failures
            )));
        }
        outcome.substituted.extend(owed.iter().copied());
        packer.owed.extend(owed);
        packer.absorb(&out.gathered)?;
        let world = current.simulation().world();
        let simulated = |region| world.region_mode(region) == Some(RegionMode::Simulated);
        let frozen = !world.regions().all(simulated) || world.region_mode(REGION).is_none();
        outcome.server_frozen_ticks += u32::from(frozen);
        packer.sync_claims(world);
        let tick = current.tick();
        if n == 1 || resync_requested {
            let join = if n == 1 {
                Some(player_entity(world, PLAYER).ok_or_else(|| invalid("the join did not run"))?)
            } else {
                None
            };
            let relevance = current.relevance(executor, REGION)?;
            let spawns = packer.resync(tick, world)?;
            let acked_input = inputs.consumed.unwrap_or(tick);
            let full = Full {
                tick,
                join,
                time_scale: current.time_scale(),
                acked_input,
                relevance,
                spawns,
            };
            let body = full.encode();
            outcome.full_bytes.push(body.len());
            to_client.send(&mut server_end, FULL, &body)?;
            resync_requested = false;
        } else if n % SEND_INTERVAL == 0 {
            let mut delta = packer.pack(world, tick, inputs.echo.take(), inputs.consumed)?;
            outcome.whole_checks += packer.check_whole(&delta, world)?;
            if plan.flip == Some((Side::Server, n))
                && let Some(row) = delta.rows.iter_mut().find(|row| row.component == Transform::KEY)
            {
                row.bytes[0] ^= 1;
            }
            let body = delta.encode();
            outcome.delta_bytes.push(body.len());
            to_client.send(&mut server_end, DELTA, &body)?;
        }
        if n % 60 == 0 {
            packer.check_mirror(world)?;
            outcome.mirror_checks += 1;
        }
        let server_hash = current.hash()?;
        if plan.scene == Scene::Capsule
            && let Some(key) = player_entity(world, PLAYER)
            && let (Some(transform), Some(input)) =
                (world.get::<Transform>(key), world.get::<PlayerInput>(key))
        {
            let yaw = yaw_of(transform);
            if let Some(previous) = previous_yaw
                && n % CYCLE > 1
            {
                let turn = f64::from(input.movement.x) / f64::from(AXIS_SCALE) * 2.0;
                let commanded = -turn / 60.0;
                outcome.yaw_errors.push(wrap(yaw - previous) - commanded);
            }
            previous_yaw = Some(yaw);
        }
        let hop = hop_of(world);
        outcome.hop_pending += u32::from(hop == Some(HOP_PENDING));
        if let Some(end) = &mut end
            && let Some((hash, predicted_hop)) = end.predicted.remove(&tick)
        {
            outcome.compared += 1;
            outcome.agreed += u64::from(hash == server_hash);
            outcome.hop_agreed += u64::from(Some(predicted_hop) == hop);
            outcome.hop_set_compared += u32::from(hop == Some(HOP_PENDING));
            end.predicted.retain(|&predicted, _| predicted.0 > tick.0);
        }
        server = Some(current);

        // The client's half: what arrived, the frame, the estimate, the inputs.
        let stalled = plan.clock == ClockScript::Drifting && STALL.contains(&n);
        if !stalled {
            // A client that knows it stalled measures no round trip across the stall: the
            // packets it sent before it waited in the link for it.
            if let Some(end) = &mut end
                && n > end.ran + 4
            {
                end.estimator.sent.clear();
            }
            for (kind, body) in receive(&mut client_side, &mut events) {
                match kind {
                    FULL => {
                        let full = Full::decode(&body)?;
                        if end.is_none() {
                            let entity =
                                full.join.ok_or_else(|| invalid("a resync before the join"))?;
                            let accepted = JoinAccepted { player: PLAYER, tick: full.tick };
                            let shared = Arc::clone(&cooked) as Arc<dyn Cooked>;
                            let config = world_config(layout);
                            let mut client = build_client(
                                executor,
                                game,
                                config,
                                accepted,
                                entity,
                                INITIAL_LEAD,
                                shared,
                            )?;
                            observe(Observed::Client(client.predictor().simulation().world()))?;
                            client.clock(ClockEstimate {
                                server_tick: full.tick,
                                lead_ticks: INITIAL_LEAD as f32,
                                rate_thousandths: 1000,
                                snap: true,
                            });
                            let clock =
                                ClientClock { base: Instant::now(), joined: n, script: plan.clock };
                            end = Some(ClientEnd {
                                client: Some(client),
                                receiver: Receiver::new(),
                                estimator: Estimator::new(),
                                clock,
                                sequence: 0,
                                newest: None,
                                arrived: false,
                                joined: n,
                                ran: n,
                                predicted: BTreeMap::new(),
                            });
                        }
                        let end = end.as_mut().expect("built");
                        if let Some(frame) = end.receiver.full(full)? {
                            end.push(frame, &mut outcome);
                        }
                    }
                    DELTA => {
                        let Some(end) = &mut end else { continue };
                        let delta = Delta::decode(&body)?;
                        let at = end.clock.periods(n);
                        if let Some(echo) = delta.echo {
                            end.estimator.echoed(echo, at);
                        }
                        if end.newest.is_none_or(|(newest, _)| delta.tick.0 > newest.0) {
                            end.newest = Some((delta.tick, at));
                            end.arrived = true;
                        }
                        if let Some(frame) = end.receiver.delta(&delta)? {
                            end.push(frame, &mut outcome);
                        }
                    }
                    _ => return Err(invalid("the client received a client's message")),
                }
            }
            if let Some(end) = &mut end {
                client_frame(
                    executor,
                    end,
                    n,
                    plan,
                    &mut outcome,
                    &mut to_server,
                    &mut client_side,
                )?;
            }
        }

        // The predicted keys of the thrown scene agree once every throw is settled.
        if plan.scene == Scene::Thrown
            && n % CYCLE == 200
            && let Some(end) = &end
        {
            let server = server.as_ref().expect("held").simulation().world();
            let client = end.client().predictor().simulation().world();
            let (expected, predicted) = (predicted_keys(server), predicted_keys(client));
            if expected != predicted {
                return Err(invalid(format!(
                    "frame {n}: the server holds {} predicted spawns, the client {}; {} in common",
                    expected.len(),
                    predicted.len(),
                    expected.intersection(&predicted).count()
                )));
            }
            outcome.key_checks += 1;
            outcome.key_counts.push(expected.len());
        }

        let client = end.as_ref().map(|end| end.client().predictor().simulation().world());
        let server_world = server.as_ref().expect("held").simulation().world();
        observe(Observed::Worlds { tick: Tick(n), server: server_world, client })?;

        // The record: both worlds and every datagram either end sent this frame.
        let mut bytes = Vec::with_capacity(64);
        bytes.extend_from_slice(&server_hash.to_le_bytes());
        if let Some(end) = &end {
            bytes.extend_from_slice(&end.client().tick().0.to_le_bytes());
            bytes.extend_from_slice(&end.client().hash()?.to_le_bytes());
        }
        for sent in std::mem::take(&mut lock(&link).sent) {
            bytes.extend_from_slice(&sent.to_le_bytes());
        }
        let record = xxh64(&bytes, 0);
        outcome.stream = xxh64(&[outcome.stream.to_le_bytes(), record.to_le_bytes()].concat(), 0);
        if plan.prefix == Some(n) {
            outcome.prefix = Some(outcome.stream);
        }
        outcome.records.push((Tick(n), record));
        observe(Observed::Record(Tick(n), record))?;
    }

    outcome.datagrams_dropped = lock(&link).dropped;
    if let Some(mut end) = end {
        let ended = Observed::Ended { client: &mut end.client, clock: &end.clock, frame: end.ran };
        observe(ended)?;
        let client = end.client.take().expect("held");
        outcome.stats = Some(client.predictor().stats());
        outcome.ring_depth = client.predictor().ring().depth();
        outcome.rate = end.estimator.rate;
        outcome.lead = client.lead();
        outcome.estimated_lead = end.estimator.lead;
        outcome.deltas_dropped = end.receiver.dropped;
        client.shutdown();
    }
    server.expect("held").shutdown();
    Ok(outcome)
}

/// One client frame and what follows it: the estimate, a resync request, the inputs.
fn client_frame(
    executor: &Executor,
    end: &mut ClientEnd,
    n: u32,
    plan: Plan,
    outcome: &mut Outcome,
    messenger: &mut Messenger,
    transport: &mut dyn Transport,
) -> Result<()> {
    let client = end.client.take().expect("held between frames");
    let scene = plan.scene;
    let mut sample = move |tick: Tick| scene.input(tick);
    let (client, out) =
        match executor.block_on(client.frame(executor, end.clock.now(n), &mut sample)) {
            Ok((client, out)) => (client, Some(out)),
            Err(ClientError { client: Some(kept), .. }) => {
                outcome.kept_failures += 1;
                (kept, None)
            }
            Err(ClientError { error, client: None }) => return Err(error),
        };
    end.client = Some(client);
    end.ran = n;
    let client = end.client.as_mut().expect("just put back");
    let held = fixed_and_proxies(client.predictor().simulation().world());
    outcome.fixed_and_proxies = outcome.fixed_and_proxies.max(held);
    let tick = client.tick();
    outcome.offsets.push((n, i64::from(tick.0) - i64::from(n)));
    let mut resync = out.is_none();
    if let Some(out) = &out {
        outcome.ticks_per_frame[(out.ticks as usize).min(5)] += 1;
        let reconciled = &out.reconciled;
        if reconciled.resimulated > 0 {
            let per_tick = out.rollback.as_secs_f64() * 1e6 / f64::from(reconciled.resimulated);
            outcome.rollback_us.push(per_tick);
        }
        for correction in &reconciled.corrections {
            match correction.cause {
                CorrectionCause::Mismatch => outcome.mismatches.push(correction.tick),
                CorrectionCause::Substituted => outcome.substitutions += 1,
                CorrectionCause::Repaired => outcome.repairs += 1,
            }
            outcome.divergence_entities += u64::from(correction.divergence.entities);
            outcome.divergence_fields += u64::from(correction.divergence.fields);
        }
        if out.resync.is_some() {
            end.predicted.clear();
            // The join's snap resyncs onto the full frame already queued.
            resync = n != end.joined;
        } else if let Some(oldest) = reconciled.corrections.iter().map(|c| c.tick.0).min() {
            end.predicted.retain(|predicted, _| predicted.0 < oldest);
        }
    }
    // Compared from the first frame whose world holds the player: the join's full frame is
    // folded a frame after the join.
    if let Some(hop) = hop_of(client.predictor().simulation().world()) {
        end.predicted.insert(tick, (client.hash()?, hop));
    }
    if n.is_multiple_of(30) {
        // A backend write of each slot not yet encoded (D143): it reads the copies and
        // changes no state, but costs about 11 ms a call on the thrown replays (1,258 calls,
        // 13.6 s, in a ten-minute server-clock replay in the first review's probe), which
        // lands in the replay's `wall_s` and in no rollback timer. Every 30 frames keeps the
        // ring's peak figure what it was before the copy.
        let ring = client.predictor().ring().bytes();
        let ring_bytes = &mut outcome.ring_bytes;
        ring_bytes.world = ring_bytes.world.max(ring.world);
        ring_bytes.physics = ring_bytes.physics.max(ring.physics);
        ring_bytes.record = ring_bytes.record.max(ring.record);
    }

    let now = end.clock.periods(n);
    if std::mem::take(&mut end.arrived)
        && let Some(newest) = end.newest
        && let Some(estimate) = end.estimator.update(n, newest, now, tick)
    {
        outcome.rates.push(estimate.rate_thousandths);
        client.clock(estimate);
    }
    if resync {
        outcome.resyncs.push(n);
        if !end.receiver.awaiting_full {
            end.receiver.awaiting_full = true;
            let request = ResyncRequest { tick, reason: ResyncReason::BehindCatchUp };
            messenger.send(transport, RESYNC, &encode_resync(&request))?;
        }
    }
    let mut samples: Vec<(Tick, PlayerInput)> =
        client.history().iter().map(|(tick, input)| (tick, *input)).collect();
    if plan.flip == Some((Side::Client, n))
        && let Some((_, newest)) = samples.last_mut()
    {
        newest.movement.x ^= 1;
    }
    let packet = InputPacket { sequence: end.sequence, ack: end.receiver.ack, samples };
    messenger.send(transport, INPUT, &packet.encode())?;
    end.estimator.sent(end.sequence, now);
    end.sequence = end.sequence.wrapping_add(1);
    Ok(())
}

// ---- What every recorded run must show ----

/// Whether a substitution at `tick` is one the plan causes: the drifting client's stall,
/// or a fault window of the thrown scene, each with the redundancy's reach after it.
fn planned_substitution(plan: Plan, tick: Tick) -> bool {
    let t = tick.0;
    let stall = plan.clock == ClockScript::Drifting && (STALL.start..STALL.end + 40).contains(&t);
    let faulted = (t.saturating_sub(24)..=t)
        .any(|frame| plan.scene.faults().window(Side::Client, frame).is_some());
    stall || faulted
}

fn planned_resync(plan: Plan, frame: u32) -> bool {
    plan.clock == ClockScript::Drifting && (STALL.start..STALL.end + 60).contains(&frame)
}

fn check_outcome(plan: Plan, outcome: &Outcome) {
    assert_eq!(outcome.records.len(), plan.ticks as usize);
    assert_eq!(outcome.kept_failures, 0, "no client frame failed");
    assert_eq!(outcome.refused, 0, "the inbox accepted every staged frame");
    let stray: Vec<_> =
        outcome.substituted.iter().filter(|&&tick| !planned_substitution(plan, tick)).collect();
    assert!(stray.is_empty(), "substitutions outside the plan: {stray:?}");
    let stray: Vec<_> =
        outcome.resyncs.iter().filter(|&&frame| !planned_resync(plan, frame)).collect();
    assert!(stray.is_empty(), "resyncs outside the stall: {stray:?}");
    assert!(outcome.whole_checks > 0 && outcome.mirror_checks > 0);
    // The carrier is set once a cycle, before phase 120, for the one tick between
    // `Stage::Post` and the steering that spends it.
    let cycles = (plan.ticks + CYCLE - 120) / CYCLE;
    assert_eq!(outcome.hop_pending, cycles, "the carrier fires once a cycle");
    // The carrier, written after the step and read by the next tick's steering, is
    // reconciled like any other column: the prediction world's agrees with the server's at
    // every tick compared.
    assert!(outcome.compared > 0);
    assert_eq!(outcome.hop_agreed, outcome.compared, "the predicted carrier is the server's");
    // Among them the tick it is set on, but in the claim scene: a prediction a correction
    // follows within the lead is not compared, and that scene corrects at every update
    // while the character pushes the unclaimed crate, which is when the carrier is set.
    if plan.scene != Scene::Claim {
        assert!(outcome.hop_set_compared > 0, "a tick with the carrier set was compared");
    }
    let stalls = plan.clock == ClockScript::Drifting && plan.ticks >= STALL.end + 60;
    if stalls {
        assert!(
            outcome.resyncs.iter().any(|&frame| planned_resync(plan, frame)),
            "the stall resyncs"
        );
    }
    if plan.clock == ClockScript::Drifting {
        assert!(
            outcome.ticks_per_frame[0] > 0 && outcome.ticks_per_frame[2] > 0,
            "the late wakes show in the ticks per frame: {:?}",
            outcome.ticks_per_frame
        );
    }
    // The estimate reaches the timer root: over the last 150 estimates the client runs, on
    // average, the rate that makes up its clock's half percent, and holds the lead the
    // estimate names.
    if plan.ticks >= RECORDED {
        let last = &outcome.rates[outcome.rates.len().saturating_sub(150)..];
        let mean = last.iter().map(|&rate| f64::from(rate)).sum::<f64>() / last.len() as f64;
        let expected = if plan.clock == ClockScript::Drifting { 1005.0 } else { 1000.0 };
        assert!((mean - expected).abs() <= 2.0, "the settled rate is {mean}, not {expected}");
    }
    assert_eq!(
        outcome.lead,
        outcome.estimated_lead.ceil() as u32,
        "the client's lead is the estimate's"
    );
    // Past the first estimates and outside the stall, the client stands its lead ahead.
    let wandering: Vec<_> = outcome
        .offsets
        .iter()
        .filter(|&&(frame, _)| frame > 120 && !planned_resync(plan, frame))
        .filter(|&&(_, offset)| !(3..=9).contains(&offset))
        .collect();
    assert!(
        wandering.is_empty(),
        "the client wandered from its lead: {:?}",
        &wandering[..wandering.len().min(8)]
    );
    if plan.scene == Scene::Thrown && plan.ticks >= 3 * CYCLE {
        assert_eq!(outcome.key_checks, plan.ticks / CYCLE);
        assert!(outcome.substituted.iter().any(|tick| (CYCLE..2 * CYCLE).contains(&tick.0)));
        assert!(outcome.substituted.iter().any(|tick| (2 * CYCLE..3 * CYCLE).contains(&tick.0)));
    }
}

// ---- Under SOUR ----

/// What a recording test builds from the runner's session.
struct RunnerSession {
    session: Session,
    executor: Executor,
    layout: LayoutConfig,
}

impl RunnerSession {
    fn from_env() -> Result<Self> {
        let session = Session::from_env().map_err(protocol_error)?;
        let config = *session.config();
        let layout = match config.layout {
            Layout::Production => layout_mode(ChunkCapacity::Production, 0)?,
            Layout::Perturbed { capacity: Capacity::Rows(n), seed } => {
                layout_mode(ChunkCapacity::Rows(n), seed)?
            }
            Layout::Perturbed { capacity: Capacity::Bytes(n), seed } => {
                layout_mode(ChunkCapacity::Bytes(n), seed)?
            }
        };
        let executor = start_executor(Workers {
            frame_workers: config.workers.map(NonZeroU32::get),
            chaos: config.chaos_seed.map(chaos),
            ..Workers::default()
        })?;
        Ok(Self { session, executor, layout })
    }

    /// Opens a recorder with what the executor and `world` report they were built with.
    fn recorder(&self, test: &str, world: &World) -> Result<protocol::Recorder> {
        let config = self.session.config();
        let applied = RunConfig {
            workers: config.workers.map(|_| {
                NonZeroU32::new(self.executor.frame_worker_count()).expect("at least one worker")
            }),
            layout: applied_layout(world.layout())?,
            chaos_seed: self.executor.chaos_config().map(|chaos| chaos.seed),
        };
        self.session.recorder(test, &applied).map_err(protocol_error)
    }
}

fn protocol_error(error: protocol::ProtocolError) -> Error {
    Error::new(ErrorKind::InvalidData, error.to_string())
}

/// The layout a world reports, as the runner spells it.
fn applied_layout(layout: &LayoutConfig) -> Result<Layout> {
    Ok(match layout_mode_of(layout)? {
        (ChunkCapacity::Production, _) => Layout::Production,
        (ChunkCapacity::Rows(n), seed) => Layout::Perturbed { capacity: Capacity::Rows(n), seed },
        (ChunkCapacity::Bytes(n), seed) => Layout::Perturbed { capacity: Capacity::Bytes(n), seed },
    })
}

fn chaos(seed: u64) -> ChaosConfig {
    ChaosConfig { seed, shuffle_steals: true, inject_yields: true, force_split: true, record: true }
}

/// One value per frame over the physics bytes of both worlds: `PhysicsSnapshot::hash` of the
/// server's world, then of the prediction world once the client is built. Recorded beside
/// the frame's record, never folded into it (D145): the two solvers never agree (architecture
/// section 16), where the rows the record hashes may.
fn physics_hash(server: &World, client: Option<&World>) -> Result<u64> {
    let mut bytes = Vec::with_capacity(16);
    bytes.extend_from_slice(&snapshot(server)?.hash().to_le_bytes());
    if let Some(client) = client {
        bytes.extend_from_slice(&snapshot(client)?.hash().to_le_bytes());
    }
    Ok(xxh64(&bytes, 0))
}

/// Records `scene` under the runner's session, as two streams: the frame's records under
/// `test` and the physics hash of each frame under `test` with `::physics` appended. Each
/// stream writes the server world's applied line and its records, the client world's applied
/// line once it is built, and the chaos log at the end.
fn recorded(scene: Scene, test: &str) {
    let runner = RunnerSession::from_env().unwrap();
    let physics_test = format!("{test}::physics");
    let mut recorders = None;
    let mut observe = |observed: Observed<'_>| -> Result<()> {
        match observed {
            Observed::Server(world) => {
                let records = runner.recorder(test, world)?;
                recorders = Some((records, runner.recorder(&physics_test, world)?));
            }
            Observed::Client(world) => {
                runner.recorder(test, world)?;
                runner.recorder(&physics_test, world)?;
            }
            Observed::Worlds { tick, server, client } => {
                let (_, physics) = recorders.as_mut().expect("opened with the server");
                physics.record(tick.0, physics_hash(server, client)?).map_err(protocol_error)?;
            }
            Observed::Record(tick, hash) => {
                let (recorder, _) = recorders.as_mut().expect("opened with the server");
                recorder.record(tick.0, hash).map_err(protocol_error)?;
            }
            Observed::Ended { .. } => {}
        }
        Ok(())
    };
    let plan = Plan::recorded(scene);
    let outcome = run(&runner.executor, runner.layout, plan, &mut observe).unwrap();
    check_outcome(plan, &outcome);
    if let Some(log) = runner.executor.execution_log() {
        for name in [test, &physics_test] {
            runner
                .session
                .write_execution_log(name, log.as_bytes())
                .map_err(protocol_error)
                .unwrap();
        }
    }
}

#[test]
fn a_dynamic_capsule_is_recorded() {
    recorded(Scene::Capsule, "lactose_app::spike::a_dynamic_capsule_is_recorded");
}

#[test]
fn a_kinematic_character_is_recorded() {
    recorded(Scene::Kinematic, "lactose_app::spike::a_kinematic_character_is_recorded");
}

#[test]
fn fifty_thrown_bodies_are_recorded() {
    recorded(Scene::Thrown, "lactose_app::spike::fifty_thrown_bodies_are_recorded");
}

#[test]
fn a_claim_in_contact_is_recorded() {
    recorded(Scene::Claim, "lactose_app::spike::a_claim_in_contact_is_recorded");
}

// ---- Outside the runner ----

fn executor(workers: u32) -> Executor {
    start_executor(Workers { frame_workers: Some(workers), ..Workers::default() })
        .expect("the executor")
}

fn quiet(_: Observed<'_>) -> Result<()> {
    Ok(())
}

/// The link delivers every datagram exactly its delay after it was sent, in an order drawn
/// within the frame; it loses about its share of messages, every fragment of one together,
/// and never a control message; and all of that follows its seed.
#[test]
fn the_link_reorders_only_within_a_tick_and_follows_its_seed() {
    let deliveries = |seed: u64| {
        let mut link = Link::new(seed, Faults::NONE);
        let mut delivered = Vec::new();
        for frame in 0..2_000_u32 {
            link.now = frame;
            for message in 0..3_u16 {
                let message = (frame as u16).wrapping_mul(3).wrapping_add(message);
                let kind = if message % 50 == 0 { FULL } else { DELTA };
                for index in 0..2_u8 {
                    let mut datagram = vec![kind];
                    datagram.extend_from_slice(&message.to_le_bytes());
                    datagram.extend_from_slice(&[index, 2]);
                    datagram.extend_from_slice(&frame.to_le_bytes());
                    link.send(Side::Server, &datagram);
                }
            }
            for datagram in link.deliver(Side::Client) {
                let sent = u32::from_le_bytes(datagram[5..9].try_into().expect("four"));
                let message = u16::from_le_bytes([datagram[1], datagram[2]]);
                delivered.push((frame, sent, datagram[0], message, datagram[3]));
            }
            assert!(link.deliver(Side::Server).is_empty(), "nothing went the other way");
        }
        delivered
    };
    let delivered = deliveries(LINK_SEED);
    for &(frame, sent, ..) in &delivered {
        assert_eq!(frame, sent + DELAY, "delivered exactly the delay after it was sent");
    }
    let sent = 3 * 2 * (2_000 - DELAY as usize);
    let lost = sent - delivered.iter().filter(|d| d.1 < 2_000 - DELAY).count();
    assert!((sent / 200..sent / 50).contains(&lost), "{lost} of {sent} lost");
    assert_eq!(lost % 2, 0, "both fragments of a message share its fate");
    let controls = delivered.iter().filter(|d| d.2 == FULL && d.1 < 2_000 - DELAY).count();
    assert_eq!(controls, 2 * (0..6_000 - 3 * DELAY as usize).filter(|m| m % 50 == 0).count());
    let reordered = delivered
        .windows(2)
        .filter(|pair| pair[0].0 == pair[1].0 && (pair[1].3, pair[1].4) < (pair[0].3, pair[0].4))
        .count();
    assert!(reordered > 1_000, "the order within a frame is drawn: {reordered}");
    assert_eq!(deliveries(LINK_SEED), delivered, "the same seed, the same link");
    assert_ne!(deliveries(LINK_SEED + 1), delivered, "another seed, another link");
}

/// One bit of one datagram, flipped on either end as it is sent, changes the record of the
/// frame it was sent in and none before: the stream hash covers the packets, and neither
/// world has seen the bit by then.
#[test]
fn a_packet_differing_by_one_bit_changes_the_record_from_its_tick() {
    let executor = executor(2);
    let plan = Plan { ticks: 90, ..Plan::recorded(Scene::Capsule) };
    let clean = run(&executor, LayoutConfig::default(), plan, &mut quiet).unwrap();
    for side in [Side::Server, Side::Client] {
        let flipped = Plan { flip: Some((side, 60)), ..plan };
        let dirty = run(&executor, LayoutConfig::default(), flipped, &mut quiet).unwrap();
        let first = clean.records.iter().zip(&dirty.records).position(|(a, b)| a != b);
        assert_eq!(first, Some(59), "{side:?}: the record of frame 60 is the first to differ");
    }
    executor.shutdown();
}

/// The thrown scene's records are the same under a perturbed layout with four workers as
/// under production with one.
#[test]
fn the_records_are_the_same_under_another_layout_and_worker_count() {
    let plan = Plan { ticks: 240, ..Plan::recorded(Scene::Thrown) };
    let records = |layout: LayoutConfig, workers: u32| {
        let executor = executor(workers);
        let outcome = run(&executor, layout, plan, &mut quiet).unwrap();
        executor.shutdown();
        check_outcome(plan, &outcome);
        outcome.records
    };
    let production = records(LayoutConfig::default(), 1);
    let perturbed = layout_mode(ChunkCapacity::Rows(7), 5).unwrap();
    assert_eq!(records(perturbed, 4), production);
}

// ---- The measurement ----

fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn bits(value: u64) -> f64 {
    Scalar::from_canonical_bits(value as lactose_math::ScalarBits).to_f64()
}

/// The figures of one measured run as one JSON object.
fn report(plan: Plan, outcome: &mut Outcome, recorded_stream: u64) -> String {
    let minutes = f64::from(plan.ticks) / 3_600.0;
    let stats = outcome.stats.unwrap_or_default();
    let absorbed = stats.absorbed;
    let tolerance = PredictionTolerance::DEFAULT;
    let worst_rollback = outcome.rollback_us.iter().copied().fold(0.0, f64::max);
    let median_rollback = median(&mut outcome.rollback_us);
    let mean = |values: &[usize]| values.iter().sum::<usize>() as f64 / values.len().max(1) as f64;
    // How long after a throw the claimed bodies kept correcting, in the worst cycle.
    let settle = if plan.scene == Scene::Thrown {
        (0..plan.ticks / CYCLE)
            .filter_map(|cycle| {
                let throw = cycle * CYCLE + 60;
                outcome
                    .mismatches
                    .iter()
                    .filter(|tick| (throw..cycle * CYCLE + CYCLE).contains(&tick.0))
                    .map(|tick| tick.0 - throw)
                    .max()
            })
            .max()
            .unwrap_or(0)
    } else {
        0
    };
    // The mismatches of each cycle, and the first after the join's cycle: where a scene
    // whose client agrees with the server leaves that agreement.
    let mut per_cycle = vec![0u32; plan.ticks.div_ceil(CYCLE) as usize];
    for tick in &outcome.mismatches {
        if let Some(count) = per_cycle.get_mut((tick.0 / CYCLE) as usize) {
            *count += 1;
        }
    }
    let departure = outcome.mismatches.iter().map(|tick| tick.0).filter(|&t| t >= CYCLE).min();
    let departure = departure.map_or_else(|| "null".to_string(), |tick| tick.to_string());
    let yaw_worst = outcome.yaw_errors.iter().map(|e| e.abs()).fold(0.0, f64::max);
    let yaw_mean = outcome.yaw_errors.iter().map(|e| e.abs()).sum::<f64>()
        / outcome.yaw_errors.len().max(1) as f64;
    format!(
        concat!(
            "{{\"scene\":\"{}\",\"clock_script\":{},\"ticks\":{},\"prefix_equal\":{},",
            "\"corrections_per_minute\":{{\"mismatch\":{:.2},\"substituted\":{:.2},\"repaired\":{:.2}}},",
            "\"tolerance\":{{\"translation\":{:e},\"rotation\":{:e},\"linear\":{:e},\"angular\":{:e}}},",
            "\"absorbed\":{{\"fields\":{},\"translation\":{:e},\"rotation\":{:e},\"linear\":{:e},\"angular\":{:e}}},",
            "\"rollback_us_per_tick\":{{\"median\":{:.1},\"worst\":{:.1},\"rollbacks\":{}}},",
            "\"divergence\":{{\"entities\":{},\"fields\":{},\"mismatched_entities\":{},\"mismatched_fields\":{}}},",
            "\"deepest_correction\":{},\"resyncs\":{},\"substituted_ticks\":{},\"unechoed_spawns\":{},",
            "\"hash_agreement\":{{\"compared\":{},\"agreed\":{},\"carrier_agreed\":{},\"carrier_set_compared\":{}}},",
            "\"carrier_pending_ticks\":{},\"departure_tick\":{},\"mismatches_per_cycle\":{:?},",
            "\"thrown_settle_ticks\":{},\"predicted_keys_at_checks\":{:?},\"yaw_error_rad_per_tick\":{{\"mean\":{:e},\"worst\":{:e}}},",
            "\"ring\":{{\"depth\":{},\"world_bytes\":{},\"physics_bytes\":{},\"record_bytes\":{}}},",
            "\"update_bytes\":{{\"mean\":{:.0},\"max\":{}}},\"full_bytes_max\":{},",
            "\"datagrams_dropped\":{},\"rate\":{},\"lead\":{},",
            "\"server_frozen_ticks\":{},\"prediction_fixed_and_proxies\":{}}}"
        ),
        plan.scene.name(),
        plan.clock == ClockScript::Drifting,
        plan.ticks,
        if plan.clock == ClockScript::Drifting {
            if outcome.prefix == Some(recorded_stream) { "true" } else { "false" }
        } else {
            "null"
        },
        outcome.mismatches.len() as f64 / minutes,
        outcome.substitutions as f64 / minutes,
        outcome.repairs as f64 / minutes,
        tolerance.translation.to_f64(),
        tolerance.rotation.to_f64(),
        tolerance.linear.to_f64(),
        tolerance.angular.to_f64(),
        absorbed.fields,
        bits(absorbed.translation),
        bits(absorbed.rotation),
        bits(absorbed.linear),
        bits(absorbed.angular),
        median_rollback,
        worst_rollback,
        outcome.rollback_us.len(),
        outcome.divergence_entities,
        outcome.divergence_fields,
        stats.mismatched_entities,
        stats.mismatched_fields,
        stats.deepest_correction,
        outcome.resyncs.len(),
        outcome.substituted.len(),
        stats.unechoed_spawns,
        outcome.compared,
        outcome.agreed,
        outcome.hop_agreed,
        outcome.hop_set_compared,
        outcome.hop_pending,
        departure,
        per_cycle,
        settle,
        outcome.key_counts,
        yaw_mean,
        yaw_worst,
        outcome.ring_depth,
        outcome.ring_bytes.world,
        outcome.ring_bytes.physics,
        outcome.ring_bytes.record,
        mean(&outcome.delta_bytes),
        outcome.delta_bytes.iter().max().copied().unwrap_or(0),
        outcome.full_bytes.iter().max().copied().unwrap_or(0),
        outcome.datagrams_dropped,
        outcome.rate,
        outcome.lead,
        outcome.server_frozen_ticks,
        outcome.fixed_and_proxies,
    )
}

/// Replays `scene` to ten minutes of ticks under `clock` on four workers and prints its
/// figures as one JSON object on stdout. Under the clock script the stream hash over the
/// first `RECORDED` ticks equals the recording test's.
fn measure(scene: Scene, clock: ClockScript) {
    let executor = executor(4);
    let recorded =
        run(&executor, LayoutConfig::default(), Plan::recorded(scene), &mut quiet).unwrap();
    let plan = Plan { ticks: MEASURED, clock, prefix: Some(RECORDED), ..Plan::recorded(scene) };
    let mut outcome = run(&executor, LayoutConfig::default(), plan, &mut quiet).unwrap();
    executor.shutdown();
    println!("{}", report(plan, &mut outcome, recorded.stream));
    if clock == ClockScript::Drifting {
        assert_eq!(
            outcome.prefix,
            Some(recorded.stream),
            "{}: the replay records as the test",
            scene.name()
        );
    }
}

// One test per scene and clock, each under the ten-minute limit `.config/nextest.toml`
// gives these tests. Run in release, one at a time so that the rollback times are the
// scene's own: `cargo nextest run -p lactose-app --test integration --release --run-ignored
// only --test-threads 1 --no-capture spike::`. Not shorter: on the server's clock the
// thrown scene corrects nothing until the client's solver history departs from the
// server's, minutes in, and every cycle after, so a run that ends before the departure
// reports almost none.

#[test]
#[ignore = "ten minutes of ticks, in release"]
fn ten_minutes_of_the_dynamic_capsule_with_the_clock_script() {
    measure(Scene::Capsule, ClockScript::Drifting);
}

#[test]
#[ignore = "ten minutes of ticks, in release"]
fn ten_minutes_of_the_dynamic_capsule_on_the_server_clock() {
    measure(Scene::Capsule, ClockScript::Exact);
}

#[test]
#[ignore = "ten minutes of ticks, in release"]
fn ten_minutes_of_the_kinematic_character_with_the_clock_script() {
    measure(Scene::Kinematic, ClockScript::Drifting);
}

#[test]
#[ignore = "ten minutes of ticks, in release"]
fn ten_minutes_of_the_kinematic_character_on_the_server_clock() {
    measure(Scene::Kinematic, ClockScript::Exact);
}

#[test]
#[ignore = "ten minutes of ticks, in release"]
fn ten_minutes_of_fifty_thrown_bodies_with_the_clock_script() {
    measure(Scene::Thrown, ClockScript::Drifting);
}

#[test]
#[ignore = "ten minutes of ticks, in release"]
fn ten_minutes_of_fifty_thrown_bodies_on_the_server_clock() {
    measure(Scene::Thrown, ClockScript::Exact);
}

#[test]
#[ignore = "ten minutes of ticks, in release"]
fn ten_minutes_of_the_claim_in_contact_with_the_clock_script() {
    measure(Scene::Claim, ClockScript::Drifting);
}

#[test]
#[ignore = "ten minutes of ticks, in release"]
fn ten_minutes_of_the_claim_in_contact_on_the_server_clock() {
    measure(Scene::Claim, ClockScript::Exact);
}

// Not recorded: the claim on approach holds one contact between claimed bodies, the
// character's with the lone crate, so its absorbed maxima are that contact's (D124's
// single-contact rule); the claim scene's mix the proxies and the stack in.
#[test]
#[ignore = "ten minutes of ticks, in release"]
fn ten_minutes_of_a_claim_on_approach_on_the_server_clock() {
    measure(Scene::Approach, ClockScript::Exact);
}

// ---- The slot and the rollback ----

/// Whether `cargo xtask measure --short` runs the test: fewer repetitions, the same figures.
fn short() -> bool {
    std::env::var_os("LACTOSE_MEASURE_SHORT").is_some()
}

/// One figure for `cargo xtask measure`, which reads the lines that start `measure: `.
fn figure(name: &str, value: impl std::fmt::Display) {
    println!("measure: {name} {value}");
}

fn micros(span: Duration) -> f64 {
    span.as_secs_f64() * 1e6
}

fn median_of(mut spans: Vec<Duration>) -> Duration {
    spans.sort();
    spans[spans.len() / 2]
}

/// The frames of the thrown scene before its measurement: the throw at frame 60 and ninety
/// frames of the fifty bodies in flight and landing.
const THROWN_FRAMES: u32 = 150;

/// Runs `scene` on the server's clock to [`THROWN_FRAMES`] on four workers, as the spike's
/// measurement does, then hands `measure` the executor, the client, its clock and the last
/// frame it ran.
fn after_the_throw(
    scene: Scene,
    mut measure: impl FnMut(&Executor, &mut Option<Client>, &ClientClock, u32) -> Result<()>,
) {
    let executor = executor(4);
    let plan = Plan { ticks: THROWN_FRAMES, clock: ClockScript::Exact, ..Plan::recorded(scene) };
    let mut ended = false;
    let mut observe = |observed: Observed<'_>| match observed {
        Observed::Ended { client, clock, frame } => {
            ended = true;
            measure(&executor, client, clock, frame)
        }
        _ => Ok(()),
    };
    run(&executor, LayoutConfig::default(), plan, &mut observe).unwrap();
    assert!(ended, "the client joined and ran to the end");
    executor.shutdown();
}

/// Runs the client's frame `n` alone, at `clock`'s time for it with `scene`'s sample, and
/// puts the client back.
fn alone_frame(
    executor: &Executor,
    held: &mut Option<Client>,
    clock: &ClientClock,
    scene: Scene,
    n: u32,
) -> Result<ClientOutput> {
    let mut sample = |tick: Tick| scene.input(tick);
    let client = held.take().expect("held between frames");
    match executor.block_on(client.frame(executor, clock.now(n), &mut sample)) {
        Ok((client, out)) => {
            *held = Some(client);
            Ok(out)
        }
        Err(ClientError { error, client }) => {
            *held = client;
            Err(error)
        }
    }
}

/// Criterion 5 (architecture section 22, plan decision 5.1): one prediction slot of the
/// thrown scene's client, its fifty bodies in flight, with Rapier, in an arena that holds
/// the most D139 admits: five fixed colliders and 59 unclaimed bodies, which the prediction
/// world holds as kinematic proxies, 64 together. The client first runs two frames alone,
/// so that its newest slot holds no server frame and weighs what this take weighs.
///
/// The take is a copy of what `lactose-predict` takes after every predicted tick, timed by
/// part: the world snapshot, the physics snapshot (the backend's copy of its world, D143),
/// and the record of every claimed entity's replicated persistent components but
/// `Teleported`. Each part is taken into a ring of its own as deep as the client's, which
/// is full before the first timed round, and each part's timer includes the drop of the
/// part the ring evicts for it, as `Ring::push` drops the oldest slot. Criterion 5 reads
/// `snapshot_us`, the world and physics snapshots together; `take_us` adds the record. The
/// restore is what a rollback restores, the world then the physics, from the newest slot;
/// criterion 5 reads `restore_us`. Each figure is the median of 200 rounds (20 in a short
/// run), after as many untimed rounds as the ring is deep. The three sizes of the newest
/// slot are asserted equal to the client's ring's for its newest slot, so that the copy
/// cannot drift from the take unseen, and the world hash after the restores is the hash
/// before them. `ring_bytes_first_us` is the sum of `size_in_bytes` over the held physics
/// snapshots, the first call encoding each but the newest (which the size check encoded),
/// as `Ring::bytes` on a full ring does; and
/// `ring_bytes_again_us` the same sum again. `cargo xtask measure prediction-slot` runs it.
#[test]
#[ignore = "a measurement, run by `cargo xtask measure`: timings are reported, not asserted"]
fn cost_of_a_prediction_slot() {
    let rounds = if short() { 20 } else { 200 };
    after_the_throw(Scene::Proxies, |executor, held, clock, last| {
        for n in 1..=2 {
            alone_frame(executor, held, clock, Scene::Proxies, last + n)?;
        }
        let client = held.as_mut().expect("held between frames");
        let (tick, hash) = (client.tick(), client.hash()?);
        let ring = client.predictor().ring().slot_bytes(tick).expect("the newest slot");
        let depth = usize::try_from(client.predictor().ring().depth()).expect("a small depth");
        let world = client.predictor_mut().simulation_mut().world_mut();
        let recorded: Vec<ComponentKey> = world
            .registry()
            .iter()
            .filter(|info| {
                info.role() == TypeRole::Component
                    && info.persistence() == Persistence::Persistent
                    && info.tracking().replicated
                    && info.key() != Teleported::KEY
            })
            .map(TypeInfo::key)
            .collect();
        let claimed = world.claimed(PLAYER).count();
        assert!(claimed > THROWN as usize, "the character and the thrown bodies: {claimed}");
        let fixed = world.query::<(&Collider, lactose_ecs::Without<RigidBody>)>().count();
        let proxies = world.query::<(&RigidBody, lactose_ecs::Without<ClaimedBy>)>().count();
        assert_eq!(fixed, FIXED as usize, "the arena's fixed colliders");
        assert_eq!(fixed + proxies, PREDICTION_ROWS as usize, "the most D139 admits");
        assert_eq!(fixed_and_proxies(world), fixed + proxies, "the replays' count agrees");
        let entities = world.entities_in(REGION).count();

        type Record = BTreeMap<StableKey, (RegionId, BTreeMap<ComponentKey, Vec<u8>>)>;
        /// Appends `part` to `ring` and drops the oldest beyond `depth`.
        fn push<T>(ring: &mut VecDeque<T>, part: T, depth: usize) {
            ring.push_back(part);
            if ring.len() > depth {
                drop(ring.pop_front());
            }
        }
        let mut worlds = VecDeque::with_capacity(depth + 1);
        let mut physics = VecDeque::with_capacity(depth + 1);
        let mut records: VecDeque<Record> = VecDeque::with_capacity(depth + 1);
        let mut times: [Vec<Duration>; 6] = Default::default();
        for round in 0..depth + rounds {
            let start = Instant::now();
            push(&mut worlds, world.snapshot(), depth);
            let taken_world = Instant::now();
            push(&mut physics, lactose_physics_api::snapshot(world)?, depth);
            let taken_physics = Instant::now();
            let record: Record = world
                .claimed(PLAYER)
                .filter_map(|key| {
                    let components = recorded
                        .iter()
                        .filter_map(|&component| {
                            Some((component, world.get_erased(key, component)?.to_vec()))
                        })
                        .collect();
                    Some((key, (world.region_of(key)?, components)))
                })
                .collect();
            push(&mut records, record, depth);
            let taken = Instant::now();

            let start_restore = Instant::now();
            world.restore(worlds.back().expect("just taken"))?;
            let restored_world = Instant::now();
            lactose_physics_api::restore(world, physics.back().expect("just taken"))?;
            let restored = Instant::now();
            if round >= depth {
                times[0].push(taken_world - start);
                times[1].push(taken_physics - taken_world);
                times[2].push(taken - taken_physics);
                times[3].push(restored_world - start_restore);
                times[4].push(restored - restored_world);
                times[5].push(restored - start_restore);
            }
        }
        assert_eq!(client.hash()?, hash, "a restore of the slot just taken changes nothing");
        assert_eq!((worlds.len(), physics.len(), records.len()), (depth, depth, depth));
        let record_bytes = records
            .back()
            .expect("the newest")
            .values()
            .flat_map(|(_, components)| components.values())
            .map(Vec::len);
        let bytes = SlotBytes {
            world: worlds.back().expect("the newest").size_in_bytes(),
            physics: physics.back().expect("the newest").size_in_bytes(),
            record: record_bytes.sum(),
        };
        assert_eq!(bytes, ring, "this take weighs what the ring's newest slot weighs");
        // The newest physics snapshot is encoded by now; the sum encodes the others.
        let ring_bytes = || physics.iter().map(|taken| taken.size_in_bytes()).sum::<usize>();
        let start = Instant::now();
        let ring_physics_bytes = ring_bytes();
        let first = start.elapsed();
        let start = Instant::now();
        assert_eq!(ring_bytes(), ring_physics_bytes);
        let again = start.elapsed();

        let [world_us, physics_us, record_us, world_restore_us, physics_restore_us, restore_us] =
            times.map(|spans| micros(median_of(spans)));
        let us = |value: f64| format!("{value:.1}");
        figure("workers", 4);
        figure("frames", THROWN_FRAMES + 2);
        figure("tick", tick.0);
        figure("rounds", rounds);
        figure("ring_depth", depth);
        figure("entities", entities);
        figure("claimed", claimed);
        figure("fixed_colliders", fixed);
        figure("proxies", proxies);
        figure("world_snapshot_us", us(world_us));
        figure("physics_snapshot_us", us(physics_us));
        figure("snapshot_us", us(world_us + physics_us));
        figure("record_us", us(record_us));
        figure("take_us", us(world_us + physics_us + record_us));
        figure("world_restore_us", us(world_restore_us));
        figure("physics_restore_us", us(physics_restore_us));
        figure("restore_us", us(restore_us));
        figure("world_bytes", bytes.world);
        figure("physics_bytes", bytes.physics);
        figure("record_bytes", bytes.record);
        figure("ring_physics_bytes", ring_physics_bytes);
        figure("ring_bytes_first_us", us(micros(first)));
        figure("ring_bytes_again_us", us(micros(again)));
        Ok(())
    });
}

/// How far back the rollback's correction lands: a frame for the tick six before the
/// client's, whose next frame predicts one more tick before it reconciles.
const ROLLBACK_DEPTH: u32 = 6;
/// Frames the client runs alone first, so that a correction that far back is after every
/// frame the server sent it.
const ALONE: u32 = 8;

/// Criterion 7 (architecture section 22, recorded): a seven-tick rollback on the client of
/// the thrown scene, its fifty bodies in flight, with Rapier: a frame for the tick
/// [`ROLLBACK_DEPTH`] before the client's, whose exact row moves the character a metre,
/// then one client frame, which predicts its tick, restores the slot of the frame's tick,
/// corrects it and re-simulates the seven ticks to the present. `ClientOutput::rollback`
/// is the reconcile's wall time; each figure is the median of 20 rollbacks (3 in a short
/// run). `cargo xtask measure rollback-seven-ticks` runs it.
#[test]
#[ignore = "a measurement, run by `cargo xtask measure`: timings are reported, not asserted"]
fn cost_of_a_seven_tick_rollback() {
    let rollbacks = if short() { 3 } else { 20 };
    after_the_throw(Scene::Thrown, |executor, held, clock, last| {
        let frame = |held: &mut Option<Client>, n: u32| {
            alone_frame(executor, held, clock, Scene::Thrown, n)
        };
        for n in 1..=ALONE {
            frame(held, last + n)?;
        }
        let (mut spans, mut resimulated) = (Vec::new(), Vec::new());
        for n in 1..=rollbacks {
            let client = held.as_mut().expect("held between frames");
            let present = client.tick();
            let world = client.predictor().simulation().world();
            let key = player_entity(world, PLAYER).expect("the character");
            let at = *world.get::<Transform>(key).expect("the character's transform");
            let metre = Vec3::new(scalar(1.0), Scalar::ZERO, Scalar::ZERO);
            let moved = Transform::new(at.translation + metre, at.rotation);
            let mut staged = StagedFrame::new(Tick(present.0 - ROLLBACK_DEPTH));
            let bytes = lactose_reflect::bytemuck::bytes_of(&moved);
            assert!(staged.push_row(key, Transform::KEY, staged.tick, true, bytes));
            assert!(client.push(Arc::new(staged)), "the frame is after every one accepted");
            let out = frame(held, last + ALONE + n)?;
            assert_eq!(out.reconciled.corrections.len(), 1, "one correction, at the frame's tick");
            assert!(out.resync.is_none(), "a correction inside the ring, not a resync");
            resimulated.push(out.reconciled.resimulated);
            spans.push(out.rollback);
        }
        assert!(
            resimulated.iter().all(|&ticks| ticks == ROLLBACK_DEPTH + 1),
            "every rollback re-simulates seven ticks: {resimulated:?}"
        );
        let span = median_of(spans);
        figure("workers", 4);
        figure("frames", THROWN_FRAMES + ALONE);
        figure("rollbacks", rollbacks);
        figure("resimulated", ROLLBACK_DEPTH + 1);
        figure("rollback_us", format!("{:.1}", micros(span)));
        figure("per_tick_us", format!("{:.1}", micros(span) / f64::from(ROLLBACK_DEPTH + 1)));
        Ok(())
    });
}
