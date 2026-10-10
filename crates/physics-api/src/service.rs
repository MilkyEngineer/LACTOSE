//! The [`Physics`] resource (architecture section 4): the backend behind one `Res` and
//! `ResMut` type, so the schedule orders every query against the step; what `app` inserts
//! through [`setup`]; and the physics snapshot the prediction world rolls back with
//! (architecture section 10), which carries this crate's bookkeeping and the sync's
//! pending work beside the backend's bytes.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use lactose_core::{NetId, Result, StableKey, Tick};
use lactose_ecs::{ChangeMark, LogCursor, LogEntry, RegionId, RegionMode, World};
use lactose_math::{Scalar, Vec3};
use lactose_sim::TypeTable;

use crate::backend::{PhysicsBackend, StepOutput};
use crate::command::PhysicsCommand;
use crate::components::{
    BodyHandle, Collider, Joint, JointHandle, RigidBody, Shape, Teleported, Transform, Velocity,
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
    /// `app` sizes the solver pool as the frame worker count minus this, at least one
    /// thread, so the two do not oversubscribe the performance cores and a one-worker
    /// executor (SOUR's workers mode) still gets a one-thread pool rather than rayon's
    /// "every CPU" default (architecture section 16).
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

/// What the sync knows about one row that is in the solver.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct BodyEntry {
    handle: BodyHandle,
    /// A fixed collider of a `Collider`-only row: outside the rolled-back state.
    fixed: bool,
}

/// This crate's bookkeeping and the sync's pending work: the one map between rows and the
/// solver, and what the next sync would see. A [`PhysicsSnapshot`] carries a copy beside
/// the backend's bytes and [`restore`] puts it back, so that a re-simulated tick's sync
/// makes exactly the backend calls the first run's did; the derived handle columns are
/// never consulted.
#[derive(Clone, Default)]
struct SyncState {
    /// The rows that are bodies or fixed colliders in the solver, by origin key.
    bodies: BTreeMap<StableKey, BodyEntry>,
    /// The `Net` key the server bound to each predicted body, so a command or filter may
    /// name the body by either key; refreshed at every physics sync point.
    aliases: BTreeMap<NetId, StableKey>,
    /// The joint rows in the solver, by origin key.
    joints: BTreeMap<StableKey, JointHandle>,
    /// Joint rows whose bodies are both named and not both in the solver, retried at
    /// every sync point. A joint with an emptied reference is in neither map until its
    /// row changes.
    waiting_joints: BTreeSet<StableKey>,
    /// The mode of every loaded region at the last sync, so a change is seen once.
    modes: BTreeMap<RegionId, RegionMode>,
    /// What the last write-back wrote, replaced by every write-back, so the sync tells its
    /// own write-back, which the change stamps also report, from a write the game made.
    written: BTreeMap<StableKey, (Transform, Velocity)>,
    /// Structural log entries the sync has not processed: those a snapshot found after the
    /// cursor, carried across a restore and processed before the log since the restore.
    pending_log: Vec<LogEntry>,
    /// Keys the next sync treats as changed whatever the stamps say: those a snapshot found
    /// changed since the mark across the five columns, and those [`corrected`] names.
    pending_keys: BTreeSet<StableKey>,
    /// Commands the next sync hands the step before what it takes from the channel: those
    /// a snapshot found in the channel, not yet taken.
    pending_commands: Vec<PhysicsCommand>,
    /// Whether the next sync diffs every row against `bodies` and `joints` instead of
    /// reading the log: set when the cursor or the mark is stale (a world restore without a
    /// physics restore, a whole-world load, `set_tick`, the stamp rebase), or when a
    /// snapshot was taken in that state.
    diff_all: bool,
}

/// The physics world of this simulation world: a service resource, inserted by [`setup`],
/// written by this crate's three systems and read by anything that queries.
///
/// One resource type for both, rather than a `dyn` query trait beside a `dyn` backend
/// trait, so the schedule sees every conflict: `Res<Physics>` in `Stage::Input` and
/// `Stage::Simulation` reads the previous step's state, in `Stage::Post` this tick's, and
/// in `Stage::Physics` it must be ordered after [`sets::STEP`](crate::sets::STEP) or the
/// schedule fails to build. Kernels on every worker read it at once, so it is `Sync` and
/// its queries take `&self`.
pub struct Physics {
    backend: Box<dyn PhysicsBackend>,
    config: PhysicsConfig,
    /// Ties a snapshot to the resource it was taken from; compared by address only.
    identity: Arc<()>,
    sync: SyncState,
    /// Where in the structural log the last sync stopped; `None` until the first sync and
    /// after a stale read.
    cursor: Option<LogCursor>,
    /// The point the last sync compared row changes against; `None` as for `cursor`.
    mark: Option<ChangeMark>,
    /// The commands the sync took this tick, in order, for the step; empty between ticks.
    commands: Vec<PhysicsCommand>,
    /// The step's lists, kept between ticks for their capacity; what the write-back writes.
    output: StepOutput,
}

impl Physics {
    pub(crate) fn new(backend: Box<dyn PhysicsBackend>, config: PhysicsConfig) -> Self {
        Self {
            backend,
            config,
            identity: Arc::new(()),
            sync: SyncState::default(),
            cursor: None,
            mark: None,
            commands: Vec::new(),
            output: StepOutput::default(),
        }
    }

    /// What `app` decided.
    #[must_use]
    pub fn config(&self) -> PhysicsConfig {
        self.config
    }

