//! The [`Physics`] resource (architecture section 4): the backend behind one `Res` and
//! `ResMut` type, so the schedule orders every query against the step; what `app` inserts
//! through [`setup`]; and the physics snapshot the prediction world rolls back with
//! (architecture section 10).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use lactose_core::{NetId, Result, StableKey, Tick};
use lactose_ecs::{ChangeMark, LogCursor, RegionId, RegionMode, World};
use lactose_math::{Scalar, Vec3};
use lactose_sim::TypeTable;

use crate::backend::{PhysicsBackend, StepOutput};
use crate::components::{
    BodyHandle, Collider, Joint, JointHandle, RigidBody, Shape, Transform, Velocity,
};
use crate::query::{QueryFilter, Ray, RayHit, ShapeHit};

/// What `app` decides for the physics world, handed to [`setup`].
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PhysicsConfig {
    /// Gravity in units per second squared, in world space; each body scales it by its
    /// [`RigidBody::gravity_scale`].
    pub gravity: Vec3,
    /// How many of the executor's frame-critical workers go on taking jobs while the step
    /// blocks on the solver pool: the reservation the step passes to `block_in_place`.
    /// `app` sizes the solver pool as the frame worker count minus this, so the two do not
    /// oversubscribe the performance cores (architecture section 16).
    pub reserved_workers: u32,
}

impl PhysicsConfig {
    /// Nine point eight one units per second squared downwards along y.
    pub const EARTH_GRAVITY: Vec3 = Vec3::new(Scalar::ZERO, Scalar::from_f64(-9.81), Scalar::ZERO);
}

impl Default for PhysicsConfig {
    /// Earth gravity and one reserved worker: what a test uses. `app` decides both.
    fn default() -> Self {
        Self { gravity: Self::EARTH_GRAVITY, reserved_workers: 1 }
    }
}

/// The physics world of this simulation world: a service resource, inserted by [`setup`],
/// written by this crate's two systems and read by anything that queries.
///
/// One resource type for both, rather than a `dyn` query trait beside a `dyn` backend
/// trait, so the schedule sees every conflict: `Res<Physics>` in `Stage::Simulation` reads
/// the previous step's state, in `Stage::Post` this tick's, and in `Stage::Physics` it must
/// be ordered against [`sets::STEP`](crate::sets::STEP) or the schedule fails to build.
/// Kernels on every worker read it at once, so it is `Sync` and its queries take `&self`.
pub struct Physics {
    backend: Box<dyn PhysicsBackend>,
    config: PhysicsConfig,
    /// Ties a snapshot to the resource it was taken from; compared by address only.
    identity: Arc<()>,
    /// The rows that are bodies in the solver, by origin key, with the handle the backend
    /// gave each: the one map between rows and the backend, which the derived
    /// [`BodyHandle`] column mirrors.
    bodies: BTreeMap<StableKey, BodyHandle>,
    /// The `Net` key the server bound to each predicted body, so a command or filter may
    /// name the body by either key; refreshed at every physics sync point.
    aliases: BTreeMap<NetId, StableKey>,
    /// The joint rows in the solver, by origin key.
    joints: BTreeMap<StableKey, JointHandle>,
    /// Joint rows whose bodies are not both in the solver yet.
    waiting_joints: BTreeSet<StableKey>,
    /// The mode of every loaded region at the last sync, so a change is seen once.
    modes: BTreeMap<RegionId, RegionMode>,
    /// Where in the structural log the last sync stopped; `None` means rebuild everything
    /// from rows at the next sync.
    cursor: Option<LogCursor>,
    /// The point the last sync compared row changes against; `None` as for `cursor`.
    mark: Option<ChangeMark>,
    /// What the last step wrote back, so the sync tells its own write-back, which the
    /// change stamps also report, from a write the game made.
    written: BTreeMap<StableKey, (Transform, Velocity)>,
    /// The step's lists, kept between ticks for their capacity.
    output: StepOutput,
}

impl Physics {
    pub(crate) fn new(backend: Box<dyn PhysicsBackend>, config: PhysicsConfig) -> Self {
        Self {
            backend,
            config,
            identity: Arc::new(()),
            bodies: BTreeMap::new(),
            aliases: BTreeMap::new(),
            joints: BTreeMap::new(),
            waiting_joints: BTreeSet::new(),
            modes: BTreeMap::new(),
            cursor: None,
            mark: None,
            written: BTreeMap::new(),
            output: StepOutput::default(),
        }
    }

    /// What `app` decided.
    #[must_use]
    pub fn config(&self) -> PhysicsConfig {
        self.config
    }

    /// Whether `key` names a row that is a body or fixed collider in the solver, by its
    /// origin key or, for a bound predicted entity, its `Net` key. True from the physics
    /// sync point after the row became one until the sync point after it stopped.
    #[must_use]
    pub fn is_body(&self, key: StableKey) -> bool {
        todo!()
    }

    /// How many rows are bodies or fixed colliders in the solver.
    #[must_use]
    pub fn body_count(&self) -> usize {
        self.bodies.len()
    }

