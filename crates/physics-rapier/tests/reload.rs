//! Architecture section 15.5's first test with physics (D124): a world reloaded in place
//! has the physics state a fresh process loading the same rows has, bit for bit. Through
//! `physics-api`'s sync, step and write-back with this backend on one solver thread, and
//! through `lactose-sim`'s frame: every region exported at tick `t0`, then unloaded and
//! loaded from its bundle (`World::load_region`) in one frame, or unloaded in one and
//! loaded in the next, against a fresh process loading the same bundles at the same tick;
//! and a client's re-join, `set_tick` then the full frame, against a fresh client.
//! The world hash after every tick is equal; the twin that never reloaded is not, so the
//! equality is the reload rule's doing; and every body and joint row has its handle row.
//! Two scenes across two regions: a floor and a pusher beside a three-crate stack, and a
//! toppling pile of 75 bodies of three shapes with a fixed and a limited spherical joint
//! whose bodies are in different regions. And a kinematic character through a portal (D126):
//! a `Transform` written in `Stage::Post` sweeps it, a carried `Teleport` places it.

#![forbid(unsafe_code)]

#[path = "../../physics-api/tests/common/mod.rs"]
mod common;

use common::*;
use lactose_core::{StableKey, Tick};
use lactose_ecs::{
    ChannelWriter, Mut, RegionConfig, RegionId, RegionMode, Res, StableKeyOf, WorldKind, system,
};
use lactose_math::{Quat, Scalar, Vec3};
use lactose_physics_api::{
    BodyHandle, Collider, Joint, JointHandle, JointKind, PhysicsBackend, PhysicsCommands,
    PhysicsExt, RigidBody, Shape, Teleported, Transform, Velocity,
};
use lactose_reflect::component;
use lactose_sim::{CurrentTick, RegionBundle, Registration, Stage, TypeTable};

const REGIONS: [RegionId; 2] = [REGION, OTHER_REGION];
const FLOOR: u64 = 1;
const PUSHER: u64 = 10;
const FIRST_CRATE: u64 = 20;
const FIRST_PILE: u64 = 10;
const PILE: u64 = 75;
const JOINTS: [u64; 2] = [1000, 1001];
/// How far past `t0` a client re-joins, its tick set between runs.
const REJOIN: u32 = 400;

fn f(x: f64) -> Scalar {
    Scalar::from_f64(x)
}

fn rapier() -> Box<dyn PhysicsBackend> {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    lactose_physics_rapier::backend(pool)
}

/// A world at `tick` with `OTHER_REGION` beside the harness's region, both simulated.
fn harness(tick: u32) -> Harness {
    let mut options = Options::new(WorldKind::Server);
    options.tick = Tick(tick);
    let mut h = options.build_with(rapier(), CallLog::default());
    let other = RegionConfig { id: OTHER_REGION, seed: 2, mode: RegionMode::Simulated };
    h.world_mut().create_region(other).unwrap();
    h
}

/// A fresh process at `tick` with nothing loaded: the bundles bring both regions.
fn fresh(tick: u32) -> Harness {
    let mut h = harness(tick);
    for region in REGIONS {
        h.world_mut().unload_region(region).unwrap();
    }
    h
}

fn floor(h: &mut Harness) {
    let floor = Collider::new(Shape::cuboid(Vec3::new(f(50.0), f(1.0), f(50.0))))
        .with_offset(Vec3::new(f(0.0), f(-1.0), f(0.0)));
    h.world_mut().spawn(key(FLOOR), OTHER_REGION, (Transform::IDENTITY, floor)).unwrap();
}

