//! What the tests of this crate share: authored data built in code (skeletons, clips,
//! machines), a store that publishes them, and a simulation of one region with this
//! crate's systems and the recording systems the tests read back.

#![allow(dead_code, reason = "each test binary uses part of the shared support")]

use lactose_anim::{
    AnimEvent, AnimEventKey, AnimParams, AnimRig, AnimState, Bone, BoneBody, BoneJoint, BoneKey,
    BoneLink, BoneMoved, BonePose, BoneRole, Bones, Clip, ClipEvent, Condition, Phase, RootMotion,
    RootSample, SimPose, Skeleton, Socket, State, StateKey, StateMachine, Transition, sets,
};
use lactose_asset_api::{
    AssetId, AssetRef, AssetVersion, LoadPriority, PinnedTable, PublishedAssets, Readiness,
    SimulationAsset, UntypedHandle, VersionTable,
};
use lactose_core::{NetId, PlayerId, Result, StableKey};
use lactose_ecs::{
    ClaimedBy, Commands, Ctx, Events, Executor, LayoutConfig, Mut, RegionConfig, RegionId,
    RegionMode, Res, ResMut, StableKeyOf, SystemSet, WorldKind, system,
};
use lactose_math::{Quat, Scalar, Vec3};
use lactose_physics_api::{
    Collider, CollisionGroups, JointKind, PhysicsConfig, RigidBody, Shape, Transform,
};
use lactose_reflect::component;
use lactose_sim::{
    CurrentTick, Registration, Simulation, SimulationBuilder, Stage, TickInput, TypeTable,
};

#[path = "../../../physics-api/tests/common/fake.rs"]
pub mod fake;

pub use fake::{CallLog, Fake};

pub const REGION: RegionId = RegionId(1);
pub const PLAYER: PlayerId = PlayerId(1);

/// A cooked `Net` key.
pub fn key(n: u64) -> StableKey {
    StableKey::Net(NetId::cooked(n))
}

pub fn s(value: f64) -> Scalar {
    Scalar::from_f64(value)
}

pub fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(s(x), s(y), s(z))
}

pub fn yaw(angle: f64) -> Quat {
    Quat::from_rotation_y(s(angle))
}

pub fn executor(workers: u32) -> Executor {
    Executor::builder().frame_workers(workers).background_workers(1).build().unwrap()
}

// ---- Assets ----

pub const SKELETON: AssetRef<Skeleton> = AssetRef::new(AssetId::from_name("test/skeleton"));
pub const MACHINE: AssetRef<StateMachine> = AssetRef::new(AssetId::from_name("test/machine"));

pub const fn clip_ref(name: &str) -> AssetRef<Clip> {
    AssetRef::new(AssetId::from_name(name))
}

/// The bone group set every test skeleton's bones share, the groups that may hit a bone,
/// and the character capsule's group (`README.md`, "Bones and ragdolls").
pub const CAPSULE_GROUP: u32 = 1;
pub const BONE_GROUP: u32 = 1 << 1;
pub const HIT_GROUP: u32 = 1 << 2;

/// A body for a bone: a small sphere in the bone group, hit by the hit group alone, jointed
/// to its nearest ancestor with a body when `joint`.
pub fn bone_body(joint: bool) -> BoneBody {
    let collider = Collider::new(Shape::sphere(s(0.1)))
        .with_groups(CollisionGroups::new(BONE_GROUP, HIT_GROUP));
    let joint = joint.then_some(BoneJoint {
        kind: JointKind::Spherical,
        anchor_parent: v(0.0, 1.0, 0.0),
        anchor_bone: Vec3::ZERO,
        axis: Vec3::Y,
        limits: None,
    });
    BoneBody { body: RigidBody::dynamic(), collider, joint }
}

const ROLES: [BoneRole; 8] = [
    BoneRole::Hips,
    BoneRole::Spine,
    BoneRole::Head,
    BoneRole::HandLeft,
    BoneRole::HandRight,
    BoneRole::FootLeft,
    BoneRole::FootRight,
    BoneRole::Other,
];

