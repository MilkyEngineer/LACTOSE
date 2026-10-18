//! Consumer sketches against the surface: what a game crate, `anim`, a backend crate, `app`
//! and `predict` write, type-checked, and `app`'s build run for each world kind so the
//! registrations of sections 1 and 2 are shown to build under the real schedule (the
//! round-1 review's probe E). They pin the surface the README promises: a change that
//! breaks one is a signature change to list.

#![forbid(unsafe_code)]
#![allow(dead_code, unused_variables, missing_docs, clippy::too_many_arguments)]

use lactose_core::{EntityRef, NetId, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{
    ChannelWriter, ClaimedBy, Commands, Event, EventWriter, Events, Executor, LayoutConfig, Mut,
    OwnedBy, Res, StableKeyOf, SystemSet, World, WorldKind, system,
};
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::conformance::snapshot_restore;
use lactose_physics_api::{
    BodyDesc, BodyHandle, BodyType, Collider, ContactEvent, ContactPhase, Joint, JointHandle,
    JointKind, Physics, PhysicsBackend, PhysicsCommand, PhysicsCommands, PhysicsConfig, PhysicsExt,
    QueryFilter, Ray, RayHit, RigidBody, Shape, ShapeHit, StepInput, StepOutput, Teleported,
    Transform, TriggerEvent, Velocity, corrected, locked_axes, register_systems, register_types,
    restore, sets, setup, snapshot,
};
use lactose_reflect::component;
use lactose_sim::{
    Registration, Simulation, SimulationBuilder, Stage, TickDelta, TickInput, TypeTable,
};

// ---- 1. A game crate: bodies as rows, impulses through the channel, queries anywhere ----

/// A bomb, armed (nonzero) by an impact in `Stage::Post` and set off by the next tick's
/// simulation stage: a plain-value carrier, written through `Mut`. Replicated, since the
/// kernels that write it run in the prediction world.
#[component(key = "sketch::bomb", persistent, replicated)]
pub struct Bomb(#[field(id = 1)] pub u32);

/// The other body of the impact that armed a bomb, kept for the next tick: a keyed
/// carrier, which the schedule refuses `Mut` on, so it is spawned with the bomb and
/// written and cleared through `Commands::set` on the bomb's own row.
#[component(key = "sketch::push_target", persistent, replicated)]
pub struct PushTarget(#[field(id = 1)] pub EntityRef);

/// A push from a bomb to the other body of its impact.
#[derive(Clone, Copy)]
pub struct Push {
    pub target: StableKey,
    pub strength: Scalar,
}

impl Event for Push {
    fn target(&self) -> StableKey {
        self.target
    }
}

/// A blast reaching a body the bomb's overlap query found.
#[derive(Clone, Copy)]
pub struct Blast {
    pub target: StableKey,
    pub centre: Vec3,
    pub strength: Scalar,
}

impl Event for Blast {
    fn target(&self) -> StableKey {
        self.target
    }
}

pub static MOVEMENT: SystemSet = SystemSet::new("sketch::movement");
/// A once-per-tick writer of body rows, ordered against the kernels that write them.
pub static RESPAWN_SET: SystemSet = SystemSet::new("sketch::respawn_set").after(&[&MOVEMENT]);
/// The bombs the previous tick armed, after every writer of the rows they read.
pub static DETONATION: SystemSet = SystemSet::new("sketch::detonation").after(&[&RESPAWN_SET]);
/// The bodies a bomb pushed, after the events reach them.
pub static RECEIVE: SystemSet = SystemSet::new("sketch::receive").after(&[&DETONATION]);
/// `Stage::Physics` after the step: this tick's `Physics` and events, last tick's rows.
pub static AFTER_STEP: SystemSet = SystemSet::new("sketch::after_step").after(&[&sets::STEP]);

/// The section 6.1 example on a body: a direct `Velocity` write the sync honours, and an
/// impulse on a spawn's placeholder resolved at apply, through the channel's writer.
#[system(name = "sketch::throw", worlds = [server, prediction], in_set = MOVEMENT)]
fn throw(
    dt: Res<TickDelta>,
    cmd: Commands,
    physics: ChannelWriter<PhysicsCommands>,
    key: StableKeyOf,
    vel: Mut<Velocity>,
    bomb: &Bomb,
) {
    vel.linear.y -= Scalar::from_f64(9.81) * dt.0;
    let thrown = cmd.spawn((
        Transform::at(Vec3::Y),
        Velocity::ZERO,
        RigidBody::dynamic(),
        Collider::new(Shape::sphere(Scalar::HALF)).with_events(Scalar::ONE).with_material(3),
        Bomb(0),
        PushTarget(EntityRef::NONE),
    ));
    physics.apply_impulse(thrown, Vec3::new(Scalar::ONE, Scalar::TWO, Scalar::ZERO));
    physics.apply_torque_impulse(*key, Vec3::X);
    physics.teleport(*key, Transform::IDENTITY);
}

/// A query from a per-entity kernel in `Stage::Simulation`: the previous step's state,
/// read on every worker at once.
#[system(name = "sketch::ground_check", worlds = [server, prediction], in_set = MOVEMENT)]
fn ground_check(
    physics: Res<Physics>,
    key: StableKeyOf,
    pos: &Transform,
    body: Option<&BodyHandle>,
) {
    let is_body = body.is_some() && physics.is_body(*key);
    let ray = Ray::new(pos.translation, -Vec3::Y, Scalar::TWO);
    let hit: Option<RayHit> = physics.cast_ray(&ray, &QueryFilter::excluding(*key));
    let swept: Option<ShapeHit> = physics.cast_shape(
        &Shape::capsule(Scalar::ONE, Scalar::HALF),
        pos,
        -Vec3::Y,
        Scalar::ONE,
        &QueryFilter::ALL,
    );
    let mut nearby = Vec::new();
    physics.overlap(&Shape::sphere(Scalar::TWO), pos, &QueryFilter::default(), &mut nearby);
}

/// Contact-driven game code in `Stage::Post`, which needs no edge after the write-back (an
/// exclusive system runs first at the stage's sync point). It records no command, since a
/// writer of the channel after the step fails the build (D126): an impact arms the bomb's
/// own row through `Mut` and keeps the other body's key through `Commands::set`, and the
/// next tick's simulation stage acts on them. No solver number is stored.
#[system(
    name = "sketch::impact_damage",
    worlds = [server, prediction],
    writes = [PushTarget]
)]
fn impact_damage(
    cmd: Commands,
    contacts: Events<ContactEvent>,
    triggers: Events<TriggerEvent>,
    key: StableKeyOf,
    pos: &Transform,
    teleported: Option<&Teleported>,
    bomb: Mut<Bomb>,
) {
    for contact in contacts.for_target(*key) {
        if contact.phase == ContactPhase::Impact && contact.impulse > Scalar::ONE {
            let _ = (contact.point, contact.normal);
            bomb.0 = 1;
            cmd.set(*key, PushTarget(EntityRef::new(contact.other)));
        }
    }
    for trigger in triggers.for_target(*key) {
        let _ = (trigger.other, trigger.phase);
    }
    // Extract compares the tick with the last one it covered; zero means never.
    let teleported_at: Tick = teleported.map_or(Tick(0), |flag| flag.0);
}

/// The next tick's half of the relay, on an armed bomb: a `Blast` to each key the overlap
/// query returns (the previous step's state) and a `Push` to the impact's other body, from
/// the bomb's own row; then disarmed through `Mut` and its carrier cleared through
/// `Commands::set`.
#[system(
    name = "sketch::detonate",
    worlds = [server, prediction],
    in_set = DETONATION,
    writes = [PushTarget]
)]
fn detonate(
    physics: Res<Physics>,
    cmd: Commands,
    blasts: EventWriter<Blast>,
    pushes: EventWriter<Push>,
    key: StableKeyOf,
    pos: &Transform,
    bomb: Mut<Bomb>,
    target: &PushTarget,
) {
    if bomb.0 == 0 {
        return;
    }
    let strength = Scalar::from_u32(4);
    let mut found = Vec::new();
    physics.overlap(
        &Shape::sphere(Scalar::from_u32(3)),
        pos,
        &QueryFilter::excluding(*key),
        &mut found,
    );
    for body in found {
        blasts.emit(Blast { target: body, centre: pos.translation, strength });
    }
    if let Some(other) = target.0.get() {
        pushes.emit(Push { target: other, strength });
        cmd.set(*key, PushTarget(EntityRef::NONE));
    }
    bomb.0 = 0;
}

