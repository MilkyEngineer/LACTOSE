//! What the integration tests share: the test backend ([`fake`]), a harness that builds a
//! simulation with this crate's three systems around it, and scripted systems that act as
//! game code in each stage (a kernel in `Stage::Simulation`, a once-per-tick system after
//! `sets::STEP`, a kernel in `Stage::Post`), so each test states what the game does at
//! which tick and asserts on what the backend was told.

#![allow(dead_code, reason = "each test file uses part of the shared module")]

pub mod fake;

use std::sync::Arc;

use lactose_asset_api::AssetSet;
use lactose_bundle::{Baseline, ComponentSchema, FieldLayout, Overlay, Record};
use lactose_core::{NetId, PlayerId, StableKey, Tick};
use lactose_ecs::{
    ClaimedBy, Commands, Events, Executor, LayoutConfig, Mut, RegionConfig, RegionId, RegionMode,
    Res, ResMut, StableKeyOf, SystemSet, World, WorldKind, system,
};
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::{
    Collider, ContactEvent, Joint, Physics, PhysicsBackend, PhysicsCommand, PhysicsConfig,
    PhysicsExt, RigidBody, Shape, Teleported, Transform, TriggerEvent, Velocity, register_systems,
    register_types, sets, setup,
};
use lactose_reflect::component;
use lactose_sim::{
    CurrentTick, RegionBundle, Registration, Simulation, SimulationBuilder, Stage, TickInput,
    TypeTable,
};

pub use fake::{Call, CallLog, Fake};

pub const REGION: RegionId = RegionId(1);
pub const OTHER_REGION: RegionId = RegionId(2);
pub const PLAYER: PlayerId = PlayerId(1);

/// A `Net` key.
pub fn key(n: u64) -> StableKey {
    StableKey::Net(NetId::cooked(n))
}

pub fn v(x: i32, y: i32, z: i32) -> Vec3 {
    Vec3::new(Scalar::from_i32(x), Scalar::from_i32(y), Scalar::from_i32(z))
}

pub fn at(x: i32, y: i32, z: i32) -> Transform {
    Transform::at(v(x, y, z))
}

pub fn sphere() -> Collider {
    Collider::new(Shape::sphere(Scalar::HALF))
}

/// A body that stays where it is unless pushed: no gravity, so it moves only when a test
/// moves it and its writes are exactly the test's.
pub fn floating() -> RigidBody {
    let mut body = RigidBody::dynamic();
    body.gravity_scale = Scalar::ZERO;
    body
}

pub fn executor(workers: u32) -> Executor {
    Executor::builder().frame_workers(workers).background_workers(1).build().unwrap()
}

/// What a scripted system does.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    /// Writes the entity's `Transform` through `Mut` (simulation kernel only).
    WriteTransform(Transform),
    /// Writes the entity's `Velocity` through `Mut` (simulation kernel only).
    WriteVelocity(Velocity),
    /// Writes the entity's `RigidBody` through `Mut` (simulation kernel only).
    WriteBody(RigidBody),
    /// Records a physics command.
    Command(PhysicsCommand),
    /// Spawns a floating sphere body here and records an impulse on it by placeholder.
    Spawn(Transform),
    /// Despawns an entity through `Commands`.
    Despawn(StableKey),
    /// Writes an entity's `Transform` through `Commands::set`.
    SetTransform(StableKey, Transform),
    /// Writes an entity's `Velocity` through `Commands::set`.
    SetVelocity(StableKey, Velocity),
    /// Writes an entity's `RigidBody` through `Commands::set`.
    SetBody(StableKey, RigidBody),
    /// Removes an entity's `RigidBody` through `Commands::remove`.
    RemoveBody(StableKey),
    /// Writes an entity's `Collider` through `Commands::set`.
    SetCollider(StableKey, Collider),
    /// Writes an entity's `Joint` through `Commands::set`.
    SetJoint(StableKey, Joint),
    /// Moves an entity to a region through `Commands::move_to_region`.
    Move(StableKey, RegionId),
    /// Claims an entity for player one through `Commands::set`.
    Claim(StableKey),
    /// Releases an entity's claim through `Commands::remove`.
    Release(StableKey),
}

/// Where a scripted action runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum When {
    /// The entity's kernel in `Stage::Simulation`.
    Simulation,
    /// A once-per-tick system in `Stage::Physics`, in a set after `sets::STEP`.
    AfterStep,
    /// The entity's kernel in `Stage::Post`, after the write-back.
    Post,
}

/// The game code of a test: what happens at which tick, from which entity's kernel.
#[derive(Default)]
pub struct Script {
    pub actions: Vec<(u32, When, StableKey, Action)>,
    /// Whether player one gets an input tick every tick, so a spawn from a claimed entity
    /// is predicted.
    pub input_ticks: bool,
    /// The keys whose `Physics::is_body` the [`Bookkeeping`] row counts.
    pub watched: Vec<StableKey>,
}

