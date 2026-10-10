//! Queries and the schedule's shape (`README.md`, "Queries" and "The tick"): `cast_ray`
//! ties go to the lowest key; `overlap` appends in ascending key order, each key once; a
//! `Stage::Physics` reader of `Physics` needs an edge after `sets::STEP`; an unordered
//! `Stage::Post` reader of `Transform` builds and reads this tick's rows (probe S1); another
//! crate's exclusive `Stage::Post` system fails to build until `app` orders it (probe S2).

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::{Result, StableKey};
use lactose_ecs::{Mut, Res, SystemSet, World, WorldKind, system};
use lactose_math::Scalar;
use lactose_physics_api::{
    CollisionGroups, Physics, PhysicsConfig, QueryFilter, Ray, Shape, Transform, Velocity,
    register_systems, register_types, sets, setup,
};
use lactose_reflect::component;
use lactose_sim::{Registration, Simulation, SimulationBuilder, Stage, TickInput, TypeTable};

#[test]
fn a_ray_hits_the_nearest_admitted_collider_and_ties_go_to_the_lowest_key() {
    let mut h = Harness::new(WorldKind::Server);
    let (near, tied_high, tied_low, sensor, other_group) = (key(1), key(9), key(4), key(2), key(3));
    // Two colliders at exactly one place, and in front of them a sensor and a collider in
    // another group.
    h.spawn(tied_high, (at(0, 0, 10), sphere()));
    h.spawn(tied_low, (at(0, 0, 10), sphere()));
    h.spawn(sensor, (at(0, 0, 4), sphere().sensor()));
    h.spawn(other_group, (at(0, 0, 6), sphere().with_groups(CollisionGroups::new(2, u32::MAX))));
    h.spawn(near, (at(0, 0, 20), sphere()));
    h = h.run(1);
    let physics = h.physics();
    assert_eq!(physics.body_count(), 5);
    assert_eq!(physics.config(), PhysicsConfig::default());
    let ray = Ray::new(v(0, 0, 0), v(0, 0, 1), Scalar::from_i32(100));
    let only_one = QueryFilter { groups: CollisionGroups::new(1, 1), ..QueryFilter::ALL };
    let hit = physics.cast_ray(&ray, &only_one).unwrap();
    assert_eq!((hit.body, hit.distance), (tied_low, Scalar::from_f64(9.5)));
    // Excluding the lower of the tie gives the higher, at the same distance.
    let without = QueryFilter { exclude: Some(tied_low), ..only_one };
    assert_eq!(physics.cast_ray(&ray, &without).unwrap().body, tied_high);
    // The default filter is every group: the collider in group two is first; with sensors
    // admitted, the sensor before it.
    assert_eq!(physics.cast_ray(&ray, &QueryFilter::ALL).unwrap().body, other_group);
    let sensors = QueryFilter { sensors: true, ..QueryFilter::ALL };
    assert_eq!(physics.cast_ray(&ray, &sensors).unwrap().body, sensor);
    // A swept sphere meets the same colliders, inflated by its radius.
    let swept = physics
        .cast_shape(
            &Shape::sphere(Scalar::HALF),
            &Transform::IDENTITY,
            v(0, 0, 1),
            Scalar::from_i32(100),
            &only_one,
        )
        .unwrap();
    assert_eq!((swept.body, swept.distance), (tied_low, Scalar::from_i32(9)));
}

#[test]
fn overlap_appends_in_ascending_key_order_each_key_once() {
    for threads in [1, 4] {
        let mut options = Options::new(WorldKind::Server);
        options.threads = threads;
        let mut h = options.build();
        for n in [8, 3, 5, 1, 12] {
            h.spawn(key(n), (at(0, 0, 0), sphere()));
        }
        h.spawn(key(2), (at(50, 0, 0), sphere()));
        h = h.run(1);
        let sentinel = StableKey::Placeholder(0);
        let mut hits = vec![sentinel];
        h.physics().overlap(
            &Shape::sphere(Scalar::ONE),
            &Transform::IDENTITY,
            &QueryFilter::ALL,
            &mut hits,
        );
        assert_eq!(hits, [sentinel, key(1), key(3), key(5), key(8), key(12)], "{threads} threads");
    }
}

// ---- The schedule's shape ----