    /// The first hit along `ray` among the colliders `filter` admits, in the state the
    /// last step left: [`PhysicsBackend::cast_ray`], with the key resolved to the origin
    /// key. Ties at equal distance go to the lowest key.
    #[must_use]
    pub fn cast_ray(&self, ray: &Ray, filter: &QueryFilter) -> Option<RayHit> {
        todo!()
    }

    /// The first hit of `shape` at `pose` swept along the unit vector `direction` for
    /// `max_distance`: [`PhysicsBackend::cast_shape`], likewise.
    #[must_use]
    pub fn cast_shape(
        &self,
        shape: &Shape,
        pose: &Transform,
        direction: Vec3,
        max_distance: Scalar,
        filter: &QueryFilter,
    ) -> Option<ShapeHit> {
        todo!()
    }

    /// Appends to `hits` every entity `filter` admits whose collider overlaps `shape` at
    /// `pose`, in ascending origin-key order, each once: [`PhysicsBackend::overlap`].
    #[allow(
        clippy::ptr_arg,
        reason = "facade: the body appends, which a slice cannot take; the implementation \
                  lane removes this with the `todo!()`"
    )]
    pub fn overlap(
        &self,
        shape: &Shape,
        pose: &Transform,
        filter: &QueryFilter,
        hits: &mut Vec<StableKey>,
    ) {
        todo!()
    }
}

/// Registers this crate's components with `types`: [`Transform`], [`Velocity`],
/// [`RigidBody`], [`Collider`], [`Joint`], [`BodyHandle`] and [`JointHandle`]. `app` calls
/// it beside every other crate's `register_types`.
pub fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<Transform>()?;
    types.register::<Velocity>()?;
    types.register::<RigidBody>()?;
    types.register::<Collider>()?;
    types.register::<Joint>()?;
    types.register::<BodyHandle>()?;
    types.register::<JointHandle>()?;
    Ok(())
}

/// Inserts the [`Physics`] resource holding `backend`, which `app` built with the solver
/// pool outside the boundary, into `world`: the setup function of architecture section
/// 16, called from a `SimulationBuilder::setup` closure beside
/// [`register_systems`](crate::register_systems) in `SimulationBuilder::systems`. Fails
/// with `InvalidData` if the world already holds one.
pub fn setup(
    world: &mut World,
    backend: Box<dyn PhysicsBackend>,
    config: PhysicsConfig,
) -> Result<()> {
    todo!()
}

/// The physics world at one tick, taken beside a `WorldSnapshot` of the same world at the
/// same point and restored beside it (architecture section 10).
///
/// The bytes are the backend's: every body, collider and joint with its solver state and
/// the key-to-handle map. Nothing of this crate's own bookkeeping is in them, because all
/// of it is rebuilt from rows at a [`restore`]: which rows are bodies and their handles
/// come from the restored [`BodyHandle`] and [`JointHandle`] columns, the waiting joints
/// from the [`Joint`] rows without a handle, the region modes from the world. The size is
/// what milestone 1 measures against the prediction-world budget.
pub struct PhysicsSnapshot {
    identity: Arc<()>,
    tick: Tick,
    bytes: Vec<u8>,
}

impl PhysicsSnapshot {
    /// The tick the snapshot was taken at: the world's tick at the time.
    #[must_use]
    pub fn tick(&self) -> Tick {
        self.tick
    }

    /// How many bytes the backend wrote.
    #[must_use]
    pub fn size_in_bytes(&self) -> usize {
        self.bytes.len()
    }

    /// `xxh64` of the bytes with seed zero: the place where architecture section 8.3 adds
    /// the physics world to the world hash once the backend's serialisation is shown to be
    /// order-stable (`docs/verification.md`). Until then SOUR records nothing of it and two
    /// equal physics worlds may hash apart.
    #[must_use]
    pub fn hash(&self) -> u64 {
        todo!()
    }
}

/// Copies the physics world of `world`'s [`Physics`] resource, between ticks, at the point
/// `World::snapshot` is taken. Fails with `NotFound` if the world holds no [`Physics`], or
/// as the backend's [`snapshot`](PhysicsBackend::snapshot) does.
pub fn snapshot(world: &World) -> Result<PhysicsSnapshot> {
    todo!()
}

/// Replaces the physics world of `world`'s [`Physics`] resource with `snapshot`, after
/// `World::restore` of the matching world snapshot: the world's tick must equal
/// [`PhysicsSnapshot::tick`], the snapshot must come from this resource, and the rows must
/// be the ones the physics snapshot was taken beside, which the restored [`BodyHandle`]
/// and [`JointHandle`] columns are. Then this crate's bookkeeping is rebuilt from those
/// rows and a fresh log cursor and mark are taken, so the next sync is incremental.
///
/// Fails with `NotFound` if the world holds no [`Physics`], with `InvalidData` if the
/// snapshot is from another resource or the world's tick differs (restore the world
/// first), or as the backend's [`restore`](PhysicsBackend::restore) does; on failure the
/// physics world is unchanged and the next sync rebuilds it from rows. A world restored
/// without a physics restore gets the same rebuild: consistent, but the solver's state is
/// fresh and a re-simulation from it converges rather than matches.
pub fn restore(world: &mut World, snapshot: &PhysicsSnapshot) -> Result<()> {
    todo!()
}