impl Script {
    pub fn at(&mut self, tick: u32, when: When, source: StableKey, action: Action) -> &mut Self {
        self.actions.push((tick, when, source, action));
        self
    }

    fn actions(
        &self,
        tick: u32,
        when: When,
        source: Option<StableKey>,
    ) -> impl Iterator<Item = Action> {
        self.actions
            .iter()
            .filter(move |&&(t, w, s, _)| {
                t == tick && w == when && source.is_none_or(|key| key == s)
            })
            .map(|&(.., action)| action)
            .collect::<Vec<_>>()
            .into_iter()
    }
}

/// What `Physics` reports, written every tick in `Stage::Post` onto the rows that carry
/// one, so the world's hash sees the sync's bookkeeping and not only the rows it moves: a
/// body the sync lost or kept too long changes the hash even if it never moves a row.
#[component(key = "test::bookkeeping", persistent)]
#[derive(PartialEq, Eq, Debug, Default)]
pub struct Bookkeeping {
    /// `Physics::body_count`.
    #[field(id = 1)]
    pub bodies: u32,
    /// How many of `Script::watched` are bodies.
    #[field(id = 2)]
    pub watched: u32,
}

/// What the recording system in `Stage::Post` saw.
#[derive(Default)]
pub struct Recorded {
    pub contacts: Vec<(u32, ContactEvent)>,
    pub triggers: Vec<(u32, TriggerEvent)>,
}

fn act(cmd: &mut Commands<'_>, action: Action) {
    match action {
        Action::Command(command) => cmd.physics().push(command),
        Action::Spawn(transform) => {
            let spawned = cmd.spawn((transform, Velocity::ZERO, floating(), sphere()));
            cmd.physics().apply_impulse(spawned, Vec3::X);
        }
        Action::Despawn(key) => cmd.despawn(key),
        Action::SetTransform(key, transform) => cmd.set(key, transform),
        Action::SetVelocity(key, velocity) => cmd.set(key, velocity),
        Action::SetBody(key, body) => cmd.set(key, body),
        Action::RemoveBody(key) => cmd.remove::<RigidBody>(key),
        Action::SetCollider(key, collider) => cmd.set(key, collider),
        Action::SetJoint(key, joint) => cmd.set(key, joint),
        Action::Move(key, region) => cmd.move_to_region(key, region),
        Action::Claim(key) => cmd.set(key, ClaimedBy { player: PLAYER }),
        Action::Release(key) => cmd.remove::<ClaimedBy>(key),
        Action::WriteTransform(_) | Action::WriteVelocity(_) | Action::WriteBody(_) => {
            panic!("a `Mut` write is the simulation kernel's")
        }
    }
}

pub static INPUT_SET: SystemSet =
    SystemSet::new("test::input_set").after(&[&lactose_sim::sets::INPUT]);
pub static AFTER_STEP_SET: SystemSet = SystemSet::new("test::after_step_set").after(&[&sets::STEP]);

#[system(name = "test::input_tick", worlds = [server, prediction], exclusive, in_set = INPUT_SET)]
fn input_tick(world: &mut World, script: Res<Script>) {
    if script.input_ticks {
        let tick = world.tick();
        world.set_input_tick(PLAYER, tick);
    }
}

#[system(name = "test::simulation", worlds = [server, prediction])]
fn simulation_kernel(
    cmd: Commands,
    key: StableKeyOf,
    now: Res<CurrentTick>,
    script: Res<Script>,
    pos: Mut<Transform>,
    vel: Option<Mut<Velocity>>,
    body: Option<Mut<RigidBody>>,
) {
    for action in script.actions(now.get().0, When::Simulation, Some(*key)) {
        match action {
            Action::WriteTransform(transform) => *pos = transform,
            Action::WriteVelocity(velocity) => {
                if let Some(vel) = vel.as_mut() {
                    **vel = velocity;
                }
            }
            Action::WriteBody(rigid) => {
                if let Some(body) = body.as_mut() {
                    **body = rigid;
                }
            }
            other => act(&mut cmd, other),
        }
    }
}

#[system(name = "test::after_step", worlds = [server, prediction], in_set = AFTER_STEP_SET)]
fn after_step(cmd: Commands, now: Res<CurrentTick>, script: Res<Script>) {
    for action in script.actions(now.get().0, When::AfterStep, None) {
        act(&mut cmd, action);
    }
}