/// A stand-in for another crate's component, written by a `Stage::Post` reader.
#[component(key = "queries_test::seen", persistent)]
#[derive(PartialEq, Debug)]
pub struct Seen(#[field(id = 1)] pub Scalar);

/// A `Stage::Physics` set with no edge after the step.
pub static UNORDERED: SystemSet = SystemSet::new("queries_test::unordered");
/// A `Stage::Physics` set after the step.
pub static ORDERED: SystemSet = SystemSet::new("queries_test::ordered").after(&[&sets::STEP]);
/// Another crate's exclusive `Stage::Post` system.
pub static OTHER_EXCLUSIVE_SET: SystemSet = SystemSet::new("queries_test::other_exclusive_set");

#[system(name = "queries_test::unordered_query", worlds = [server, prediction], in_set = UNORDERED)]
fn unordered_query(physics: Res<Physics>) {
    let _ = physics.body_count();
}

#[system(name = "queries_test::ordered_query", worlds = [server, prediction], in_set = ORDERED)]
fn ordered_query(physics: Res<Physics>) {
    let _ = physics.body_count();
}

/// Probe S1: a per-entity reader of the stepped rows in `Stage::Post`, with no edge.
#[system(name = "queries_test::post_reader", worlds = [server, prediction])]
fn post_reader(pos: &Transform, seen: Mut<Seen>) {
    seen.0 = pos.translation.x;
}

/// Probe S2: another exclusive system in `Stage::Post`.
#[system(name = "queries_test::other_exclusive", worlds = [server, prediction], exclusive, in_set = OTHER_EXCLUSIVE_SET)]
fn other_exclusive(world: &mut World) {
    let _ = world.tick();
}

fn build(register: fn(&mut Registration)) -> Result<Simulation> {
    let mut types = TypeTable::new();
    register_types(&mut types)?;
    types.register::<Seen>()?;
    let backend = Box::new(Fake::new(CallLog::default()));
    SimulationBuilder::new(WorldKind::Server, types)
        .setup(move |world| setup(world, backend, PhysicsConfig::default()))
        .systems(register_systems)
        .systems(register)
        .build()
}

#[test]
fn a_physics_stage_reader_of_physics_needs_an_edge_after_the_step() {
    let error = build(|r| {
        r.stage(Stage::Physics).sets(&[&UNORDERED]).add(&UNORDERED_QUERY);
    })
    .err()
    .unwrap();
    assert!(error.to_string().contains("queries_test::unordered_query"), "{error}");
    build(|r| {
        r.stage(Stage::Physics).sets(&[&ORDERED]).add(&ORDERED_QUERY);
    })
    .unwrap();
}

#[test]
fn an_unordered_post_reader_of_transform_reads_this_ticks_rows() {
    let mut simulation = build(|r| {
        r.stage(Stage::Post).add(&POST_READER);
    })
    .unwrap();
    let world = simulation.world_mut();
    world
        .create_region(lactose_ecs::RegionConfig {
            id: REGION,
            seed: 1,
            mode: lactose_ecs::RegionMode::Simulated,
        })
        .unwrap();
    let sliding = Velocity { linear: v(6, 0, 0), angular: v(0, 0, 0) };
    world
        .spawn(key(1), REGION, (at(0, 0, 0), sliding, floating(), sphere(), Seen(Scalar::NEG_ONE)))
        .unwrap();
    let executor = executor(1);
    for _ in 0..3 {
        simulation = executor.block_on(simulation.step(&executor, TickInput::default())).unwrap();
        let world = simulation.world();
        // What the reader saw is the row the write-back wrote this tick.
        assert_eq!(
            world.get::<Seen>(key(1)).unwrap().0,
            world.get::<Transform>(key(1)).unwrap().translation.x
        );
        assert!(world.get::<Seen>(key(1)).unwrap().0 > Scalar::ZERO);
    }
}

#[test]
fn another_exclusive_post_system_fails_to_build_until_app_orders_it() {
    let error = build(|r| {
        r.stage(Stage::Post).sets(&[&OTHER_EXCLUSIVE_SET]).add(&OTHER_EXCLUSIVE);
    })
    .err()
    .unwrap();
    assert!(error.to_string().contains("queries_test::other_exclusive"), "{error}");
    build(|r| {
        r.stage(Stage::Post).sets(&[&OTHER_EXCLUSIVE_SET]).add(&OTHER_EXCLUSIVE);
        r.order(&sets::WRITE_BACK, &OTHER_EXCLUSIVE_SET);
    })
    .unwrap();
}