/// A floor and a pusher in `OTHER_REGION`, a three-crate stack in `REGION` that the pusher
/// runs into.
fn stack(h: &mut Harness) -> Vec<StableKey> {
    floor(h);
    let at = |x: f64, y: f64| Transform::at(Vec3::new(f(x), f(y), f(0.0)));
    let pushing = Velocity { linear: Vec3::new(f(2.0), f(0.0), f(0.0)), angular: Vec3::ZERO };
    let ball = Collider::new(Shape::sphere(f(0.5)));
    let pusher = (at(-2.0, 0.5), pushing, RigidBody::dynamic(), ball);
    h.world_mut().spawn(key(PUSHER), OTHER_REGION, pusher).unwrap();
    for i in 0..3_u32 {
        let crate_ = Collider::new(Shape::cuboid(Vec3::new(f(0.4), f(0.4), f(0.4))));
        let pose = at(0.0, 0.4 + 0.8 * f64::from(i));
        h.spawn(
            key(FIRST_CRATE + u64::from(i)),
            (pose, Velocity::ZERO, RigidBody::dynamic(), crate_),
        );
    }
    let mut keys: Vec<StableKey> = (0..3).map(|i| key(FIRST_CRATE + i)).collect();
    keys.extend([key(FLOOR), key(PUSHER)]);
    keys
}

/// 75 bodies in three layers, alternating between the regions, leaning so they topple,
/// and two joints in `REGION` between bodies of both regions.
fn pile(h: &mut Harness) -> Vec<StableKey> {
    floor(h);
    for i in 0..PILE {
        let shape = match i % 3 {
            0 => Shape::sphere(f(0.45)),
            1 => Shape::cuboid(Vec3::new(f(0.4), f(0.4), f(0.4))),
            _ => Shape::capsule(f(0.2), f(0.3)),
        };
        let place = |n: u64| f64::from(u32::try_from(n).unwrap());
        let (column, layer) = (i % 25, place(i / 25));
        let lean = 0.01 * layer;
        let x = 1.1 * place(column % 5) - 2.2 + lean;
        let z = 1.1 * place(column / 5) - 2.2;
        let position = Vec3::new(f(x), f(0.6 + layer), f(z));
        let pose = Transform::new(position, Quat::from_rotation_y(f(lean)));
        let region = if i % 2 == 0 { REGION } else { OTHER_REGION };
        let body = (pose, Velocity::ZERO, RigidBody::dynamic(), Collider::new(shape));
        h.world_mut().spawn(key(FIRST_PILE + i), region, body).unwrap();
    }
    let (right, left) = (Vec3::new(f(0.55), f(0.0), f(0.0)), Vec3::new(f(-0.55), f(0.0), f(0.0)));
    let fixed = Joint::new(JointKind::Fixed, key(FIRST_PILE), key(FIRST_PILE + 1))
        .with_anchors(right, left);
    let spherical = Joint::new(JointKind::Spherical, key(FIRST_PILE + 2), key(FIRST_PILE + 3))
        .with_anchors(right, left)
        .with_limits(f(-0.4), f(0.4));
    h.spawn(key(JOINTS[0]), (fixed,));
    h.spawn(key(JOINTS[1]), (spherical,));
    let mut keys: Vec<StableKey> = (0..PILE).map(|i| key(FIRST_PILE + i)).collect();
    keys.push(key(FLOOR));
    keys
}

/// The world hash now and after each of `ticks` further ticks.
fn hashes(mut h: Harness, ticks: u32) -> Vec<(u32, u64)> {
    let mut hashes = vec![(h.tick(), h.simulation.hash().unwrap())];
    for _ in 0..ticks {
        h = h.run(1);
        hashes.push((h.tick(), h.simulation.hash().unwrap()));
    }
    hashes
}

/// Every body's handle row and every joint's, in key order.
fn handles(
    h: &Harness,
    bodies: &[StableKey],
) -> (Vec<Option<BodyHandle>>, Vec<Option<JointHandle>>) {
    let world = h.world();
    let bodies = bodies.iter().map(|&body| world.get::<BodyHandle>(body).copied()).collect();
    let joints = JOINTS.iter().map(|&n| world.get::<JointHandle>(key(n)).copied()).collect();
    (bodies, joints)
}