#[system(
    name = "test::post",
    worlds = [server, prediction],
    writes = [Transform, Velocity, RigidBody, Collider, Joint]
)]
fn post_kernel(
    cmd: Commands,
    key: StableKeyOf,
    now: Res<CurrentTick>,
    script: Res<Script>,
    _pos: &Transform,
) {
    for action in script.actions(now.get().0, When::Post, Some(*key)) {
        act(&mut cmd, action);
    }
}

#[system(name = "test::bookkeeping", worlds = [server, prediction])]
fn bookkeeping(physics: Res<Physics>, script: Res<Script>, book: Mut<Bookkeeping>) {
    let watched = script.watched.iter().filter(|&&key| physics.is_body(key)).count();
    *book = Bookkeeping {
        bodies: u32::try_from(physics.body_count()).unwrap(),
        watched: u32::try_from(watched).unwrap(),
    };
}

#[system(name = "test::record", worlds = [server, prediction])]
fn record(
    now: Res<CurrentTick>,
    contacts: Events<ContactEvent>,
    triggers: Events<TriggerEvent>,
    recorded: ResMut<Recorded>,
) {
    let tick = now.get().0;
    recorded.contacts.extend(contacts.iter().map(|&event| (tick, event)));
    recorded.triggers.extend(triggers.iter().map(|&event| (tick, event)));
}

/// The scripted systems, beside this crate's.
pub fn register_script(registration: &mut Registration) {
    registration.stage(Stage::Input).sets(&[&INPUT_SET]).add(&INPUT_TICK);
    registration.stage(Stage::Simulation).add(&SIMULATION_KERNEL);
    registration.stage(Stage::Physics).sets(&[&AFTER_STEP_SET]).add(&AFTER_STEP);
    registration.stage(Stage::Post).add(&POST_KERNEL).add(&BOOKKEEPING).add(&RECORD);
}

/// A simulation of one kind with the test backend behind this crate's systems and the
/// scripted systems beside them, one simulated region, stepped on its own executor.
pub struct Harness {
    pub simulation: Simulation,
    pub executor: Executor,
    pub calls: CallLog,
}

/// How to build a [`Harness`].
pub struct Options {
    pub kind: WorldKind,
    pub layout: LayoutConfig,
    pub workers: u32,
    pub threads: u32,
    pub contacts: bool,
    pub config: PhysicsConfig,
    pub tick: Tick,
}

impl Options {
    pub fn new(kind: WorldKind) -> Self {
        Self {
            kind,
            layout: LayoutConfig::default(),
            workers: 1,
            threads: 1,
            contacts: true,
            config: PhysicsConfig::default(),
            tick: Tick(0),
        }
    }

    pub fn build(self) -> Harness {
        let calls = CallLog::default();
        let mut fake = Fake::new(calls.clone()).with_threads(self.threads);
        if !self.contacts {
            fake = fake.without_contacts();
        }
        self.build_with(Box::new(fake), calls)
    }

    /// The harness around another backend, whose calls `calls` records if it records any.
    pub fn build_with(self, backend: Box<dyn PhysicsBackend>, calls: CallLog) -> Harness {
        let config = self.config;
        let mut types = TypeTable::new();
        register_types(&mut types).unwrap();
        types.register::<Bookkeeping>().unwrap();
        let mut simulation = SimulationBuilder::new(self.kind, types)
            .layout(self.layout)
            .tick(self.tick)
            .setup(move |world| setup(world, backend, config))
            .setup(|world| {
                world.insert_service::<Script>(Box::new(Script::default()));
                world.insert_service::<Recorded>(Box::new(Recorded::default()));
                Ok(())
            })
            .systems(register_systems)
            .systems(register_script)
            .build()
            .unwrap();
        let region = RegionConfig { id: REGION, seed: 1, mode: RegionMode::Simulated };
        simulation.world_mut().create_region(region).unwrap();
        Harness { simulation, executor: executor(self.workers), calls }
    }
}

impl Harness {
    pub fn new(kind: WorldKind) -> Self {
        Options::new(kind).build()
    }

    pub fn world(&self) -> &World {
        self.simulation.world()
    }

    pub fn world_mut(&mut self) -> &mut World {
        self.simulation.world_mut()
    }

    pub fn script(&mut self) -> &mut Script {
        self.world_mut().service_mut::<Script>().unwrap()
    }

    pub fn physics(&self) -> &Physics {
        self.world().service::<Physics>().unwrap()
    }

    pub fn recorded(&self) -> &Recorded {
        self.world().service::<Recorded>().unwrap()
    }

    pub fn tick(&self) -> u32 {
        self.simulation.tick().0
    }

    /// Runs one tick whose input sync point applies `input`: a frame's unloads and region
    /// bundles, as `lactose-sim` applies them.
    pub fn run_with(mut self, input: TickInput) -> Self {
        self.simulation =
            self.executor.block_on(self.simulation.step(&self.executor, input)).unwrap();
        self
    }

