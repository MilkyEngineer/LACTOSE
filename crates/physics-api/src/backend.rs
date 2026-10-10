//! The trait a backend implements (architecture section 16): bodies, colliders and joints
//! keyed by `StableKey`, the step, snapshot and restore as bytes, and the queries. The
//! backend never sees the world: the sync tells it what rows became, and the step hands it
//! the tick's commands and takes back what moved and what touched.

use lactose_core::{Result, StableKey, Tick};
use lactose_math::{Scalar, Vec3};

use crate::command::PhysicsCommand;
use crate::components::{
    BodyHandle, Collider, Joint, JointHandle, RigidBody, Shape, Transform, Velocity,
};
use crate::event::{ContactEvent, TriggerEvent};
use crate::query::{QueryFilter, Ray, RayHit, ShapeHit};

/// What the sync gives the backend when a row becomes a body: the rows as they are at
/// that physics sync point. At least one of `body` and `collider` is `Some`.
#[derive(Clone, Copy, Debug)]
pub struct BodyDesc<'a> {
    /// The effective body, or `None` for a fixed collider. The row's `RigidBody` as the
    /// backend is to treat it: in a `Prediction` world a row without `ClaimedBy` is a proxy
    /// (architecture section 9.4), and the sync hands it with `body_type = Kinematic`
    /// whatever the row says, here and in every later `set_body`; the backend holds no
    /// world-kind rule.
    pub body: Option<RigidBody>,
    /// The collider, or `None` for a body with no collider.
    pub collider: Option<&'a Collider>,
    /// Where it is.
    pub transform: &'a Transform,
    /// How it moves, or `None` for at rest.
    pub velocity: Option<&'a Velocity>,
    /// Whether its region is not `Simulated`, which makes it a fixed collider until
    /// [`PhysicsBackend::set_frozen`] says otherwise (architecture section 9.7).
    pub frozen: bool,
}

/// What one step is given.
#[derive(Clone, Copy, Debug)]
pub struct StepInput<'a> {
    /// The tick being simulated.
    pub tick: Tick,
    /// The fixed delta in seconds, `TickDelta`.
    pub dt: Scalar,
    /// The world's gravity, from [`PhysicsConfig`](crate::PhysicsConfig).
    pub gravity: Vec3,
    /// Every command the sync took since the previous step, in command order, keys
    /// resolved: a plain slice, so a backend's own test steps it without a world.
    pub commands: &'a [PhysicsCommand],
}

/// One body the step moved.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BodyState {
    /// The body.
    pub key: StableKey,
    /// Where it is after the step.
    pub transform: Transform,
    /// How it moves after the step.
    pub velocity: Velocity,
    /// Whether the body was placed without a sweep during this step (a `Teleport` command,
    /// or a `set_transform` on a dynamic or fixed body before it): the write-back writes
    /// the tick into [`Teleported`](crate::Teleported), the no-interpolate flag of
    /// architecture section 5.3.
    pub teleported: bool,
}

/// What one step reports: filled by [`PhysicsBackend::step`] into vectors the step system
/// keeps and clears between ticks, so a steady state allocates nothing.
#[derive(Default, Debug)]
pub struct StepOutput {
    /// Every body whose transform or velocity the step changed or that it placed: every
    /// awake dynamic body, every kinematic body that was moved, every body teleported, and
    /// nothing sleeping or fixed. Any order, each key at most once; the write-back sorts by
    /// key before it writes.
    pub bodies: Vec<BodyState>,
    /// Every contact that started or stopped, for pairs with events enabled on either
    /// side, and every impact for a target with events enabled on its own side, once per
    /// (`target`, `other`, `phase`) at most. Any order; the step system sorts before it
    /// emits.
    pub contacts: Vec<ContactEvent>,
    /// Every sensor overlap that started or stopped, for pairs with events enabled, once
    /// per (`target`, `other`, `phase`) at most. Any order.
    pub triggers: Vec<TriggerEvent>,
}