/// The original at `t0`, the keys of its bodies, and its regions' bundles.
fn original(
    scene: fn(&mut Harness) -> Vec<StableKey>,
    t0: u32,
) -> (Harness, Vec<StableKey>, Vec<(RegionId, RegionBundle)>) {
    let mut h = harness(0);
    let bodies = scene(&mut h);
    h = h.run(t0);
    let bundles = REGIONS.iter().map(|&region| (region, bundle(h.world(), region))).collect();
    (h, bodies, bundles)
}

fn reloads_equal_a_fresh_process(
    scene: fn(&mut Harness) -> Vec<StableKey>,
    joints: usize,
    t0: u32,
    ticks: u32,
) {
    // A fresh process loading the save at t0 + 1, and the twin that never reloads.
    let (twin, bodies, bundles) = original(scene, t0);
    let fresh_load = fresh(t0).run_with(frame([], bundles.clone()));
    let fresh_handles = handles(&fresh_load, &bodies);
    assert!(fresh_handles.0.iter().all(Option::is_some));
    assert_eq!(fresh_handles.1.iter().filter(|handle| handle.is_some()).count(), joints);
    let fresh_hashes = hashes(fresh_load, ticks);
    let twin_hashes = hashes(twin.run(1), ticks);
    assert_ne!(twin_hashes, fresh_hashes, "the scene must depend on the solver's state");

    // sim's in-place reload: every region unloaded and loaded again in one frame.
    let (same, _, _) = original(scene, t0);
    let same = same.run_with(frame(REGIONS, bundles.clone()));
    assert_eq!(handles(&same, &bodies), fresh_handles, "t0 {t0}, one frame");
    assert_eq!(hashes(same, ticks), fresh_hashes, "t0 {t0}, one frame");

    // A client's re-join: `set_tick` between runs, then the full frame in the first step,
    // against a fresh client loading the same bundles at that tick.
    let (mut joined, _, _) = original(scene, t0);
    joined.world_mut().set_tick(Tick(t0 + REJOIN)).unwrap();
    let joined = joined.run_with(frame(REGIONS, bundles.clone()));
    let client = fresh(t0 + REJOIN).run_with(frame([], bundles.clone()));
    assert_eq!(handles(&joined, &bodies), handles(&client, &bodies), "t0 {t0}, re-join");
    assert_eq!(hashes(joined, ticks), hashes(client, ticks), "t0 {t0}, re-join");

    // Unloaded in one frame and loaded in the next, against a fresh process loading at
    // that tick.
    let (two, _, _) = original(scene, t0);
    let two = two.run_with(frame(REGIONS, [])).run_with(frame([], bundles.clone()));
    let fresh_later = fresh(t0 + 1).run_with(frame([], bundles));
    assert_eq!(handles(&two, &bodies), handles(&fresh_later, &bodies), "t0 {t0}, two frames");
    assert_eq!(hashes(two, ticks), hashes(fresh_later, ticks), "t0 {t0}, two frames");
}

#[test]
fn a_stack_reloaded_in_place_equals_a_fresh_process() {
    for t0 in [30, 60] {
        reloads_equal_a_fresh_process(stack, 0, t0, 180);
    }
}

#[test]
fn a_pile_with_joints_reloaded_in_place_equals_a_fresh_process() {
    reloads_equal_a_fresh_process(pile, 2, 60, 120);
}