    /// Runs `ticks` ticks.
    pub fn run(mut self, ticks: u32) -> Self {
        for _ in 0..ticks {
            self.simulation = self
                .executor
                .block_on(self.simulation.step(&self.executor, TickInput::default()))
                .unwrap();
        }
        self
    }

    /// Spawns a row directly in the region.
    pub fn spawn(&mut self, key: StableKey, bundle: impl lactose_ecs::Bundle) {
        self.world_mut().spawn(key, REGION, bundle).unwrap();
    }

    /// Spawns a floating sphere body claimed by player one.
    pub fn spawn_body(&mut self, key: StableKey, transform: Transform) {
        let claim = ClaimedBy { player: PLAYER };
        self.spawn(key, (transform, Velocity::ZERO, floating(), sphere(), claim));
    }

    /// The calls since the last `take`, without snapshots and restores.
    pub fn take(&self) -> Vec<Call> {
        self.calls
            .take()
            .into_iter()
            .filter(|call| !matches!(call, Call::Snapshot | Call::Restore))
            .collect()
    }
}

/// One row as the physics rows describe it.
pub type Row = (StableKey, Transform, Option<Velocity>, Option<Teleported>, Option<RigidBody>);

/// Every row with a `Transform`, ascending by key.
pub fn rows(world: &World) -> Vec<Row> {
    let mut keys: Vec<StableKey> =
        world.columns::<Transform>().flat_map(|column| column.keys().to_vec()).collect();
    keys.sort_unstable();
    keys.into_iter()
        .map(|key| {
            (
                key,
                *world.get::<Transform>(key).unwrap(),
                world.get::<Velocity>(key).copied(),
                world.get::<Teleported>(key).copied(),
                world.get::<RigidBody>(key).copied(),
            )
        })
        .collect()
}

/// The bundle that loads `region` back as a save would: its export as an overlay of whole
/// rows over an empty baseline, which `lactose-sim`'s frame hands to `World::load_region`
/// (rows stamped at the current point and logged as spawned, no epoch). The rows' keys
/// are cooked, so each is a `SpawnFromBaseline`.
pub fn bundle(world: &World, region: RegionId) -> RegionBundle {
    let export = world.export_region(region).unwrap();
    let schemas: Vec<ComponentSchema> = world
        .registry()
        .iter()
        .map(|info| {
            let fields = info.layout().fields().iter().filter_map(|field| {
                let (offset, size) = (field.offset(), field.size());
                Some(FieldLayout {
                    id: field.id()?,
                    offset: u32::try_from(offset).unwrap(),
                    size: u32::try_from(size).unwrap(),
                })
            });
            let size = u32::try_from(info.layout().size()).unwrap();
            ComponentSchema::new(info.key(), info.version(), size, fields).unwrap()
        })
        .collect();
    let baseline = Baseline::from_rows(AssetSet::new(), schemas.clone(), []).unwrap();
    let mut records = Vec::new();
    for row in &export.rows {
        assert!(row.transient.is_empty(), "the physics scenes hold no transient component");
        records.push(Record::SpawnFromBaseline { id: row.id, components: row.components.clone() });
        if row.owner.is_some() {
            records.push(Record::OwnershipChange { id: row.id, owner: row.owner });
        }
    }
    let overlay = Overlay::new(baseline.content_hash(), schemas, records).unwrap();
    RegionBundle {
        seed: export.seed,
        mode: world.region_mode(region).unwrap(),
        baseline: Arc::new(baseline),
        overlay: Some(Arc::new(overlay)),
        resources: export.resources,
        transient_resources: export.transient_resources,
        assets: None,
    }
}

/// A frame that unloads `regions` and loads `bundles`: `lactose-sim`'s in-place reload
/// when they name the same regions.
pub fn frame(
    regions: impl IntoIterator<Item = RegionId>,
    bundles: impl IntoIterator<Item = (RegionId, RegionBundle)>,
) -> TickInput {
    TickInput {
        unloaded: regions.into_iter().collect(),
        loaded: bundles.into_iter().collect(),
        ..TickInput::default()
    }
}

/// The calls of `calls` other than steps.
pub fn sync_calls(calls: &[Call]) -> Vec<Call> {
    calls.iter().filter(|call| !matches!(call, Call::Step { .. })).cloned().collect()
}

/// The step calls of `calls`, as (tick, commands).
pub fn steps(calls: &[Call]) -> Vec<(Tick, Vec<PhysicsCommand>)> {
    calls
        .iter()
        .filter_map(|call| match call {
            Call::Step { tick, commands } => Some((*tick, commands.clone())),
            _ => None,
        })
        .collect()
}
