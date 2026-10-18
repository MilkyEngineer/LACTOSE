//! The [`Physics`] resource (architecture section 4): the backend behind one `Res` and
//! `ResMut` type, so the schedule orders every query against the step; what `app` inserts
//! through [`setup`]; and the physics snapshot the prediction world rolls back with
//! (architecture section 10), which carries this crate's bookkeeping and the sync's
//! pending work beside the backend's bytes.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use lactose_core::{Error, ErrorKind, NetId, Result, StableKey, Tick, xxh64};
use lactose_ecs::{ChangeMark, LogCursor, LogEntry, RegionId, RegionMode, World};
use lactose_math::{Scalar, Vec3};
use lactose_sim::TypeTable;

use crate::backend::{PhysicsBackend, StepOutput};
use crate::command::{PhysicsCommand, PhysicsCommands};
use crate::components::{
    BodyHandle, Collider, Joint, JointHandle, RigidBody, Shape, Teleported, Transform, Velocity,
};
use crate::query::{QueryFilter, Ray, RayHit, ShapeHit};
use crate::sync::{Changes, changed_since, log_since_pass};

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

/// A joint in the solver: its handle, the row's value it was created from, and the two
/// bodies it joins, so that the removal of either body finds it and a change of the value
/// is told from a rewrite of the same one.
#[derive(Clone, Copy)]
pub(crate) struct JointEntry {
    pub(crate) handle: JointHandle,
    pub(crate) joint: Joint,
    pub(crate) body_a: StableKey,
    pub(crate) body_b: StableKey,
}

/// This crate's bookkeeping and the sync's pending work: the one map between rows and the
/// solver, and what the next sync would see. A [`PhysicsSnapshot`] carries a copy beside
/// the backend's bytes and [`restore`] puts it back, so that a re-simulated tick's sync
/// makes exactly the backend calls the first run's did; the derived handle columns are
/// never consulted.
#[derive(Clone, Default)]
pub(crate) struct SyncState {
    /// The rows that are bodies or fixed colliders in the solver, by origin key, with the
    /// handle the backend gave each.
    pub(crate) bodies: BTreeMap<StableKey, BodyHandle>,
    /// The bodies the backend holds frozen: those whose region was not `Simulated` when
    /// the sync last looked at them.
    pub(crate) frozen: BTreeSet<StableKey>,
    /// The `Net` key the server bound to each predicted body, so a command or filter may
    /// name the body by either key. A bind is permanent, so an entry stays until its body
    /// is removed.
    pub(crate) aliases: BTreeMap<NetId, StableKey>,
    /// The predicted bodies with no bind yet: the only ones each physics sync point looks
    /// up, so the sync's cost does not grow with the predicted bodies already bound.
    pub(crate) unbound: BTreeSet<StableKey>,
    /// The joint rows in the solver, by origin key.
    pub(crate) joints: BTreeMap<StableKey, JointEntry>,
    /// Joint rows whose bodies are both named and not both in the solver, retried at
    /// every sync point. A joint with an emptied reference is in neither map until its
    /// row changes.
    pub(crate) waiting_joints: BTreeSet<StableKey>,
    /// The mode of every loaded region at the last sync, so a change is seen once.
    pub(crate) modes: BTreeMap<RegionId, RegionMode>,
    /// What the last write-back wrote, ascending by key, replaced by every write-back, so
    /// the sync tells its own write-back, which the change stamps also report, from a
    /// write the game made.
    pub(crate) written: Vec<(StableKey, Transform, Velocity)>,
    /// Structural log entries the sync has not processed: those a snapshot found after the
    /// cursor, carried across a restore and processed before the log since the restore.
    /// When a full diff is due they are read only by its reload test (D124).
    pub(crate) pending_log: Vec<LogEntry>,
    /// Keys the next sync treats as changed whatever the stamps say, with the columns:
    /// those a snapshot found changed since the mark across the five columns, and those
    /// [`corrected`] names, all five.
    pub(crate) pending_keys: BTreeMap<StableKey, Changes>,
    /// Commands the next sync hands the step before what it takes from the channel: those
    /// a snapshot found in the channel, not yet taken.
    pub(crate) pending_commands: Vec<PhysicsCommand>,
    /// Whether the next sync diffs every row against `bodies` and `joints` instead of
    /// reading the log: set by a sync that emptied the solver, so the next one creates in
    /// key order as a fresh process's first sync does, and in a snapshot taken while the
    /// cursor or the mark was stale (a world restore without a physics restore, a
    /// whole-world load, `set_tick`, the stamp rebase). A stale cursor or mark at the sync
    /// itself has the same effect.
    pub(crate) diff_all: bool,
}