/// A chain of `bones` bones, each a unit above its parent and turned a little about `+Y`,
/// the first seven with roles; `bodies` says which bones carry a body (the first with one
/// is the ragdoll's root, every later one jointed to its nearest ancestor with a body). One
/// socket on the last bone.
pub fn chain(bones: usize, bodies: impl Fn(usize) -> bool) -> Skeleton {
    let mut seen_body = false;
    let bones = (0..bones)
        .map(|index| {
            let body = bodies(index).then(|| {
                let body = bone_body(seen_body);
                seen_body = true;
                body
            });
            Bone {
                key: BoneKey(100 + index as u64),
                parent: index.checked_sub(1).map(|parent| parent as u8),
                role: ROLES.get(index).copied().unwrap_or(BoneRole::Other),
                rest: BonePose::new(v(0.0, 1.0, 0.0), yaw(0.05 * index as f64)),
                body,
            }
        })
        .collect::<Vec<_>>();
    let last = (bones.len() - 1) as u8;
    let sockets = vec![Socket {
        key: BoneKey(900),
        bone: last,
        offset: BonePose::new(v(0.0, 0.0, -0.5), Quat::IDENTITY),
    }];
    Skeleton { bones, sockets }
}

/// Frame `frame`'s sample of bone `bone` in the test clips: a unit above the parent, a
/// little forward per frame, turned by an angle of the frame and the bone.
pub fn sample(frame: u32, bone: u8, salt: f64) -> BonePose {
    let f = f64::from(frame);
    let b = f64::from(bone);
    BonePose::new(v(0.0, 1.0, -0.01 * f), yaw(salt + 0.07 * f + 0.013 * b))
}

/// A clip for [`SKELETON`] of `bones` bones and `frames` frames lasting `duration` ticks,
/// its samples by [`sample`] with `salt`, no root track and no events.
pub fn clip(bones: u8, frames: u32, duration: u32, looping: bool, salt: f64) -> Clip {
    let samples = (0..frames)
        .flat_map(|frame| (0..bones).map(move |bone| sample(frame, bone, salt)))
        .collect();
    Clip {
        skeleton: SKELETON,
        duration_ticks: duration,
        frames,
        looping,
        bones,
        samples,
        root: Vec::new(),
        events: Vec::new(),
    }
}

/// `clip` with a root track walking `step` along `-Z` per frame and turning `turn` radians
/// per frame: `frames + 1` entries for a loop, `frames` for a one-shot.
pub fn with_root(mut clip: Clip, step: f64, turn: f64) -> Clip {
    let entries = clip.frames + u32::from(clip.looping);
    clip.root = (0..entries)
        .map(|frame| RootSample {
            translation: v(0.0, 0.0, -step * f64::from(frame)),
            yaw: s(turn * f64::from(frame)),
        })
        .collect();
    clip
}

/// `clip` with events at `phases`, each keyed by its index.
pub fn with_events(mut clip: Clip, phases: &[Phase]) -> Clip {
    clip.events = phases
        .iter()
        .enumerate()
        .map(|(index, &phase)| ClipEvent { phase, event: AnimEventKey(index as u64 + 1) })
        .collect();
    clip
}

/// A state playing `clip` at `speed`, keyed by `name`.
pub fn state(name: &str, clip: AssetRef<Clip>, speed: f64) -> State {
    State { key: StateKey::from_name(name), clip, speed: s(speed), speed_param: None }
}

/// A transition `from` → `to` under `conditions`: no blend, the entry phase set to zero.
pub fn transition(from: Option<u16>, to: u16, conditions: &[Condition]) -> Transition {
    Transition {
        from,
        to,
        conditions: conditions.to_vec(),
        blend_ticks: 0,
        keep_phase: false,
        entry_phase: Phase::ZERO,
    }
}

/// The assets a test publishes, by ID.
#[derive(Clone, Default)]
pub struct Library {
    entries: Vec<UntypedHandle>,
}