impl StepOutput {
    /// Empties every list, keeping their capacity.
    pub fn clear(&mut self) {
        self.bodies.clear();
        self.contacts.clear();
        self.triggers.clear();
    }
}

/// A physics backend: what `lactose-physics-rapier` implements and `app` hands to
/// [`setup`](crate::setup) with the solver pool built outside the boundary.
///
/// # What the backend is told, and when
///
/// Every call is made by this crate from the systems it registers, in one fixed order per
/// tick (`README.md`, "The tick"): at the physics sync point the sync applies region mode
/// changes, the structural log and the rows that changed, body by body in log order and
/// then key order, and takes the commands; then the step applies the commands and steps;
/// at the sync point that begins `Stage::Post` the write-back writes what moved. Keys are
/// origin `StableKey`s; the backend keeps its own key-to-handle map, which its snapshot
/// bytes include, and the handles it returns are what the sync writes into the derived
/// [`BodyHandle`] and [`JointHandle`] rows.
///
/// # Validation
///
/// The backend validates every value it is given (finite scalars, unit quaternions,
/// positive extents and radii, a mass that is zero or positive, a joint's `limit_min` no
/// greater than its `limit_max` when it is limited, a non-zero axis on a revolute or
/// prismatic joint; a fixed or spherical joint's axis is unused and may be zero). An
/// invalid value is
/// logged at target `physics` and the call does nothing; `create_body` and `create_joint`
/// return `InvalidData`, and the sync retries the row when it changes. A backend therefore
/// never aborts the process on a row's bytes.
///
/// # Equal values
///
/// Every `set_*` method compares with what the body holds and does nothing when equal:
/// a full diff after a stale cursor pushes every row, and an equal value must not wake a
/// body, bump the solver's active-set epoch or rebuild its graph (Rapier's `get_mut`
/// does; compare first). A differing value wakes the body.
///
/// # Determinism (architecture sections 1, 2 and 8.2)
///
/// The backend is inside the determinism boundary. For one sequence of calls with equal
/// arguments it produces equal [`StepOutput`]s (as sets: the order of each list is free),
/// equal query answers and equal [`snapshot`](Self::snapshot) bytes under any thread count
/// of its solver pool and whatever ran before on those threads, and identical on x86-64
/// and aarch64: the obligation is section 2's. The verification items name the two known
/// gaps of the pinned Rapier (platform-libm `sin_cos` on limited joints, the sign of zero
/// in SIMD min/max), which the backend lane closes before SOUR's architecture mode gates;
/// until then the property is unproven, not excused. It reads no clock, no thread or
/// worker identity and no thread-local state; its pool's threads enter
/// `lactose_core::ThreadClass::PhysicsSolver` at start, which sets their floating-point
/// state; solver threads number the frame worker count minus the reservation, at least
/// one. Every number it computes goes through its pinned math crates, and what reaches a
/// row or an event is converted to [`Scalar`] exactly. A method called for a key it does
/// not hold does nothing and logs at target `physics`; it never panics on a key.
pub trait PhysicsBackend: Send + Sync + 'static {
    /// Creates the body or fixed collider for `key` from `desc`. The sync calls it once per
    /// row that becomes a body, never for a key it already holds.
    fn create_body(&mut self, key: StableKey, desc: &BodyDesc<'_>) -> Result<BodyHandle>;

    /// Removes the body of `key`, and with it every joint attached to it; the sync removes
    /// those joints' handles and keeps the joints waiting.
    fn remove_body(&mut self, key: StableKey);

    /// Replaces the body's effective [`RigidBody`]: `Some` with the new value (a type
    /// change included: a ragdoll going limp, or a prediction-world row claimed or
    /// released, which the sync hands as kinematic while unclaimed and as the row's type
    /// while claimed), `None` when the row lost the component and the entity is a fixed
    /// collider now. The handle stays.
    fn set_body(&mut self, key: StableKey, body: Option<&RigidBody>);

    /// Replaces the body's [`Collider`] likewise; `None` when the row lost the component.
    fn set_collider(&mut self, key: StableKey, collider: Option<&Collider>);

    /// A [`Transform`] row write: places a dynamic or fixed body there with no sweep (the
    /// next [`BodyState`] of the body reports `teleported`), and sets where a kinematic
    /// body sweeps to during the next step.
    fn set_transform(&mut self, key: StableKey, transform: &Transform);

    /// A [`Velocity`] row write. Ignored for a kinematic body, whose velocity the sweep
    /// gives.
    fn set_velocity(&mut self, key: StableKey, velocity: &Velocity);

    /// Makes the body a fixed collider at its current pose, or makes it its declared type
    /// again, keeping its velocity (the sync restores the row's velocity afterwards).
    fn set_frozen(&mut self, key: StableKey, frozen: bool);

    /// Creates the joint `key` between the bodies `body_a` and `body_b`, both of which the
    /// backend holds, from the row's [`Joint`].
    fn create_joint(
        &mut self,
        key: StableKey,
        joint: &Joint,
        body_a: StableKey,
        body_b: StableKey,
    ) -> Result<JointHandle>;

    /// Removes the joint `key`.
    fn remove_joint(&mut self, key: StableKey);

    /// Applies `input.commands` in order, then advances the world by `input.dt`, and
    /// reports what moved and what touched into `output`, which arrives cleared. A command
    /// for a key it does not hold, or an impulse or force on a kinematic, fixed or frozen
    /// body, is dropped and logged at target `physics`.
    ///
    /// Called inside the step system's `block_in_place`, the one sanctioned blocking wait
    /// (architecture section 16): the solver pool may be used freely here, and the
    /// executor's workers beyond the reservation are parked meanwhile.
    fn step(&mut self, input: &StepInput<'_>, output: &mut StepOutput);

    /// Appends the physics world to `out`. The bytes are the backend's whole world, with
    /// each collider's [`Shape`] value (an asset reference once meshes exist) in place of
    /// the backend's shape serialisation: every body, fixed collider and joint with its
    /// solver state, and the key-to-handle map, so that [`restore`](Self::restore) of the
    /// bytes rebuilds a world that steps on identically. The bytes are a function of the
    /// state; whether they are stable enough to hash across equal states reached by
    /// different operation orders is a milestone 1 verification item, not a contract.
    fn snapshot(&self, out: &mut Vec<u8>) -> Result<()>;

    /// Replaces the whole physics world with one taken by [`snapshot`](Self::snapshot) of
    /// this backend type, rebuilding each collider's shape from the carried [`Shape`]
    /// value while deserialising, with no change flag, wake or mass update: nothing of
    /// this crate replays `set_collider` afterwards, and nothing reconciles fixed
    /// colliders. `InvalidData` leaves the world unchanged.
    fn restore(&mut self, bytes: &[u8]) -> Result<()>;

    /// The first hit along `ray` among the colliders `filter` admits, with the lowest
    /// `StableKey` among hits at exactly equal distance.
    fn cast_ray(&self, ray: &Ray, filter: &QueryFilter) -> Option<RayHit>;

    /// The first hit of `shape`, placed at `pose` and swept along the unit vector
    /// `direction` for `max_distance`, among the colliders `filter` admits; ties as
    /// [`cast_ray`](Self::cast_ray).
    fn cast_shape(
        &self,
        shape: &Shape,
        pose: &Transform,
        direction: Vec3,
        max_distance: Scalar,
        filter: &QueryFilter,
    ) -> Option<ShapeHit>;

    /// Appends to `hits` the key of every collider `filter` admits that `shape` at `pose`
    /// overlaps, in ascending key order, each key once.
    fn overlap(
        &self,
        shape: &Shape,
        pose: &Transform,
        filter: &QueryFilter,
        hits: &mut Vec<StableKey>,
    );
}