/// A body a bomb pushed or blasted: the impulse computed from its own row and the event,
/// recorded through its writer in this tick's simulation stage, so the sync takes it for
/// this tick's step. In the prediction world it runs on a claimed body alone, which is
/// the command a client predicts.
#[system(name = "sketch::knockback", worlds = [server, prediction], in_set = RECEIVE)]
fn knockback(
    physics: ChannelWriter<PhysicsCommands>,
    pushes: Events<Push>,
    blasts: Events<Blast>,
    key: StableKeyOf,
    pos: &Transform,
) {
    for push in pushes.for_target(*key) {
        physics.apply_impulse(*key, Vec3::Y * push.strength);
    }
    for blast in blasts.for_target(*key) {
        physics.apply_impulse(*key, (pos.translation - blast.centre) * blast.strength);
    }
}

/// A once-per-tick system with no `Mut`: a row write through `Commands::set`, declared,
/// which the sync treats like any other row write; its set is ordered after the kernels
/// that write the same columns.
#[system(name = "sketch::respawn", worlds = [server], in_set = RESPAWN_SET, writes = [Transform, Velocity])]
fn respawn(cmd: Commands) {
    let key = StableKey::Net(NetId::cooked(1));
    cmd.set(key, Transform::at(Vec3::ZERO));
    cmd.set(key, Velocity::ZERO);
}