impl Library {
    pub fn new() -> Self {
        Self::default()
    }

    /// The library with `asset` published under `reference`.
    pub fn with<T: SimulationAsset>(mut self, reference: AssetRef<T>, asset: T) -> Self {
        self.entries.retain(|handle| handle.id() != reference.id());
        self.entries.push(UntypedHandle::new(reference.id(), version(), asset));
        self
    }

    /// A table of every asset.
    pub fn pin(&self) -> PinnedTable {
        self.pin_without(&[])
    }

    /// A table of every asset but `missing`.
    pub fn pin_without(&self, missing: &[AssetId]) -> PinnedTable {
        let table: VersionTable = self
            .entries
            .iter()
            .map(UntypedHandle::id)
            .filter(|id| !missing.contains(id))
            .map(|id| (id, version()))
            .collect();
        PinnedTable::pin(&Store(&self.entries), &table).ready().unwrap()
    }
}

fn version() -> AssetVersion {
    AssetVersion::from_bits(1).unwrap()
}

struct Store<'a>(&'a [UntypedHandle]);

impl PublishedAssets for Store<'_> {
    fn request(&self, _: AssetId, _: AssetVersion, _: LoadPriority) {}

    fn get(&self, id: AssetId, _: AssetVersion) -> Readiness<UntypedHandle> {
        match self.0.iter().find(|handle| handle.id() == id) {
            Some(handle) => Readiness::Ready(handle.clone()),
            None => Readiness::Pending,
        }
    }
}

// ---- The simulation ----

/// What the recording systems saw, in order: every `AnimEvent` after the evaluation and
/// every `BoneMoved` after the fit, with the tick.
#[derive(Default)]
pub struct Log {
    pub events: Vec<(u32, AnimEvent)>,
    pub moved: Vec<(u32, BoneMoved)>,
}

/// What a character's own kernels saw this tick: the `AnimEvent`s targeted at it (read by
/// a system after the advance with `for_target`), its pose's bone count after the
/// evaluation and after the fit.
#[component(key = "test::seen", persistent, replicated)]
#[derive(PartialEq, Eq, Debug, Default)]
pub struct Seen {
    #[field(id = 1)]
    pub events: u32,
    #[field(id = 2)]
    pub simulation_bones: u32,
    #[field(id = 3)]
    pub post_bones: u32,
}

pub static AFTER_EVALUATE: SystemSet =
    SystemSet::new("test::after_evaluate").after(&[&sets::EVALUATE]);
pub static AFTER_BONES: SystemSet = SystemSet::new("test::after_bones").after(&[&sets::BONES]);

#[system(name = "test::record_events", worlds = [server, prediction], in_set = AFTER_EVALUATE)]
fn record_events(now: Res<CurrentTick>, events: Events<AnimEvent>, log: ResMut<Log>) {
    let tick = now.get().0;
    log.events.extend(events.iter().map(|&event| (tick, event)));
}

#[system(name = "test::seen_simulation", worlds = [server, prediction], in_set = AFTER_EVALUATE)]
fn seen_simulation(key: StableKeyOf, events: Events<AnimEvent>, pose: &SimPose, seen: Mut<Seen>) {
    let mine = u32::try_from(events.for_target(*key).count()).unwrap();
    seen.set_if_neq(Seen { events: mine, simulation_bones: pose.len() as u32, post_bones: 0 });
}

#[system(name = "test::record_moved", worlds = [server, prediction], in_set = AFTER_BONES)]
fn record_moved(now: Res<CurrentTick>, moved: Events<BoneMoved>, log: ResMut<Log>) {
    let tick = now.get().0;
    log.moved.extend(moved.iter().map(|&event| (tick, event)));
}

#[system(name = "test::seen_post", worlds = [server, prediction], in_set = AFTER_BONES)]
fn seen_post(pose: &SimPose, seen: Mut<Seen>) {
    seen.set_if_neq(Seen { post_bones: pose.len() as u32, ..*seen });
}

