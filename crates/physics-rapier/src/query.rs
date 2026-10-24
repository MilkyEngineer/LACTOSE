//! The queries, over the state the last step left: Rapier's broad-phase tree finds the
//! candidates, the exact test decides, and the answer is the least (distance, key) or the
//! ascending keys, so neither the tree's shape nor its iteration order reaches it.

use lactose_core::StableKey;
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::{QueryFilter, Ray, RayHit, Shape, ShapeHit, Transform};
use rapier3d_f64::geometry::{BoundingVolume, Collider, ColliderHandle};
use rapier3d_f64::math::Vector;
use rapier3d_f64::parry::query;
use rapier3d_f64::parry::query::ShapeCastOptions;
use rapier3d_f64::pipeline::{QueryFilter as SolverFilter, QueryFilterFlags};

use crate::backend::{Rapier, groups};
use crate::convert::{key_code, key_from_code, pose, shared_shape, vec3, vector};

/// The keyed filter as the solver's: sensors out unless asked for, the groups tested as two
/// colliders are, and the excluded key's collider out by its `user_data`.
fn with_filter<R>(filter: &QueryFilter, run: impl FnOnce(SolverFilter<'_>) -> R) -> R {
    let excluded = filter.exclude.map(key_code);
    let predicate =
        move |_: ColliderHandle, collider: &Collider| Some(collider.user_data) != excluded;
    let flags =
        if filter.sensors { QueryFilterFlags::empty() } else { QueryFilterFlags::EXCLUDE_SENSORS };
    run(SolverFilter {
        flags,
        groups: Some(groups(filter.groups)),
        exclude_collider: None,
        exclude_rigid_body: None,
        predicate: Some(&predicate),
    })
}

/// Whether `(distance, key)` comes before the best so far: nearer, or as near with a lower key.
fn better<T>(best: Option<&(Scalar, StableKey, T)>, distance: Scalar, key: StableKey) -> bool {
    best.is_none_or(|&(d, k, _)| distance < d || (distance == d && key < k))
}

/// A unit normal, or `fallback` when the solver gives none (a cast that starts inside).
fn unit_or(normal: Vector, fallback: Vec3) -> Vec3 {
    vec3(normal).try_normalize().unwrap_or(fallback)
}

pub(crate) fn cast_ray(backend: &Rapier, ray: &Ray, filter: &QueryFilter) -> Option<RayHit> {
    let usable = ray.origin.is_finite()
        && ray.direction.is_finite()
        && ray.direction.try_normalize().is_some()
        && ray.max_distance.is_finite()
        && ray.max_distance >= Scalar::ZERO;
    if !usable {
        return None;
    }
    let solver_ray = rapier3d_f64::geometry::Ray::new(vector(ray.origin), vector(ray.direction));
    let max = ray.max_distance.to_f64();
    let mut best: Option<(Scalar, StableKey, Vector)> = None;
    with_filter(filter, |solver_filter| {
        for (_, collider, hit) in backend.world.intersect_ray(solver_ray, max, true, solver_filter)
        {
            let Some(key) = key_from_code(collider.user_data) else { continue };
            let distance = Scalar::from_f64(hit.time_of_impact);
            if better(best.as_ref(), distance, key) {
                best = Some((distance, key, hit.normal));
            }
        }
    });
    best.map(|(distance, body, normal)| RayHit {
        body,
        distance,
        point: ray.origin + ray.direction * distance,
        normal: unit_or(normal, -ray.direction),
    })
}

pub(crate) fn cast_shape(
    backend: &Rapier,
    shape: &Shape,
    at: &Transform,
    direction: Vec3,
    max_distance: Scalar,
    filter: &QueryFilter,
) -> Option<ShapeHit> {
    let usable = shape.validate().is_ok()
        && at.validate().is_ok()
        && direction.is_finite()
        && direction.try_normalize().is_some()
        && max_distance.is_finite()
        && max_distance >= Scalar::ZERO;
    if !usable {
        return None;
    }
    let cast = shared_shape(shape);
    let start = pose(at);
    let velocity = vector(direction);
    let max = max_distance.to_f64();
    let from = cast.compute_aabb(&start);
    let swept = from.merged(&from.translated(velocity * max));
    let options = ShapeCastOptions {
        max_time_of_impact: max,
        target_distance: 0.0,
        stop_at_penetration: true,
        compute_impact_geometry_on_penetration: true,
    };
    let mut best: Option<(Scalar, StableKey, (Vector, Vector))> = None;
    with_filter(filter, |solver_filter| {
        for (_, collider) in backend.world.intersect_aabb_conservative(swept, solver_filter) {
            let Some(key) = key_from_code(collider.user_data) else { continue };
            let hit = query::cast_shapes(
                &start,
                velocity,
                &*cast,
                collider.position(),
                Vector::ZERO,
                collider.shape(),
                options,
            );
            let Ok(Some(hit)) = hit else { continue };
            let distance = Scalar::from_f64(hit.time_of_impact);
            if better(best.as_ref(), distance, key) {
                let place = collider.position();
                let world = (place.transform_point(hit.witness2), place.rotation * hit.normal2);
                best = Some((distance, key, world));
            }
        }
    });
    best.map(|(distance, body, (point, normal))| ShapeHit {
        body,
        distance,
        point: vec3(point),
        normal: unit_or(normal, -direction),
    })
}

pub(crate) fn overlap(
    backend: &Rapier,
    shape: &Shape,
    at: &Transform,
    filter: &QueryFilter,
    hits: &mut Vec<StableKey>,
) {
    if shape.validate().is_err() || at.validate().is_err() {
        return;
    }
    let probe = shared_shape(shape);
    let mut found: Vec<StableKey> = with_filter(filter, |solver_filter| {
        backend
            .world
            .intersect_shape(pose(at), &*probe, solver_filter)
            .filter_map(|(_, collider)| key_from_code(collider.user_data))
            .collect()
    });
    found.sort_unstable();
    found.dedup();
    hits.extend(found);
}