/// A `Stage::Physics` system after the step: this tick's queries and events, and no row
/// of a body, which it would read a tick late and could not write.
#[system(name = "sketch::probe_physics", worlds = [server], in_set = AFTER_STEP)]
fn probe_physics(physics: Res<Physics>, contacts: Events<ContactEvent>, bomb: &Bomb) {}

/// A reader of the stepped rows: `Stage::Post`, no edge needed.
#[system(name = "sketch::settle", worlds = [server])]
fn settle(physics: Res<Physics>, pos: &Transform, vel: &Velocity) {}

pub fn register_types_game(types: &mut TypeTable) -> Result<()> {
    types.register::<Bomb>()?;
    types.register::<PushTarget>()?;
    register_types(types)
}

pub fn register_systems_game(registration: &mut Registration) {
    registration
        .stage(Stage::Simulation)
        .sets(&[&MOVEMENT, &RESPAWN_SET, &DETONATION, &RECEIVE])
        .add(&THROW)
        .add(&GROUND_CHECK)
        .add(&RESPAWN)
        .add(&DETONATE)
        .add(&KNOCKBACK);
    registration.stage(Stage::Physics).sets(&[&AFTER_STEP]).add(&PROBE_PHYSICS);
    registration.stage(Stage::Post).add(&IMPACT_DAMAGE).add(&SETTLE);
}

// ---- 2. anim: a ragdoll as owned bone rows and joint rows ----