impl SyncState {
    /// What the last write-back wrote for `key`.
    pub(crate) fn written(&self, key: StableKey) -> Option<(Transform, Velocity)> {
        let index = self.written.binary_search_by_key(&key, |entry| entry.0).ok()?;
        let (_, transform, velocity) = self.written[index];
        Some((transform, velocity))
    }
}

/// The physics world of this simulation world: a service resource, inserted by [`setup`],
/// written by this crate's three systems and read by anything that queries.
///
/// One resource type for both, rather than a `dyn` query trait beside a `dyn` backend
/// trait, so the schedule sees every conflict: `Res<Physics>` in `Stage::Input` and
/// `Stage::Simulation` reads the previous step's state, in `Stage::Post` this tick's, and
/// in `Stage::Physics` it must be ordered after [`sets::STEP`](crate::sets::STEP) or the
/// schedule fails to build. Kernels on every worker read it at once, so it is `Sync` and
/// its queries take `&self`. A world that runs a system with a
/// `ChannelWriter<PhysicsCommands>` and not the sync fails to build its schedule
/// (`ScheduleError::ChannelWithoutConsumer`), and one that runs the sync and holds no
/// `Physics` fails `run_tick` before anything runs: `app` and tests insert one or record
/// nothing.
pub struct Physics {
    pub(crate) backend: Box<dyn PhysicsBackend>,
    pub(crate) config: PhysicsConfig,
    /// Ties a snapshot to the resource it was taken from; compared by address only.
    identity: Arc<()>,
    pub(crate) sync: SyncState,
    /// Where in the structural log the last sync stopped; `None` until the first sync.
    pub(crate) cursor: Option<LogCursor>,
    /// The point the last sync compared row changes against; `None` as for `cursor`.
    pub(crate) mark: Option<ChangeMark>,
    /// The tick the last sync ran in: the tick the step simulates.
    pub(crate) tick: Tick,
    /// The commands the sync took this tick, in order, for the step; empty between ticks.
    pub(crate) commands: Vec<PhysicsCommand>,
    /// The step's lists, kept between ticks for their capacity; what the write-back writes.
    pub(crate) output: StepOutput,
    /// The backend's bytes at [`setup`], before it held anything: what a solver the sync
    /// empties is restored to, so that an empty solver is a fresh one (D124).
    pub(crate) fresh: Vec<u8>,
}

impl Physics {
    pub(crate) fn new(backend: Box<dyn PhysicsBackend>, config: PhysicsConfig) -> Result<Self> {
        let mut fresh = Vec::new();
        backend.snapshot(&mut fresh)?;
        Ok(Self {
            backend,
            config,
            identity: Arc::new(()),
            sync: SyncState::default(),
            cursor: None,
            mark: None,
            tick: Tick(0),
            commands: Vec::new(),
            output: StepOutput::default(),
            fresh,
        })
    }

    /// What `app` decided.
    #[must_use]
    pub fn config(&self) -> PhysicsConfig {
        self.config
    }

    /// Whether `key` names a row that is a body or fixed collider in the solver, by its
    /// origin key or, for a bound predicted entity, a `Net` key the sync has seen bound (one
    /// bound at this tick's input sync point resolves from the physics sync point on; after a
    /// rollback, one bound after the restore point resolves from the first re-simulated
    /// physics sync point on, since the world keeps binds across a restore). True from the
    /// physics sync point after the row became one until the sync point after it stopped.
    #[must_use]
    pub fn is_body(&self, key: StableKey) -> bool {
        self.sync.bodies.contains_key(&self.resolve(key))
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
        self.backend.cast_ray(ray, &self.resolve_filter(filter))
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
        self.backend.cast_shape(shape, pose, direction, max_distance, &self.resolve_filter(filter))
    }

    /// Appends to `hits` every entity `filter` admits whose collider overlaps `shape` at
    /// `pose`, in ascending origin-key order, each once: [`PhysicsBackend::overlap`].
    pub fn overlap(
        &self,
        shape: &Shape,
        pose: &Transform,
        filter: &QueryFilter,
        hits: &mut Vec<StableKey>,
    ) {
        let start = hits.len();
        self.backend.overlap(shape, pose, &self.resolve_filter(filter), hits);
        // The backend promises the order; what it appended is put in it regardless, so an
        // answer never depends on the backend's iteration order.
        let appended = &mut hits[start..];
        if !appended.is_sorted_by(|a, b| a < b) {
            appended.sort_unstable();
            let mut tail = hits.split_off(start);
            tail.dedup();
            hits.append(&mut tail);
        }
    }

