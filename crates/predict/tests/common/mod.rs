//! What the integration tests share: a small game that runs in the prediction world (a
//! player who walks by its input, a derived cache that follows `Changed<Position>`, an
//! emitter of presentation events, an extract pass that counts what it sees), the
//! prediction world `app` builds, with or without the physics test backend, a rig that
//! keeps the predictor across runs as `app` does, and the frames a server would send.

#![allow(dead_code, reason = "each test file uses part of the shared module")]

#[path = "../../../physics-api/tests/common/fake.rs"]
pub mod fake;

use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};

use lactose_bundle::ComponentData;
use lactose_core::{Error, NetId, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{
    Changed, ClaimedBy, Commands, Executor, LayoutConfig, Mut, PresentEvent, PresentEvents,
    PresentKind, RegionConfig, RegionId, RegionMode, Res, StableKeyOf, World, WorldKind, system,
};
use lactose_input::{Action, Actions, Axis2, PlayerInput, PlayerInputApplier};
use lactose_math::{Scalar, Vec3};
use lactose_net_api::{InterpolationSlot, StagedFrame, StagedInbox, StagedSpawn};
use lactose_physics_api::{
    BodyDesc, BodyHandle, Collider, Joint, JointHandle, PhysicsBackend, PhysicsConfig, QueryFilter,
    Ray, RayHit, RigidBody, Shape, ShapeHit, StepInput, StepOutput, Transform, Velocity,
};
use lactose_predict::{
    CorrectionCause, DEFAULT_DEPTH, Divergence, LocalPlayer, PredictError, Predictor, Reconciled,
    check_prediction_reads,
};
use lactose_reflect::{Reflect, component};
use lactose_sim::{Pass, Registration, Simulation, SimulationBuilder, Stage, TickInput, TypeTable};

use fake::Fake;

// ---- The game ----

/// Replicated: the player's walks by its input; the others' move only when the server says.
#[component(key = "test::position", persistent, replicated)]
#[derive(PartialEq, Eq, Debug, Default)]
pub struct Position {
    #[field(id = 1)]
    pub x: i32,
    #[field(id = 2)]
    pub y: i32,
}

/// Replicated, written by no system.
#[component(key = "test::health", persistent, replicated)]
#[derive(PartialEq, Eq, Debug, Default)]
pub struct Health(#[field(id = 1)] pub i32);

/// Replicated and transient, written by no system: the server sends it, the input sync
/// point zeroes it.
#[component(key = "test::aim", transient, replicated)]
#[derive(PartialEq, Eq, Debug, Default)]
pub struct Aim(#[field(id = 1)] pub i32);

/// Derived: a cache of `Position` a prediction system maintains from `Changed<Position>`.
#[component(key = "test::trail", derived)]
#[derive(PartialEq, Eq, Debug, Default)]
pub struct Trail {
    #[field(id = 1)]
    pub x: i32,
    #[field(id = 2)]
    pub updates: u32,
}

/// Persistent and not replicated: what the build check refuses a prediction system.
#[component(key = "test::diary", persistent)]
#[derive(PartialEq, Eq, Debug, Default)]
pub struct Diary(#[field(id = 1)] pub u32);

/// A presentation event: a footstep every tick the player has an input, a jump on the
/// press edge.
#[component(key = "test::sound", derived)]
#[derive(PartialEq, Eq, Debug, Default)]
pub struct Sound(#[field(id = 1)] pub u32);

impl PresentEvent for Sound {
    const KIND: PresentKind = PresentKind::from_name("test::sound");
}

pub const FOOTSTEP: Sound = Sound(1);
pub const JUMP: Sound = Sound(2);

/// What the extract pass saw: the rows `Changed<Position>` passed, per pass.
#[derive(Default)]
pub struct Seen(pub Mutex<Vec<u32>>);

#[system(name = "test::walk", worlds = [server, prediction])]
fn walk(input: &PlayerInput, position: Mut<Position>) {
    position.x += i32::from(input.movement.x);
    if input.pressed.contains(Action::Jump) {
        position.y += 10;
    }
    if input.held.contains(Action::Jump) {
        position.y += 1;
    }
}

/// Throws a ball, a predicted spawn in the prediction world, on the press edge of `Throw`.
#[system(name = "test::throw", worlds = [server, prediction])]
fn throw(cmd: Commands, input: &PlayerInput) {
    if input.pressed.contains(Action::Throw) {
        cmd.spawn((Health(1),));
    }
}

/// Kicks the ball, key 5, into [`OTHER_REGION`] on the press edge of `Interact`: a game
/// system that moves a proxy in the prediction world, which the frames never confirm.
/// Journaled by the sample, so a rollback replays it.
#[system(name = "test::kick", worlds = [server, prediction])]
fn kick(cmd: Commands, input: &PlayerInput) {
    if input.pressed.contains(Action::Interact) {
        cmd.move_to_region(key(5), OTHER_REGION);
    }
}

#[system(name = "test::trail", worlds = [server, prediction])]
fn trail(_changed: Changed<Position>, position: &Position, trail: Mut<Trail>) {
    trail.x = position.x;
    trail.updates += 1;
}

#[system(name = "test::chirp", worlds = [server, prediction])]
fn chirp(present: PresentEvents, key: StableKeyOf, input: &PlayerInput) {
    present.emit(*key, FOOTSTEP);
    if input.pressed.contains(Action::Jump) {
        present.emit(*key, JUMP);
    }
}

#[system(name = "test::extract", worlds = [prediction])]
fn extract(_changed: Changed<Position>, seen: Res<Seen>) {
    if let Some(last) = seen.0.lock().expect("never poisoned").last_mut() {
        *last += 1;
    }
}

pub fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<Position>()?;
    types.register::<Health>()?;
    types.register::<Aim>()?;
    types.register::<Trail>()?;
    types.register::<Diary>()?;
    lactose_input::register_types(types)?;
    lactose_net_api::register_types(types)?;
    lactose_predict::register_types(types)?;
    lactose_physics_api::register_types(types)
}

pub fn types() -> TypeTable {
    let mut types = TypeTable::new();
    register_types(&mut types).expect("the game registers");
    types
}

pub fn register_game(registration: &mut Registration) {
    registration.stage(Stage::Simulation).add(&WALK).add(&THROW).add(&KICK);
    registration.stage(Stage::Post).add(&TRAIL).add(&CHIRP);
    registration.pass(Pass::Extract).add(&EXTRACT);
}

// ---- Keys, regions, values ----

pub const PLAYER: PlayerId = PlayerId(1);
pub const REGION: RegionId = RegionId(1);
pub const OTHER_REGION: RegionId = RegionId(2);
/// A region no prediction world here loads.
pub const UNLOADED: RegionId = RegionId(9);

pub fn net(n: u64) -> NetId {
    NetId::runtime(NonZeroU64::new(n).expect("non-zero")).expect("a runtime counter")
}

pub fn key(n: u64) -> StableKey {
    StableKey::Net(net(n))
}

/// The local player's own entity.
pub fn me() -> StableKey {
    key(1)
}

/// An entity the server has claimed for the local player that no input moves.
pub fn cart() -> StableKey {
    key(2)
}

/// An entity the local player has not claimed: a proxy.
pub fn proxy() -> StableKey {
    key(3)
}

pub fn local() -> LocalPlayer {
    LocalPlayer { player: PLAYER, entity: me() }
}

pub fn data<T: Reflect>(value: T) -> ComponentData {
    ComponentData { component: T::KEY, bytes: lactose_reflect::bytemuck::bytes_of(&value).to_vec() }
}

pub fn bytes<T: Reflect>(value: &T) -> &[u8] {
    lactose_reflect::bytemuck::bytes_of(value)
}

pub fn at(x: i32, y: i32) -> Position {
    Position { x, y }
}

pub fn moving(x: i16) -> PlayerInput {
    PlayerInput { movement: Axis2 { x, y: 0 }, ..PlayerInput::IDLE }
}

/// Standing, with `Throw` pressed.
pub fn throwing() -> PlayerInput {
    let throw = Actions::EMPTY.with(Action::Throw);
    PlayerInput { held: throw, pressed: throw, ..PlayerInput::IDLE }
}

/// Standing, with `Interact` pressed: the kick of key 5 into [`OTHER_REGION`].
pub fn kicking() -> PlayerInput {
    let kick = Actions::EMPTY.with(Action::Interact);
    PlayerInput { held: kick, pressed: kick, ..PlayerInput::IDLE }
}

/// Moving by `x` with `Jump` held, and pressed on this sample if `pressed`.
pub fn jumping(x: i16, pressed: bool) -> PlayerInput {
    let jump = Actions::EMPTY.with(Action::Jump);
    PlayerInput { held: jump, pressed: if pressed { jump } else { Actions::EMPTY }, ..moving(x) }
}

pub fn transform(x: i32) -> Transform {
    Transform::at(Vec3::new(Scalar::from_i32(x), Scalar::ZERO, Scalar::ZERO))
}

/// A dynamic body that stays where it is put: no gravity, no velocity.
pub fn floating() -> RigidBody {
    let mut body = RigidBody::dynamic();
    body.gravity_scale = Scalar::ZERO;
    body
}

pub fn sphere() -> Collider {
    Collider::new(Shape::sphere(Scalar::HALF))
}

// ---- Frames ----

/// The spawn of an entity the server has claimed for the local player.
pub fn claimed(region: RegionId, mut components: Vec<ComponentData>) -> StagedSpawn {
    components.sort_by_key(|data| data.component);
    StagedSpawn { origin: None, region, slot: None, components }
}

/// The spawn of a proxy.
pub fn remote(region: RegionId, index: u32, mut components: Vec<ComponentData>) -> StagedSpawn {
    components.sort_by_key(|data| data.component);
    let slot = Some(InterpolationSlot { index, generation: 1 });
    StagedSpawn { origin: None, region, slot, components }
}

/// The server's full frame on join: the player's entity at the origin, the cart at 50 with
/// health 10 (and its derived cache, so the prediction system has a row to maintain), and
/// a proxy at -50.
pub fn join_frame(tick: Tick) -> StagedFrame {
    let mut frame = StagedFrame::resync(tick);
    frame.spawns.insert(net(1), claimed(REGION, vec![data(at(0, 0))]));
    let cart = vec![data(at(50, 0)), data(Health(10)), data(Trail::default())];
    frame.spawns.insert(net(2), claimed(REGION, cart));
    frame.spawns.insert(net(3), remote(REGION, 0, vec![data(at(-50, 0))]));
    frame
}

/// Stages an exact row of `value` for `key` at the frame's tick.
pub fn exact<T: Reflect>(frame: &mut StagedFrame, key: StableKey, value: &T) {
    assert!(frame.push_row(key, T::KEY, frame.tick, true, bytes(value)));
}

/// Stages a dequantised row of `value` for `key` at the frame's tick.
pub fn dequantised<T: Reflect>(frame: &mut StagedFrame, key: StableKey, value: &T) {
    assert!(frame.push_row(key, T::KEY, frame.tick, false, bytes(value)));
}

// ---- The prediction world ----

pub fn executor(workers: u32) -> Executor {
    Executor::builder().frame_workers(workers).background_workers(1).build().expect("an executor")
}

/// The tick the tests' prediction worlds are built at: the join tick 4 plus a lead of 6.
pub const START: Tick = Tick(10);
pub const JOIN: Tick = Tick(4);

/// The prediction world `app` builds, with `backend` behind the physics if `Some`, and
/// both regions loaded and simulated.
pub fn simulation(layout: LayoutConfig, backend: Option<Box<dyn PhysicsBackend>>) -> Simulation {
    let mut builder = SimulationBuilder::new(WorldKind::Prediction, types())
        .layout(layout)
        .tick(START)
        .setup(lactose_net_api::setup)
        .setup(|world| {
            world.insert_service::<Seen>(Box::new(Seen::default()));
            Ok(())
        });
    if let Some(backend) = backend {
        builder = builder
            .setup(move |world| {
                lactose_physics_api::setup(world, backend, PhysicsConfig::default())
            })
            .systems(lactose_physics_api::register_systems);
    }
    let mut simulation = builder
        .systems(lactose_input::register_systems)
        .systems(lactose_net_api::register_systems)
        .systems(register_game)
        .check(check_prediction_reads)
        .inputs(Box::new(PlayerInputApplier::new()))
        .build()
        .expect("the prediction world builds");
    for (id, seed) in [(REGION, 11), (OTHER_REGION, 12)] {
        let config = RegionConfig { id, seed, mode: RegionMode::Simulated };
        simulation.world_mut().create_region(config).expect("a new region");
    }
    simulation
}

/// The predictor across runs, as `app` keeps it: a failure that hands it back keeps it.
pub struct Rig {
    predictor: Option<Predictor>,
    pub executor: Executor,
}

impl Rig {
    pub fn new(simulation: Simulation, workers: u32) -> Self {
        Self::with_executor(simulation, executor(workers))
    }

    pub fn with_executor(simulation: Simulation, executor: Executor) -> Self {
        let predictor = Predictor::new(simulation, local(), DEFAULT_DEPTH).expect("a predictor");
        Self { predictor: Some(predictor), executor }
    }

    /// A rig on the production layout, one worker, no physics, joined.
    pub fn joined() -> Self {
        Self::joined_with(LayoutConfig::default(), 1)
    }

    /// A rig joined at [`JOIN`]: the join frame applied by the first step, at tick 11,
    /// and reconciled.
    pub fn joined_with(layout: LayoutConfig, workers: u32) -> Self {
        let mut rig = Self::new(simulation(layout, None), workers);
        rig.join();
        rig
    }

    pub fn join(&mut self) {
        self.push(join_frame(JOIN));
        self.step(PlayerInput::IDLE);
        let reconciled = self.reconcile();
        assert_eq!(reconciled.confirmed, Some(JOIN));
        assert!(reconciled.corrections.is_empty());
    }

    pub fn p(&self) -> &Predictor {
        self.predictor.as_ref().expect("the predictor is kept")
    }

    pub fn p_mut(&mut self) -> &mut Predictor {
        self.predictor.as_mut().expect("the predictor is kept")
    }

    pub fn world(&self) -> &World {
        self.p().simulation().world()
    }

    pub fn world_mut(&mut self) -> &mut World {
        self.p_mut().simulation_mut().world_mut()
    }

    pub fn tick(&self) -> Tick {
        self.p().tick()
    }

    pub fn hash(&self) -> u64 {
        self.p().simulation().hash().expect("a hash")
    }

    pub fn inbox(&mut self) -> &mut StagedInbox {
        self.world_mut().service_mut::<StagedInbox>().expect("net-api's setup")
    }

    pub fn push(&mut self, frame: StagedFrame) {
        assert!(self.inbox().push(Arc::new(frame)), "the inbox accepts the frame");
    }

    /// One predicted tick with `input` as the local player's sample.
    pub fn step(&mut self, input: PlayerInput) {
        if let Err(error) = self.try_step(input) {
            panic!("the step failed: {error}");
        }
    }

    pub fn try_step(&mut self, input: PlayerInput) -> Result<()> {
        let next = Tick(self.tick().0 + 1);
        let mut frame = TickInput::default();
        frame.inputs.insert(PLAYER, input.to_player_sample(next));
        self.try_step_with(frame)
    }

    pub fn try_step_with(&mut self, input: TickInput) -> Result<()> {
        let predictor = self.predictor.take().expect("the predictor is kept");
        self.settle(self.executor.block_on(predictor.step(&self.executor, input)))
    }

    /// Steps to `tick` with `sample(t)` for each tick `t`.
    pub fn step_to(&mut self, tick: u32, sample: impl Fn(u32) -> PlayerInput) {
        while self.tick().0 < tick {
            let next = self.tick().0 + 1;
            self.step(sample(next));
        }
    }

    pub fn reconcile(&mut self) -> Reconciled {
        match self.try_reconcile() {
            Ok(reconciled) => reconciled,
            Err(error) => panic!("the reconciliation failed: {error}"),
        }
    }

    pub fn try_reconcile(&mut self) -> Result<Reconciled> {
        let predictor = self.predictor.take().expect("the predictor is kept");
        match self.executor.block_on(predictor.reconcile(&self.executor)) {
            Ok((predictor, reconciled)) => {
                self.predictor = Some(predictor);
                Ok(reconciled)
            }
            Err(error) => self.settle(Err(error)).map(|()| unreachable!()),
        }
    }

    pub fn passes(&mut self) {
        self.world()
            .service::<Seen>()
            .expect("the setup")
            .0
            .lock()
            .expect("never poisoned")
            .push(0);
        let predictor = self.predictor.take().expect("the predictor is kept");
        let result = self.executor.block_on(predictor.passes(&self.executor));
        if let Err(error) = self.settle(result) {
            panic!("the passes failed: {error}");
        }
    }

    fn settle(&mut self, result: core::result::Result<Predictor, PredictError>) -> Result<()> {
        match result {
            Ok(predictor) => {
                self.predictor = Some(predictor);
                Ok(())
            }
            Err(PredictError { error, predictor }) => {
                self.predictor = predictor;
                Err(error)
            }
        }
    }

    pub fn position(&self, key: StableKey) -> Option<Position> {
        self.world().get::<Position>(key).copied()
    }
}

/// A correction as a tuple: (tick, depth, cause, entities, divergence).
pub type Summary = (Tick, u32, CorrectionCause, Vec<StableKey>, Divergence);

pub fn summaries(reconciled: &Reconciled) -> Vec<Summary> {
    reconciled
        .corrections
        .iter()
        .map(|c| (c.tick, c.depth, c.cause, c.entities.clone(), c.divergence))
        .collect()
}

/// The player that claims `key`, if any.
pub fn claim(world: &World, key: StableKey) -> Option<PlayerId> {
    world.get::<ClaimedBy>(key).map(|claim| claim.player)
}

/// The sample the tests' player sends for each tick unless a test says otherwise.
pub fn walking(_tick: u32) -> PlayerInput {
    moving(1)
}

// ---- A backend whose snapshot fails on demand ----

/// The test backend, with its snapshot refused while `fail` holds `true`.
pub struct Flaky {
    pub inner: Fake,
    pub fail: Arc<Mutex<bool>>,
}

impl PhysicsBackend for Flaky {
    fn create_body(&mut self, key: StableKey, desc: &BodyDesc<'_>) -> Result<BodyHandle> {
        self.inner.create_body(key, desc)
    }

    fn remove_body(&mut self, key: StableKey) {
        self.inner.remove_body(key);
    }

    fn set_body(&mut self, key: StableKey, body: Option<&RigidBody>) {
        self.inner.set_body(key, body);
    }

    fn set_collider(&mut self, key: StableKey, collider: Option<&Collider>) {
        self.inner.set_collider(key, collider);
    }

    fn set_transform(&mut self, key: StableKey, transform: &Transform) {
        self.inner.set_transform(key, transform);
    }

    fn set_velocity(&mut self, key: StableKey, velocity: &Velocity) {
        self.inner.set_velocity(key, velocity);
    }

    fn set_frozen(&mut self, key: StableKey, frozen: bool) {
        self.inner.set_frozen(key, frozen);
    }

    fn create_joint(
        &mut self,
        key: StableKey,
        joint: &Joint,
        body_a: StableKey,
        body_b: StableKey,
    ) -> Result<JointHandle> {
        self.inner.create_joint(key, joint, body_a, body_b)
    }

    fn remove_joint(&mut self, key: StableKey) {
        self.inner.remove_joint(key);
    }

    fn step(&mut self, input: &StepInput<'_>, output: &mut StepOutput) {
        self.inner.step(input, output);
    }

    fn snapshot(&self, out: &mut Vec<u8>) -> Result<()> {
        if *self.fail.lock().expect("never poisoned") {
            return Err(Error::new(lactose_core::ErrorKind::Unsupported, "refused by the test"));
        }
        self.inner.snapshot(out)
    }

    fn restore(&mut self, bytes: &[u8]) -> Result<()> {
        self.inner.restore(bytes)
    }

    fn cast_ray(&self, ray: &Ray, filter: &QueryFilter) -> Option<RayHit> {
        self.inner.cast_ray(ray, filter)
    }

    fn cast_shape(
        &self,
        shape: &Shape,
        pose: &Transform,
        direction: Vec3,
        max_distance: Scalar,
        filter: &QueryFilter,
    ) -> Option<ShapeHit> {
        self.inner.cast_shape(shape, pose, direction, max_distance, filter)
    }

    fn overlap(
        &self,
        shape: &Shape,
        pose: &Transform,
        filter: &QueryFilter,
        hits: &mut Vec<StableKey>,
    ) {
        self.inner.overlap(shape, pose, filter, hits);
    }
}
