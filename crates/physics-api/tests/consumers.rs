//! Compile-only consumer sketches against the facade: what a game crate, `anim`, a backend
//! crate, `app` and `predict` write, type-checked and never run (every deciding body of
//! the crate is `todo!()`). They pin the surface the README promises: a change that breaks
//! one is a signature change to list.

#![forbid(unsafe_code)]
#![allow(dead_code, unused_variables, missing_docs)]

use lactose_core::{NetId, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{
    ClaimedBy, Commands, Events, Executor, Mut, OwnedBy, Res, StableKeyOf, SystemSet, World,
    WorldKind, system,
};
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::conformance::snapshot_restore;
use lactose_physics_api::{
    BodyDesc, BodyHandle, BodyType, Collider, ContactEvent, ContactPhase, Joint, JointHandle,
    JointKind, Physics, PhysicsBackend, PhysicsConfig, PhysicsExt, QueryFilter, Ray, RayHit,
    RigidBody, Shape, ShapeHit, StepInput, StepOutput, Transform, TriggerEvent, Velocity,
    locked_axes, register_systems, register_types, restore, sets, setup, snapshot,
};
use lactose_reflect::component;
use lactose_sim::{
    Registration, Simulation, SimulationBuilder, Stage, TickDelta, TickInput, TypeTable,
};

// ---- 1. A game crate: bodies as rows, impulses through the channel, queries anywhere ----

#[component(key = "sketch::bomb", persistent)]
pub struct Bomb(#[field(id = 1)] pub u32);

pub static MOVEMENT: SystemSet = SystemSet::new("sketch::movement");

/// The section 6.1 example on a body: a direct `Velocity` write the sync honours, and an
/// impulse on a spawn's placeholder resolved at apply.
#[system(name = "sketch::throw", worlds = [server, prediction], in_set = MOVEMENT)]
fn throw(dt: Res<TickDelta>, cmd: Commands, key: StableKeyOf, vel: Mut<Velocity>, bomb: &Bomb) {
    vel.linear.y -= Scalar::from_f64(9.81) * dt.0;
    let thrown = cmd.spawn((
        Transform::at(Vec3::Y),
        Velocity::ZERO,
        RigidBody::dynamic(),
        Collider::new(Shape::sphere(Scalar::HALF)),
        Bomb(0),
    ));
    cmd.physics().apply_impulse(thrown, Vec3::new(Scalar::ONE, Scalar::TWO, Scalar::ZERO));
    cmd.physics().apply_torque_impulse(*key, Vec3::X);
    cmd.physics().teleport(*key, Transform::IDENTITY);
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

/// Contacts are read in `Stage::Post`, indexed by target.
#[system(name = "sketch::impact_damage", worlds = [server, prediction])]
fn impact_damage(
    contacts: Events<ContactEvent>,
    triggers: Events<TriggerEvent>,
    key: StableKeyOf,
    bomb: &Bomb,
) {
    for contact in contacts.for_target(*key) {
        if contact.phase == ContactPhase::Started && contact.impulse > Scalar::ONE {
            let _ = (contact.other, contact.point, contact.normal);
        }
    }
    for trigger in triggers.for_target(*key) {
        let _ = (trigger.other, trigger.phase);
    }
}

/// A once-per-tick system with no `Mut`: a row write through `Commands::set`, declared,
/// which the sync treats like any other row write.
#[system(name = "sketch::respawn", worlds = [server], writes = [Transform, Velocity])]
fn respawn(cmd: Commands) {
    let key = StableKey::Net(NetId::cooked(1));
    cmd.set(key, Transform::at(Vec3::ZERO));
    cmd.set(key, Velocity::ZERO);
}

/// A reader of the stepped state placed in `Stage::Physics` itself: its set is after the
/// step's, or the schedule fails to build.
pub static AFTER_STEP: SystemSet = SystemSet::new("sketch::after_step").after(&[&sets::STEP]);

#[system(name = "sketch::settle", worlds = [server], in_set = AFTER_STEP)]
fn settle(physics: Res<Physics>, pos: &Transform) {}

pub fn register_types_game(types: &mut TypeTable) -> Result<()> {
    types.register::<Bomb>()?;
    register_types(types)
}

pub fn register_systems_game(registration: &mut Registration) {
    registration.stage(Stage::Simulation).sets(&[&MOVEMENT]).add(&THROW).add(&GROUND_CHECK);
    registration.stage(Stage::Physics).sets(&[&AFTER_STEP]).add(&SETTLE);
    registration.stage(Stage::Post).add(&IMPACT_DAMAGE);
    registration.stage(Stage::Simulation).add(&RESPAWN);
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
        let _: Option<BodyType> = desc.body.map(|body| body.body_type);
        let _: &Transform = desc.transform;
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

    fn clear(&mut self) {}

    fn step(&mut self, input: &StepInput<'_>, output: &mut StepOutput) {
        for entry in input.commands.iter() {
            let _ = (entry.source, entry.entry.body());
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

// ---- 5. predict: snapshot beside the world's, restore after it ----

fn rollback(simulation: &mut Simulation) -> Result<()> {
    let world_snapshot = simulation.world().snapshot();
    let physics_snapshot = snapshot(simulation.world())?;
    let _ = (physics_snapshot.tick(), physics_snapshot.size_in_bytes(), physics_snapshot.hash());
    simulation.world_mut().restore(&world_snapshot)?;
    restore(simulation.world_mut(), &physics_snapshot)
}

// ---- 6. A backend's test: the conformance run, twice, equal ----

fn conformance(executor: &Executor) -> Result<()> {
    let one = snapshot_restore(executor, backend_with_pool(), PhysicsConfig::default())?;
    let many = snapshot_restore(executor, backend_with_pool(), PhysicsConfig::default())?;
    assert_eq!(one, many);
    let _: Vec<(Tick, u64)> = one.first;
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
            Collider::new(Shape::sphere(Scalar::ONE)),
            ClaimedBy { player: PlayerId(1) },
        ),
    )?;
    let _ = TickInput::default();
    Ok(())
}