    /// The origin key of `key`: itself, or the predicted body a `Net` key is bound to.
    fn resolve(&self, key: StableKey) -> StableKey {
        match key {
            StableKey::Net(net) => self.sync.aliases.get(&net).copied().unwrap_or(key),
            _ => key,
        }
    }

    fn resolve_filter(&self, filter: &QueryFilter) -> QueryFilter {
        QueryFilter { exclude: filter.exclude.map(|key| self.resolve(key)), ..*filter }
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
/// [`register_systems`](crate::register_systems) in `SimulationBuilder::systems`. It
/// keeps the backend's [`snapshot`](PhysicsBackend::snapshot) of its empty world, which
/// the sync restores whenever it empties the solver (`README.md`, "Bodies"). Fails with
/// `InvalidData` if the world already holds one, or as that snapshot does.
pub fn setup(
    world: &mut World,
    backend: Box<dyn PhysicsBackend>,
    config: PhysicsConfig,
) -> Result<()> {
    if world.service::<Physics>().is_some() {
        return Err(Error::new(ErrorKind::InvalidData, "the world already holds a `Physics`"));
    }
    world.insert_service::<Physics>(Box::new(Physics::new(backend, config)?));
    Ok(())
}

/// The physics world at one tick, taken beside a `WorldSnapshot` of the same world at the
/// same point and restored beside it (architecture section 10).
///
/// Two parts. The backend's bytes: its whole world, every body, fixed collider and joint
/// with its solver state and the key-to-handle map, with each collider's [`Shape`] value
/// in place of the backend's shape serialisation. And this crate's bookkeeping with the
/// sync's pending work: which rows are bodies and joints and their handles, the waiting
/// joints, the region modes, the aliases and the unbound predicted bodies, what the last
/// write-back wrote, the structural log entries after the sync's cursor in order, the keys
/// changed since its mark across the five columns, and the channel's entries not yet taken
/// (`World::channel`), so that after [`restore`] a re-simulated tick's sync makes exactly
/// the backend calls the first run's did, apart from the calls [`corrected`] adds. The size
/// is what milestone 1 measures against the prediction-world budget, for the claimed bodies
/// plus the statics, which decides how many statics a prediction world may hold.
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
        xxh64(&self.bytes, 0)
    }
}

/// Copies the physics world of `world`'s [`Physics`] resource, between ticks, at the point
/// `World::snapshot` is taken: the backend's bytes, and this crate's bookkeeping with what
/// the next sync would see (the log since its cursor, the keys changed since its mark, the
/// channel's untaken entries; or that a full diff is due, with the log the world retains
/// since the sync's last pass for its reload test, when the cursor or mark is stale).
/// Fails with `NotFound` if the world holds no [`Physics`], or as the backend's
/// [`snapshot`](PhysicsBackend::snapshot) does.
pub fn snapshot(world: &World) -> Result<PhysicsSnapshot> {
    let physics = physics(world)?;
    let mut bytes = Vec::new();
    physics.backend.snapshot(&mut bytes)?;
    let mut sync = physics.sync.clone();
    let current = match (physics.cursor, physics.mark) {
        (Some(mut cursor), Some(mark)) if world.is_current(mark) => {
            world.structural_log().since(&mut cursor).map(|entries| (entries, mark))
        }
        _ => None,
    };
    match current {
        Some((entries, mark)) if !sync.diff_all => {
            sync.pending_log.extend(entries.copied());
            changed_since(world, mark, &mut sync.pending_keys);
        }
        _ => {
            // The next sync diffs every row, which covers whatever was pending; the log
            // since its last pass is kept for its reload test alone, so a rollback to this
            // point decides as the first run did.
            sync.diff_all = true;
            sync.pending_keys.clear();
            if !sync.bodies.is_empty()
                && let Some(entries) = log_since_pass(world, physics.cursor)
            {
                sync.pending_log.extend(entries.copied());
            }
        }
    }
    sync.pending_commands
        .extend(world.channel::<PhysicsCommands>().iter().map(|entry| entry.entry));
    Ok(PhysicsSnapshot { identity: Arc::clone(&physics.identity), tick: world.tick(), bytes, sync })
}

