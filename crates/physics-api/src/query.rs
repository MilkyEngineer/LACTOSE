//! Queries over the stepped state (architecture section 4): what `Res<Physics>` answers in
//! any stage, keyed by `StableKey` and measured in `Scalar`s.

use lactose_core::StableKey;
use lactose_math::{Scalar, Vec3};

use crate::components::CollisionGroups;

/// A ray: `origin + direction * t` for `t` in `[0, max_distance]`. `direction` is a unit
/// vector, so distances are in world units; a backend may assert it in debug builds.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Ray {
    /// Where the ray starts.
    pub origin: Vec3,
    /// Which way it goes, unit length.
    pub direction: Vec3,
    /// How far it reaches.
    pub max_distance: Scalar,
}

impl Ray {
    /// A ray from `origin` along the unit vector `direction` for `max_distance`.
    #[must_use]
    pub const fn new(origin: Vec3, direction: Vec3, max_distance: Scalar) -> Self {
        Self { origin, direction, max_distance }
    }
}

/// What a query considers. The default considers every solid collider of every group and
/// excludes nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct QueryFilter {
    /// The groups the query belongs to and touches, tested against each collider's
    /// [`CollisionGroups`] as two colliders are tested against each other.
    pub groups: CollisionGroups,
    /// An entity whose colliders are skipped: usually the caster's own.
    pub exclude: Option<StableKey>,
    /// Whether sensors are hit.
    pub sensors: bool,
}

impl QueryFilter {
    /// Every solid collider, every group, nothing excluded.
    pub const ALL: Self = Self { groups: CollisionGroups::ALL, exclude: None, sensors: false };

    /// [`ALL`](Self::ALL) with `entity` excluded.
    #[must_use]
    pub const fn excluding(entity: StableKey) -> Self {
        Self { exclude: Some(entity), ..Self::ALL }
    }
}

impl Default for QueryFilter {
    fn default() -> Self {
        Self::ALL
    }
}

/// What a ray hit first.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RayHit {
    /// The entity whose collider was hit.
    pub body: StableKey,
    /// How far along the ray, in world units.
    pub distance: Scalar,
    /// Where, in world space.
    pub point: Vec3,
    /// The surface normal there, unit length, pointing out of the collider.
    pub normal: Vec3,
}

/// What a swept shape hit first.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ShapeHit {
    /// The entity whose collider was hit.
    pub body: StableKey,
    /// How far the shape travelled before touching, in world units.
    pub distance: Scalar,
    /// The point of first contact on the hit collider, in world space.
    pub point: Vec3,
    /// The surface normal there, unit length, pointing out of the hit collider.
    pub normal: Vec3,
}