/// What the server-only ragdoll kernel does to a character, at a tick.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RagdollAction {
    /// `Bones::spawn` with the character's current bones and pose.
    Spawn,
    /// `Bones::set_limp` with this value.
    Limp(bool),
}

/// The ragdoll script: `(tick, character, action)`.
#[derive(Default)]
pub struct Ragdoll {
    pub actions: Vec<(u32, StableKey, RagdollAction)>,
}

/// The game's ragdoll kernel as `tests/consumers.rs` sketches it: server-only, after
/// `sets::BONES`, declaring what the two helpers write.
#[system(
    name = "test::ragdoll",
    worlds = [server],
    in_set = AFTER_BONES,
    writes = [Bones, BoneLink, RigidBody]
)]
#[expect(clippy::too_many_arguments, reason = "a kernel's parameters are its row columns")]
fn ragdoll(
    ctx: Ctx,
    cmd: Commands,
    now: Res<CurrentTick>,
    script: Res<Ragdoll>,
    key: StableKeyOf,
    rig: &AnimRig,
    bones: Option<&Bones>,
    pose: &SimPose,
    root: &Transform,
) {
    let Some(assets) = ctx.assets() else { return };
    let Ok(skeleton) = assets.get(rig.skeleton) else { return };
    let tick = now.get().0;
    for &(at, target, action) in &script.actions {
        if at != tick || target != *key {
            continue;
        }
        match action {
            RagdollAction::Spawn => Bones::spawn(&mut cmd, *key, bones, skeleton, root, pose),
            RagdollAction::Limp(limp) => {
                if let Some(bones) = bones {
                    bones.set_limp(&mut cmd, *key, skeleton, limp);
                }
            }
        }
    }
}

fn register_recording(registration: &mut Registration) {
    registration
        .stage(Stage::Simulation)
        .sets(&[&AFTER_EVALUATE])
        .add(&RECORD_EVENTS)
        .add(&SEEN_SIMULATION);
    registration
        .stage(Stage::Post)
        .sets(&[&AFTER_BONES])
        .add(&RECORD_MOVED)
        .add(&SEEN_POST)
        .add(&RAGDOLL);
}

pub fn register_test_types(types: &mut TypeTable) -> Result<()> {
    lactose_physics_api::register_types(types)?;
    lactose_anim::register_types(types)?;
    types.register::<Seen>()
}

/// How to build a [`Fixture`].
#[derive(Clone, Copy)]
pub struct Options {
    pub kind: WorldKind,
    pub layout: LayoutConfig,
    pub workers: u32,
    /// More systems beside this crate's and the recording ones.
    pub systems: Option<fn(&mut Registration)>,
    /// A setup beside the log's, run when the world is built.
    pub setup: Option<fn(&mut lactose_ecs::World) -> Result<()>>,
    /// Whether physics-api's systems run, with the test backend behind them.
    pub physics: bool,
}

impl Options {
    pub fn new(kind: WorldKind) -> Self {
        Self {
            kind,
            layout: LayoutConfig::default(),
            workers: 1,
            systems: None,
            setup: None,
            physics: false,
        }
    }

    pub fn build(self) -> Fixture {
        self.build_on(executor(self.workers))
    }

    /// The fixture stepped on `executor`, as a SOUR test builds its own.
    pub fn build_on(self, executor: Executor) -> Fixture {
        let mut types = TypeTable::new();
        register_test_types(&mut types).unwrap();
        let calls = CallLog::default();
        let mut builder = SimulationBuilder::new(self.kind, types)
            .layout(self.layout)
            .setup(|world| {
                world.insert_service::<Log>(Box::new(Log::default()));
                world.insert_service::<Ragdoll>(Box::new(Ragdoll::default()));
                Ok(())
            })
            .systems(lactose_anim::register_systems)
            .systems(register_recording);
        if self.physics {
            let backend = Box::new(Fake::new(calls.clone()));
            builder = builder
                .setup(move |world| {
                    lactose_physics_api::setup(world, backend, PhysicsConfig::default())
                })
                .systems(lactose_physics_api::register_systems);
        }
        if let Some(setup) = self.setup {
            builder = builder.setup(setup);
        }
        if let Some(systems) = self.systems {
            builder = builder.systems(systems);
        }
        if self.kind == WorldKind::Prediction {
            builder = builder.check(lactose_predict::check_prediction_reads);
        }
        let mut simulation = builder.build().unwrap();
        let region = RegionConfig { id: REGION, seed: 1, mode: RegionMode::Simulated };
        simulation.world_mut().create_region(region).unwrap();
        Fixture { simulation: Some(simulation), executor, calls }
    }
}