/// Replaces the physics world of `world`'s [`Physics`] resource with `snapshot`, after
/// `World::restore` of the matching world snapshot: the world's tick must equal
/// [`PhysicsSnapshot::tick`] and the snapshot must come from this resource. The backend's
/// [`restore`](PhysicsBackend::restore) of the bytes rebuilds its whole world, shapes
/// included, with no change flag, wake or mass update; nothing is replayed through
/// `set_collider` and no fixed collider is reconciled. Then the carried bookkeeping and
/// pending work replace this crate's, and a fresh cursor and mark are taken from the
/// restored world. The next sync processes the pending log entries before the log since
/// the restore, the union of the pending keys and the newly changed keys in key order, and
/// the pending commands before what it takes.
///
/// Fails with `NotFound` if the world holds no [`Physics`], with `InvalidData` if the
/// snapshot is from another resource or the world's tick differs (restore the world
/// first), or as the backend's restore does; on failure nothing changes and the next sync
/// diffs every row against the bookkeeping it has. A world restored without a physics
/// restore gets that diff too: consistent where rows agree, the solver's state kept for
/// every body whose rows did not change, and a re-simulation from it converges rather than
/// matches.
pub fn restore(world: &mut World, snapshot: &PhysicsSnapshot) -> Result<()> {
    let tick = world.tick();
    let cursor = world.structural_log().cursor();
    let mark = world.mark();
    let physics = world
        .service_mut::<Physics>()
        .ok_or_else(|| Error::new(ErrorKind::NotFound, "the world holds no `Physics`"))?;
    if !Arc::ptr_eq(&physics.identity, &snapshot.identity) {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "the physics snapshot was taken from another `Physics`",
        ));
    }
    if tick != snapshot.tick {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "the world is at tick {} and the physics snapshot at tick {}: restore the \
                 world first",
                tick.0, snapshot.tick.0
            ),
        ));
    }
    physics.backend.restore(&snapshot.bytes)?;
    physics.sync = snapshot.sync.clone();
    physics.cursor = Some(cursor);
    physics.mark = Some(mark);
    physics.commands.clear();
    physics.output.clear();
    Ok(())
}

/// Tells the sync that `lactose-predict` corrected the rows of `keys`: each key joins the
/// pending set, and the next sync reads its `Transform`, `Velocity`, `RigidBody`,
/// `Collider` and `Joint` rows as changed and pushes what differs from the solver. The
/// correction is written stamped (D122, which supersedes D119's unrecorded write for that
/// crate), so a `Changed<T>` reader sees it once; this call remains the contract whether
/// or not the writer stamps, since the pending set, not the ECS's marks, is what the
/// backend is promised. Called after [`restore`], before the next step, for every body row
/// corrected; a key that is not a body is ignored. Fails with `NotFound` if the world holds
/// no [`Physics`].
pub fn corrected(world: &mut World, keys: &[StableKey]) -> Result<()> {
    let origins: Vec<StableKey> = keys.iter().filter_map(|&key| world.origin(key)).collect();
    let physics = world
        .service_mut::<Physics>()
        .ok_or_else(|| Error::new(ErrorKind::NotFound, "the world holds no `Physics`"))?;
    for key in origins {
        if physics.sync.bodies.contains_key(&key) {
            *physics.sync.pending_keys.entry(key).or_default() |= Changes::ALL;
        }
    }
    Ok(())
}

fn physics(world: &World) -> Result<&Physics> {
    world
        .service::<Physics>()
        .ok_or_else(|| Error::new(ErrorKind::NotFound, "the world holds no `Physics`"))
}

/// The bookkeeping itself, which no public surface shows: what the sync looks up at each
/// physics sync point for the aliases of predicted bodies.
#[cfg(test)]
mod tests {
    use lactose_core::{PlayerId, PredictionKey};
    use lactose_ecs::{Executor, RegionConfig, WorldKind};
    use lactose_sim::{Simulation, SimulationBuilder, TickInput};

    use super::*;
    use crate::backend::{BodyDesc, StepInput};
    use crate::systems::register_systems;

    /// A backend that holds nothing: only the bookkeeping is under test.
    struct Null(u32);