/// A character at a portal, a plain-value carrier the post stage arms (a stand-in for the
/// contact that decides it) and the next tick's simulation stage acts on (D126).
#[component(key = "test::portal", persistent, replicated)]
pub struct Portal(#[field(id = 1)] pub u32);

/// When the character reaches the portal, and where the portal leads: a fixed destination,
/// as a level's cooked data would give it.
const PORTAL_TICK: u32 = 10;
const CHARACTER: u64 = 5;
const CRATE: u64 = 6;

fn exit() -> Transform {
    Transform::at(Vec3::new(f(3.0), f(0.41), f(0.0)))
}

#[system(name = "test::enter_portal", worlds = [server, prediction])]
fn enter_portal(now: Res<CurrentTick>, portal: Mut<Portal>) {
    if now.get().0 == PORTAL_TICK {
        portal.0 = 1;
    }
}

/// The character's own kernel records the teleport on itself.
#[system(name = "test::go_through", worlds = [server, prediction])]
fn go_through(key: StableKeyOf, portal: Mut<Portal>, physics: ChannelWriter<PhysicsCommands>) {
    if portal.0 != 0 {
        physics.teleport(*key, exit());
        portal.0 = 0;
    }
}

fn portal_types(types: &mut TypeTable) -> lactose_core::Result<()> {
    types.register::<Portal>().map(|_| ())
}

fn portal_systems(registration: &mut Registration) {
    registration.stage(Stage::Simulation).add(&GO_THROUGH);
    registration.stage(Stage::Post).add(&ENTER_PORTAL);
}

/// The character and the crate after `PORTAL_TICK + 3` ticks, and the crate before the
/// character moved: through a `Transform` written in `Stage::Post`, or through the
/// carrier.
fn through_the_portal(carried: bool) -> (Transform, Option<Teleported>, Transform, Transform) {
    let mut options = Options::new(WorldKind::Server);
    options.game = Some(Game { types: portal_types, systems: portal_systems });
    let mut h = options.build_with(rapier(), CallLog::default());
    let other = RegionConfig { id: OTHER_REGION, seed: 2, mode: RegionMode::Simulated };
    h.world_mut().create_region(other).unwrap();
    floor(&mut h);
    let at = |x: f64, y: f64| Transform::at(Vec3::new(f(x), f(y), f(0.0)));
    let cube = || Collider::new(Shape::cuboid(Vec3::new(f(0.4), f(0.4), f(0.4))));
    // The kinematic character, and a crate touching its +x face.
    let (pose, kinematic) = (at(-3.0, 0.41), RigidBody::kinematic());
    if carried {
        h.spawn(key(CHARACTER), (pose, Velocity::ZERO, kinematic, cube(), Portal(0)));
    } else {
        h.spawn(key(CHARACTER), (pose, Velocity::ZERO, kinematic, cube()));
        h.script().at(
            PORTAL_TICK,
            When::Post,
            key(CHARACTER),
            Action::SetTransform(key(CHARACTER), exit()),
        );
    }
    h.spawn(key(CRATE), (at(-2.2, 0.4), Velocity::ZERO, RigidBody::dynamic(), cube()));
    h = h.run(PORTAL_TICK);
    let crate_before = *h.world().get::<Transform>(key(CRATE)).unwrap();
    h = h.run(3);
    let world = h.world();
    (
        *world.get::<Transform>(key(CHARACTER)).unwrap(),
        world.get::<Teleported>(key(CHARACTER)).copied(),
        crate_before,
        *world.get::<Transform>(key(CRATE)).unwrap(),
    )
}

/// Review probe s2 (D126): a kinematic body given a `Transform` through `Commands::set`
/// in `Stage::Post` is swept to it through a crate touching its face, which is thrown;
/// a `Teleport` recorded in the next tick's `Stage::Simulation` by the body's own kernel,
/// from a carrier the post stage armed, places it, and the crate stays at rest.
#[test]
fn a_post_stage_transform_write_sweeps_where_a_carried_teleport_places() {
    let (character, teleported, before, after) = through_the_portal(false);
    assert_eq!(character.translation.x, exit().translation.x);
    assert_eq!(teleported, None, "a sweep is not a teleport");
    assert!(after.translation.x - before.translation.x > f(5.0), "thrown: {after:?}");

    let (character, teleported, before, after) = through_the_portal(true);
    assert_eq!(character.translation.x, exit().translation.x);
    assert_eq!(teleported, Some(Teleported(Tick(PORTAL_TICK + 1))));
    assert!((after.translation.x - before.translation.x).abs() < f(0.05), "moved: {after:?}");
}