#[component(key = "sketch::bone", persistent)]
pub struct Bone(#[field(id = 1)] pub u8);

/// Bones are kinematic while animated: a `Transform` write sweeps them. Joints are rows
/// owned by a bone, so they despawn with the character.
#[system(name = "sketch::spawn_ragdoll", worlds = [server, prediction])]
fn spawn_ragdoll(cmd: Commands, key: StableKeyOf, root: &Transform, bomb: &Bomb) {
    let bone = |index: u8| {
        let mut body = RigidBody::kinematic();
        body.locked_axes = locked_axes::NONE;
        (
            Transform::new(root.translation, root.rotation),
            body,
            Collider::new(Shape::capsule(Scalar::HALF, Scalar::from_f64(0.1)))
                .with_offset(Vec3::Y * Scalar::HALF),
            OwnedBy { owner: (*key).into() },
            Bone(index),
        )
    };
    let hip = cmd.spawn(bone(0));
    let thigh = cmd.spawn(bone(1));
    let joint = Joint::new(JointKind::Spherical, hip, thigh)
        .with_anchors(Vec3::ZERO, Vec3::Y)
        .with_limits(-Scalar::ONE, Scalar::ONE);
    cmd.spawn((joint, OwnedBy { owner: hip.into() }));
}

/// Going limp: the bones become dynamic through a row write the sync picks up.
#[system(name = "sketch::ragdoll", worlds = [server, prediction])]
fn ragdoll(body: Mut<RigidBody>, bone: &Bone) {
    if body.body_type != BodyType::Dynamic {
        body.body_type = BodyType::Dynamic;
    }
}

/// Sockets read the stepped bone transforms in `Stage::Post`.
#[system(name = "sketch::sockets", worlds = [server, prediction])]
fn sockets(pos: &Transform, bone: &Bone, handle: Option<&BodyHandle>, joint: Option<&JointHandle>) {
    let socket = pos.transform_point(Vec3::X);
}

pub fn register_systems_anim(registration: &mut Registration) {
    registration.stage(Stage::Simulation).add(&SPAWN_RAGDOLL).add(&RAGDOLL);
    registration.stage(Stage::Post).add(&SOCKETS);
}

// ---- 3. A backend crate: the trait, with the pool it was built with ----

struct Fake;

impl PhysicsBackend for Fake {
    fn create_body(&mut self, key: StableKey, desc: &BodyDesc<'_>) -> Result<BodyHandle> {
        // The effective body: already kinematic for a prediction-world proxy.
        let _: Option<BodyType> = desc.body.map(|body| body.body_type);
        let _: &Transform = desc.transform;
        let fixed = desc.frozen || desc.body.is_none();
        Ok(BodyHandle::new(0, 1))
    }

    fn remove_body(&mut self, key: StableKey) {}

    fn set_body(&mut self, key: StableKey, body: Option<&RigidBody>) {}

    fn set_collider(&mut self, key: StableKey, collider: Option<&Collider>) {}

    fn set_transform(&mut self, key: StableKey, transform: &Transform) {}

    fn set_velocity(&mut self, key: StableKey, velocity: &Velocity) {}

    fn set_frozen(&mut self, key: StableKey, frozen: bool) {}

    fn create_joint(
        &mut self,
        key: StableKey,
        joint: &Joint,
        body_a: StableKey,
        body_b: StableKey,
    ) -> Result<JointHandle> {
        Ok(JointHandle::new(0, 1))
    }

    fn remove_joint(&mut self, key: StableKey) {}

    fn step(&mut self, input: &StepInput<'_>, output: &mut StepOutput) {
        for command in input.commands {
            let _: StableKey = command.body();
            if let PhysicsCommand::Teleport { body, transform } = command {
                let _ = (body, transform);
            }
        }
        let _ = (input.tick, input.dt, input.gravity);
        output.bodies.clear();
    }

    fn snapshot(&self, out: &mut Vec<u8>) -> Result<()> {
        Ok(())
    }

    fn restore(&mut self, bytes: &[u8]) -> Result<()> {
        Ok(())
    }

    fn cast_ray(&self, ray: &Ray, filter: &QueryFilter) -> Option<RayHit> {
        None
    }

    fn cast_shape(
        &self,
        shape: &Shape,
        pose: &Transform,
        direction: Vec3,
        max_distance: Scalar,
        filter: &QueryFilter,
    ) -> Option<ShapeHit> {
        None
    }

    fn overlap(
        &self,
        shape: &Shape,
        pose: &Transform,
        filter: &QueryFilter,
        hits: &mut Vec<StableKey>,
    ) {
    }
}

/// What the backend crate exports: a constructor taking the pool `app` built.
fn backend_with_pool() -> Box<dyn PhysicsBackend> {
    Box::new(Fake)
}

/// A backend's own unit test of `step`: no world, no schedule, a plain slice of commands.
fn step_without_a_world(backend: &mut dyn PhysicsBackend) {
    let commands =
        [PhysicsCommand::ApplyImpulse { body: StableKey::Net(NetId::cooked(2)), impulse: Vec3::Y }];
    let input = StepInput {
        tick: Tick(1),
        dt: Scalar::ONE / Scalar::from_u32(60),
        gravity: PhysicsConfig::EARTH_GRAVITY,
        commands: &commands,
    };
    let mut output = StepOutput::default();
    backend.step(&input, &mut output);
    let _ = output.bodies.iter().filter(|state| state.teleported).count();
}

// ---- 4. app: the setup closure and the registration functions ----

fn app_build(kind: WorldKind) -> Result<Simulation> {
    let mut types = TypeTable::new();
    register_types_game(&mut types)?;
    let backend = backend_with_pool();
    let config = PhysicsConfig { reserved_workers: 2, ..PhysicsConfig::default() };
    SimulationBuilder::new(kind, types)
        .setup(move |world| setup(world, backend, config))
        .systems(register_systems)
        .systems(register_systems_game)
        .systems(register_systems_anim)
        .build()
}

/// Sections 1 and 2 each build beside this crate's systems in every world kind. Together
/// they do not: `sketch::spawn_ragdoll` reads `Transform` in `Stage::Simulation` and
/// `sketch::respawn` declares writing it, with no set to order them by, so `app_build`
/// above fails until `anim` puts its systems in a set `app` can order.
#[test]
fn the_sketches_build_in_every_world_kind() {
    let build = |kind: WorldKind, game: bool| {
        let mut types = TypeTable::new();
        register_types_game(&mut types)?;
        types.register::<Bone>()?;
        let backend = backend_with_pool();
        let builder = SimulationBuilder::new(kind, types)
            .setup(move |world| setup(world, backend, PhysicsConfig::default()))
            .systems(register_systems);
        if game {
            builder.systems(register_systems_game).build()
        } else {
            builder.systems(register_systems_anim).build()
        }
    };
    for kind in [WorldKind::Server, WorldKind::Prediction, WorldKind::Client] {
        for game in [true, false] {
            if let Err(error) = build(kind, game) {
                panic!("{kind:?}, {}: {error}", if game { "section 1" } else { "section 2" });
            }
        }
    }
}

// ---- 5. predict: snapshot beside the world's, restore after it, corrections named ----

fn rollback(simulation: &mut Simulation, corrected_bodies: &[StableKey]) -> Result<()> {
    let world_snapshot = simulation.world().snapshot();
    let physics_snapshot = snapshot(simulation.world())?;
    let _ = (physics_snapshot.tick(), physics_snapshot.size_in_bytes(), physics_snapshot.hash());
    simulation.world_mut().restore(&world_snapshot)?;
    restore(simulation.world_mut(), &physics_snapshot)?;
    // Corrections are written through `ColumnMut::write` without recording; the sync is
    // told which rows.
    corrected(simulation.world_mut(), corrected_bodies)
}

// ---- 6. A backend's test: the conformance run, per pool size and layout, equal ----

fn conformance(executor: &Executor) -> Result<()> {
    let layout = LayoutConfig::default();
    let one = snapshot_restore(executor, backend_with_pool(), PhysicsConfig::default(), layout)?;
    let many = snapshot_restore(executor, backend_with_pool(), PhysicsConfig::default(), layout)?;
    assert_eq!(one, many);
    let _: Vec<(Tick, u64)> = one.first;
    let _: (u64, u64) = one.physics_hash;
    Ok(())
}

// ---- 7. The conformance scene's direct spawns, as the function performs them ----

fn direct_scene(world: &mut World, region: lactose_ecs::RegionId) -> Result<()> {
    world.spawn(
        StableKey::Net(NetId::cooked(1)),
        region,
        (
            Transform::IDENTITY,
            Collider::new(Shape::cuboid(Vec3::new(
                Scalar::from_i32(50),
                Scalar::ONE,
                Scalar::from_i32(50),
            ))),
        ),
    )?;
    world.spawn(
        StableKey::Net(NetId::cooked(2)),
        region,
        (
            Transform::at(Vec3::Y * Scalar::from_i32(3)),
            RigidBody::dynamic(),
            Collider::new(Shape::sphere(Scalar::ONE)).with_events(Scalar::ZERO),
            ClaimedBy { player: PlayerId(1) },
        ),
    )?;
    // A proxy: a body row without a claim, handed to the backend as kinematic in a
    // prediction world.
    world.spawn(
        StableKey::Net(NetId::cooked(3)),
        region,
        (Transform::IDENTITY, RigidBody::dynamic(), Collider::new(Shape::cuboid(Vec3::ONE))),
    )?;
    let _ = TickInput::default();
    Ok(())
}