    impl PhysicsBackend for Null {
        fn create_body(&mut self, _: StableKey, _: &BodyDesc<'_>) -> Result<BodyHandle> {
            self.0 += 1;
            Ok(BodyHandle::new(self.0, 1))
        }
        fn remove_body(&mut self, _: StableKey) {}
        fn set_body(&mut self, _: StableKey, _: Option<&RigidBody>) {}
        fn set_collider(&mut self, _: StableKey, _: Option<&Collider>) {}
        fn set_transform(&mut self, _: StableKey, _: &Transform) {}
        fn set_velocity(&mut self, _: StableKey, _: &Velocity) {}
        fn set_frozen(&mut self, _: StableKey, _: bool) {}
        fn create_joint(
            &mut self,
            _: StableKey,
            _: &Joint,
            _: StableKey,
            _: StableKey,
        ) -> Result<JointHandle> {
            Ok(JointHandle::new(0, 1))
        }
        fn remove_joint(&mut self, _: StableKey) {}
        fn step(&mut self, _: &StepInput<'_>, _: &mut StepOutput) {}
        fn snapshot(&self, _: &mut Vec<u8>) -> Result<()> {
            Ok(())
        }
        fn restore(&mut self, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn cast_ray(&self, _: &Ray, _: &QueryFilter) -> Option<RayHit> {
            None
        }
        fn cast_shape(
            &self,
            _: &Shape,
            _: &Transform,
            _: Vec3,
            _: Scalar,
            _: &QueryFilter,
        ) -> Option<ShapeHit> {
            None
        }
        fn overlap(&self, _: &Shape, _: &Transform, _: &QueryFilter, _: &mut Vec<StableKey>) {}
    }

    const REGION: RegionId = RegionId(1);

    fn predicted(ordinal: u32) -> PredictionKey {
        PredictionKey { player: PlayerId(1), input_tick: Tick(1), ordinal }
    }

    fn net(ordinal: u32) -> NetId {
        NetId::from_bits(1_000 + u64::from(ordinal)).unwrap()
    }

    fn spawn(simulation: &mut Simulation, ordinal: u32) {
        let body =
            (Transform::IDENTITY, RigidBody::dynamic(), Collider::new(Shape::sphere(Scalar::ONE)));
        let key = StableKey::Predicted(predicted(ordinal));
        simulation.world_mut().spawn(key, REGION, body).unwrap();
    }

    fn bookkeeping(simulation: &Simulation) -> (usize, usize) {
        let physics = simulation.world().service::<Physics>().unwrap();
        (physics.sync.unbound.len(), physics.sync.aliases.len())
    }

    #[test]
    fn a_bound_predicted_body_is_never_looked_up_again() {
        let mut types = TypeTable::new();
        register_types(&mut types).unwrap();
        let mut simulation = SimulationBuilder::new(WorldKind::Server, types)
            .setup(|world| setup(world, Box::new(Null(0)), PhysicsConfig::default()))
            .systems(register_systems)
            .build()
            .unwrap();
        let region = RegionConfig { id: REGION, seed: 1, mode: RegionMode::Simulated };
        simulation.world_mut().create_region(region).unwrap();
        let executor = Executor::builder().frame_workers(1).background_workers(1).build().unwrap();
        let step = |simulation: Simulation| {
            executor.block_on(simulation.step(&executor, TickInput::default())).unwrap()
        };
        // Forty predicted bodies, half of them bound before the first sync.
        for ordinal in 0..40 {
            spawn(&mut simulation, ordinal);
            if ordinal % 2 == 0 {
                simulation.world_mut().bind(predicted(ordinal), net(ordinal)).unwrap();
            }
        }
        simulation = step(simulation);
        assert_eq!(bookkeeping(&simulation), (20, 20), "only the unbound are looked up again");
        // The rest bound between ticks: found at the next sync, and then looked up no more.
        for ordinal in (1..40).step_by(2) {
            simulation.world_mut().bind(predicted(ordinal), net(ordinal)).unwrap();
        }
        simulation = step(simulation);
        assert_eq!(bookkeeping(&simulation), (0, 40));
        // A snapshot carries the set, and a restore puts it back.
        let world_snapshot = simulation.world().snapshot();
        let taken = snapshot(simulation.world()).unwrap();
        assert!(taken.sync.unbound.is_empty() && taken.sync.aliases.len() == 40);
        // A removed body takes its alias with it; a new one is unbound until it is bound.
        simulation.world_mut().despawn(StableKey::Predicted(predicted(0))).unwrap();
        spawn(&mut simulation, 40);
        simulation = step(simulation);
        assert_eq!(bookkeeping(&simulation), (1, 39));
        let physics = simulation.world().service::<Physics>().unwrap();
        assert!(
            !physics.is_body(StableKey::Net(net(0))) && physics.is_body(StableKey::Net(net(1)))
        );
        simulation.world_mut().restore(&world_snapshot).unwrap();
        restore(simulation.world_mut(), &taken).unwrap();
        assert_eq!(bookkeeping(&simulation), (0, 40));
    }
}
