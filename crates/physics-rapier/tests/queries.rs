//! Queries over the state the last step left: the nearest admitted hit, ties to the lowest
//! key, sensors only when asked for, groups tested both ways, the excluded key skipped;
//! `overlap` appended ascending, each key once; the same answers under every pool size and
//! creation order; nothing created since the last step is seen.

#![forbid(unsafe_code)]

mod support;

use lactose_core::StableKey;
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::{CollisionGroups, PhysicsBackend, QueryFilter, Ray, Shape, Transform};
use support::*;

const NEAR: u64 = 1;
const SENSOR: u64 = 2;
const OTHER_GROUP: u64 = 3;
const TIED_LOW: u64 = 4;
const TIED_HIGH: u64 = 9;

/// The scene of `physics-api`'s query test, created in `order`, on `threads` threads.
fn scene(threads: usize, order: &[u64]) -> Box<dyn PhysicsBackend> {
    let mut b = backend(threads);
    for &n in order {
        let (z, collider) = match n {
            NEAR => (20.0, ball()),
            SENSOR => (4.0, ball().sensor()),
            OTHER_GROUP => (6.0, ball().with_groups(CollisionGroups::new(2, u32::MAX))),
            _ => (10.0, ball()),
        };
        create(&mut *b, key(n), None, Some(collider), at(0.0, 0.0, z));
    }
    step(&mut *b, 1, &[]);
    b
}

fn answers(b: &dyn PhysicsBackend) -> Vec<Option<(StableKey, u64)>> {
    let ray = Ray::new(Vec3::ZERO, v(0.0, 0.0, 1.0), s(100.0));
    let only_one = QueryFilter { groups: CollisionGroups::new(1, 1), ..QueryFilter::ALL };
    let filters = [
        only_one,
        QueryFilter { exclude: Some(key(TIED_LOW)), ..only_one },
        QueryFilter::ALL,
        QueryFilter { sensors: true, ..QueryFilter::ALL },
    ];
    let mut out: Vec<_> = filters
        .iter()
        .map(|filter| {
            b.cast_ray(&ray, filter).map(|hit| (hit.body, hit.distance.to_canonical_bits()))
        })
        .collect();
    let swept = b.cast_shape(
        &Shape::sphere(Scalar::HALF),
        &Transform::IDENTITY,
        v(0.0, 0.0, 1.0),
        s(100.0),
        &only_one,
    );
    out.push(swept.map(|hit| (hit.body, hit.distance.to_canonical_bits())));
    out
}

#[test]
fn the_nearest_admitted_hit_wins_and_ties_go_to_the_lowest_key() {
    let b = scene(1, &[TIED_HIGH, TIED_LOW, SENSOR, OTHER_GROUP, NEAR]);
    let ray = Ray::new(Vec3::ZERO, v(0.0, 0.0, 1.0), s(100.0));
    let only_one = QueryFilter { groups: CollisionGroups::new(1, 1), ..QueryFilter::ALL };
    let hit = b.cast_ray(&ray, &only_one).unwrap();
    assert_eq!(hit.body, key(TIED_LOW));
    assert!((hit.distance - s(9.5)).abs() < s(1e-12), "{hit:?}");
    assert!((hit.point.z - s(9.5)).abs() < s(1e-12));
    assert!(hit.normal.z < s(-0.999), "out of the collider, back along the ray: {hit:?}");
    let without = QueryFilter { exclude: Some(key(TIED_LOW)), ..only_one };
    let other = b.cast_ray(&ray, &without).unwrap();
    assert_eq!((other.body, other.distance), (key(TIED_HIGH), hit.distance));
    assert_eq!(b.cast_ray(&ray, &QueryFilter::ALL).unwrap().body, key(OTHER_GROUP));
    let sensors = QueryFilter { sensors: true, ..QueryFilter::ALL };
    assert_eq!(b.cast_ray(&ray, &sensors).unwrap().body, key(SENSOR));
    // Out of reach.
    let short = Ray::new(Vec3::ZERO, v(0.0, 0.0, 1.0), s(3.0));
    assert_eq!(b.cast_ray(&short, &sensors), None);

    let swept = b
        .cast_shape(
            &Shape::sphere(Scalar::HALF),
            &Transform::IDENTITY,
            v(0.0, 0.0, 1.0),
            s(100.0),
            &only_one,
        )
        .unwrap();
    assert_eq!(swept.body, key(TIED_LOW));
    assert!((swept.distance - s(9.0)).abs() < s(1e-6), "{swept:?}");
    assert!((swept.point.z - s(9.5)).abs() < s(1e-6), "{swept:?}");
    assert!(swept.normal.z < s(-0.999), "{swept:?}");
}

#[test]
fn the_answers_do_not_depend_on_the_pool_or_the_creation_order() {
    let reference = answers(&*scene(1, &[NEAR, SENSOR, OTHER_GROUP, TIED_LOW, TIED_HIGH]));
    for (threads, order) in [
        (2, [TIED_HIGH, TIED_LOW, OTHER_GROUP, SENSOR, NEAR]),
        (4, [SENSOR, TIED_HIGH, NEAR, TIED_LOW, OTHER_GROUP]),
    ] {
        assert_eq!(answers(&*scene(threads, &order)), reference);
    }
}

#[test]
fn overlap_appends_in_ascending_key_order_each_key_once() {
    for threads in [1, 4] {
        let mut b = backend(threads);
        for n in [8, 3, 5, 1, 12] {
            create(&mut *b, key(n), None, Some(ball()), Transform::IDENTITY);
        }
        create(&mut *b, key(2), None, Some(ball()), at(50.0, 0.0, 0.0));
        create(&mut *b, key(7), None, Some(ball().sensor()), Transform::IDENTITY);
        // Not stepped yet: nothing is in the query's state.
        let mut hits = Vec::new();
        b.overlap(&Shape::sphere(Scalar::ONE), &Transform::IDENTITY, &QueryFilter::ALL, &mut hits);
        assert_eq!(hits, []);
        step(&mut *b, 1, &[]);
        let sentinel = StableKey::Placeholder(0);
        let mut hits = vec![sentinel];
        b.overlap(&Shape::sphere(Scalar::ONE), &Transform::IDENTITY, &QueryFilter::ALL, &mut hits);
        assert_eq!(hits, [sentinel, key(1), key(3), key(5), key(8), key(12)], "{threads} threads");
    }
}

#[test]
fn an_invalid_query_answers_nothing() {
    let b = scene(1, &[TIED_LOW]);
    let bad = Ray::new(Vec3::ZERO, Vec3::ZERO, s(100.0));
    assert_eq!(b.cast_ray(&bad, &QueryFilter::ALL), None);
    let flat = Shape::sphere(Scalar::ZERO);
    let mut hits = Vec::new();
    b.overlap(&flat, &Transform::IDENTITY, &QueryFilter::ALL, &mut hits);
    assert_eq!(hits, []);
    let nowhere = at(f64::NAN, 0.0, 0.0);
    let up = v(0.0, 0.0, 1.0);
    assert_eq!(
        b.cast_shape(&Shape::sphere(Scalar::ONE), &nowhere, up, s(9.0), &QueryFilter::ALL),
        None
    );
}