/// One region's simulation with this crate's systems, stepped on its own executor.
pub struct Fixture {
    simulation: Option<Simulation>,
    pub executor: Executor,
    /// What the test backend was told, when physics runs.
    pub calls: CallLog,
}

impl Fixture {
    pub fn new(kind: WorldKind) -> Self {
        Options::new(kind).build()
    }

    pub fn simulation(&self) -> &Simulation {
        self.simulation.as_ref().unwrap()
    }

    pub fn simulation_mut(&mut self) -> &mut Simulation {
        self.simulation.as_mut().unwrap()
    }

    pub fn world(&self) -> &lactose_ecs::World {
        self.simulation().world()
    }

    pub fn world_mut(&mut self) -> &mut lactose_ecs::World {
        self.simulation_mut().world_mut()
    }

    /// Pins `table` as the region's assets, as the input sync point does.
    pub fn assets(&mut self, table: PinnedTable) {
        self.world_mut().set_region_assets(REGION, table).unwrap();
    }

    /// Spawns a character with the five components of this crate, claimed by the player so
    /// a prediction world runs it.
    pub fn spawn(&mut self, key: StableKey, rig: AnimRig) {
        self.spawn_at(key, rig, Transform::IDENTITY);
    }

    pub fn spawn_at(&mut self, key: StableKey, rig: AnimRig, root: Transform) {
        let bundle = (
            root,
            rig,
            AnimState::UNSET,
            AnimParams::ZERO,
            RootMotion::ZERO,
            SimPose::EMPTY,
            Seen::default(),
            ClaimedBy { player: PLAYER },
        );
        self.world_mut().spawn(key, REGION, bundle).unwrap();
    }

    pub fn step(&mut self) {
        let simulation = self.simulation.take().unwrap();
        let next = self.executor.block_on(simulation.step(&self.executor, TickInput::default()));
        self.simulation = Some(next.unwrap());
    }

    pub fn run(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.step();
        }
    }

    pub fn tick(&self) -> u32 {
        self.simulation().tick().0
    }

    pub fn hash(&self) -> u64 {
        self.simulation().hash().unwrap()
    }

    pub fn state(&self, key: StableKey) -> AnimState {
        *self.world().get::<AnimState>(key).unwrap()
    }

    pub fn params(&self, key: StableKey) -> AnimParams {
        *self.world().get::<AnimParams>(key).unwrap()
    }

    pub fn motion(&self, key: StableKey) -> RootMotion {
        *self.world().get::<RootMotion>(key).unwrap()
    }

    pub fn pose(&self, key: StableKey) -> SimPose {
        *self.world().get::<SimPose>(key).unwrap()
    }

    pub fn seen(&self, key: StableKey) -> Seen {
        *self.world().get::<Seen>(key).unwrap()
    }

    /// Rewrites the character's parameters between ticks, as a `sets::DRIVE` writer would.
    pub fn set_params(&mut self, key: StableKey, write: impl FnOnce(&mut AnimParams)) {
        let mut params = self.params(key);
        write(&mut params);
        let mut row = self.world_mut().get_mut::<AnimParams>(key).unwrap();
        row.set_if_neq(params);
    }

    pub fn log(&self) -> &Log {
        self.world().service::<Log>().unwrap()
    }

    pub fn take_events(&mut self) -> Vec<(u32, AnimEvent)> {
        std::mem::take(&mut self.world_mut().service_mut::<Log>().unwrap().events)
    }
}