    /// Whether `key` names a row that is a body or fixed collider in the solver, by its
    /// origin key or, for a bound predicted entity, a `Net` key the sync has seen bound (one
    /// bound at this tick's input sync point resolves from the physics sync point on). True
    /// from the physics sync point after the row became one until the sync point after it
    /// stopped.
    #[must_use]
    pub fn is_body(&self, key: StableKey) -> bool {
        todo!()
    }

    /// How many rows are bodies or fixed colliders in the solver.
    #[must_use]
    pub fn body_count(&self) -> usize {
        self.sync.bodies.len()
    }

    /// The first hit along `ray` among the colliders `filter` admits, in the state the
    /// last step left: [`PhysicsBackend::cast_ray`], with `filter.exclude` resolved as
    /// [`is_body`](Self::is_body) resolves a key and the hit's key an origin key. Ties at
    /// equal distance go to the lowest key.
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
/// [`Teleported`], [`RigidBody`], [`Collider`], [`Joint`], [`BodyHandle`] and
/// [`JointHandle`]. `app` calls it beside every other crate's `register_types`.
pub fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<Transform>()?;
    types.register::<Velocity>()?;
    types.register::<Teleported>()?;
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
/// Two parts. The backend's bytes: the rolled-back solver state, every body that is not a
/// fixed collider of a `Collider`-only row and every joint, with the key-to-handle map and
/// without shapes, which [`restore`] has the backend rebuild from the `Collider` rows. And
/// this crate's bookkeeping with the sync's pending work: which rows are bodies and joints
/// and their handles, the waiting joints, the region modes, the aliases, what the last
/// write-back wrote, the structural log entries after the sync's cursor in order, the keys
/// changed since its mark across the five columns, and the channel's entries not yet taken
/// (`World::channel`), so that after [`restore`] a re-simulated tick's sync makes exactly
/// the backend calls the first run's did. The size is what milestone 1 measures against
/// the prediction-world budget, for the claimed bodies plus the statics.
pub struct PhysicsSnapshot {
    identity: Arc<()>,
    tick: Tick,
    bytes: Vec<u8>,
    sync: SyncState,
}

impl PhysicsSnapshot {
    /// The tick the snapshot was taken at: the world's tick at the time.
    #[must_use]
    pub fn tick(&self) -> Tick {
        self.tick
    }

    /// How many bytes the backend wrote; the carried bookkeeping is not counted.
    #[must_use]
    pub fn size_in_bytes(&self) -> usize {
        self.bytes.len()
    }

    /// `xxh64` of the backend's bytes with seed zero: the place where architecture section
    /// 8.3 adds the physics world to the world hash once the backend's serialisation is
    /// shown to be order-stable (`docs/verification.md`). Until then SOUR records nothing
    /// of it and two equal physics worlds may hash apart.
    #[must_use]
    pub fn hash(&self) -> u64 {
        todo!()
    }
}

/// Copies the physics world of `world`'s [`Physics`] resource, between ticks, at the point
/// `World::snapshot` is taken: the backend's bytes, and this crate's bookkeeping with what
/// the next sync would see (the log since its cursor, the keys changed since its mark, the
/// channel's untaken entries; or that a full diff is due, when the cursor or mark is
/// stale). Fails with `NotFound` if the world holds no [`Physics`], or as the backend's
/// [`snapshot`](PhysicsBackend::snapshot) does.
pub fn snapshot(world: &World) -> Result<PhysicsSnapshot> {
    todo!()
}

/// Replaces the physics world of `world`'s [`Physics`] resource with `snapshot`, after
/// `World::restore` of the matching world snapshot: the world's tick must equal
/// [`PhysicsSnapshot::tick`] and the snapshot must come from this resource. In order: the
/// backend's [`restore`](PhysicsBackend::restore) of the bytes; then the fixed colliders
/// of `Collider`-only rows, which the bytes do not carry, are made to match the carried
/// bookkeeping (one the backend holds and the snapshot does not is removed, one the
/// snapshot holds and the backend does not is created from the restored rows, its handle
/// refreshed); then every carried body gets its shape back through
/// [`set_collider`](PhysicsBackend::set_collider) from its `Collider` row; then the
/// carried bookkeeping and pending work replace this crate's, and a fresh cursor and mark
/// are taken from the restored world. The next sync processes the pending log entries
/// before the log since the restore, the union of the pending keys and the newly changed
/// keys in key order, and the pending commands before what it takes.
///
/// Fails with `NotFound` if the world holds no [`Physics`], with `InvalidData` if the
/// snapshot is from another resource or the world's tick differs (restore the world
/// first), or as the backend's restore does; on failure nothing changes and the next sync
/// diffs every row against the bookkeeping it has. A world restored without a physics
/// restore gets that diff too: consistent where rows agree, the solver's state kept for
/// every body whose rows did not change, and a re-simulation from it converges rather than
/// matches.
pub fn restore(world: &mut World, snapshot: &PhysicsSnapshot) -> Result<()> {
    todo!()
}

/// Tells the sync that `lactose-predict` corrected the rows of `keys` without recording the
/// writes (`ColumnMut::write` without marking, so that `Changed<T>` does not see a
/// correction): each key joins the pending set, and the next sync reads its `Transform`,
/// `Velocity`, `RigidBody`, `Collider` and `Joint` rows as changed and pushes what differs
/// from the solver. Called after [`restore`], before the next step, for every body row
/// corrected; a key that is not a body is ignored. Fails with `NotFound` if the world holds
/// no [`Physics`].
pub fn corrected(world: &mut World, keys: &[StableKey]) -> Result<()> {
    todo!()
}
